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

### Named-graph IRI mapping (`_pgrdf_graphs`, **shipped — Phase A slices 120 → 115**)

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

The same migration carries a
`SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graphs', '');`
registration so `pg_dump` includes the row data (not just the
extension-managed DDL). Without this call, the seed row and every
user-bound IRI would be silently dropped on restore. Round-trip
discipline is locked end-to-end by
[`tests/regression/scripts/pg-dump-roundtrip.sh`](../tests/regression/scripts/pg-dump-roundtrip.sh)
(slice 110), wired into `just test-pg-dump-roundtrip` and
`just test-conformance`.

**Slice 119 — synthetic IRI for integer-keyed `add_graph`.** The
v0.3 `pgrdf.add_graph(id BIGINT)` UDF (signature unchanged) now
inserts `(id, 'urn:pgrdf:graph:' || id::text)` into
`_pgrdf_graphs` after creating the LIST partition, under
`ON CONFLICT (graph_id) DO NOTHING` so re-calls stay idempotent.
v0.3 callers gain a queryable IRI mapping for every graph without
any signature change.

**Slice 118 — IRI-keyed overload `pgrdf.add_graph(iri TEXT) → BIGINT`.**
Idempotent on the IRI: a repeat call returns the existing
`graph_id` without creating a second partition. On a fresh IRI it
allocates the next id (smallest unused positive integer via
`COALESCE(MAX(graph_id), 0) + 1`) under a
`LOCK TABLE _pgrdf_graphs IN SHARE ROW EXCLUSIVE MODE` so
concurrent callers can't compute the same id and race the INSERT.
The IRI is INSERTed into `_pgrdf_graphs` *before* re-entering
through the integer overload — slice 119's synthetic-IRI insert
then no-ops on `ON CONFLICT (graph_id) DO NOTHING`, preserving the
user-supplied IRI verbatim. Empty / whitespace-only IRI panics
with the stable `add_graph: iri must be non-empty` prefix.
RFC-3987 syntax validation is deferred to a later slice (no
`oxiri` dependency in v0.4.1).

**Slice 117 — explicit-binding overload
`pgrdf.add_graph(id BIGINT, iri TEXT) → BIGINT`.** Caller supplies
both halves; idempotent on a matching `(id, iri)` pair. Conflicts
panic with the stable `add_graph:` prefix —
`add_graph: graph_id <N> is bound to a different IRI (<existing>)`
when `id` is bound to a non-synthetic IRI different from the
request, or
`add_graph: iri <iri> is bound to a different graph_id (<existing>)`
when the IRI is bound to a different `graph_id`. The synthetic
placeholder `urn:pgrdf:graph:{id}` (the slice-119 seed that the
integer overload assigns automatically) is treated as
**upgradable**: when `id` currently points at its synthetic IRI
and the requested IRI is unbound elsewhere, the row is UPDATEd in
place — covering the common sequence `add_graph(42)` →
`add_graph(42, 'http://example.org/g42')`. Concurrent writers
serialised by the same lock idiom as slice 118. Negative `id` and
empty IRI rejected with the same stable prefixes shared by the
other two overloads.

**Slices 116 / 115 — symmetric lookups
`pgrdf.graph_id(iri TEXT) → BIGINT` and
`pgrdf.graph_iri(id BIGINT) → TEXT`.** Read-only resolution in
both directions; `NULL` on miss (no panic — NULL is the
lookup-miss signal, distinct from an actual SPI error which still
propagates with the stable `graph_id:` / `graph_iri:` prefix).
Both UDFs are marked `#[pg_extern(strict)]` so Postgres
short-circuits a NULL argument to NULL output without invoking
the function body. Both wrap their `SELECT … WHERE … LIMIT 1` in
a scalar subquery so SPI always returns "exactly one row" (NULL
or the bound value), dodging the
`SpiTupleTable positioned before the start` empty-result trip.

With slice 115 landed, the §3.2 UDF surface is **fully shipped**:
the three `add_graph` overloads plus the symmetric
`graph_id` / `graph_iri` lookups. SPARQL `GRAPH { … }` translation
follows in slices 114 → 112; the W3C-shape fixtures land in slice
111; the `pg_dump` round-trip script in slice 110. Spec:
[`SPEC.pgRDF.LLD.v0.4 §3`](../specs/SPEC.pgRDF.LLD.v0.4.md#3-named-graph-scoping-and-iri-mapping-new).

#### UDF surface — worked example

The five UDFs compose as follows. The session starts with one row
in `_pgrdf_graphs` (the `graph_id = 0` synthetic seed) and gains
two more across the example:

```sql
-- (1) Integer-keyed surface — slice 119 auto-binds the synthetic IRI.
SELECT pgrdf.add_graph(42::bigint);
--  → 42
SELECT pgrdf.graph_iri(42::bigint);
--  → 'urn:pgrdf:graph:42'

-- (2) IRI-keyed surface — slice 118 auto-allocates the next id.
SELECT pgrdf.add_graph('http://example.org/g1');
--  → 43       -- COALESCE(MAX(graph_id), 0) + 1 against existing rows
SELECT pgrdf.graph_id('http://example.org/g1');
--  → 43

-- (3) Explicit-binding surface — slice 117 with the synthetic
--     placeholder upgrade path.
SELECT pgrdf.add_graph(42::bigint, 'http://example.org/g42');
--  → 42       -- in-place UPDATE: synthetic urn:pgrdf:graph:42 is replaced
SELECT pgrdf.graph_iri(42::bigint);
--  → 'http://example.org/g42'

-- (4) Lookup misses are NULL (no panic, no error).
SELECT pgrdf.graph_id('http://example.org/unbound');
--  → NULL
SELECT pgrdf.graph_iri(9999::bigint);
--  → NULL

-- (5) The mapping survives pg_dump/restore via the
--     pg_extension_config_dump('_pgrdf_graphs', '') registration —
--     covered by tests/regression/scripts/pg-dump-roundtrip.sh.
```

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

## 2.4 Graph-level lifecycle UDFs (LLD v0.4 §5, **Phase B in flight — slice 99 → v0.4.2**)

Partition-level primitives over `_pgrdf_quads`. Constant-time DDL
where possible (DETACH/DROP for `drop_graph`, TRUNCATE for
`clear_graph`, DETACH/ATTACH metadata swap for `move_graph`). All
four UDFs land across the Phase B countdown (slices 99 → 96) toward
the v0.4.2 cut.

### `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN DEFAULT TRUE) → BIGINT` (**shipped — Phase B slice 99**)

Removes the LIST partition `_pgrdf_quads_g<id>` from the parent
`_pgrdf_quads` via `ALTER TABLE ... DETACH PARTITION` followed by
`DROP TABLE`, then deletes the matching `_pgrdf_graphs` row.
Returns the count of triples that lived in the partition at the
time of the drop.

`cascade => TRUE` (the default) drops the partition regardless of
content. `cascade => FALSE` errors with the stable
`drop_graph: inferred rows present` prefix if any `is_inferred =
TRUE` row exists — the strict-mode signal for downstream
maintenance flows that want to gate the drop on the
materialisation state of the graph.

Guards (stable error prefixes per the error-message contract):

```
drop_graph: graph_id must be >= 0, got <N>            -- negative id
drop_graph: cannot drop default partition (graph_id = 0)  -- catch-all
drop_graph: inferred rows present (graph_id = <N>); ...   -- cascade=false guard
```

Idempotent: dropping a graph_id whose partition doesn't exist
returns 0 (no error). A stranded `_pgrdf_graphs` row pointing at a
non-existent partition is pruned on this path so the IRI mapping
converges with reality on a crash-recovery code path. Post-drop,
`pgrdf.graph_iri(id)` and `pgrdf.graph_id(iri)` both return NULL.

Concurrency: the metadata window takes an `ACCESS EXCLUSIVE` lock
on the parent `_pgrdf_quads`. SELECT/UPDATE traffic on unrelated
graphs blocks briefly for the duration; this is documented for the
"long-running maintenance" workflow per LLD v0.4 §5.2.

```sql
-- Drop a graph + return the triple count that was in it.
SELECT pgrdf.add_graph(42);
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
     VALUES (1, 2, 3, 42), (4, 5, 6, 42);
SELECT pgrdf.drop_graph(42);
--  → 2  (triples removed)

-- Idempotent — re-dropping returns 0.
SELECT pgrdf.drop_graph(42);
--  → 0

-- Strict mode blocks inferred content.
SELECT pgrdf.drop_graph(43, cascade => false);
--  ERROR:  drop_graph: inferred rows present (graph_id = 43); ...
```

Regression coverage: `tests/regression/sql/88-drop-graph.sql` locks
the full surface (idempotent absent, happy path returning row
count, cascade-FALSE-inferred guard, cascade-TRUE-inferred
override, default-partition guard, negative-id guard). Pgrx
integration tests in `src/storage/graphs.rs` cover the idempotent
absent + happy + cascade-FALSE + default-partition + negative-id
paths under the pgrx `pg_test` harness.

Spec: [LLD v0.4 §5.1 / §5.2](../specs/SPEC.pgRDF.LLD.v0.4.md#5-graph-level-lifecycle-udfs-new).

### `pgrdf.clear_graph` / `copy_graph` / `move_graph` (🚧 slices 98 → 96)

Carry forward in the Phase B countdown — see LLD v0.4 §5.1 for the
signatures and §5.2 for the partition-DDL idioms each one uses.

## 2.5 What's NOT in storage

- **No vacuum tuning yet.** Standard autovacuum suffices for v0.2;
  tuning lives in [`docs/10-roadmap.md`](10-roadmap.md) Phase 4.
- **No TOAST tuning.** Literals are stored inline; long literals
  (≥ 2 KB) compress under default TOAST policy.
- **No PostgreSQL custom scan hooks** at v0.3.0 — Phase 2.x
  performance follow-on per LLD §4.2.
- **No foreign keys** from `_pgrdf_quads` to `_pgrdf_dictionary`.
  Intentional — the loader enforces referential integrity by
  resolving ids before INSERT, and FKs would slow the hot path.
