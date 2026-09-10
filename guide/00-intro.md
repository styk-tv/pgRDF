# 00 — What pgRDF is

pgRDF is a PostgreSQL extension that turns a PostgreSQL database into
an RDF store. You load RDF and query it with SPARQL. You can reason
over it with OWL 2 RL or RDFS, validate it with SHACL, and fingerprint
and export it, all through SQL functions in the same database as the
rest of your data.

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.add_graph('http://example.org/g');
SELECT pgrdf.parse_turtle('<http://example.org/a> <http://example.org/knows> <http://example.org/b> .',
                          pgrdf.graph_id('http://example.org/g'));
SELECT * FROM pgrdf.sparql('SELECT ?s ?o WHERE { ?s <http://example.org/knows> ?o }');
```

There is no separate server, protocol or sync job: any PostgreSQL
client (psql, psycopg, JDBC, node-postgres, pgx, …) already works.

## What you can do with it

| Area | Highlights | Guide |
|---|---|---|
| Load | Turtle, N-Triples, TriG, N-Quads; inline strings or server-side files; parallel bulk loading for very large dumps | [02](02-loading-rdf.md) |
| Query | SPARQL 1.1 `SELECT` / `ASK` / `CONSTRUCT` / `DESCRIBE`, aggregates, property paths, named graphs | [03](03-querying.md) |
| Update | SPARQL 1.1 UPDATE: `INSERT` / `DELETE` `DATA` and `WHERE`, graph management | [03](03-querying.md#sparql-update) |
| Reason | OWL 2 RL or RDFS materialization, stored beside your data | [04](04-reasoning.md) |
| Manage graphs | inventory, copy / move / clear / drop, carve subgraphs, write locks, integrity checks | [05](05-graphs.md) |
| Validate | W3C SHACL Core plus SHACL-SPARQL, returning a JSONB report | [06](06-validation-recipes.md) |
| Identify and export | canonical graph digests (W3C RDFC-1.0), canonical N-Triples export, a portable manifest | [07](07-identity-and-export.md) |
| Diagnose | typed SQLSTATE errors, per-query completeness, capability listing | [08](08-errors-and-diagnostics.md) |

## Who it's for

- **Teams already on PostgreSQL** who want RDF, SPARQL or SHACL without
  running a second database.
- **Knowledge-graph builders** who want to join graph data with
  relational tables in one query.
- **Tooling and pipeline authors** who need validation, reasoning and
  reproducible graph fingerprints in one place.

## Scale

The same extension handles a few triples and a few billion. A complete
Wikidata "truthy" dump (8.2 billion triples) has been loaded into a
single instance, and the full load → reason → query pipeline has run at
LUBM-500 scale (a 112-million-quad materialized closure).

## Requirements

- **PostgreSQL 18**, loaded with `shared_preload_libraries = 'pgrdf'`.
- **Linux on x86-64 or arm64, glibc 2.39 or newer**: Debian 13,
  Ubuntu 24.04 or later, or the official `postgres:18` Docker image.
  musl-based systems such as Alpine are not supported.
- A superuser to run `CREATE EXTENSION pgrdf`.

Hosted services such as Amazon RDS, Cloud SQL or Azure Database only
allow extensions from their own catalogue, and pgRDF is not in them.
Run your own PostgreSQL (a VM, Kubernetes, or a provider that allows
custom extensions).

## What it doesn't do

- **Federated queries** (`SERVICE`) are not supported.
- **Reasoning** covers the OWL 2 RL and RDFS profiles, not full OWL 2
  DL, EL or QL.
- **RDF-star / RDF 1.2 quoted triples** are rejected at load time.
- **A few SPARQL functions and forms** are not available yet. The
  [list is short](03-querying.md#not-supported).

## Conventions

- Everything lives in the `pgrdf` schema: functions are called as
  `pgrdf.<name>(…)`. Run `SET search_path = pgrdf, public;` to drop the
  prefix.
- Graphs are identified by IRI. SQL functions take the graph's numeric
  id, which `pgrdf.graph_id(iri)` looks up.
- Tables whose names start with `pgrdf._pgrdf_` are internal storage.
  Read graph information through `pgrdf.graph_inventory()` and the
  other functions instead. Internal tables can change between
  releases.
- `pgrdf.surface()` lists every function and whether it is part of
  the supported API.

## Next

[01 — Install](01-install.md), or jump straight into the
[ten-minute tour](tour.md).
