# 08 — Testing strategy

Five layers, each with a coverage gate that ratchets upward per phase.
A phase is **complete** only when its column in the table below is
green. CI enforces the current phase's gates; nightly runs the upper
layers as a forward-looking signal.

## Layer matrix

Phase numbering matches v0.3 LLD [`10-roadmap.md`](10-roadmap.md):
Phase 1 = core storage + build automation; Phase 2 = SPARQL
functional coverage; Phase 3 = storage performance; Phase 4 =
inference (OWL 2 RL); Phase 5 = validation (SHACL); Phase 6 =
CI + W3C conformance + release. v0.3 engine surface is
**feature-complete** modulo the explicitly deferred Phase 3
step 3b (heap_multi_insert) and Phase 5's blocked SHACL
integration (ERRATA E-009).

| Layer | Runtime | Pre-v0.3 | v0.3 (current) | v0.4 target |
|---|---|---|---|---|
| Rust unit (`cargo test`) | sec | smoke | parser + executor + cache primitives | full storage coverage |
| pgrx integration (`cargo pgrx test`) | ~30 s | 79 ✅ | **93 ✅** | + heap_multi_insert tests |
| pg_regress golden | ~1 min | 25 ✅ | **33 ✅** | + W3C TTL-manifest runner outputs |
| W3C-shape harness | ~5 s on top of regression | — | **23 ✅** | superseded by the TTL-manifest runner |
| LUBM-shape harness | ~3 s on top of regression | — | **3 ✅** | superseded by LUBM-1/10/100 real benchmarks |
| Ontology smoke | sec each, manual | 24 ontologies, 17 134 triples ✅ | (same set) | (same set) |
| W3C SPARQL 1.1 conformance (full manifest) | min | scaffolded | runner not wired ⏳ | ≥ 30 % pass |
| W3C SHACL conformance | min | scaffolded | not wired ⏳ (blocked, ERRATA E-009) | ≥ 50 % pass once E-009 clears |
| LUBM perf (real LUBM gen + cross-engine) | min | — | scaffold only | LUBM-10 vs Jena TDB / Apache AGE |

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
just test-all          # narrow bar: just test && just test-regression
just test-conformance  # every compose-based harness: regression + W3C-shape + LUBM-shape
just test-everything   # the lot: pgrx integration + test-conformance
just smoke-cold        # wipe compose, rebuild, re-up, run test-conformance
```

`just test-everything` is the comprehensive entry point — pgrx
integration plus every compose-based harness end-to-end. `just
smoke-cold` is the cold-compose verification: it tears compose down
with `compose-down`, rebuilds the extension, brings compose back up,
recreates the extension, and runs `test-conformance` against the
fresh stack. Use it after touching anything in `compose/`,
`fixtures/`, or the test SQL fixtures themselves — those changes
can pass on a warm compose and break on the next cold boot.

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
