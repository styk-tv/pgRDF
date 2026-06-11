# LUBM-10 baseline — final combined ingest path

Committed reference snapshot: [`baseline.lubm-10.combined.json`](./baseline.lubm-10.combined.json).

Captured at **v0.5.43** under the final combined dict path
(`pgrdf.ingest_dict_path = 'combined'`, the v0.5.37+ default) by
[`benchmark-runner.sh`](../benchmark-runner.sh). This is the richer
sibling of [`baseline.lubm-10.json`](./baseline.lubm-10.json) (the
`run-lubm.sh` contract consumed by `compare-to-baseline.py`, kept in
its own schema). Volatile fields (timestamp, host, git sha/branch)
are stripped so the file is a stable run-to-run comparison anchor.

## What it locks

- **Ingest** — 1,316,700 ABox triples + 293 Tbox triples, with the
  Phase-0 breakdown (`parse_ms` / `dict_ms` / `insert_ms`). The
  `parse_ms` figure is honest as of v0.5.43 (a parse-timer accounting
  bug in `ingest_turtle_combined` — it measured only the iterator
  unwrap, not the parse — was fixed in the same release).
- **Materialize** — RDFS + OWL-RL `materialize_ms` +
  `triples_inferred` per profile.
- **Q1-Q14** — `count` + `elapsed_ms_median` per profile. The counts
  are the authoritative regression values; they are *also* locked in
  [`queries/expected-counts.json`](./queries/expected-counts.json),
  which the runner reconciles on every run.

## Headline (combined vs pre-TA-7 baseline v0.5.36)

| Metric | v0.5.36 baseline | v0.5.43 combined | Δ |
|---|---|---|---|
| Ingest e2e | 24,505 ms | 16,539 ms | **−32.5 %** |
| Ingest dict phase | 17,331 ms | 9,746 ms | **−43.8 %** |
| RDFS inferred | 287,422 | 287,422 | identical |
| OWL-RL inferred | 815,968 | 815,968 | identical |
| Q1-Q14 counts | — | — | **zero drift** |

The win is entirely on the ingest dict phase (TA-7's batched +
hot-cache resolution); materialize is reasoner-bound (the
`reasonable` crate's forward-chaining) and unchanged within run
noise. Correctness is byte-identical: same `dict_db_calls`, same
inferred totals, same per-query counts. This validates the combined
path at 10× LUBM-1 scale ahead of the LUBM-100 first measurement.

Tolerances: timings are dev-host indicative (±30 %); counts and
inferred totals are exact.
