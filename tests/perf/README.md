# Performance benchmarks

LUBM (Lehigh University Benchmark) is the canonical OWL/SPARQL store
benchmark. We compare pgRDF against Apache Jena TDB and Apache AGE
at LUBM-10 (~1.3M triples) and LUBM-100 (~13M triples) scales.

## Layout (target)

    tests/perf/
    ├── README.md                  (this file)
    ├── run-lubm.sh                (driver; Phase 2)
    ├── queries/                   (the 14 LUBM SPARQL queries)
    └── data/                      (generator output; gitignored)

## LUBM generator

LUBM ships a Java generator at https://swat.cse.lehigh.edu/projects/lubm/.
The runner script wraps it; the produced `*.owl` files are converted
to N-Triples and bulk-loaded via `pgrdf.load_file()`.

## Output

`target/perf-report.json` — a per-engine, per-query matrix of:
- cold-start latency (ms)
- warm latency (ms, median of 10 runs)
- result row count (sanity check)

Tracked release-over-release in [docs/09-release.md](../../docs/09-release.md).

## Gates per phase

- Phase 1: not run.
- Phase 2: LUBM-1 smoke (correctness only).
- Phase 3: LUBM-10 baseline numbers in the report.
- Phase 4: LUBM-100 comparison against Jena TDB + AGE.
