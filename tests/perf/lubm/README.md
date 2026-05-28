# tests/perf/lubm — LUBM benchmark test bed (TF-12 intake)

The LUBM-shape and LUBM-shacl-sparql fixtures in this perf tree
exercise the Track H SHACL-SPARQL pipeline; this directory is the
**proper LUBM test bed** for Track F (v0.6 performance + correctness
gating). Locked to **localhost only** per the
[[lubm-localhost-only]] memory.

## Layout

    tests/perf/lubm/
    ├── README.md                        (this file)
    ├── generator/                       (containerised UBA generator — TRACKED source)
    │   ├── Dockerfile                   (eclipse-temurin JRE + UBA jar)
    │   ├── generate.sh                  (entrypoint)
    │   └── README.md
    ├── schema/
    │   └── baseline.schema.json         (lubm.expected.json schema lock)
    ├── data/                            (.gitignored — output of generator)
    │   └── .gitkeep
    └── .gitignore

## Pin points (TF-12 intake — locked 2026-05-28)

| Question | Decision |
|---|---|
| Fixture format | UBA generator output (the Lehigh SWAT canonical LUBM). Generator runs inside a `pgrdf-lubm-generator` container, never on the host's JRE. Output written to a docker named volume `pgrdf-lubm-data` and mounted read-only into `pgrdf-pgrdf-postgres` for ingest. Data is **discardable** (gitignored); the generator source + Dockerfile are **tracked** so any teammate can reproduce the same fixture without a Java toolchain on their machine. |
| Sibling fixtures | We keep `tests/perf/lubm-shape/` and `tests/perf/lubm-shacl-sparql/` as **handcrafted micro-fixtures** that run on every CI (small, in-`git diff`, no docker volume needed). The UBA-generated LUBM-10 in this directory is the dev-gate baseline. LUBM-100 / LUBM-1000 would generate larger volumes via the same container at higher `univ_count`. |
| Baseline schema | `schema/baseline.schema.json` — JSON Schema 2020-12. Per-fixture: `{conforms, violations, elapsed_ms, plan_cache_hits, dict_lookups}` per validate mode + a `comparison_tolerance` block. Rich enough to track perf regression, structured enough to diff cleanly in CI. |
| Comparison tolerance | Per-fixture `elapsed_ms_pct` tolerance (default ±50% — CI runner noise floor); correctness fields (`conforms`, `violations`) are exact-match. The exact-match-on-correctness rule is non-negotiable; tolerance applies only to timing. |
| Runtime discipline | Docker only via Colima. Never podman, never host Java. All volumes + container names prefixed `pgrdf-` so they don't collide with other parallel agents on this workstation (per [[docker-only-pgrdf-prefix]]). |

## How to (re)generate LUBM-N

```bash
# Build the generator image (one time, ~200 MB once cached).
just lubm-build

# Generate LUBM-10 (10 universities ≈ 1.3 M triples) into the
# pgrdf-lubm-data docker volume. About ~30s on Colima.
just lubm-gen 10

# The pgrdf compose stack can now mount the volume to ingest:
just lubm-load 10
```

The output of `just lubm-gen 10` is a tree of `.owl` files under the
`pgrdf-lubm-data:/data/lubm-10/` path inside the docker volume. A
follow-up TF-11 step converts these to N-Triples for `parse_turtle` /
`parse_nquads` ingest into pgRDF.

## What does NOT belong here

- Per the [[lubm-localhost-only]] memory: **no** Azure / cloud /
  hosted-runner planning. The test bed targets the maintainer's
  workstation and the GitHub Actions runners; nothing beyond.
- Per [[micro-release-all-workflows-green]]: any change here that
  affects what CI runs must be verified through every workflow
  triggered by the push, not just `ci.yml`.

## Cross-reference

- [`_WIP/SPEC.pgRDF.OPENBENCHMARK.v1.0.md`](../../../_WIP/SPEC.pgRDF.OPENBENCHMARK.v1.0.md)
  — the broader benchmark trajectory (WatDiv, BSBM, LDBC SPB).
  LUBM is the smallest representative; the rest of OPENBENCHMARK
  lives in spec land until the user opens that scope explicitly.
- [`_WIP/SPEC.ROADMAP.TRACK.TASKS.v1.0-devel.md`](../../../_WIP/SPEC.ROADMAP.TRACK.TASKS.v1.0-devel.md)
  — §6 Track F task list; TF-12 closed by this intake.
