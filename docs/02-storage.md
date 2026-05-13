# 02 — Storage

Three sub-components, each with a clear contract.

## 2.1 Shared dictionary (`_pgrdf_dictionary`)

Maps RDF terms (URI / BlankNode / Literal) to 64-bit integers.

Schema is declared in [`sql/schema_v0_2_0.sql`](../sql/schema_v0_2_0.sql)
and loaded into the extension via `extension_sql_file!` in `src/lib.rs`.

**Per-backend cache (in-process):** `LruCache<u64, i64>` keyed by a
hash of the term. Standard LRU, cheap to populate from cold reads.

**Cross-backend cache (shmem):** the v0.2 LLD §4.1 contract is an
**instance-wide** `RwLock<LruCache<u64, i64>>` backed by `pgrx::shmem`.
This is what allows two different psql sessions to share the cost of
the first cold lookup. Implementation lives in `src/storage/dict.rs`.

**Read flow:**

```
hash(RdfTerm) ─► shmem cache ─hit──► return id
                      │
                      └─miss──► Spi.query SELECT id FROM _pgrdf_dictionary …
                                     │
                                     └─ insert into shmem ─► return id
```

**Write flow (ingestion):** the bulk loader (§2.3) batches dictionary
inserts, takes the write lock on the shmem cache only once per batch
to populate it post-COPY.

## 2.2 Hexastore (`_pgrdf_quads`)

Partitioned table holding the quads (subject, predicate, object, graph,
is_inferred). Partition strategy: **LIST on `graph_id`**, default
partition for everything not explicitly created.

Six covering indexes (one for each permutation of S, P, O) using the
`INCLUDE (is_inferred)` clause so that the most common SPARQL BGP
shapes resolve via index-only scans. v0.2.0 ships SPO / POS / OSP;
SOP / PSO / OPS join the index set in v0.3 once we measure the
trade-off against ingestion write amplification.

## 2.3 Bulk loader (`src/storage/loader.rs`)

The fast ingestion path uses `COPY _pgrdf_quads FROM STDIN (FORMAT BINARY)`.

```
N-Triples / Turtle ─► parse ─► resolve IDs via shmem dict ─► binary tuple stream ─► COPY
```

Row-by-row INSERT is ~50× slower than this path on commodity hardware
and is reserved for ad-hoc single-quad writes.

**Graph routing:** the loader uses the partition key (`graph_id`) to
ensure tuples land in the correct partition without round-trip through
the default partition.

## 2.4 What's NOT in storage

- **No vacuum tuning yet.** Standard autovacuum suffices for v0.2;
  tuning lives in `docs/10-roadmap.md` Phase 3.
- **No TOAST tuning.** Literals are stored inline; long literals
  (≥ 2 KB) compress under default TOAST policy.
- **No PostgreSQL custom scan hooks** at v0.2.0 — Phase 2 deliverable
  per LLD §6.2 (Query Engine & Shared Memory).
