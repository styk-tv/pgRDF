# pgRDF user guide

This directory is the **use documentation** — written for people who
want to run pgRDF in their applications, not for people working on
the extension's internals (those docs live under [`../docs/`](../docs/)).

The user guide is intentionally separate so changes here don't
require touching the engineering plan, and vice versa.

## Read in order

| Page | Scope |
|---|---|
| [00-intro.md](00-intro.md) | What pgRDF is, who it's for, what it does + does not do today. |
| [01-install.md](01-install.md) | Local-dev install (compose) and Kubernetes install (init-container drop-in). |
| [02-loading-rdf.md](02-loading-rdf.md) | `pgrdf.load_turtle`, `pgrdf.parse_turtle`, integer + IRI-keyed graph allocation (`pgrdf.add_graph` × 3 overloads, `pgrdf.graph_id`, `pgrdf.graph_iri`), the verbose stats UDFs. |
| [03-querying.md](03-querying.md) | `pgrdf.sparql` — SPARQL SELECT over BGPs, FILTER / OPTIONAL / UNION / MINUS / aggregates / BIND / ASK, named-graph `GRAPH <iri> { … }` and `GRAPH ?g { … }` scoping, JSONB row shape, combining with regular SQL. |

## Client integrations

| Client | Page |
|---|---|
| Python (psycopg, asyncpg, rdflib bridge sketch) | [clients/python.md](clients/python.md) |
| Rust (tokio-postgres, sqlx) | [clients/rust.md](clients/rust.md) |
| Node.js / TypeScript (`pg`, `postgres.js`) | [clients/typescript.md](clients/typescript.md) |
| Go (`pgx` v5) | [clients/go.md](clients/go.md) |

Java (JDBC), Ruby (`pg`), and the rest of the Postgres ecosystem
connect identically — every pgRDF capability is a SQL UDF. Example
pages for those land as the surface stabilizes.

## Reporting back

If something here is wrong or unclear, open an issue at
[styk-tv/pgRDF](https://github.com/styk-tv/pgRDF/issues) — the user
guide is meant to evolve with the surface, and friction reports are
the cheapest way to improve it.
