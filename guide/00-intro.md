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

## What's in pgRDF today (Alpha, v0.3 engine feature-complete)

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
| OWL 2 RL materialization via `reasonable` (`pgrdf.materialize`) | ✅ |
| Operator surface — `pgrdf.stats()`, `pgrdf.shmem_reset()`, `pgrdf.plan_cache_clear()` | ✅ |
| Regression suite + W3C-shape harness in CI (PR-gate + nightly) | ✅ |
| SHACL validation — surface stub; real impl blocked by [ERRATA E-009](../specs/ERRATA.v0.2.md) | 🚧 |
| 2× ingest target (true COPY BINARY / heap_multi_insert) | ⏳ v0.4 |
| Full W3C SPARQL 1.1 TTL-manifest runner against `w3c/rdf-tests` | ⏳ v0.4 |
| LUBM-10 / LUBM-100 cross-engine benchmarks (Jena TDB, Apache AGE) | ⏳ v0.4 |
| SPARQL surface — VALUES, property paths beyond simple seq, multi-triple OPTIONAL, CONSTRUCT, DESCRIBE, aggregates over UNION, BIND-in-FILTER | ⏳ v0.4 |

For the long-form plan see
[`docs/10-roadmap.md`](../docs/10-roadmap.md).

## What pgRDF is NOT

- **Not a federated SPARQL endpoint.** `SERVICE` clauses are out of
  scope for the v0 series.
- **Not a full OWL 2 reasoner.** The OWL 2 RL profile is supported via
  the `reasonable` crate; EL/QL profiles aren't.
- **Not RDF-star.** Quoted triples in subject / object position are
  rejected at load time; pgRDF v0.3 treats them as out-of-scope.
  Adding RDF 1.2 triple-term support is tracked under
  [ERRATA E-009](../specs/ERRATA.v0.2.md) (the same feature-unification
  issue that blocks the real SHACL impl).
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
