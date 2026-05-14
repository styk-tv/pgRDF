# 08 — Testing strategy

Five layers, each with a coverage gate that ratchets upward per phase.
A phase is **complete** only when its column in the table below is
green. CI enforces the current phase's gates; nightly runs the upper
layers as a forward-looking signal.

## Layer matrix

Phase numbering matches the (revised) [`10-roadmap.md`](10-roadmap.md):
Phase 1 = core storage + build automation; Phase 2 = query engine
+ storage performance; Phase 3 = extended SPARQL surface
(current — through step 6 / MINUS); Phase 4 = inference + validation;
Phase 5 = release + LUBM.

| Layer | Runtime | Phase 1 | Phase 2 | Phase 3 (current) | Phase 4 | Phase 5 |
|---|---|---|---|---|---|---|
| Rust unit (`cargo test`) | sec | smoke only | parser AST coverage | filter/op coverage | reasoner correctness | full storage coverage |
| pgrx integration (`cargo pgrx test`) | ~30s | CREATE EXTENSION + drop ✅ | ingest + basic SPARQL ✅ | **56 ✅** | inference materialization | SHACL validation |
| pg_regress golden | ~1min | basic schema ✅ | core SPARQL ✅ | **19 ✅** | inference + query | full SHACL conformance |
| Ontology smoke | sec each, manual | — | 24 ontologies, 17 134 triples ✅ | (same set) | (same set) | (same set) |
| W3C SPARQL 1.1 | min | scaffolded | ≥ 30 % pass | runner not wired ⏳ | ≥ 70 % pass | ≥ 95 % pass |
| W3C SHACL | min | scaffolded | runner runs | not wired ⏳ | ≥ 50 % pass | ≥ 90 % pass |
| LUBM perf | min | — | LUBM-1 smoke | LUBM-10 baseline | LUBM-10 (carry) | LUBM-100 vs Jena/AGE |

Test counts are absolute (cumulative, not per-phase). The number
ratchets with every commit on `main`; a green build is required to
merge.

Coverage gates per phase live in [10-roadmap.md](10-roadmap.md).

## Layer 1 — Rust unit tests

Plain `cargo test`. Used for pure-Rust logic that doesn't need a live
Postgres backend: SPARQL parser shape, JSONB shaping, etc.

```bash
cargo test --no-default-features --features pg17
```

PG 18 deferred per ERRATA E-006.

## Layer 2 — pgrx integration (`#[pg_test]`)

`cargo pgrx test` spins up a managed Postgres, installs the extension,
and runs annotated `#[pg_test]` functions. The smoke test in
`src/lib.rs` (`test_version_matches_cargo`) is the canonical example;
the bulk of the surface lives in `src/{storage,query}/*.rs::tests`.

On a Mac host this runs inside the Colima/docker builder container
via `just test`; native macOS pgrx test still hits link errors.

```bash
just test           # = cargo pgrx test pg17 inside the Linux builder
```

## Layer 3 — pg_regress golden tests

`tests/regression/sql/*.sql` runs against the compose Postgres; each
file's stdout is diffed against `tests/regression/expected/*.out`.

```bash
just test-regression
just test-all       # = just test && just test-regression
```

New tests start by baselining (`ACCEPT=1 just test-regression`), but
the discipline is to hand-compute expected outputs from the SQL
fixture and **never use ACCEPT for new query coverage** — that defeats
the empirical-verification goal. ACCEPT is reserved for unrelated
output-format churn (e.g. a Postgres minor-version output change).

## Layer 3.5 — ontology smoke (manual)

`tests/perf/smoke-ontologies.sh` loads each TTL under
`fixtures/ontologies/` via `pgrdf.load_turtle` and prints the
triple count. Used to catch regressions in the Turtle parser
against real-world ontologies (FOAF, PROV, SKOS, RDFS, OWL, etc. —
24 ontologies, 17 134 triples on the 2026-05-13 fetch). Not in the
CI gate today; the fetched ontologies live under a gitignored
directory.

```bash
fixtures/ontologies.sh        # one-time fetch (also gitignored output)
tests/perf/smoke-ontologies.sh
```

## Layer 4 — W3C SPARQL 1.1 (⏳ not wired)

W3C maintains the SPARQL 1.1 test suite at `w3c/rdf-tests`. The
plan is to pull it as a git submodule and run a manifest-driven
runner against pgRDF. **Not wired today.** The runner crate
`pgrdf-w3c-sparql` exists only in the LLD; landing it is a
Phase 4 deliverable.

```bash
# (planned)
git submodule update --init tests/w3c-sparql/fixtures
cargo run -p pgrdf-w3c-sparql -- tests/w3c-sparql/fixtures/sparql11/manifest.ttl
```

## Layer 5 — W3C SHACL (⏳ not wired)

Mirror of layer 4 against the `w3c/data-shapes` test suite.
Lands with Phase 4.

## Layer 6 — LUBM perf (⏳ not wired)

LUBM (Lehigh University Benchmark) is the de facto OWL/SPARQL store
benchmark. We compare against Apache Jena TDB and Apache AGE at
LUBM-10 / LUBM-100 scale. Results land in `target/perf-report.json`
and are tracked over time in `docs/09-release.md` per release.

## Regression discipline

- **Every bug fix gets a regression test.** No exceptions; the test
  reproduces the failure before the fix lands.
- **Every new UDF gets a `#[pg_test]`.** Wired into the CI matrix.
- **Coverage gates ratchet but never lower.** A phase's gate is a
  floor for all subsequent phases.

## What we don't test (yet)

- W3C SPARQL 1.1 manifest pass-rate — Phase 4 deliverable.
- W3C SHACL manifest pass-rate — Phase 4 deliverable.
- Concurrent transaction correctness under load — Phase 5 deliverable.
- Crash recovery / partial-COPY abort — lands with COPY BINARY
  (LLD §4.3), Phase 2.x backlog.
- Replication / streaming. Out of scope for v0.x.
