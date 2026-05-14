# pgRDF

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-14%20%7C%2015%20%7C%2016%20%7C%2017-336791?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![pgrx](https://img.shields.io/badge/pgrx-0.16-cc6633?logo=rust&logoColor=white)](https://github.com/pgcentralfoundation/pgrx)
[![Rust](https://img.shields.io/badge/rust-stable-cc6633?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Status](https://img.shields.io/badge/status-alpha%20%E2%80%94%20phase%203%20perf-yellow)](docs/10-roadmap.md)
[![Tests](https://img.shields.io/badge/tests-88%20pgrx%20%2B%2027%20regression-brightgreen)](#tests)
[![SPARQL](https://img.shields.io/badge/SPARQL-FILTER%20%2F%20OPTIONAL%20%2F%20UNION%20%2F%20MINUS%20%2F%20AGGREGATES-blue)](guide/03-querying.md)
[![ShmemCache](https://img.shields.io/badge/shmem%20dict%20cache-LLD%20%C2%A74.1-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![PlanCache](https://img.shields.io/badge/prepared%20plan%20cache-LLD%20%C2%A74.2-success)](specs/SPEC.pgRDF.LLD.v0.3.md)

**A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning.**

> Treat Postgres as the storage + execution engine for your knowledge
> graph. Load Turtle, query via SPARQL, validate via SHACL, materialize
> inferences via OWL 2 RL — all addressable from any Postgres client.

| | |
|---|---|
| **Status** | Alpha. Storage CRUD, Turtle ingest (Phase 2.0–2.2). SPARQL SELECT/ASK with N-pattern BGPs + FILTER + DISTINCT/LIMIT/OFFSET/ORDER BY + OPTIONAL + UNION + MINUS + aggregates (COUNT, SUM, AVG, MIN, MAX, GROUP_CONCAT, SAMPLE) + HAVING + BIND. **v0.3 Phase 3 step 1 (shmem dict cache, LLD §4.1) + step 2 (prepared-plan cache, LLD §4.2) shipped** — cross-backend term-id reuse + per-backend SPI plan reuse; SPARQL dynamic SQL is fully parameterised. |
| **Supported PG** | 14, 15, 16, 17 (PG 18 blocked on pgrx upstream — see [ERRATA](specs/ERRATA.v0.2.md) E-006). |
| **Install** | Drop-in via per-file bind mounts (local) or init-container fetch (K8s) per [SPEC.pgRDF.INSTALL.v0.2](specs/SPEC.pgRDF.INSTALL.v0.2.md). No image rebuild. |
| **Repo** | [styk-tv/pgRDF](https://github.com/styk-tv/pgRDF) |

## What you can do today

```sql
-- One-time install
CREATE EXTENSION pgrdf;

-- Load any Turtle file from the server-side filesystem
SELECT pgrdf.load_turtle('/fixtures/ontologies/foaf.ttl', 100);
--  → 631

-- See structured ingest stats (timing, cache hits, batches)
SELECT pgrdf.load_turtle_verbose('/fixtures/ontologies/prov.ttl', 200, 'http://www.w3.org/ns/prov#');
--  → {"triples": 1789, "dict_cache_hits": 4612, "dict_db_calls": 783, "quad_batches": 2, "elapsed_ms": 142.7}

-- Manage per-graph LIST partitions for cheap whole-graph drops
SELECT pgrdf.add_graph(42);
SELECT pgrdf.count_quads(42);

-- Inspect the dictionary directly
SELECT * FROM pgrdf._pgrdf_dictionary WHERE term_type = 1 LIMIT 5;
```

### SPARQL

```sql
-- Multi-pattern BGP, shared variables become joins
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?p ?n ?m
     WHERE { ?p foaf:name ?n .
             ?p foaf:mbox ?m }'
);
--  → {"p": "http://example.com/alice", "n": "Alice", "m": "mailto:a@x"}

-- FILTER over the BGP — identity, boolean composition, term-type tests
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?o
     WHERE { ?s ?p ?o FILTER(isIRI(?o) && ?p = foaf:knows) }'
);

-- Numeric ordering + REGEX in a single query
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n
     WHERE { ?s foaf:name ?n .
             ?s <http://example.com/age> ?age
             FILTER(?age >= 30 && REGEX(?n, "^A", "i")) }'
);

-- OPTIONAL — mbox stays NULL when the person has no foaf:mbox
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?m
     WHERE { ?s foaf:name ?n
             OPTIONAL { ?s foaf:mbox ?m } }'
);
--  → {"s": "http://example.com/alice", "n": "Alice", "m": "mailto:a@x"}
--  → {"s": "http://example.com/bob",   "n": "Bob",   "m": null}

-- UNION — either branch contributes solutions; unbound vars come as null
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?m
     WHERE { { ?s foaf:name ?n }
             UNION
             { ?s foaf:mbox ?m } }'
);

-- Aggregates with GROUP BY — count of triples per predicate
SELECT * FROM pgrdf.sparql(
  'SELECT ?p (COUNT(?o) AS ?n)
     WHERE { ?s ?p ?o }
   GROUP BY ?p ORDER BY DESC(?n)'
);
--  → {"p": "http://xmlns.com/foaf/0.1/name", "n": "4"}

-- Inspect the parsed shape without executing
SELECT pgrdf.sparql_parse('SELECT ?s WHERE { ?s ?p ?o OPTIONAL { ?s <http://x/n> ?n } }');
--  → {"form": "SELECT", ..., "unsupported_algebra": ["LeftJoin (OPTIONAL)"]}
```

See [`guide/03-querying.md`](guide/03-querying.md) for the full
SELECT surface (BGPs with N patterns, FILTER expressions, constants
in any position, combining with regular SQL). OPTIONAL / UNION /
numeric ordering / aggregates land in subsequent Phase 3 slices.

## Quickstart for users

Full walkthrough lives under [`guide/`](guide/). Five-minute path:

```bash
# 1. Boot stock postgres:17.4 with the extension files bind-mounted
just build-ext        # builds pgrdf.so/.control/.sql in a Linux container
just compose-up       # podman compose up -d
just psql             # opens a psql shell to the pgrdf database

# 2. Inside psql
pgrdf=# CREATE EXTENSION pgrdf;
pgrdf=# SELECT pgrdf.version();
        --  → 0.2.0
pgrdf=# SELECT pgrdf.parse_turtle('@prefix ex: <http://e.com/> . ex:a ex:p ex:b .', 1);
        --  → 1
```

Want to integrate from your application?

- **Python** — [`guide/clients/python.md`](guide/clients/python.md)
  (psycopg + asyncpg, plus a sketch of using pgRDF as an rdflib backend)
- **Rust** — [`guide/clients/rust.md`](guide/clients/rust.md)
  (tokio-postgres and sqlx examples)
- **Node.js / TypeScript** — [`guide/clients/typescript.md`](guide/clients/typescript.md)
  (`pg`, `postgres.js`, `pg-cursor` streaming, typed bindings)
- **Go** — [`guide/clients/go.md`](guide/clients/go.md)
  (`pgx` v5, `pgxpool`, bulk-ingest pattern, sqlc tie-in)

## Documentation

Two parallel doc tracks:

### Use documentation — [`guide/`](guide/)

For people running pgRDF in their applications.

- [00 — Introduction](guide/00-intro.md)
- [01 — Install](guide/01-install.md)
- [02 — Loading RDF](guide/02-loading-rdf.md)
- [03 — Querying with SPARQL](guide/03-querying.md)
- [Clients › Python](guide/clients/python.md)
- [Clients › Rust](guide/clients/rust.md)

### Engineering / build plan — [`docs/`](docs/)

For people working on pgRDF itself.

- [01 — Architecture](docs/01-architecture.md)
- [02 — Storage](docs/02-storage.md)
- [03 — Query](docs/03-query.md)
- [04 — Inference](docs/04-inference.md)
- [05 — Validation](docs/05-validation.md)
- [06 — Installation (spec walkthrough)](docs/06-installation.md)
- [07 — Development](docs/07-development.md)
- [08 — Testing](docs/08-testing.md)
- [09 — Release](docs/09-release.md)
- [10 — Roadmap](docs/10-roadmap.md)

### Authoritative specs

- [SPEC.pgRDF.LLD.v0.3.md](specs/SPEC.pgRDF.LLD.v0.3.md) — **current** low-level design (supersedes v0.2)
- [SPEC.pgRDF.LLD.v0.2.md](specs/SPEC.pgRDF.LLD.v0.2.md) — historical (§4.1/§4.2/§4.3 internals still referenced)
- [SPEC.pgRDF.INSTALL.v0.2.md](specs/SPEC.pgRDF.INSTALL.v0.2.md) — runtime install on stock PG containers (unchanged in v0.3)
- [ERRATA.v0.2.md](specs/ERRATA.v0.2.md) — corrections discovered during implementation

## Tests

| Layer | What it gates | Run |
|---|---|---|
| pgrx integration | UDF correctness inside a managed PG | `just test` |
| pg_regress-style | UDF correctness over the wire to compose Postgres | `just test-regression` |
| Ontology smoke | Real-world Turtle parses cleanly | `tests/perf/smoke-ontologies.sh` |
| Full bar | Both `just test` + `just test-regression` | `just test-all` |

Phase 2.0–2.2 + Phase 3 SPARQL surface (steps 1–12) + Phase 3 steps
1–2 storage perf (shmem dict cache + prepared-plan cache): **88 pgrx
integration tests + 27 regression files passing.** External smoke
covers 24
well-known ontologies (W3C, Apache Jena, ValueFlows, ConceptKernel
v3.7) for ~17,000 triples loaded. Workflow.ttl held out due to a
non-RFC IRI in the source — see [ERRATA E-007 / TEST.ONTOLOGY-SET.md](TEST.ONTOLOGY-SET.md).

## License

Apache-2.0. See [LICENSE](LICENSE).
