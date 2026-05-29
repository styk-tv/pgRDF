# LUBM-10 baseline (TF-9)

The companion `baseline.lubm-10.json` is the first checked-in TF-10
runner output — the dev-gate baseline for v0.6's LUBM scale claim.
Per the TF-12 intake, **correctness fields are exact-match in CI**;
timing is tolerance-compared per the per-fixture
`comparison_tolerance.elapsed_ms_pct` field. Both fixtures in this
baseline carry the conservative ±50% default (CI runner noise floor)
— tighten only when we have multiple consecutive runs that hold.

## How to reproduce

```bash
just lubm-build              # one-time, ~30s, image ~295 MB
just lubm-gen 10             # ~10s, populates pgrdf-lubm-data volume
just build-ext               # builds compose/extensions/pgrdf.so
bash tests/perf/lubm/run-lubm.sh 10        # writes target/perf-report.json
```

To overwrite this baseline (only when an intentional perf shift is
expected):

```bash
OUTFILE=tests/perf/lubm/baseline.lubm-10.json \
JSON_SCHEMA_VALIDATE=1 \
    bash tests/perf/lubm/run-lubm.sh 10
```

## Fixture: `lubm-10-ingest-nt`

End-to-end bulk-load of LUBM-10's N-Triples (1,316,700 triples, ~195
MB) via `pgrdf.load_turtle_verbose()` from a server-side path inside
a clean postgres+pgrdf sidecar.

- **modes.default.elapsed_ms** — total ingest time in ms. The
  recorded number is `LoaderStats.elapsed_ms` (wall-clock around the
  oxttl parse + batched-INSERT loop).
- **modes.default.dict_lookups** — sum of `dict_cache_hits` +
  `shmem_cache_hits` + `dict_db_calls` (diagnostic only — not a
  gate). Useful for spotting cache-miss regressions.

A subsequent rerun should land inside ±50% of the baseline
`elapsed_ms` on the same hardware. CI runner gates land in TF-8.

## Fixture: `lubm-10-q14-graduate-students`

Median of 3 measured runs of LUBM Q14 ("find all GraduateStudent
instances") via `pgrdf.sparql('SELECT (COUNT(?s) AS ?n) WHERE { ?s
a <…#GraduateStudent> }')`. One warm pass discarded first to damp
out cold-cache jitter.

- **modes.default.elapsed_ms** — median of the three measured psql
  `\timing` values.

The Q14 result count is `24019` at `-seed 0` for LUBM-10 (the UBA
generator is deterministic; the runner asserts this on every run).

## What the baseline is **not**

- **Not a release-gate.** That's TF-6 (LUBM-100, runs on demand +
  release-tag CI).
- **Not cross-engine.** Comparison vs Apache Jena TDB / Virtuoso OS /
  Oxigraph / QLever is OPENBENCHMARK v1.0+ work, parked per
  `_WIP/SPEC.pgRDF.OPENBENCHMARK.v1.0.md`.
- **Not a single-number perf claim.** The publishable headline number
  is computed by aggregating the v1.0+ benchmark mix; LUBM-10 is the
  smallest tier of that, lockable on a laptop.

## Schema

`schema/baseline.schema.json` — JSON Schema 2020-12. The runner
validates against it when `JSON_SCHEMA_VALIDATE=1` is passed (CI
sets this; local dev defaults off so the runner has no Python
dependency).

The `schema_version` is `v0.6` at v0.6 cycle entry. Any field add /
remove / rename bumps the version, and consumers refuse to parse
unknown versions (per the schema's own field description).
