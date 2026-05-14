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

### Shmem dictionary cache (LLD §4.1, NOT yet shipped)

LLD §4.1 specifies a **process-instance-wide**
`RwLock<LruCache<u64, i64>>` backed by `pgrx::shmem`, keyed by a
hash of the RDF term. That's what would let two psql sessions
share the cost of the first cold lookup (and let the next ingest
call skip work the previous call paid for).

Not in any commit yet — tracked in
[`docs/10-roadmap.md`](10-roadmap.md) Phase 2.x backlog. The
per-call HashMap is the stepping-stone delivery; it gives most
of the within-call benefit while we work on the cross-call /
cross-backend shape.

When it lands, the read flow will be:

```
hash(RdfTerm) ─► shmem cache ─hit──► return id
                      │
                      └─miss──► Spi.query SELECT id FROM _pgrdf_dictionary …
                                     │
                                     └─ insert into shmem ─► return id
```

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

## 2.3 Bulk loader (`src/storage/loader.rs`)

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

### COPY BINARY (LLD §4.3, NOT yet shipped)

LLD §4.3 specifies `COPY _pgrdf_quads FROM STDIN (FORMAT BINARY)`
through `pgrx`'s native COPY API. The batched-INSERT path is the
stepping-stone delivery; COPY BINARY is a Phase 2.x backlog item
(typically another 2–5× speedup over batched INSERT on commodity
hardware). Tracked in `docs/10-roadmap.md` Phase 2.x backlog.

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
- **No PostgreSQL custom scan hooks** at v0.2.0 — Phase 2.x
  performance follow-on per LLD §4.2.
- **No foreign keys** from `_pgrdf_quads` to `_pgrdf_dictionary`.
  Intentional — the loader enforces referential integrity by
  resolving ids before INSERT, and FKs would slow the hot path.
