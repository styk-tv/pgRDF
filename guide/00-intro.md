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

-- (Soon, Phase 2.2 step 5)
SELECT * FROM pgrdf.sparql('SELECT ?s ?n WHERE { ?s foaf:name ?n }');
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

## What's in pgRDF today (Alpha, Phase 2.2)

| Feature | Status |
|---|---|
| Triple storage (partitioned hexastore + dictionary) | ✅ |
| Per-graph LIST partitions with O(seconds) whole-graph drop | ✅ |
| Turtle ingest with `oxttl` (strict RFC 3987 IRIs) | ✅ |
| Literal datatypes, language tags, blank nodes, `rdf:List` | ✅ |
| Per-call ingest dict cache + batched INSERTs | ✅ |
| SPARQL 1.1 query (BGP-only first cut) | ⏳ next step |
| OWL 2 RL materialization | ⏳ Phase 3 |
| SHACL validation → JSONB report | ⏳ Phase 3 |
| W3C SPARQL 1.1 conformance ≥ 95% | ⏳ Phase 4 |

For the long-form plan see
[`docs/10-roadmap.md`](../docs/10-roadmap.md).

## What pgRDF is NOT

- **Not a federated SPARQL endpoint.** `SERVICE` clauses are out of
  scope for the v0 series.
- **Not a full OWL 2 reasoner.** The OWL 2 RL profile is supported via
  the `reasonable` crate; EL/QL profiles aren't.
- **Not RDF-star.** Quoted triples in subject / object position are
  rejected at load time; pgRDF v0.2 treats them as out-of-scope per
  SPEC.pgRDF.LLD §2.
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

## Next

Install + first triples: see [01-install.md](01-install.md).
