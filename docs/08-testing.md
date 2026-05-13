# 08 — Testing strategy

Five layers, each with a coverage gate that ratchets upward per phase.
A phase is **complete** only when its column in the table below is
green. CI enforces the current phase's gates; nightly runs the upper
layers as a forward-looking signal.

## Layer matrix

| Layer | Runtime | Phase 1 | Phase 2 | Phase 3 | Phase 4 |
|---|---|---|---|---|---|
| Rust unit (`cargo test`) | sec | smoke only | parser AST coverage | reasoner correctness | full storage coverage |
| pgrx integration (`cargo pgrx test`) | ~30s | CREATE EXTENSION + drop | ingestion + basic SPARQL | inference materialization | SHACL validation |
| pg_regress golden | ~1min | basic schema | core SPARQL forms | inference + query | full SHACL conformance |
| W3C SPARQL 1.1 | min | scaffolded | ≥ 30% pass | ≥ 70% pass | ≥ 95% pass |
| W3C SHACL | min | scaffolded | runner runs | ≥ 50% pass | ≥ 90% pass |
| LUBM perf | min | — | LUBM-1 smoke | LUBM-10 baseline | LUBM-100 vs Jena/AGE |

Coverage gates per phase live in [10-roadmap.md](10-roadmap.md).

## Layer 1 — Rust unit tests

Plain `cargo test`. Used for pure-Rust logic that doesn't need a live
Postgres backend: SPARQL parser, JSONB shaping, SHACL report
construction.

```bash
cargo test --no-default-features --features pg18
```

## Layer 2 — pgrx integration (`#[pg_test]`)

`cargo pgrx test` spins up a managed Postgres, installs the extension,
and runs annotated `#[pg_test]` functions. The smoke test in
`src/lib.rs` (`test_version_matches_cargo`) is the canonical example.

```bash
cargo pgrx test pg18
```

## Layer 3 — pg_regress golden tests

`tests/regression/sql/*.sql` runs against the extension; the output is
diffed against `tests/regression/expected/*.out`. New tests start as
unexpected-diff failures so the contributor sees the output and
either accepts it (`mv` to expected) or fixes the regression.

```bash
just test-regression   # (TODO Phase 1)
```

## Layer 4 — W3C SPARQL 1.1

W3C maintains the SPARQL 1.1 test suite at `w3c/rdf-tests`. We pull
it as a git submodule and run a manifest-driven runner against pgRDF.

```bash
git submodule update --init tests/w3c-sparql/fixtures
cargo run -p pgrdf-w3c-sparql -- tests/w3c-sparql/fixtures/sparql11/manifest.ttl
```

Per-test outcomes are reported to `target/w3c-sparql-report.json` for
CI artifact upload.

## Layer 5 — W3C SHACL

Mirror of layer 4 against the `w3c/data-shapes` test suite.

## Layer 6 — LUBM perf (parallel to regression)

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

- Concurrent transaction correctness under load — Phase 4 deliverable.
- Crash recovery / partial-COPY abort. Phase 3.
- Replication / streaming. Out of scope for v0.x.
