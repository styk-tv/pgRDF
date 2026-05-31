# LUBM-1 baseline (TA-12)

The companion `baseline.lubm-1.json` is the **smallest tier** of the
LUBM dev-gate ladder — the "before" line for Track A's parse_turtle /
ingest-path optimization spikes (TA-11 `heap_multi_insert`, TA-10
`CopyBinary`, TA-9 decision, TA-7 GUC-controlled implementation).

The LUBM-10 baseline (TF-9) is the dev-gate that perf-nightly runs;
this LUBM-1 baseline is the granular reference that lets the Track A
spikes show speedup at a smaller, faster-to-iterate scale. LUBM-1 is
~13× smaller than LUBM-10 (103,104 vs 1,316,700 triples), iterates
in ~1.5s on a laptop, and surfaces parse-path overhead vs storage
overhead better because the dict_lookups : triples ratio is similar
to LUBM-10 but the absolute wall-clock is small enough to compare
prototypes without grinding through a full LUBM-10 run.

## How to reproduce

```bash
just lubm-build              # one-time, ~30s, image ~295 MB
just lubm-gen 1              # ~2s, populates pgrdf-lubm-data volume
just build-ext               # builds compose/extensions/pgrdf.so
OUTFILE=tests/perf/lubm/baseline.lubm-1.json \
    bash tests/perf/lubm/run-lubm.sh 1
```

To overwrite this baseline (only when an intentional perf shift is
expected — e.g. TA-7 lands a new ingest path):

```bash
OUTFILE=tests/perf/lubm/baseline.lubm-1.json \
JSON_SCHEMA_VALIDATE=1 \
    bash tests/perf/lubm/run-lubm.sh 1
```

## Fixture: `lubm-1-ingest-nt`

End-to-end bulk-load of LUBM-1's N-Triples (103,104 triples, ~16 MB)
via `pgrdf.load_turtle_verbose()` from a server-side path inside a
clean postgres+pgrdf sidecar.

- **modes.default.elapsed_ms** — total ingest time in ms.
- **modes.default.dict_lookups** — sum of `dict_cache_hits` +
  `shmem_cache_hits` + `dict_db_calls` (diagnostic only).

Baseline numbers at v0.5.25 / pg17 / Apple-Silicon laptop / Colima
docker:

| metric             | value     | per-triple |
|--------------------|-----------|------------|
| triples            | 103,104   |            |
| elapsed_ms         | 1,518     | 14.7 μs    |
| dict_lookups       | 342,391   | 3.32       |
| effective throughput |         | ~68,000 triples/sec |

(elapsed_ms_pct tolerance is the default ±50% — CI runner noise
floor; tighten when multiple consecutive localhost runs hold.)

### Phase-0 breakdown (added v0.5.26)

Per-phase timers in `LoaderStats` (loader.rs) split the ingest
elapsed_ms into three accumulators. Total ≈ elapsed_ms minus
a small per-iteration overhead from the Instant calls
themselves.

| phase       | ms    | % of total | what it measures |
|-------------|-------|------------|------------------|
| parse_ms    | 103   | 7%         | rio Turtle `next()` calls — lexer + grammar |
| dict_ms     | 1,114 | **73%**    | every `intern_term` call: HashMap lookup for cached repeat terms + cross-shmem-cache check + `put_term_full` SPI for unique terms (26,473 calls × ~42 µs each) |
| insert_ms   | 292   | 19%       | 101 batches × `INSERT … unnest(s,p,o)` prepared plan against `_pgrdf_quads` |

Why this matters for Track A: TA-11 (`heap_multi_insert`) and
TA-10 (`COPY BINARY`) target the 19%. The 73% lever is dict
resolution. See `_WIP/SPIKE.TRACK-A.phase0-findings.md` for the
spike re-scope recommendation.

## Fixture: `lubm-1-q14-graduate-students`

Median of 3 measured runs of LUBM Q14 ("find all GraduateStudent
instances") via `pgrdf.sparql('SELECT (COUNT(?s) AS ?n) WHERE { ?s
a <…#GraduateStudent> }')`. One warm pass discarded first.

- **modes.default.elapsed_ms** — median of the three measured psql
  `\timing` values.

The Q14 result count is `1874` at `-seed 0` for LUBM-1 (the UBA
generator is deterministic; the runner asserts this on every run).

| metric        | value  |
|---------------|--------|
| q14_count     | 1,874  |
| elapsed_ms    | 1.8    |

## What this baseline is **not**

- **Not a release-gate.** That's TF-6 (LUBM-100).
- **Not the dev-gate.** That's the LUBM-10 baseline + the
  perf-nightly cron in `.github/workflows/perf-nightly.yml`.
- **Not cross-engine.** Cross-engine comparison parks at the
  OPENBENCHMARK v1.0+ tier.

## Schema

Same `schema/baseline.schema.json` (JSON Schema 2020-12) as
LUBM-10. The `schema_version` field is `v0.6`.

## Track A spike usage

TA-11 (`heap_multi_insert` prototype) and TA-10 (`CopyBinary`
prototype) measure their LUBM-1 elapsed_ms against this baseline
to compute speedup ratio. TA-9 (the decision) writes its
conclusion into `_WIP/SPEC.ROADMAP.TRACK.TASKS.v1.0-devel.md`
§10 referencing the deltas this file establishes.
