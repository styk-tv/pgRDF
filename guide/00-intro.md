# 00 — Introduction

pgRDF is a PostgreSQL extension that lets your existing Postgres
database act as an RDF triple store, SPARQL endpoint, and SHACL
validator.

## What that looks like in practice

You install the extension the same way you'd install `pg_trgm` or
`postgis`:

```sql
CREATE EXTENSION pgrdf;
```

After that, every capability is a SQL function call. There's no
sidecar process, no second protocol to learn, no separate index to
keep in sync with your relational data. Your application connects to
Postgres the way it always has — `psycopg`, `tokio-postgres`, JDBC —
and reaches RDF features through SQL.

```sql
-- Load a Turtle file
SELECT pgrdf.load_turtle('/data/foaf.ttl', 1);

-- Inspect what you got
SELECT pgrdf.count_quads(1);

-- Query it with SPARQL
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n WHERE { ?s foaf:name ?n }'
);

-- Materialize OWL 2 RL entailments + query the closure
SELECT pgrdf.materialize(1);
SELECT * FROM pgrdf.sparql(
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   SELECT ?c WHERE { <http://example.com/alice> rdf:type ?c }'
);
```

## Who pgRDF is for

- **Application teams** who already use Postgres and want RDF
  capabilities without standing up a second database (GraphDB,
  Stardog, Virtuoso, Jena Fuseki).
- **Knowledge-graph builders** who want SQL joins between their
  RDF triples and their relational tables — pgRDF triples live in
  the same Postgres database as your `customers` and `orders`.
- **Researchers + tool authors** who need a strict-by-default,
  well-isolated triple store with cheap per-graph partition drops.

## What's in pgRDF today (Alpha, v0.6.x)

| Feature | Status |
|---|---|
| Triple storage (partitioned hexastore + dictionary) | ✅ |
| Per-graph LIST partitions with O(seconds) whole-graph drop | ✅ |
| Turtle ingest with `oxttl` (strict RFC 3987 IRIs) | ✅ |
| Literal datatypes, language tags, blank nodes, `rdf:List` | ✅ |
| Cross-backend shmem dict cache (LLD §4.1) | ✅ |
| Per-backend prepared-plan cache (LLD §4.2) | ✅ |
| Bulk-INSERT plan reuse (LLD §4.3 phase A) | ✅ |
| SPARQL SELECT / ASK with BGP + FILTER + DISTINCT/LIMIT/OFFSET/ORDER BY + OPTIONAL + UNION + MINUS + aggregates (COUNT, SUM, AVG, type-aware MIN/MAX, GROUP_CONCAT, SAMPLE) + HAVING (alias **and** inline aggregate) + BIND | ✅ |
| Named-graph SPARQL — `GRAPH <iri> { … }` + `GRAPH ?g { … }` with composition into OPTIONAL / UNION / MINUS | ✅ |
| SPARQL `CONSTRUCT` — constant / variable / blank-node / multi-triple templates, GRAPH-scoped WHERE, `CONSTRUCT WHERE { … }` shorthand, round-trip ingest (`pgrdf.construct` + `pgrdf.put_construct_rows`) | ✅ v0.4 |
| OWL 2 RL materialization via `reasonable` (`pgrdf.materialize`) + the `profile` selector (`'owl-rl'` default / `'rdfs'`) | ✅ v0.5 |
| TriG / N-Quads ingest (`pgrdf.parse_trig`, `pgrdf.parse_nquads`) | ✅ v0.5 |
| Operator surface — `pgrdf.stats()`, `pgrdf.shmem_reset()`, `pgrdf.plan_cache_clear()` | ✅ |
| Regression suite + W3C-shape SPARQL harness + W3C SHACL Core manifest gate in CI (PR-gate, every PG major) | ✅ v0.5 |
| SHACL Core validation — real W3C-shape report (`pgrdf.validate(data, shapes [, mode])`), full-pass 25/25; native SHACL-SPARQL via `mode => 'pgrdf'` (authoritative; the rudof `'sparql'` mode is upstream-incomplete — [ERRATA.v0.6 E-014](../specs/ERRATA.v0.6.md)) | ✅ v0.4 / v0.5 |
| Parallel bulk loader (`load_turtle(…, bulk_load => true)`) — 2.3–3.5× ingest on LUBM-250/500 | ✅ v0.6.2–v0.6.6 |
| LUBM-10 → LUBM-500 scaling benchmarks (full LUBM-100 pass) | ✅ v0.6 |
| Deeper `heap_multi_insert` / COPY BINARY quad insert (LLD §12 phase B) | ⏳ FUTURE |
| Full W3C SPARQL 1.1 TTL-manifest runner against `w3c/rdf-tests` | ⏳ FUTURE |
| LUBM cross-engine comparison (Jena TDB, Apache AGE) | ⏳ FUTURE |
| SPARQL surface — property paths (`^`/`+`/`*`/`?`/`\|` alternation) | ✅ v0.4.5 |
| SPARQL surface — VALUES, multi-triple OPTIONAL, DESCRIBE, type-aware ORDER BY, aggregates over UNION, downstream BIND | ✅ v0.5 |

For the long-form plan see
[`docs/10-roadmap.md`](../docs/10-roadmap.md).

## What pgRDF is NOT

- **Not a federated SPARQL endpoint.** `SERVICE` clauses are out of
  scope for the v0 series.
- **Not a full OWL 2 reasoner.** The OWL 2 RL profile is supported via
  the `reasonable` crate; EL/QL profiles aren't.
- **Not RDF-star.** Quoted triples in subject / object position are
  rejected at load time; the v0 series treats them as out-of-scope.
  Adding RDF 1.2 triple-term support is a documented upstream gate
  under [ERRATA E-011](../specs/ERRATA.v0.4.md) (gated on
  `gtfierro/reasonable#50` — the same patch that gates the crates.io
  publish), with the forward design in
  [SPEC.pgRDF.LLD.v0.6-FUTURE](../specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md).
- **Not a replacement for a graph database when you don't already
  have Postgres.** If you're starting from zero with a 100M-edge
  social graph, Neo4j or DuckDB-graph are likely better fits. pgRDF
  is for the case where Postgres is already your transactional
  source of truth.

## Naming + conventions

- The extension's schema is `pgrdf`. All UDFs are `pgrdf.<name>(...)`.
- All catalog relations are `pgrdf._pgrdf_*` (underscore prefix marks
  them as internal — query them, don't write to them directly).
- Graph identifiers are `BIGINT`. `0` is the default partition;
  `pgrdf.add_graph(g)` creates a dedicated LIST partition for `g`.
  Every graph also carries an IRI in `pgrdf._pgrdf_graphs`; allocate
  by IRI with `pgrdf.add_graph(iri TEXT)` and scope SPARQL with
  `GRAPH <iri> { … }` or `GRAPH ?g { … }`. See
  [02-loading-rdf.md](02-loading-rdf.md) for the loader surface and
  [03-querying.md](03-querying.md) for the SPARQL forms.

## Next

Install + first triples: see [01-install.md](01-install.md).
