# 02 — Loading RDF

pgRDF ingests Turtle, TriG, and N-Quads (N-Triples are a Turtle
subset). This page covers the Turtle entry points — file-based
(`load_turtle`) and string-based (`parse_turtle`); the quad-bearing
formats use `parse_trig` and `parse_nquads`, which honour the graph
labels in the data.

## `pgrdf.load_turtle(path, graph_id, base_iri = NULL, bulk_load = false) → BIGINT`

Reads a Turtle file from a path the Postgres process can see and
ingests every triple. Returns the count.

```sql
SELECT pgrdf.load_turtle('/fixtures/ontologies/foaf.ttl', 1);
--  → 631
```

`path` is **server-side**. With the project's compose runtime,
`./fixtures/` on the host is bind-mounted at `/fixtures/` in the
container, so `'/fixtures/ontologies/foaf.ttl'` works directly.

For Kubernetes deployments, mount the directory containing your TTL
files into the postgres container and refer to its in-container path.

### `base_iri`

Some published Turtle documents use relative IRIs like `<#>` or
`<../foo>` that need a base URL to resolve. W3C PROV's `prov.ttl` is
the canonical example.

```sql
SELECT pgrdf.load_turtle(
  '/fixtures/ontologies/prov.ttl',
  100,
  'http://www.w3.org/ns/prov#'
);
--  → 1789
```

Pass `NULL` or `''` when your file uses absolute IRIs only (the
default). pgRDF parses strictly via `oxttl` 0.2 — anything that
fails here is genuinely off-spec, not a bug we should work around.

## `pgrdf.parse_turtle(content, graph_id, base_iri = NULL) → BIGINT`

Same loop, but the Turtle is passed as a string. Handy for small
ad-hoc snippets and for tests:

```sql
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://e.com/> . ex:a ex:p ex:b .',
  1
);
--  → 1
```

For files larger than a megabyte or so, prefer `load_turtle` — it
avoids copying the whole document through SQL.

## Verbose variants — `*_verbose` → JSONB

When you want timing or want to see the cache hit rate, swap the
non-verbose UDF for its `_verbose` twin:

```sql
SELECT pgrdf.load_turtle_verbose(
  '/fixtures/ontologies/prov.ttl',
  100,
  'http://www.w3.org/ns/prov#'
);
--  →  {"triples": 1789,
--      "dict_cache_hits": 4612,
--      "shmem_cache_hits": 0,
--      "dict_db_calls": 783,
--      "quad_batches": 2,
--      "elapsed_ms": 142.7}
```

Field meanings:

| Field | What it tells you |
|---|---|
| `triples` | Same value `load_turtle` would have returned. |
| `dict_cache_hits` | Term references resolved from the in-call HashMap. Higher = more repetition in the source. |
| `shmem_cache_hits` | Term references served by the cross-backend shmem dict cache (LLD §4.1). Non-zero on a warm postmaster reload of the same vocabulary. |
| `dict_db_calls` | Term references that went to the `_pgrdf_dictionary` (either a hit or an insert). Roughly = `distinct_terms_in_file`. |
| `quad_batches` | Number of multi-row INSERT flushes (1 per ~1000 triples). |
| `elapsed_ms` | Wall-clock time inside the function. |

`parse_turtle_verbose(content, graph_id, base_iri)` mirrors this for
in-memory input.

## Graphs

Every triple belongs to a `BIGINT` graph. Use `0` (the default
partition) for "no specific graph", or partition meaningfully:

```sql
SELECT pgrdf.add_graph(100);                   -- creates _pgrdf_quads_g100 partition
SELECT pgrdf.load_turtle('/data/users.ttl',    100);
SELECT pgrdf.load_turtle('/data/products.ttl', 101);  -- lands in default partition
```

`add_graph(g)` is idempotent — second call returns `false`, partition
stays as-is. Calling `load_turtle` with a `graph_id` you haven't
explicitly `add_graph`'d sends the tuples to the default partition;
that still works, you just lose the ability to drop the graph in
one DDL.

To drop a whole named graph cheaply, use the lifecycle UDF — it
removes both the partition and the `_pgrdf_graphs` row in one call:

```sql
SELECT pgrdf.drop_graph(100);
--  → 4321   (count of triples that were in the graph)
```

This is partition-DDL-bounded, no `DELETE` scan, no autovacuum churn.
The partition is detached + dropped and the `_pgrdf_graphs` mapping
row is removed in the same transaction.

### Named graphs by IRI

The integer `graph_id` form above is the original surface and stays
fully supported. v0.4 adds an **IRI-keyed surface** on top: every
graph carries an IRI in the `pgrdf._pgrdf_graphs(graph_id, iri)`
mapping table, so you can allocate, look up, and SPARQL-query
graphs by their RDF name rather than by an opaque integer.

Three `pgrdf.add_graph` overloads are available:

```sql
-- 1. Integer-keyed (legacy). Auto-binds a synthetic
--    urn:pgrdf:graph:<id> IRI in _pgrdf_graphs.
SELECT pgrdf.add_graph(100);

-- 2. IRI-keyed. Auto-allocates the next free graph_id, creates
--    the partition, binds the IRI. Idempotent on the IRI —
--    a second call with the same IRI returns the same id.
SELECT pgrdf.add_graph('http://example.org/users');
--  → 1

-- 3. Explicit (id, iri) pair. Use when you want both sides
--    pinned. Errors if either side conflicts with an existing
--    binding; idempotent when both sides match an existing row.
SELECT pgrdf.add_graph(200::bigint, 'http://example.org/products');
--  → true
```

Once a graph exists, load Turtle into it via the integer `graph_id`
the same way as before — `load_turtle` and `parse_turtle` haven't
changed:

```sql
-- IRI-allocated graph: feed the returned id straight into load_turtle
WITH g AS (SELECT pgrdf.add_graph('http://example.org/users') AS id)
SELECT pgrdf.load_turtle('/data/users.ttl', g.id) FROM g;

-- Or look up the id explicitly via graph_id(iri)
SELECT pgrdf.load_turtle(
  '/data/users.ttl',
  pgrdf.graph_id('http://example.org/users')
);
```

Two read-only lookup UDFs round out the surface:

```sql
SELECT pgrdf.graph_id('http://example.org/users');   --  → 1
SELECT pgrdf.graph_iri(1::bigint);                   --  → 'http://example.org/users'

-- Unknown side → NULL (no error)
SELECT pgrdf.graph_id('http://nope.example/never');  --  → NULL
SELECT pgrdf.graph_iri(999::bigint);                 --  → NULL
```

Both lookups are STRICT — `NULL` input short-circuits to `NULL`
output without an SPI round trip. Use them to translate between the
IRI you carry in your application and the integer id the storage
layer keys on.

The legacy integer overload `add_graph(id BIGINT)` still works as
before; it now also INSERTs a synthetic `urn:pgrdf:graph:<id>`
binding into `_pgrdf_graphs` so the IRI-keyed surface (SPARQL
`GRAPH ?g { … }`, `graph_iri(id)`, etc.) sees every graph regardless
of which overload created it. The synthetic IRI is an opaque URN —
you can upgrade it later by calling the explicit `add_graph(id, iri)`
form, which atomically rebinds the synthetic to your IRI.

To inspect what's bound:

```sql
SELECT graph_id, iri FROM pgrdf._pgrdf_graphs ORDER BY graph_id;
--  graph_id |               iri
-- ----------+----------------------------------
--         0 | urn:pgrdf:graph:0
--         1 | http://example.org/users
--       100 | urn:pgrdf:graph:100            -- from add_graph(100)
--       200 | http://example.org/products
```

Once a graph is allocated, scope SPARQL queries to it with
`GRAPH <iri> { … }` or `GRAPH ?g { … }` — see
[03-querying.md → Named graphs](03-querying.md#named-graphs).

### Graph lifecycle

Four partition-level lifecycle UDFs live alongside `add_graph` — all
return `BIGINT` row counts and are idempotent on absent / empty
sources. Each takes either a `BIGINT` graph id (shown below) or an
IRI `TEXT` (the v0.5.0 IRI overloads — e.g.
`pgrdf.drop_graph('http://example.org/g1')`, error
`drop_graph: unknown iri` on an unbound IRI):

```sql
-- Drop a graph entirely: detaches the partition, deletes the
-- _pgrdf_graphs row, returns the pre-drop triple count.
SELECT pgrdf.drop_graph(100);                           -- → N
SELECT pgrdf.drop_graph(100, cascade => false);         -- errors if inferred rows present

-- Wipe rows but keep the partition + IRI binding.
SELECT pgrdf.clear_graph(100);                          -- → N (rows removed)

-- Copy all rows from src to dst. Auto-creates dst partition + IRI
-- if absent. Both base and inferred rows carry forward.
SELECT pgrdf.copy_graph(100, 200);                      -- → N (rows copied)

-- Move = copy + drop src. The dst partition can't pre-hold data.
SELECT pgrdf.move_graph(100, 300);                      -- → N (rows moved)
```

Stable error prefixes for downstream tooling:
`drop_graph: cannot drop default partition`,
`drop_graph: inferred rows present`,
`copy_graph: src and dst must differ`,
`move_graph: dst graph_id <N> already has data`, and the shared
`{drop,clear,copy,move}_graph: graph_id must be >= 0` shape.
Full reference: [docs/02-storage.md §2.4](../docs/02-storage.md#24-graph-level-lifecycle-udfs-lld-v04-5-phase-b-shipped--v042).

## Counts + sanity checks

```sql
SELECT pgrdf.count_quads(100);                                          -- quads in graph 100
SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = 100;           -- same number, direct read

SELECT count(DISTINCT subject_id)::int                                  -- distinct subjects in 100
  FROM pgrdf._pgrdf_quads WHERE graph_id = 100;

-- Lookup a specific term's id (handy for joining)
SELECT id FROM pgrdf._pgrdf_dictionary
 WHERE term_type = 1
   AND lexical_value = 'http://xmlns.com/foaf/0.1/Person';
```

## Performance posture (today)

- Per-triple SPI calls drop from ~7 to ~1 after the per-call HashMap
  dict cache warms. Empirically: a 100-triple synthetic fixture hits
  185 cache references vs 115 DB references — see
  [`tests/regression/sql/25-bulk-ingest.sql`](../tests/regression/sql/25-bulk-ingest.sql)
  for the empirical assertions.
- A second load of the same Turtle (within the postmaster's
  lifetime) hits the **cross-backend shmem dict cache** (LLD §4.1):
  0 dictionary-table touches for already-seen terms. Counters in
  `load_turtle_verbose.shmem_cache_hits` and cumulative
  `pgrdf.stats()` (`shmem_hits` / `shmem_inserts`).
- Every flush of the batched `INSERT … unnest(…)` reuses a
  per-backend prepared plan (LLD §4.2 + §4.3 phase A); first flush
  primes the cache, every subsequent flush reuses it.
- The full `tests/perf/smoke-ontologies.sh` set (~17K triples across
  24 ontologies) currently completes in a few seconds total.
- For large fresh loads, `load_turtle(…, bulk_load => true)` runs a
  parallel fast path — all-cores parse, in-memory term dedup,
  self-assigned-id dictionary load, parallel triple→id resolve, and
  batched quad insert — measured at 2.3–3.5× the streaming path on
  LUBM-250/500. It applies to a fresh (empty) dictionary and falls
  back to the streaming path on a populated one. A deeper
  `heap_multi_insert` / `COPY … FORMAT BINARY` quad insert (LLD §12
  phase B) is a tracked follow-up.

## Tuning for large bulk loads (Tier-1, big-RAM)

When you're ingesting hundreds of millions to billions of quads into a
**fresh** database on a big-RAM node, the defaults leave a lot on the
table. The profile below pairs `load_turtle(…, bulk_load => true)` with
server settings that keep the bottleneck on CPU + I/O rather than WAL
and checkpoints.

### Postgres server settings

Set these in `postgresql.conf` (or `ALTER SYSTEM` + reload) **before**
the load. Values assume a dedicated box with tens of GB of RAM; scale
to your hardware.

| Setting | Suggested | Why |
|---|---|---|
| `shared_buffers` | 25–40 % of RAM | Keep the dictionary + hot index pages resident. |
| `maintenance_work_mem` | 2–8 GB | Faster index (re)builds — used by the defer-index rebuild and `ANALYZE`. |
| `max_wal_size` | 32–64 GB | Fewer checkpoints during the load; the single biggest win for sustained write throughput. |
| `checkpoint_timeout` | 30–60 min | Same — spread checkpoints out. |
| `wal_compression` | `on` | Less WAL volume on a write-heavy load. |
| `effective_io_concurrency` | 200–256 (SSD/NVMe) | Concurrent prefetch for the parallel resolve + index scans. |
| `max_parallel_maintenance_workers` | 4–8 | Parallelises the defer-index rebuild. |
| `max_parallel_workers` / `…_per_gather` | ≈ core count | Headroom for the parallel index build + later queries. |

### Durability vs. speed (read the caveat)

For a **rebuildable** bulk import — a fresh load you can simply re-run
from source if the box dies mid-load — you can trade crash durability
for throughput:

| Setting | Bulk value | Caveat |
|---|---|---|
| `synchronous_commit` | `off` | Safe-ish: a crash loses recently-committed txns but never corrupts. Fine for a reloadable import. |
| `fsync` | `off` | **Dangerous.** A crash can corrupt the cluster — only when the whole DB is disposable and you'll reload from scratch. Turn it back `on` (and restart) before the data matters. |
| `full_page_writes` | `off` | Only meaningful alongside `fsync = off`; same caveat. |

Restore `synchronous_commit` / `fsync` / `full_page_writes` to their
durable defaults — and `CHECKPOINT` — once the load completes and
before the database goes into service.

### pgRDF knobs

| Knob | Default | For big loads |
|---|---|---|
| `bulk_load => true` (a `load_turtle` arg) | `false` | Use it. The parallel fast path fires only on a **fresh** (empty) dictionary, so load the largest file first into a clean database, then load smaller files normally. |
| `pgrdf.bulk_defer_index_min` | `100000` | Above this row count the fast path drops the hexastore indexes + `unique_term`, loads heap-only, then rebuilds in parallel. The default is already the right call at scale. |
| `pgrdf.dict_batch_size` | `500` | The streaming dict batch size; irrelevant on the bulk path (it batches in-Rust). |
| `pgrdf.auto_analyze` | `on` | Leave on — the automatic post-load / post-materialize `ANALYZE` is what keeps the planner honest at scale. |

### Order of operations

1. Fresh database, server tuned as above, durability relaxed (only if
   the load is rebuildable).
2. `load_turtle('/data/big.ttl', 1, NULL, true)` — largest file first.
3. Load any smaller / incremental files (these take the streaming path
   once the dictionary is populated).
4. `pgrdf.materialize(...)` if you need inference.
5. Restore durable settings, `CHECKPOINT`, put the DB into service.

## What still doesn't work

| Symptom | Cause |
|---|---|
| `load_turtle: turtle parse error: Syntax(TurtleSyntaxError ... "No scheme found in an absolute IRI")` | Document uses relative IRIs. Pass `base_iri`. |
| `load_turtle: turtle parse error: Syntax(...) "Invalid character …"` | Genuinely off-spec IRI (e.g. colon in path segment). Fix the source — pgRDF is strict by design. |
| `load_turtle: unsupported object term (RDF-star not in v0.2 scope)` | Document uses RDF-star quoted triples. Not supported in the v0.x series. |
| `load_turtle: failed to open …` | Path isn't reachable from the postgres process. Check your container/bind-mount config. |

## Next

Wiring this into your application: see
[clients/python.md](clients/python.md) or
[clients/rust.md](clients/rust.md).
