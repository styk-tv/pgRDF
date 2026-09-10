# pgRDF user guide

Everything you need to install pgRDF, load data, and use it from your
application. The documentation site, [pgrdf.styk.tv](https://pgrdf.styk.tv),
has feature deep dives, worked examples and the internals.

## Start here

1. **[What pgRDF is](00-intro.md)**: capabilities, requirements, limits.
2. **[Install](01-install.md)**: Docker, an existing server, or from source.
3. **[A ten-minute tour](tour.md)**: load, query, reason, validate, lock
   and fingerprint a graph in one `psql` session.

## Topics

| Page | Covers |
|---|---|
| [02 — Loading RDF](02-loading-rdf.md) | Turtle, N-Triples, TriG, N-Quads; strings and files; bulk loading and tuning |
| [03 — Querying](03-querying.md) | SPARQL 1.1 query and update, named graphs, property paths, what's not supported |
| [04 — Reasoning](04-reasoning.md) | OWL 2 RL and RDFS materialization, freshness |
| [05 — Managing graphs](05-graphs.md) | inventory, copy / move / drop, carving, locks, integrity |
| [06 — Validation](06-validation-recipes.md) | SHACL Core and SHACL-SPARQL, reports, strict mode |
| [07 — Identity and export](07-identity-and-export.md) | canonical digests, N-Triples export, portable manifests |
| [08 — Errors and diagnostics](08-errors-and-diagnostics.md) | SQLSTATEs, completeness, settings, version checks |
| [09 — Function reference](09-function-reference.md) | every supported function, grouped by task |

## From your application

pgRDF is plain SQL, so every PostgreSQL driver works. These pages show
the common patterns: loading, querying, JSONB results and error codes.

| Language | Page |
|---|---|
| Python (psycopg, asyncpg, SQLAlchemy) | [clients/python.md](clients/python.md) |
| Node.js / TypeScript (pg, postgres.js) | [clients/typescript.md](clients/typescript.md) |
| Go (pgx) | [clients/go.md](clients/go.md) |
| Rust (tokio-postgres, sqlx) | [clients/rust.md](clients/rust.md) |

## Something wrong or unclear?

Open an issue at [styk-tv/pgRDF](https://github.com/styk-tv/pgRDF/issues).
