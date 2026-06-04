![pgRDF](docs/pgRDF-logo.png)

# pgRDF

[![License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-14%20%7C%2015%20%7C%2016%20%7C%2017-336791?logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![pgrx](https://img.shields.io/badge/pgrx-0.16-cc6633?logo=rust&logoColor=white)](https://github.com/pgcentralfoundation/pgrx)
[![Rust](https://img.shields.io/badge/rust-stable-cc6633?logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![Status](https://img.shields.io/badge/status-v0.5.38%20%E2%80%94%20Track%20A%20spikes%20landing%20%E2%80%94%20RDF%20%2F%20SPARQL%201.1%20%2F%20SHACL%20%2F%20OWL-brightgreen)](docs/10-roadmap.md) [![LATEST.md](https://img.shields.io/badge/LATEST.md-current%20advertised%20version-blue)](./LATEST.md)
[![Tests](https://img.shields.io/badge/tests-274%20pgrx%20%2B%2085%20regression%20%2B%2051%20W3C%20%2B%2025%20SHACL%20%2B%203%20LUBM-brightgreen)](#tests)
[![SPARQL](https://img.shields.io/badge/SPARQL-SELECT%20%2F%20ASK%20%2F%20CONSTRUCT%20%2F%20DESCRIBE%20%2F%20UPDATE%20%2F%20PATHS%20%2F%20GRAPH%20%2F%20FILTER%20%2F%20OPTIONAL%20%2F%20UNION%20%2F%20MINUS%20%2F%20AGGREGATES-blue)](guide/03-querying.md)
[![ShmemCache](https://img.shields.io/badge/shmem%20dict%20cache-LLD%20%C2%A74.1-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![PlanCache](https://img.shields.io/badge/prepared%20plan%20cache-LLD%20%C2%A74.2-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![BulkIngest](https://img.shields.io/badge/bulk%20ingest-LLD%20%C2%A74.3%20phase%20A-yellow)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![Inference](https://img.shields.io/badge/inference-OWL%202%20RL%20via%20reasonable-success)](specs/SPEC.pgRDF.LLD.v0.3.md)
[![Validation](https://img.shields.io/badge/SHACL%20validate-SHACL%20Core%20via%20shacl%200.3.1-success)](docs/05-validation.md)
[![CI](https://github.com/styk-tv/pgRDF/actions/workflows/ci.yml/badge.svg)](https://github.com/styk-tv/pgRDF/actions/workflows/ci.yml)
[![W3C](https://img.shields.io/badge/W3C%20SPARQL%201.1-51%20shape%20tests-blue)](tests/w3c-sparql/)
[![W3C SHACL](https://img.shields.io/badge/W3C%20SHACL%20Core-25%2F25-blue)](docs/05-validation.md)

**A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning.**

> Treat Postgres as the storage + execution engine for your knowledge
> graph. Load Turtle, query via SPARQL, validate via SHACL, materialize
> inferences via OWL 2 RL — all addressable from any Postgres client.

| | |
|---|---|
| **Status** | **v0.5.38 — current advertised release ([LATEST.md](./LATEST.md)). Engine surface unchanged from v0.5.0; the v0.5.10..v0.5.38 cycle ships PGXN packaging, OCI distribution with SLSA Build Provenance v1 attestations, a 5-gate release-pipeline contract ([PROVENANCE.md Rule 7](./PROVENANCE.md)), Phase-0 ingest instrumentation (`parse_ms`/`dict_ms`/`insert_ms`), and additive Track A perf-spike UDFs (`parse_turtle_dict_batched` -17% e2e, `shmem_cache_prewarm` -54% e2e — both behind explicit opt-in surfaces; default `parse_turtle` path unchanged). Pin via `oras pull ghcr.io/styk-tv/pgrdf-bundle:0.5.38` or whatever `LATEST.md` advertises at audit time.**<br><br>**Query** — SPARQL SELECT/ASK over N-pattern BGPs · FILTER · DISTINCT/LIMIT/OFFSET · **type-aware ORDER BY** · **multi-triple OPTIONAL** · UNION · MINUS · aggregates (COUNT/SUM/AVG/type-aware MIN-MAX/GROUP_CONCAT/SAMPLE) **incl. over UNION** · HAVING · **downstream BIND** · **VALUES** · named-graph scoping (`GRAPH <iri>` + `GRAPH ?g` + composition) · **CONSTRUCT** (constant/variable/blank-node/multi-triple templates · WHERE-shorthand · round-trip ingest) · **DESCRIBE** (W3C §16.4 CBD via `pgrdf.describe`) · **property paths** (`^` `+` `*` `?` · `\|` alternation · materialised-closure no-CTE fallback · `pgrdf.path_max_depth` guard).<br>**Update** — full SPARQL UPDATE: INSERT/DELETE DATA · INSERT/DELETE WHERE · DELETE+INSERT WHERE · `WITH <iri>` scoping · lifecycle algebra (`DROP`/`CLEAR`/`CREATE GRAPH` × `DEFAULT`/`NAMED`/`ALL`).<br>**Storage** — CRUD + Turtle / **TriG** / **N-Quads** ingest (`parse_turtle` / `parse_trig` / `parse_nquads`) · per-graph LIST partitions · lifecycle UDFs (`drop`/`clear`/`copy`/`move_graph`, **BIGINT + IRI overloads**) · shmem dict cache (§4.1) + prepared-plan cache (§4.2) + prepared bulk-INSERT (§4.3 phase A).<br>**Inference** — `pgrdf.materialize(graph_id, profile)` — **`owl-rl` and `rdfs`** profiles. **Validation** — `pgrdf.validate(data, shapes, mode)` → real W3C `sh:ValidationReport` JSONB; SHACL Core native (genuine W3C SHACL Core 25/25); `mode=>'sparql'` is shipped + honest, upstream-gated ([ERRATA E-012](specs/ERRATA.v0.5.md)).<br><br>**Shipped on the v0.4/v0.5 countdown:** `v0.4.0` SHACL · `v0.4.1` named-graph §3 · `v0.4.2` lifecycle UDFs §5 · `v0.4.3` SPARQL UPDATE §4 · `v0.4.4` CONSTRUCT §6 · `v0.4.5` property paths §7 · `v0.4.6` §11 SPARQL backlog · **`v0.5.0` — the complete surface** (DESCRIBE, TriG/N-Quads, IRI lifecycle overloads, `rdfs`+`owl-rl` profiles, native SHACL Core 25/25).<br>**Documented upstream gates** (honest, not defects): [E-011](specs/ERRATA.v0.4.md) — RDF 1.2 triple terms + crates.io publish gated on `gtfierro/reasonable#50`; [E-012](specs/ERRATA.v0.5.md) — SHACL-SPARQL constraint execution gated on `rudof` (#21/#94); the `mode=>'sparql'` surface ships honest.<br>**Deferred → v0.6-FUTURE:** executor.rs core-BGP carve · `heap_multi_insert` phase B · real SHACL-SPARQL engine · federated SERVICE · incremental materialisation · RDF 1.2 (see [SPEC.pgRDF.LLD.v0.6-FUTURE](specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md)). |
| **Supported PG** | 14, 15, 16, 17. PG 18 adoption stays deferred — pgrx 0.16 pin; 0.18.0 still fails to build locally and changes the schema-gen model. See [ERRATA](specs/ERRATA.v0.2.md) E-006. |
| **Install** | Three paths. **GitHub-release tarball** — per-file `:ro` bind-mount of `.so`/`.control`/`.sql` into stock `postgres:17.4-bookworm` (8 tarballs: pg14-17 × amd64/arm64 + SHA256SUMS). **Anonymous OCI** — `oras pull ghcr.io/styk-tv/pgrdf-bundle:0.5.38` (zero credentials, public; pin to whatever [`LATEST.md`](./LATEST.md) advertises at audit time — every advertised digest carries an attested SLSA Build Provenance v1, verifiable via `gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:<tag> --repo styk-tv/pgRDF`). **PGXN source install** — `pgxn install pgrdf --pg_config /path/to/pg_config` on a host with Rust 1.91 + `cargo-pgrx 0.16` (see [INSTALL.md](INSTALL.md)). Per [SPEC.pgRDF.INSTALL.v0.2](specs/SPEC.pgRDF.INSTALL.v0.2.md). |
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
        --  → 0.5.38   (whatever LATEST.md currently advertises)
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

Current bar — **274 pgrx + 85 pg_regress + 51 W3C-sparql + 25
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
