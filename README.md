![pgRDF](docs/pgRDF-logo.png)

# pgRDF

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-14%20%7C%2015%20%7C%2016%20%7C%2017-336791?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![pgrx](https://img.shields.io/badge/pgrx-0.16-cc6633?logo=rust&logoColor=white)](https://github.com/pgcentralfoundation/pgrx)
[![Rust](https://img.shields.io/badge/rust-stable-cc6633?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Status](https://img.shields.io/badge/status-v0.6.10%20%E2%80%94%20parallel%20bulk%20ingest%20%E2%80%94%20LUBM--100%20full%20pass%20%E2%80%94%20SPARQL%201.1%20%2F%20SHACL%20%2F%20OWL-brightgreen)](docs/10-roadmap.md) [![LATEST.md](https://img.shields.io/badge/LATEST.md-current%20advertised%20version-blue)](./LATEST.md)
[![Tests](https://img.shields.io/badge/tests-294%20pgrx%20%2B%2093%20regression%20%2B%2051%20W3C%20%2B%2025%20SHACL%20%2B%203%20LUBM-brightgreen)](#tests)
[![LUBM-100](https://img.shields.io/badge/LUBM--100-28%2F28%20queries%20%E2%89%A4%205s%20%C2%B7%2013.9M%20triples%20%C2%B7%20zero%20tuning-brightgreen)](tests/perf/lubm/RESULTS.m4-join-order.md)
[![Scale](https://img.shields.io/badge/scale-LUBM--500%20%C2%B7%20112M%20quads%20materialized-blue)](#proven-at-scale-lubm-10-to-lubm-500)
[![SPARQL](https://img.shields.io/badge/SPARQL-SELECT%20%2F%20ASK%20%2F%20CONSTRUCT%20%2F%20DESCRIBE%20%2F%20UPDATE%20%2F%20PATHS%20%2F%20GRAPH%20%2F%20FILTER%20%2F%20OPTIONAL%20%2F%20UNION%20%2F%20MINUS%20%2F%20AGGREGATES-blue)](guide/03-querying.md)
[![ShmemCache](https://img.shields.io/badge/shmem%20dict%20cache-LLD%20%C2%A74.1-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![PlanCache](https://img.shields.io/badge/prepared%20plan%20cache-LLD%20%C2%A74.2-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![BulkIngest](https://img.shields.io/badge/bulk%20ingest-LLD%20%C2%A74.3%20phase%20A-yellow)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![Inference](https://img.shields.io/badge/inference-OWL%202%20RL%20via%20reasonable-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![Validation](https://img.shields.io/badge/SHACL%20validate-SHACL%20Core%20via%20shacl%200.3.2-success)](docs/05-validation.md)
[![CI](https://github.com/styk-tv/pgRDF/actions/workflows/ci.yml/badge.svg)](https://github.com/styk-tv/pgRDF/actions/workflows/ci.yml)
[![W3C](https://img.shields.io/badge/W3C%20SPARQL%201.1-51%20shape%20tests-blue)](tests/w3c-sparql/)
[![W3C SHACL](https://img.shields.io/badge/W3C%20SHACL%20Core-25%2F25-blue)](docs/05-validation.md)

**A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning.**

> Treat Postgres as the storage + execution engine for your knowledge
> graph. Load Turtle, query via SPARQL, validate via SHACL, materialize
> inferences via OWL 2 RL — all addressable from any Postgres client.

pgRDF turns a single PostgreSQL instance into a complete semantic-web
engine — dictionary-encoded hexastore storage, a SPARQL 1.1 query **and**
update engine, a W3C-conformant SHACL Core validator, and an OWL 2 RL
reasoner — with no sidecar triple store and no second system to operate.
It began as a SELECT/ASK triple store and grew, release over release, into
the full SPARQL 1.1 surface (CONSTRUCT, DESCRIBE, property paths,
aggregates, named graphs, the complete UPDATE algebra), genuine W3C SHACL
Core conformance (25/25), OWL 2 RL **and** RDFS materialisation, and a
benchmark record running from LUBM-10 to a **112-million-quad** materialised
LUBM-500 closure — every release CI-built and signed with SLSA Build
Provenance v1.

## The LUBM-100 milestone

pgRDF now completes the **full LUBM-100 benchmark** — the standard,
generator-verified benchmark for RDF stores ([Lehigh University
Benchmark](https://swat.cse.lehigh.edu/projects/lubm/), 100 universities,
14 reference queries) — on ordinary hardware with **zero database tuning**:

| Measured | Result |
|---|---|
| Load 13,879,970 triples (Turtle) | **3 min 29 s** |
| OWL 2 RL reasoning → 22.5M facts, statistics refreshed automatically | **4 min 54 s** |
| All 14 queries on the loaded graph | **each ≤ 3 s** |
| All 14 queries after reasoning | **each ≤ 5 s** |

Environment: a laptop — Apple-silicon VM (8 vCPU / 32 GiB), stock
`postgres:17.4-bookworm` in Docker, **default PostgreSQL configuration**.
No manual indexes, no `ANALYZE`, no planner hints, no extension settings.
Full per-query tables and methodology:
[tests/perf/lubm/RESULTS.m4-join-order.md](tests/perf/lubm/RESULTS.m4-join-order.md).

Two engine changes close the gap from "minutes-to-timeout" to "seconds"
(shipped v0.5.45 + v0.5.46, both automatic):

- **Connected join ordering** — SPARQL graph patterns are lowered to SQL
  in a connected, selectivity-aware order and the plan is pinned, so
  multi-hop joins can never degrade into cross-product plans
  (benchmark query Q2: **649 s → 3 s** on 13.9M triples).
- **Automatic statistics after reasoning** — `pgrdf.materialize`
  refreshes planner statistics when it writes the inference closure
  (`pgrdf.auto_analyze`, default on), so queries stay fast on the
  enlarged graph (Q2 after reasoning: **timeout → 5 s**).
- **Batched closure write-back** (v0.6.1) — `materialize` writes its
  8.6M-triple inference closure in bulk batches rather than row-at-a-time,
  cutting the reasoning step **10 min 8 s → 4 min 54 s** at LUBM-100.

The result holds end-to-end: load a real-scale graph, reason over it,
and query it interactively — in one PostgreSQL instance, with the
operational surface (backups, monitoring, access control) you already
run. Verification bar at this cut: 294 integration + 93 regression +
51 W3C SPARQL + 25 W3C SHACL Core tests green, releases signed with
SLSA Build Provenance v1, three install paths (tarball / OCI / PGXN).

## Proven at scale: LUBM-10 to LUBM-500

Beyond the single-laptop LUBM-100 milestone above, pgRDF has been run end
to end across the full LUBM ladder on a dedicated 32-vCPU / 256 GiB box
(Azure `Standard_E32as_v7`, native PostgreSQL 17) — load → index → OWL-RL
materialise → SPARQL — with **every result correctness-gated against the
known LUBM answer counts**:

| LUBM-N | base triples | ingest | index | materialize (OWL-RL) * | total quads (closure) |
|---|---|---|---|---|---|
| 10  | 1.32M | 3s   | 1s  | 15s     | 2.13M |
| 100 | 13.9M | 34s  | 8s  | 4m 37s  | 22.46M |
| 250 | 34.5M | 105s | 15s | 10m 9s  | 55.88M |
| 500 | 69.1M | 192s | 47s | ~43m    | **111.83M** |

LUBM-500 builds a full materialised closure of **111.8 million quads** on a
single box (peak 146 / 256 GiB RAM) — load, reason, and query in one
PostgreSQL instance, no sharding.

<sub>\* OWL-RL materialisation is the dominant cost at scale and is single-thread-bound upstream — tracked in [#1](https://github.com/styk-tv/pgRDF/issues/1) (proposal: [gtfierro/reasonable#57](https://github.com/gtfierro/reasonable/issues/57)).</sub>

### Parallel bulk ingest — landing in the v0.6.x line

The ingest column above uses the new **parallel bulk loader** (landing in
v0.6.2). Profiling the serial loader on the 32-vCPU box showed it pinned to
~1 core, with **66–74% of ingest spent in dictionary resolution** — an
anti-join `INSERT … WHERE NOT EXISTS` plus a lookup `JOIN` over a *growing*
term index, super-linear and untouched by any config knob. The rewrite
parses across all cores (rayon), resolves triple→id in memory, and — on a
fresh load — assigns dictionary ids in Rust and bulk-inserts them, so both
heavy SQL statements **disappear from the query profile**:

| dataset | triples | serial ingest | parallel ingest | speed-up |
|---|---|---|---|---|
| LUBM-100 | 13.9M | 74–183s | **34s** | up to 5× |
| LUBM-250 | 34.5M | 240s | **105s** | 2.3× |
| LUBM-500 | 69.1M | 667s | **192s** | **3.5×** |

The advantage **grows with scale**: per-triple ingest stays near-linear
(~2.3–3.0 µs from LUBM-10 to 500) where the serial path was super-linear
(5.3 → 9.65 µs). It ships as a one-flag option —
`pgrdf.load_turtle(path, graph, bulk_load => true)` — with a safe automatic
fallback to the standard path whenever the dictionary is already populated.
At scale (above `pgrdf.bulk_defer_index_min`, v0.6.3) the same flag also
defers the hexastore + dictionary indexes and rebuilds them in parallel after
the heap-only load — the separate `index` column in the table above.

## Capabilities

Everything below runs inside one PostgreSQL instance, addressable from any client — no sidecar store, no ETL.

### Query — SPARQL 1.1

SELECT / ASK over N-pattern basic graph patterns, lowered to SQL joins on a pinned, cross-product-proof plan.

- **Filters** — identity, boolean composition, term-type tests, `REGEX`, numeric & typed comparison
- **Modifiers** — `DISTINCT`, `LIMIT` / `OFFSET`, type-aware `ORDER BY`
- **Patterns** — multi-triple `OPTIONAL`, `UNION`, `MINUS`, `VALUES`, downstream `BIND`
- **Aggregates** — `COUNT` / `SUM` / `AVG` / `MIN` / `MAX` / `GROUP_CONCAT` / `SAMPLE` with `GROUP BY` / `HAVING`, including over `UNION`
- **CONSTRUCT** and **DESCRIBE** (W3C §16.4 Concise Bounded Description)
- **Property paths** — `^` `+` `*` `?` `|`, with a materialised-closure no-CTE fast path and a depth guard
- **Named graphs** — `GRAPH <iri>` and `GRAPH ?g`, composed across OPTIONAL / UNION / MINUS

### Update — SPARQL 1.1 UPDATE

`INSERT` / `DELETE DATA`, `INSERT` / `DELETE WHERE`, `DELETE`+`INSERT WHERE`, `WITH <iri>` scoping, and the graph lifecycle algebra (`DROP` / `CLEAR` / `CREATE GRAPH` × `DEFAULT` / `NAMED` / `ALL`).

### Storage

Dictionary-encoded terms over a LIST-partitioned hexastore (SPO / POS / OSP covering indexes).

- **Ingest** — Turtle, TriG, N-Quads (`parse_turtle` / `parse_trig` / `parse_nquads`), plus the **parallel bulk loader** (`load_turtle(…, bulk_load => true)` — 2.3–3.5× on a fresh load, new in v0.6.2)
- **Per-graph lifecycle** — `drop` / `clear` / `copy` / `move_graph`, with BIGINT and IRI overloads
- **Performance** — cross-backend shared-memory dictionary cache, prepared-plan cache, prepared bulk-INSERT

### Inference — OWL 2 RL + RDFS

`pgrdf.materialize(graph, profile)` forward-chains the closure (`owl-rl` or `rdfs`), refreshes planner statistics automatically so queries stay fast on the enlarged graph, and is idempotent across calls.

### Validation — W3C SHACL Core

`pgrdf.validate(data, shapes, mode)` returns a real `sh:ValidationReport` as JSONB — genuine W3C SHACL Core conformance (25 / 25).

> **Honest scope.** A few surfaces are gated on upstream crates, not defects: RDF 1.2 triple terms + crates.io publish ([E-011](specs/ERRATA.v0.4.md) · `gtfierro/reasonable#50`) and SHACL-SPARQL constraint execution ([E-012](specs/ERRATA.v0.5.md) · `rudof`); the `mode => 'sparql'` surface ships honest. Forward backlog: [SPEC.pgRDF.LLD.v0.6-FUTURE](specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md).

### Supported PostgreSQL & install

| | |
|---|---|
| **PostgreSQL** | 14 · 15 · 16 · 17 (PG 18 deferred — pgrx 0.16 pin; [ERRATA E-006](specs/ERRATA.v0.2.md)) |
| **Install** | **OCI** — `oras pull ghcr.io/styk-tv/pgrdf-bundle:0.6.10` (public, zero-cred; every digest SLSA-attested, verify with `gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:<tag> --repo styk-tv/pgRDF`) · **tarballs** (pg14–17 × amd64/arm64) · **PGXN** — `pgxn install pgrdf`. See [INSTALL.md](INSTALL.md). |
| **Current release** | **v0.6.10** — [LATEST.md](./LATEST.md) is authoritative at audit time |
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

-- Named-graph SPARQL — GRAPH ?g binds the graph IRI per match
SELECT pgrdf.add_graph(101::bigint, 'http://example.org/g1');
SELECT pgrdf.add_graph(102::bigint, 'http://example.org/g2');
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?g (COUNT(*) AS ?n)
     WHERE { GRAPH ?g { ?s foaf:name ?n } }
   GROUP BY ?g ORDER BY ?g'
);
--  → {"g": "http://example.org/g1", "n": "3"}
--  → {"g": "http://example.org/g2", "n": "2"}

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
        --  → 0.6.10   (whatever LATEST.md currently advertises)
pgrdf=# SELECT pgrdf.parse_turtle('@prefix ex: <http://e.com/> . ex:a ex:p ex:b .', 1);
        --  → 1
```

### Required `postgresql.conf` changes

pgRDF MUST be in `shared_preload_libraries` for `_PG_init()` to run in the
postmaster context. Without it, the extension's shared-memory atomics (dict
cache + plan-cache stats) are never registered, and the first call to any
pgRDF function panics with `PgAtomic was not initialized`.

```ini
# postgresql.conf
shared_preload_libraries = 'pgrdf'         # pgRDF alone
# or:
shared_preload_libraries = 'pgrdf,pgck'    # if pgCK is also installed
                                           # — order matters: pgrdf first
```

A server restart (not just a reload) is required after editing this — preload
happens at postmaster startup. Verify after restart:

```sql
SHOW shared_preload_libraries;             -- must contain 'pgrdf'
SELECT pgrdf.parse_turtle(
  'PREFIX ex: <http://example.org/> ex:t a ex:T .', 1::bigint, 'http://example.org/');
                                           -- returns a row count, not a panic
```

The `just compose-up` Quickstart above bakes this into the bundled image;
only own-Postgres installs need to edit `postgresql.conf` manually.

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

- [SPEC.pgRDF.LLD.v0.5.md](specs/SPEC.pgRDF.LLD.v0.5.md) — **current** authoritative low-level design (supersedes v0.4)
- [SPEC.pgRDF.LLD.v0.6-FUTURE.md](specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md) — forward backlog (executor.rs core-BGP carve, `heap_multi_insert` phase B, real SHACL-SPARQL engine, federated SERVICE, incremental materialisation, RDF 1.2)
- [SPEC.pgRDF.LLD.v0.3.md](specs/SPEC.pgRDF.LLD.v0.3.md) — historical (§4.1/§4.2/§4.3 internals still referenced)
- [SPEC.pgRDF.INSTALL.v0.2.md](specs/SPEC.pgRDF.INSTALL.v0.2.md) — runtime install on stock PG containers
- [ERRATA.v0.5.md](specs/ERRATA.v0.5.md) / [ERRATA.v0.4.md](specs/ERRATA.v0.4.md) / [ERRATA.v0.2.md](specs/ERRATA.v0.2.md) — corrections + documented upstream gates discovered during implementation

## Tests

| Layer | What it gates | Run |
|---|---|---|
| pgrx integration | UDF correctness inside a managed PG | `just test` |
| pg_regress-style | UDF correctness over the wire to compose Postgres | `just test-regression` |
| Artifact parity | Mounted extension bytes match a fresh build and the live container | `just test-artifact-parity` |
| W3C-shape SPARQL | Per-test data.ttl + query.rq vs expected.jsonl | `just test-w3c` |
| LUBM-shape | LUBM-style correctness gates against a hand-authored fixture | `just test-lubm` |
| Ontology smoke | Real-world Turtle parses cleanly | `tests/perf/smoke-ontologies.sh` |
| Narrow bar | `just test` + `just test-regression` (back-compat shape) | `just test-all` |
| Compose-based bar | regression + W3C-shape + LUBM-shape | `just test-conformance` |
| Full bar | pgrx integration + test-conformance — the broadest sweep | `just test-everything` |
| Cold-compose smoke | Wipe compose, rebuild, re-up, run test-conformance | `just smoke-cold` |

`just test-everything` is the comprehensive entry point; `just
smoke-cold` is the cold-compose verification (it now includes
artifact-parity proof after rebuild, before the compose-based test
bar). Use it after touching anything in `compose/`, `fixtures/`, or
the test SQL fixtures.

Current bar — **294 pgrx + 93 pg_regress + 51 W3C-sparql + 25
W3C SHACL Core + 3 LUBM-shape** green across the full pgrx PG
14-17 matrix and the compose-based regression runtime (PG 17).
Covers:
- Storage CRUD + Turtle / TriG / N-Quads ingest.
- The full SPARQL 1.1 SELECT/ASK/CONSTRUCT/DESCRIBE surface
  (type-aware ORDER BY, multi-triple OPTIONAL, UNION, MINUS,
  VALUES, downstream BIND, aggregates incl. over UNION, HAVING,
  property paths).
- SPARQL UPDATE (INSERT/DELETE DATA + WHERE, DELETE+INSERT,
  `WITH` scoping, lifecycle algebra).
- Storage performance (shmem dict cache, prepared-plan cache,
  prepared bulk-INSERT).
- OWL 2 RL + RDFS inference (`pgrdf.materialize`, `owl-rl` /
  `rdfs` profiles) + the materialize → SPARQL round-trip.
- Genuine W3C SHACL Core validation (`pgrdf.validate`) — 25/25
  SHACL Core conformance, emitting a W3C `sh:ValidationReport`
  JSONB; `mode=>'sparql'` shipped + honest, upstream-gated
  (ERRATA E-012).
- Named-graph surface (LLD v0.4 §3) — `_pgrdf_graphs` system
  table + `pg_extension_config_dump` registration for pg_dump
  round-trip; the five-UDF surface
  (`add_graph(id)` / `add_graph(iri)` / `add_graph(id, iri)` /
  `graph_id(iri)` / `graph_iri(id)`); SPARQL `GRAPH <iri>`
  literal + `GRAPH ?g` variable forms with per-pattern scope
  composition over OPTIONAL / UNION / MINUS. Pg_regress fixtures
  `72-79` + `87`, pgrx tests in `src/storage/graphs.rs` +
  `src/query/executor.rs`, W3C-shape fixtures
  `24-graph-named-iri` / `25-graph-var-projection` /
  `26-graph-var-groupby`, and the
  `tests/regression/scripts/pg-dump-roundtrip.sh` shell-driven
  end-to-end round-trip.
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

External smoke covers **24 well-known ontologies → 17,134 triples**
(W3C, Apache Jena, ValueFlows, ConceptKernel v3.7); runs via
`tests/perf/smoke-ontologies.sh`. Per-ontology triple counts are
locked in [`tests/perf/smoke-ontologies.expected.tsv`](tests/perf/smoke-ontologies.expected.tsv);
`tests/perf/smoke-ontologies.sh --check` re-runs the smoke and
diffs against the lock-file (not gated in CI yet — the fetched
payloads are gitignored). Workflow.ttl held out due to a non-RFC
IRI in the source — see
[ERRATA E-007 / TEST.ONTOLOGY-SET.md](TEST.ONTOLOGY-SET.md).

## License

Copyright 2026 Peter Styk. Licensed under the MIT License — see [LICENSE](LICENSE) for the canonical attribution.

Project home: <https://github.com/styk-tv/pgRDF>.
