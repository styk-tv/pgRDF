# 02 — Storage

Three sub-components, each with a clear contract. Schema lives in
[`sql/schema_v0_2_0.sql`](../sql/schema_v0_2_0.sql) and is loaded
via `extension_sql_file!` in `src/lib.rs`.

## 2.1 Shared dictionary (`_pgrdf_dictionary`)

Maps RDF terms (URI / BlankNode / Literal) to 64-bit integers
with a `UNIQUE (term_type, lexical_value, datatype_iri_id,
language_tag)` constraint. A HASH index on `lexical_value`
accelerates exact-match lookups during ingestion; the BIGINT PK
covers id→term lookups.

### Per-call HashMap dict cache (shipped, Phase 2.2 step 3)

`src/storage/loader.rs::ingest_turtle_with_stats` carries a
`HashMap<(term_type, lexical, datatype, lang), i64>` scoped to a
single `pgrdf.load_turtle` / `parse_turtle` call. On a cache hit
the loader skips the SPI dictionary lookup entirely; on miss it
calls `dict::put_term_full` (which itself does an upsert via
`IS NOT DISTINCT FROM` to keep NULL datatype/lang participating
in dedup) and caches the returned id.

`pgrdf.load_turtle_verbose` / `parse_turtle_verbose` expose stats
(`dict_cache_hits`, `dict_db_calls`, `quad_batches`, `elapsed_ms`)
so callers can verify the cache is firing — exercised by
`fixtures/regression/synth-100.ttl` (115 distinct terms, 185
expected cache hits across 100 triples).

### Shmem dictionary cache (LLD §4.1, **shipped — Phase 3 step 1**)

Cross-backend, process-wide cache lives in
[`src/storage/shmem_cache.rs`](../src/storage/shmem_cache.rs). The
hot path:

```
hash(RdfTerm) ─► shmem cache ─hit──► return id (no SQL)
                      │
                      └─miss──► Spi.query SELECT id FROM _pgrdf_dictionary
                                     │
                                     └─ stage_for_commit(key, id)
                                            │
                                            └─ on XACT COMMIT: publish to shmem
```

Implementation notes:

- **Layout**: `PgLwLock<[Slot; 16 384]>` (~ 512 KiB shmem). Each
  slot carries a u128 fingerprint (two SipHash variants with
  different seeds), a generation counter, the dict id, and an
  occupied marker. Open-addressed with depth-8 linear probing;
  canonical-slot eviction on full streak.
- **Hot path latency**: shared-mode LWLock + ≤ 8 slot probes ≈ ~120
  ns on commodity hardware — well under the LLD § 4.1 < 1 µs
  acceptance target. Atomics on HITS/MISSES are `Relaxed`.
- **Counters**: `HITS / MISSES / INSERTS / EVICTIONS` as
  `PgAtomic<AtomicU64>`. Exposed via `pgrdf.stats() → JSONB`
  along with `shmem_ready` (false ⇒ extension was lazy-loaded;
  cache is no-op) and `shmem_slots` (table capacity).
- **Transaction safety**: every `put_term_full` SELECT-hit or
  INSERT goes through `stage_for_commit`. pgrx's
  `register_xact_callback(Commit | Abort, …)` publishes the
  staged list on commit, drops it on abort. A rolled-back INSERT
  never leaves an orphan id in the cache. Per-call HashMap from
  Phase 2.2 still serves as the L1 — shmem is the L2.
- **Generation invalidation**: shmem outlives
  `DROP EXTENSION pgrdf`, but the dict id sequence resets. A
  `GENERATION: PgAtomic<AtomicU64>` (init 1) is bumped by
  `pgrdf.shmem_reset()` / `shmem_cache::reset()`; every slot
  records its writer generation and lookup discards stale
  entries with one extra equality test. Operators run
  `SELECT pgrdf.shmem_reset();` after a drop-create cycle; the
  regression suite does so in `50-shmem-dict-cache.sql`.
- **Init**: `_PG_init` only registers the shmem hooks when
  `process_shared_preload_libraries_in_progress == true`. The
  compose `command:` and the pgrx-test `postgresql_conf_options`
  both set `shared_preload_libraries=pgrdf`; deployments that
  miss this get a working extension with the shmem cache
  silently disabled (lookups short-circuit, no incorrect
  behaviour).
- **Perf regression**: `tests/regression/sql/50-shmem-dict-cache.sql`
  drives three back-to-back loads of `fixtures/regression/synth-100.ttl`
  and asserts load 1 = 115 db calls / 0 shmem hits, loads 2–3 = 0
  db calls / 115 shmem hits. All values hand-computed.

## 2.2 Hexastore (`_pgrdf_quads`)

Partitioned table holding the quads (subject_id, predicate_id,
object_id, graph_id, is_inferred). Partition strategy: **LIST on
`graph_id`**, plus a default partition for any graph_id not
explicitly created.

Three covering indexes (`INCLUDE (is_inferred)`):
- `_pgrdf_idx_spo (subject_id, predicate_id, object_id)`
- `_pgrdf_idx_pos (predicate_id, object_id, subject_id)`
- `_pgrdf_idx_osp (object_id, subject_id, predicate_id)`

SOP / PSO / OPS join the index set in v0.3 once we measure the
trade-off against ingestion write amplification.

`pgrdf.add_graph(g)` creates the LIST partition for graph `g`
idempotently. `pgrdf.count_quads(g)` is a partition-pruning count.
Dropping a whole graph is `DROP TABLE _pgrdf_quads_<g>` —
seconds, not rows-times-vacuum.

### Named-graph IRI mapping (`_pgrdf_graphs`, **shipped — Phase A slice 120**)

The integer `graph_id` LIST key is a storage detail; SPARQL
users name graphs by **IRI**. The v0.4 schema extension
[`sql/schema_v0_4_0_graphs.sql`](../sql/schema_v0_4_0_graphs.sql)
adds a second system table that closes the IRI ↔ graph_id binding:

```sql
CREATE TABLE _pgrdf_graphs (
    graph_id BIGINT PRIMARY KEY,
    iri      TEXT NOT NULL UNIQUE
);
```

The default partition (`graph_id = 0`) is seeded with the synthetic
IRI `urn:pgrdf:graph:0`, so the catch-all bucket has a queryable
name from `CREATE EXTENSION` onwards.

Slice 120 lands the table; **slice 119** wires the existing
integer-keyed `pgrdf.add_graph(id BIGINT)` UDF to populate
`_pgrdf_graphs` automatically — every successful partition
creation also inserts `(id, 'urn:pgrdf:graph:' || id::text)`
under an `ON CONFLICT (graph_id) DO NOTHING` clause, so v0.3
callers gain a queryable IRI mapping for every graph they create
through the integer surface without any signature change. The
IRI-keyed UDF surface (`pgrdf.add_graph(iri)`,
`pgrdf.graph_id(iri)`, `pgrdf.graph_iri(id)`, plus a dual-arg
`pgrdf.add_graph(id, iri)` overload) lands in subsequent Phase A
slices; SPARQL `GRAPH { … }` translation lands later in Phase A.
Spec: SPEC.pgRDF.LLD.v0.4 §3.

## 2.3 Bulk loader (`src/storage/loader.rs`)

### Prepared batched INSERT (LLD §4.3 phase A, **shipped — Phase 3 step 3**)

`flush_batch` now prepares the static `INSERT … SELECT FROM
unnest(…)` exactly once per backend (via the shared `plan_cache`)
and reuses the `OwnedPreparedStatement` for every flush in every
load call. The SQL string is keyed verbatim — same cache as the
SPARQL plans — and observability rides on the same
`plan_cache_hits / misses / inserts` counters in `pgrdf.stats()`.

Per-backend savings: ~100–500 µs per batch (one parse+plan
avoided). Verified by `tests/regression/sql/52-bulk-ingest-perf.sql`
against the `synth-10k.ttl` fixture (10 000 triples = 10 batches
per load): load 1 generates 1 miss + 9 hits; loads 2–3 generate
0 misses + 10 hits each.

### Batched INSERT via unnest (shipped, Phase 2.2 step 3)

The current ingest path uses **batched multi-row INSERTs** —
not yet COPY BINARY:

```sql
INSERT INTO _pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
SELECT s, p, o, $4
  FROM unnest($1::bigint[], $2::bigint[], $3::bigint[]) AS t(s, p, o)
```

BATCH_SIZE = 1000 triples per round-trip. Combined with the
per-call HashMap dict cache, this brings SPI calls from ~7 per
triple to roughly `distinct_terms + ceil(triples/1000)`.

### COPY BINARY / heap_multi_insert (LLD §4.3 phase B, deferred to v0.4)

LLD §4.3 calls for `COPY _pgrdf_quads FROM STDIN (FORMAT BINARY)`
with a **2× over batched-INSERT** wall-clock target. Phase 3
step 3 (above) cashes the parse+plan savings but the per-batch
wall clock on synth-10k stays at ~85 ms steady-state on both the
batched-INSERT and prepared-INSERT shapes — the executor walk
(per-tuple projection + partition routing) dominates and is what
the next slice has to skip.

Two candidate paths, both FFI-heavy:

1. `pg_sys::heap_multi_insert` — writes N tuples to the heap in one
   call. Skips the executor's per-tuple wrapper and uses the
   bulk-insert AM path. Requires hand-building `TupleTableSlot`s
   and routing across partitions (or pre-resolving the partition
   relation per `graph_id`).
2. `pg_sys::BeginCopyFrom` + a callback-driven binary feed —
   higher-level than `heap_multi_insert`, handles partitions, but
   needs more glue (binary header bytes, the per-tuple binary
   tuple layout).

Tracked as **Phase 3 step 3b** in the v0.3 LLD; lands in v0.4
unless promoted forward.

### Graph routing

The loader writes directly with `graph_id = <caller-supplied>`,
so the partition router places tuples in the correct partition
without a round-trip through the default partition. If the caller
hasn't called `pgrdf.add_graph(g)` first, tuples land in the
default partition; subsequent `add_graph(g)` calls don't move
them — the partition-creation order is the caller's
responsibility.

## 2.4 What's NOT in storage

- **No vacuum tuning yet.** Standard autovacuum suffices for v0.2;
  tuning lives in [`docs/10-roadmap.md`](10-roadmap.md) Phase 4.
- **No TOAST tuning.** Literals are stored inline; long literals
  (≥ 2 KB) compress under default TOAST policy.
- **No PostgreSQL custom scan hooks** at v0.3.0 — Phase 2.x
  performance follow-on per LLD §4.2.
- **No foreign keys** from `_pgrdf_quads` to `_pgrdf_dictionary`.
  Intentional — the loader enforces referential integrity by
  resolving ids before INSERT, and FKs would slow the hot path.
