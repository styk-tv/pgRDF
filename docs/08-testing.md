# 08 — Testing strategy

Layered test bar, each with a coverage gate that ratchets upward per
phase. Layers 1–5 are wired and green today (the "five layer" test
policy in [`SPEC.pgRDF.LLD.v0.3.md`](../specs/SPEC.pgRDF.LLD.v0.3.md)
§6.1); layers 6–8 are scaffolded and gated for Phase 6 step 2. A
phase is **complete** only when its column in the table below is
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
| pg_regress golden | ~1 min | 25 ✅ | **39 ✅** | + W3C TTL-manifest runner outputs |
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

The triple-count totals are now locked in
`tests/perf/smoke-ontologies.expected.tsv` (24 rows, 17 134 triples).
`smoke-ontologies.sh --check` diffs the live counts against that
file — a parser-regression tripwire for the real-world fixture set.

```bash
fixtures/ontologies.sh                       # one-time fetch (also gitignored output)
tests/perf/smoke-ontologies.sh               # print live counts
tests/perf/smoke-ontologies.sh --check       # regression: diff vs locked .expected.tsv
```

## Layer 4 — W3C-shape SPARQL harness (✅ shipped)

`tests/w3c-sparql/` holds 23 hand-authored W3C-shape tests — each
subdirectory is one test (`data.ttl` + `query.rq` + `expected.jsonl`)
covering BGP, DISTINCT, UNION-disjoint, OPTIONAL chain, MINUS,
FILTER (isIRI / regex / IN), aggregates, ORDER BY, LIMIT/OFFSET,
BIND, ASK, STRLEN / LANG / UCASE / STR, BOUND-after-OPTIONAL,
numeric FILTER, HAVING-inline-aggregate, MIN/MAX numeric. Expected
outputs hand-verified against the W3C SPARQL 1.1 spec section they
exercise. Driven by `tests/w3c-sparql/run.sh` against the compose
Postgres.

```bash
just test-w3c
```

## Layer 5 — LUBM-shape correctness harness (✅ shipped)

`tests/perf/lubm-shape/` holds 3 hand-authored LUBM-shape tests
(`Q1-class-membership`, `Q2-professor-of`, `Q3-takes-course`) over
a hand-curated `data.ttl`. Same harness shape as layer 4. This is
the correctness gate; the real LUBM-1/10/100 cross-engine benchmark
is layer 8 (not wired).

```bash
just test-lubm
```

## Layer 6 — W3C SPARQL 1.1 full manifest (⏳ not wired)

W3C maintains the SPARQL 1.1 test suite at `w3c/rdf-tests`. The
plan is to pull it as a git submodule and run a manifest-driven
runner against pgRDF. **Not wired today.** The runner crate
`pgrdf-w3c-sparql` exists only in the LLD; landing it is a
Phase 6 step-2 deliverable (v0.3 LLD §5.4).

```bash
# (planned)
git submodule update --init tests/w3c-sparql/fixtures
cargo run -p pgrdf-w3c-sparql -- tests/w3c-sparql/fixtures/sparql11/manifest.ttl
```

## Layer 7 — W3C SHACL full manifest (⏳ not wired)

Mirror of layer 6 against the `w3c/data-shapes` test suite. Blocked
upstream by ERRATA E-009 (`shacl_validation` / `reasonable` feature
unification on `oxrdf`'s `rdf-12`). Lands with Phase 6 step 2 once
E-009 clears.

## Layer 8 — LUBM real benchmarks (⏳ not wired)

LUBM (Lehigh University Benchmark) is the de facto OWL/SPARQL store
benchmark. We compare against Apache Jena TDB and Apache AGE at
LUBM-10 / LUBM-100 scale. Results land in `target/perf-report.json`
and are tracked over time in `docs/09-release.md` per release.
Phase 6 step 2 deliverable.

## Regression discipline

- **Every bug fix gets a regression test.** No exceptions; the test
  reproduces the failure before the fix lands.
- **Every new UDF gets a `#[pg_test]`.** Wired into the CI matrix.
- **Coverage gates ratchet but never lower.** A phase's gate is a
  floor for all subsequent phases.

## What we don't test (yet)

- W3C SPARQL 1.1 manifest pass-rate — Phase 6 step 2 deliverable
  (v0.3 LLD §5.4); ratchets `≥ 30 % → ≥ 70 % → ≥ 95 %`.
- W3C SHACL manifest pass-rate — Phase 6 step 2, gated on
  ERRATA E-009; ratchets `≥ 50 % → ≥ 90 %`.
- LUBM-10 / LUBM-100 throughput vs Jena TDB + Apache AGE —
  Phase 6 step 2.
- Crash recovery / partial-COPY abort — lands with COPY BINARY
  (LLD §4.3), Phase 3 step 3b deferral.
- Replication / streaming. Out of scope for v0.x (LLD §8).
