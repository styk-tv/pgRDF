# pgRDF

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-14%20%7C%2015%20%7C%2016%20%7C%2017-336791?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![pgrx](https://img.shields.io/badge/pgrx-0.16-cc6633?logo=rust&logoColor=white)](https://github.com/pgcentralfoundation/pgrx)
[![Rust](https://img.shields.io/badge/rust-stable-cc6633?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Status](https://img.shields.io/badge/status-alpha%20%E2%80%94%20v0.3%20engine%20feature--complete-yellow)](docs/10-roadmap.md)
[![Tests](https://img.shields.io/badge/tests-93%20pgrx%20%2B%2035%20regression%20%2B%2023%20W3C%20%2B%203%20LUBM-brightgreen)](#tests)
[![SPARQL](https://img.shields.io/badge/SPARQL-FILTER%20%2F%20OPTIONAL%20%2F%20UNION%20%2F%20MINUS%20%2F%20AGGREGATES-blue)](guide/03-querying.md)
[![ShmemCache](https://img.shields.io/badge/shmem%20dict%20cache-LLD%20%C2%A74.1-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![PlanCache](https://img.shields.io/badge/prepared%20plan%20cache-LLD%20%C2%A74.2-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![BulkIngest](https://img.shields.io/badge/bulk%20ingest-LLD%20%C2%A74.3%20phase%20A-yellow)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![Inference](https://img.shields.io/badge/inference-OWL%202%20RL%20via%20reasonable-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![Validation](https://img.shields.io/badge/SHACL%20validate-stub%20%28ERRATA%20E--009%29-orange)](specs/ERRATA.v0.2.md)
[![CI](https://github.com/styk-tv/pgRDF/actions/workflows/ci.yml/badge.svg)](https://github.com/styk-tv/pgRDF/actions/workflows/ci.yml)
[![W3C](https://img.shields.io/badge/W3C%20SPARQL%201.1-23%20starter%20tests-blue)](tests/w3c-sparql/)

**A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning.**

> Treat Postgres as the storage + execution engine for your knowledge
> graph. Load Turtle, query via SPARQL, validate via SHACL, materialize
> inferences via OWL 2 RL — all addressable from any Postgres client.

| | |
|---|---|
| **Status** | Alpha — **v0.3 engine surface feature-complete**. Storage CRUD + Turtle ingest. SPARQL SELECT/ASK with N-pattern BGPs + FILTER + DISTINCT/LIMIT/OFFSET/ORDER BY + OPTIONAL + UNION + MINUS + aggregates (COUNT, SUM, AVG, type-aware MIN/MAX, GROUP_CONCAT, SAMPLE) + HAVING (alias + inline aggregate) + BIND. **Phase 3 storage perf** (shmem dict cache §4.1, prepared-plan cache §4.2, prepared bulk-INSERT §4.3 phase A). **Phase 4 inference** — `pgrdf.materialize` via `reasonable` (OWL 2 RL). **Phase 5 SHACL** — `pgrdf.validate` surface stub (real impl blocked by [ERRATA E-009](specs/ERRATA.v0.2.md)). **Phase 6** — regression suite + W3C-shape harness + LUBM-shape gates in CI. Deferred to v0.4: heap_multi_insert for the 2× ingest target, full W3C TTL-manifest runner, real LUBM + cross-engine benchmarks. |
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

### OWL 2 RL inference

```sql
-- Load an ontology + some assertions
SELECT pgrdf.add_graph(100);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.com/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Engineer rdfs:subClassOf ex:Person .
ex:Person   rdfs:subClassOf ex:Agent .
ex:alice    rdf:type        ex:Engineer .
', 100);

-- Materialize OWL 2 RL entailments. Idempotent — call as often as
-- you like; the prior is_inferred=TRUE rows are dropped first.
SELECT pgrdf.materialize(100);
--  → {"base_triples": 3, "inferred_triples_written": 11, ...}

-- The 2-hop entailment is now in the table:
SELECT * FROM pgrdf.sparql(
  'PREFIX rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   PREFIX ex:   <http://example.com/>
   SELECT ?c WHERE { ex:alice rdf:type ?c }'
);
--  → {"c": "http://example.com/Engineer"}   ← base
--  → {"c": "http://example.com/Person"}     ← inferred
--  → {"c": "http://example.com/Agent"}      ← inferred
```

See [`guide/03-querying.md`](guide/03-querying.md) for the full
SELECT/ASK surface (BGPs with N patterns, FILTER expressions,
solution modifiers, OPTIONAL / UNION / MINUS, aggregates with
HAVING, BIND for projection, combining with regular SQL). For
operator-facing observability — `pgrdf.stats()`,
`pgrdf.shmem_reset()`, `pgrdf.plan_cache_clear()` — see
[`docs/02-storage.md`](docs/02-storage.md).

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
- [Clients › Node.js / TypeScript](guide/clients/typescript.md)
- [Clients › Go](guide/clients/go.md)

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

Current bar — **93 pgrx + 35 pg_regress + 23 W3C-shape + 3
LUBM-shape = 154 tests** green across the full pgrx PG 14-17
matrix and the compose-based regression runtime (PG 17). Covers:
- Storage CRUD + Turtle ingest (Phase 2.0-2.2).
- SPARQL SELECT/ASK surface (Phase 3 steps 1-12, plus inline
  `HAVING(SUM(?v) > c)` and type-aware MIN/MAX brought forward
  from v0.4).
- Storage performance (shmem dict cache, prepared-plan cache,
  prepared bulk-INSERT).
- OWL 2 RL inference (`pgrdf.materialize`) + the
  materialize → SPARQL integration round-trip.
- SHACL stub (real impl blocked by ERRATA E-009).
- Operator surface (`pgrdf.stats()` JSONB shape contract).
- 7 negative regression signals locking the error-message
  contract for unsupported SPARQL shapes
  (`80-unsupported-shapes.sql`).
- Error-path signals locking the stable error-prefix UDFs emit
  on invalid input (`81-error-paths.sql`); first lock-in:
  `load_turtle: failed to open` on a missing path.
- Edge-case correctness signals (`62-materialize-empty.sql` →
  forward): `pgrdf.materialize()` on an empty graph returns
  `base_triples = 0`, non-negative inferred-count, and stays
  idempotent across two calls.

External smoke covers 24 well-known ontologies (W3C, Apache Jena,
ValueFlows, ConceptKernel v3.7) for ~17,000 triples loaded;
runs via `tests/perf/smoke-ontologies.sh`. Workflow.ttl held out
due to a non-RFC IRI in the source — see
[ERRATA E-007 / TEST.ONTOLOGY-SET.md](TEST.ONTOLOGY-SET.md).

## License

Apache-2.0. See [LICENSE](LICENSE).
