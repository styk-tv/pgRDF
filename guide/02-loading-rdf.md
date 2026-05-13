# 02 — Loading RDF

pgRDF supports Turtle today (other syntaxes are queued for Phase 3).
There are two entry points: file-based and string-based.

## `pgrdf.load_turtle(path, graph_id, base_iri = NULL) → BIGINT`

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
--      "dict_db_calls": 783,
--      "quad_batches": 2,
--      "elapsed_ms": 142.7}
```

Field meanings:

| Field | What it tells you |
|---|---|
| `triples` | Same value `load_turtle` would have returned. |
| `dict_cache_hits` | Term references resolved from the in-call HashMap. Higher = more repetition in the source. |
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

To drop a whole named graph cheaply:

```sql
DROP TABLE pgrdf._pgrdf_quads_g100;
```

This is constant-time, no `DELETE` scan, no autovacuum churn. The
partition is gone and the parent table reflects the new state.

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

- Per-triple SPI calls drop from ~7 to ~1 after the dict cache warms
  up (Phase 2.2). Empirically: a 100-triple synthetic fixture hits
  185 cache references vs 115 DB references — see
  [`tests/regression/sql/25-bulk-ingest.sql`](../tests/regression/sql/25-bulk-ingest.sql)
  for the empirical assertions.
- The full `tests/perf/smoke-ontologies.sh` set (~17K triples across
  24 ontologies) currently completes in a few seconds total.
- True `COPY ... FROM STDIN (FORMAT BINARY)` is the Phase 3 fast path
  for millions-of-triples-per-second instance loads.

## What still doesn't work

| Symptom | Cause |
|---|---|
| `load_turtle: turtle parse error: Syntax(TurtleSyntaxError ... "No scheme found in an absolute IRI")` | Document uses relative IRIs. Pass `base_iri`. |
| `load_turtle: turtle parse error: Syntax(...) "Invalid character …"` | Genuinely off-spec IRI (e.g. colon in path segment). Fix the source — pgRDF is strict by design. |
| `load_turtle: unsupported object term (RDF-star not in v0.2 scope)` | Document uses RDF-star quoted triples. Not supported in v0.x. |
| `load_turtle: failed to open …` | Path isn't reachable from the postgres process. Check your container/bind-mount config. |

## Next

Wiring this into your application: see
[clients/python.md](clients/python.md) or
[clients/rust.md](clients/rust.md).
