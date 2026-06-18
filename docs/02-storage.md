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

### Lifecycle UDFs (`pgrdf.clear_graph`, **Phase B slice 98**)

Phase B opens the LLD v0.4 §5 graph-level lifecycle UDF surface
— partition-level primitives that operate on `_pgrdf_quads`'s LIST
partitioning instead of N-row DELETE loops.

#### clear_graph

**`pgrdf.clear_graph(id BIGINT) → BIGINT`** (slice 98) issues
`TRUNCATE ONLY pgrdf._pgrdf_quads_g<id>` against the per-graph
partition and returns the rows-removed count (== the row count
captured immediately before the TRUNCATE). Both base and inferred
rows are wiped — the function is not `is_inferred`-discriminating
per LLD v0.4 §5.2.

Key invariants:

- **Partition shell survives.** `TRUNCATE ONLY` empties the
  per-graph partition's row storage but leaves the relation
  attached to `_pgrdf_quads`. Subsequent inserts with the same
  `graph_id` route into the same partition without falling back
  to `_pgrdf_quads_default`.
- **IRI binding survives.** The matching `_pgrdf_graphs` row is
  untouched, so `pgrdf.graph_iri(id)` keeps resolving to the
  bound IRI. This is the contrast point with `drop_graph(id)`
  (sibling slice 99), which removes both the partition and the
  IRI binding.
- **Idempotent on absent / empty graphs.** Calling against a
  `graph_id` with no LIST partition returns 0 without erroring;
  re-calling against an already-empty partition returns 0
  again. Callers can `clear_graph` blindly during cleanup
  without probing for partition existence first.
- **`graph_id = 0` is permitted.** Unlike `drop_graph(0)` (which
  rejects the default-partition id outright), `clear_graph(0)`
  is legal — it operates on the explicit `_pgrdf_quads_g0`
  partition if one was created via `add_graph(0)`, or no-ops to
  0 if not.
- **Negative id panics** with the stable prefix
  `clear_graph: graph_id must be >= 0, got <N>` — same shape
  contract as `add_graph(id BIGINT)` (slice 119) so downstream
  tooling can route on the prefix.

`TRUNCATE ONLY` (not bare `TRUNCATE`) is deliberate: `ONLY` blocks
cascade to any descendant partitions. The per-graph partitions
have no children today, but `ONLY` is defence-in-depth against a
future sub-partitioning slice silently widening the scope.

Regression coverage:
[`tests/regression/sql/89-clear-graph.sql`](../tests/regression/sql/89-clear-graph.sql)
locks all six invariants (absent-graph idempotency, load+clear
returns count, partition survives, IRI binding survives, double-
clear returns 0, `clear_graph(0)` works, negative id rejected).
Three `#[pg_test]`s in `src/storage/graphs.rs` exercise the
happy path + idempotent-absent + clear-twice shapes against a
live in-process Postgres.

#### copy_graph

**`pgrdf.copy_graph(src BIGINT, dst BIGINT) → BIGINT`** (slice 97)
issues `INSERT INTO pgrdf._pgrdf_quads_g<dst> (subject_id,
predicate_id, object_id, graph_id, is_inferred) SELECT subject_id,
predicate_id, object_id, <dst>::bigint, is_inferred FROM
pgrdf._pgrdf_quads_g<src>` and returns the count copied. The
`graph_id` projection rebinds to the destination id so the
partition router lands the rows in `dst`'s partition without
touching `_pgrdf_quads_default`. `copy_graph` is the only
lifecycle UDF that touches every row — the siblings (`drop_graph`,
`move_graph`, `clear_graph`) are all metadata-DDL-bounded.

Key invariants:

- **`is_inferred` carries forward.** Both `is_inferred = FALSE`
  and `is_inferred = TRUE` rows are copied verbatim — the
  function is not `is_inferred`-discriminating per LLD v0.4 §5.2.
  Materialised inferred content in the source survives into the
  destination as inferred, so callers don't have to re-run
  `pgrdf.materialize_owl_rl(dst)` to recover the entailments.
- **Destination auto-create.** If `_pgrdf_quads_g<dst>` does not
  exist, the function calls `pgrdf.add_graph(dst::bigint)` to
  create it. That call also binds a synthetic
  `urn:pgrdf:graph:{dst}` IRI in `_pgrdf_graphs` per slice 119,
  so `pgrdf.graph_iri(dst)` resolves post-copy even if the caller
  hadn't pre-registered the destination. A pre-existing IRI
  binding on `dst` is preserved unchanged (the partition
  existence check short-circuits before `add_graph` runs).
- **Source absence is idempotent.** Copying from a `graph_id` whose
  partition does not exist returns 0 without erroring. The
  destination partition is NOT auto-created on this short-circuit
  path — `copy_graph(absent_src, fresh_dst)` is a clean no-op.
  Matches the §5.2 idempotency contract.
- **Re-call duplicates.** Calling `copy_graph(src, dst)` twice
  against the same pair appends another copy of `src`'s rows into
  `dst` — the function does NOT clear `dst` before inserting.
  Callers needing strict re-call idempotency should invoke
  `pgrdf.clear_graph(dst)` before the second copy. This is the
  `ADD` (W3C SPARQL 1.1 Update §3.2.6) vs `COPY` distinction
  pushed into the caller's responsibility.
- **`src == dst` is rejected** with the stable prefix
  `copy_graph: src and dst must differ` — the self-copy
  degenerate case has no defined semantics on a partitioned table
  (an `INSERT … SELECT` from a table into itself would interleave
  scan + insert unpredictably) and is surfaced rather than
  silently double-written.
- **Negative ids panic** with the stable prefix
  `copy_graph: graph_id must be >= 0, got src=<S>, dst=<D>` —
  matches the error-shape contract `add_graph(id BIGINT)` (slice
  119) and the other lifecycle UDFs already established.

The single-statement `INSERT INTO … SELECT` runs in the calling
statement's transaction, so a concurrent INSERT on `src` arriving
mid-copy is either visible (and copied) or not (and missed) per
the snapshot the calling SELECT pinned — standard MVCC semantics,
no partition-DDL lock involved on this path. Cost scales linearly
with `src`'s row count (the partition-DDL siblings are O(1) in row
count by contrast); plan a long-running maintenance window for
copies on a large source.

```sql
-- Copy graph 42's content into a fresh graph 100. The dst
-- partition is auto-created.
SELECT pgrdf.add_graph(42);
INSERT INTO pgrdf._pgrdf_quads
  (subject_id, predicate_id, object_id, graph_id, is_inferred)
VALUES (1, 1, 1, 42, false),
       (2, 2, 2, 42, true);

SELECT pgrdf.copy_graph(42::bigint, 100::bigint);
--  → 2  (rows copied — both base and inferred carry forward)
SELECT pgrdf.graph_iri(100::bigint);
--  → urn:pgrdf:graph:100  (synthetic IRI bound by auto-create)

-- Re-call duplicates without an intervening clear.
SELECT pgrdf.copy_graph(42::bigint, 100::bigint);
--  → 2  (the function returns the src count; dst now holds 4 rows)

-- Strict idempotency: clear first, then copy.
SELECT pgrdf.clear_graph(100::bigint);  --  → 4
SELECT pgrdf.copy_graph(42::bigint, 100::bigint);  --  → 2
```

Regression coverage:
[`tests/regression/sql/90-copy-graph.sql`](../tests/regression/sql/90-copy-graph.sql)
locks the seven invariants (absent-src idempotency with no dst
auto-create, load + copy returns count + dst auto-created +
graph_iri resolves, `is_inferred` preserved, src untouched,
re-call duplicates + clear-then-copy round-trip, `src == dst`
rejected, negative ids rejected). Three `#[pg_test]`s in
`src/storage/graphs.rs` cover the happy path, absent-src
short-circuit, and `src == dst` rejection paths against a live
in-process Postgres.

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

The parallel bulk loader (`bulk_load => true`, v0.6.2–v0.6.6) already
delivers a 2.3–3.5× fast path; this deeper `heap_multi_insert` /
COPY-BINARY quad insert (LLD §12 phase B) remains a tracked
follow-up for a larger win.

### Graph routing

The loader writes directly with `graph_id = <caller-supplied>`,
so the partition router places tuples in the correct partition
without a round-trip through the default partition. If the caller
hasn't called `pgrdf.add_graph(g)` first, tuples land in the
default partition; subsequent `add_graph(g)` calls don't move
them — the partition-creation order is the caller's
responsibility.

### TriG / N-Quads ingest (✅ Phase G group G2, LLD v0.5 §4)

Quad-format siblings of `parse_turtle`. Both honour graph IRIs and
reuse the same batched-insert path (`flush_batch` /
`QUAD_INSERT_SQL` prepared plan), partition-routed per resolved
`graph_id` via a per-graph batch buffer:

```sql
pgrdf.parse_trig(content TEXT, default_graph_id BIGINT DEFAULT 0,
                 strict BOOLEAN DEFAULT FALSE) → JSONB
pgrdf.parse_nquads(content TEXT, default_graph_id BIGINT DEFAULT 0,
                   strict BOOLEAN DEFAULT FALSE) → JSONB
```

- **TriG** — Turtle plus inline `GRAPH <iri> { … }` blocks. Triples
  outside any GRAPH block land in `default_graph_id`.
- **N-Quads** — the 4-position line format; the 4th-position graph
  IRI routes the quad; 3-position lines fall to `default_graph_id`.
- **Graph IRI resolution** (v0.4 §3.2): a bound IRI → its existing
  `graph_id`; an unbound IRI → `pgrdf.add_graph(iri)` auto-allocates
  a fresh id + LIST partition **by default**, or — under
  `strict => TRUE` — is **rejected** with the stable prefix
  `parse_trig: unknown graph iri <iri>` /
  `parse_nquads: unknown graph iri <iri>`. Resolution happens
  *before* a quad is buffered, so a strict rejection raises with no
  partial rows (all-or-nothing within the call — the raise rolls
  back the enclosing statement).
- **JSONB stats** mirror `parse_turtle_verbose` (triples,
  dict/shmem cache hits, dict_db_calls, quad_batches, elapsed_ms)
  and add a `graphs` array of the resolved destination graph ids in
  first-seen order.

Round-trip behaviour: a TriG document re-emitted via
`pgrdf.construct` per graph (`{ ?s ?p ?o }` scoped by
`GRAPH <iri>`) reproduces that graph's triple set exactly
(quad-set isomorphism per graph) — there is no full-TriG
re-serialiser UDF in v0.5.

## 2.4 Graph-level lifecycle UDFs (LLD v0.4 §5, **Phase B shipped — v0.4.2**)

Partition-level primitives over `_pgrdf_quads`. Constant-time DDL
where possible (DETACH/DROP for `drop_graph`, TRUNCATE for
`clear_graph`, DETACH/ATTACH metadata swap for `move_graph`). All
four UDFs landed across the Phase B countdown (slices 99 → 96), and
slice 95 wires the end-to-end integration test. The full surface
ships in **v0.4.2**.

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

### `pgrdf.clear_graph` / `copy_graph` (✅ slices 98 + 97)

Both ship; see the `#### clear_graph` and `#### copy_graph`
subsections under §2.2 above for the row-touching pair.

### `pgrdf.move_graph(src BIGINT, dst BIGINT) → BIGINT` (**shipped — Phase B slice 96**)

Migrates every quad in graph `src` to graph `dst` and removes the
`src` partition. Returns the count of triples moved (== the row
count of `src` at copy time).

**Implementation strategy — compose over siblings.** The v0.4.2
implementation is `pgrdf.copy_graph(src, dst)` followed by
`pgrdf.drop_graph(src, cascade => TRUE)`. Both halves run in the
calling statement's transaction, so a rollback unwinds both.
Semantically equivalent to the LLD §5.2 "DETACH partition + rebind
`FOR VALUES IN (<dst>)` + ATTACH" path, but tractable without the
partition-constraint dance that a true metadata-only swap would
require (every row's `graph_id` column would need updating to
satisfy the post-rebind LIST constraint, which itself is a row
scan). The §5.2 "metadata-only" claim is therefore aspirational
for the v0.4.2 compose; deferred as a v0.6-FUTURE perf
optimisation (see `specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md`). For
small-graph workloads the compose is fast; for very large graphs
it scans
twice (once during copy, once during drop's pre-count).

Guards (stable error prefixes per the error-message contract):

```
move_graph: graph_id must be >= 0                              -- negative id
move_graph: src and dst must differ (both = <N>)               -- self-move
move_graph: dst graph_id <N> already has data (<M> rows); …    -- dst non-empty
```

Idempotent: when `src` partition is absent, `move_graph` returns
0 without erroring (short-circuit on the existence check; the
compose's copy/drop steps are never invoked). The dst-has-data
guard runs the pg_class existence check + row count *before*
invoking the compose, so a caller probing the workflow with an
empty pre-built dst sees the move succeed and the data routed in.

`_pgrdf_graphs` invalidation: the compose inherits slice 99's
behaviour — the `src` row is removed (drop step), and the `dst`
row is allocated if absent (copy step's responsibility per
slice 97). If `dst` was already bound to a different IRI, that
binding is preserved (slice 97 must not clobber a pre-existing
binding).

```sql
-- Move every quad in graph 42 to graph 43, return the count.
SELECT pgrdf.move_graph(42, 43);

-- Idempotent — moving an absent src is a 0-return.
SELECT pgrdf.move_graph(9999, 100);  -- → 0

-- Self-move is rejected (would be destructive).
SELECT pgrdf.move_graph(42, 42);
--  ERROR:  move_graph: src and dst must differ (both = 42)
```

Regression coverage: `tests/regression/sql/91-move-graph.sql` locks
the full surface (happy path with row count, idempotent absent,
src==dst rejection, dst-has-data rejection, negative-id rejection).
Pgrx integration tests in `src/storage/graphs.rs` cover the happy
path + absent-src + self-move + negative-id + dst-has-data shapes
under the pgrx `pg_test` harness. The happy path depends on
slice 97's `copy_graph` at runtime; the standalone shape tests
(self-move, negative, dst-has-data, absent) are independent.

Spec: [LLD v0.4 §5.1 / §5.2](../specs/SPEC.pgRDF.LLD.v0.4.md#5-graph-level-lifecycle-udfs-new).

### End-to-end lifecycle composition (✅ slice 95)

The four §5 UDFs compose cleanly under the loader / dict / hexastore
they all sit on top of:
[`tests/regression/sql/92-lifecycle-end-to-end.sql`](../tests/regression/sql/92-lifecycle-end-to-end.sql)
covers the load → copy → drop round-trip, `move_graph` as a faithful
`copy_graph + drop_graph` compose, `clear_graph` isolation across a
shared dictionary, SPARQL `GRAPH <iri>` projection survival post-copy,
and the drop-then-rebind loop. Per-UDF files (88/89/90/91) lock each
UDF's invariants in isolation; slice 95 pins their interactions.

### IRI-keyed overloads (✅ Phase G group G1, LLD v0.5 §7)

Each lifecycle UDF gains an IRI-keyed overload so callers don't have
to wrap every IRI in `pgrdf.graph_id(iri)`:

```sql
pgrdf.drop_graph(iri TEXT, cascade BOOLEAN DEFAULT TRUE) → BIGINT
pgrdf.clear_graph(iri TEXT)                              → BIGINT
pgrdf.copy_graph(src_iri TEXT, dst_iri TEXT)             → BIGINT
pgrdf.move_graph(src_iri TEXT, dst_iri TEXT)             → BIGINT
```

Semantics are **identical** to the BIGINT overloads — the IRI
overload resolves `iri → graph_id` via `_pgrdf_graphs.iri` and
dispatches to the *same* BIGINT UDF (the partition-DDL logic is
single-sourced; pgrx surfaces both signatures under one SQL name and
Postgres dispatches on argument type, the same pattern `add_graph`
uses).

The **one intentional difference** vs the BIGINT overloads (§7.1):
an unbound IRI is an **error** with the stable prefix
`<fn>: unknown iri`, *not* the BIGINT overloads' no-op-returns-0 on
an absent id. A BIGINT id is a raw partition selector (absent ⇒
nothing to do); an IRI is a *name* the caller asserts is bound, so a
miss is a programming error.

```sql
-- Equivalent to pgrdf.drop_graph(pgrdf.graph_id('http://ex.org/g1')):
SELECT pgrdf.drop_graph('http://ex.org/g1');     -- → pre-drop row count

SELECT pgrdf.clear_graph('http://ex.org/g1');    -- empties, keeps binding
SELECT pgrdf.copy_graph('http://ex.org/a',
                         'http://ex.org/b');      -- → src row count
SELECT pgrdf.move_graph('http://ex.org/a',
                         'http://ex.org/b');      -- src unbound after

-- Unbound IRI → error (distinct from the BIGINT no-op):
SELECT pgrdf.drop_graph('http://ex.org/nope');   -- ERROR: drop_graph: unknown iri "http://ex.org/nope"
SELECT pgrdf.drop_graph(99999::bigint);          -- → 0 (BIGINT no-op, unchanged)
```

Regression coverage:
[`tests/regression/sql/118-lifecycle-iri-overloads.sql`](../tests/regression/sql/118-lifecycle-iri-overloads.sql)
locks IRI≡BIGINT equivalence, binding preservation, copy/move
mirror semantics, all four unknown-iri errors, the BIGINT no-op
contrast, and composition with the v0.4 §4 SPARQL UPDATE lifecycle
algebra (drop-by-IRI then `CREATE GRAPH <same-iri>` rebinds).
Pgrx integration tests in `src/storage/graphs.rs`.

Spec: [LLD v0.5 §7](../specs/SPEC.pgRDF.LLD.v0.5.md).

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
