# TA-10 spike — insert_ms cost decomposition + COPY BINARY verdict

**Status:** spike completed v0.5.33. **Negative result (same
class as TA-11)**: COPY BINARY would not meaningfully help at
LUBM-1 scale. The dominant insert_ms cost component is
**hexastore index maintenance** (~51% of total), which COPY
BINARY does not address (COPY routes through PG's INSERT
machinery and triggers the same per-row index maintenance).
**Recommended verdict: TA-10 MEASURED NOT WORTH IMPLEMENTING.**

## What this spike does

Instead of implementing COPY BINARY (which would be ~200 lines
of unsafe Rust against `pgrx::pg_sys` CopyFrom + binary tuple
encoding), the spike *decomposes* the LUBM-1 insert_ms cost by
adding one cost component at a time on top of the TA-11 prelim's
UNLOGGED-flat measurement. Whichever component dominates is
what COPY BINARY would need to address. If it doesn't, the
unsafe code isn't worth writing.

Four variants of the same prepared `INSERT ... unnest($1,$2,$3)`
SQL, each against a different target shape:

- **A** — UNLOGGED flat (bulk-insert mechanic only; from TA-11)
- **B** — LOGGED flat (A + WAL writing)
- **C** — LOGGED + 3 hexastore indexes (B + index maintenance)
- **D** — LOGGED partitioned, 1 partition (B + partition routing)

Then compared to the real LUBM-1 baseline (partitioned + indexed
+ real triple data) = 312ms / 103,104 triples = 3.0 µs/triple.

## Results (100,000 synthetic triples, pg17, Apple-Silicon, Colima)

| variant | target | elapsed_ms | per-triple (µs) | Δ from previous | attribution |
|---|---|---:|---:|---:|---|
| A | UNLOGGED flat | 42.2 | 0.42 | — | bulk-insert mechanic |
| B | LOGGED flat | 54.0 | 0.54 | +12 ms | WAL writing |
| **C** | **LOGGED + SPO/POS/OSP indexes** | **213.4** | **2.13** | **+159 ms** | **3 hexastore indexes** |
| D | LOGGED + 1 partition | 54.9 | 0.55 | +1 ms | partition routing (negligible) |
| LUBM-1 baseline | _pgrdf_quads (partitioned + indexed + real data) | 312 | 3.0 | — | reality |

C accounts for **71% of the LUBM-1 baseline**. The remaining
30% gap = real-data variability + partitioned-AND-indexed
combined (spike tested either OR, not both).

## Cost decomposition (% of LUBM-1 insert_ms = 312ms)

| component | ms | % |
|---|---:|---:|
| Bulk-insert mechanic | 42 | 13.4% |
| WAL writing | 12 | 3.8% |
| Partition routing | 1 | 0.3% |
| **Hexastore index maintenance** | **159** | **51.1%** |
| Real-data + partitioned×indexed interaction | ~98 | ~31.4% |

## What COPY BINARY would address

COPY BINARY's distinct advantages over the prepared INSERT
... unnest path:

- **Skips planner + executor.** Saves ~50–100 µs per batch.
  At BATCH_SIZE=1000 / 100 batches = ~10ms total. Compared to
  213ms LOGGED-indexed: **~5% improvement**.
- **Denser binary payload.** Marginal for 100k bigint-only
  tuples; the arrays in unnest are already efficient bigint[].
- **Routes via INSERT machinery natively.** Same partition
  routing + same index maintenance. **Does NOT save the 51%
  index maintenance cost.**

Best-case COPY BINARY improvement at LUBM-1 scale:
**~5–10%** of insert_ms = ~1% of total ingest e2e.

For ~200 lines of unsafe Rust + binary tuple encoding +
partition routing complexity, this is poor ROI compared to
the dict-path spikes already shipped:

- TA-D3 (batched dict resolution): **-17% e2e** (v0.5.27)
- TA-D2 (shmem cache prewarm): **-54% e2e** (v0.5.28)

## Verdict / TA-9 input

**NOT WORTH IMPLEMENTING** (same conclusion as TA-11).

The real Track A insert-path lever is **hexastore index
maintenance**, not any of the bulk-insert mechanisms the
TA-10 / TA-11 specs targeted. Possible future Track-A item
(not in current scope):

> **TA-NEW-Z** (suggested for v0.7+): bulk-ingest mode that
> conditionally drops the three hexastore indexes, runs the
> batched-dict + bulk INSERT pipeline, then `CREATE INDEX`
> rebuilds them in parallel. Classic PG bulk-load pattern.
> Probable e2e improvement at LUBM-1: 40-50% (saving most
> of the 159ms index-maintenance cost during the ingest
> hot path, eating it as a one-shot at the end).

This is OUT OF SCOPE for the current Track A. The spec'd
TA-11 / TA-10 spikes both close as measured-not-worth-
implementing; TA-NEW-Z is the natural successor if Track A
wants more ingest perf beyond TA-D3 + TA-D2.

## Track A spike chain — final summary

| Spike | Result | Ship verdict |
|---|---|---|
| TA-12 LUBM-1 baseline | 103,104 triples / 1,489 ms / 1.8 ms Q14 | shipped v0.5.23 |
| Phase-0 instrumentation | parse=7% / dict=73% / insert=19% | shipped v0.5.26 |
| TA-D3 batched dict resolution | -17% e2e | shipped v0.5.27 (additive UDF) |
| TA-D2 shmem cache prewarm | -54% e2e | shipped v0.5.28 (additive UDF) |
| TA-11 heap_multi_insert prelim | 4% theoretical ceiling | shipped v0.5.32 not-implemented |
| TA-10 COPY BINARY prelim | 5-10% theoretical ceiling | shipped v0.5.33 not-implemented |
| TA-D1 (dict-path decision) | OPEN — combine TA-D3 + TA-D2 into default path | next |
| TA-9 (insert-path decision) | OPEN — write up TA-11/TA-10 verdicts + flag TA-NEW-Z option | after TA-D1 |

## Reproducing the spike

```sh
# Build with the spike UDFs
just build-ext

# Boot postgres + install + run the four variants
docker run --rm -d --name pgrdf-ta10 \
  -e POSTGRES_PASSWORD=v -e POSTGRES_USER=v -e POSTGRES_DB=v \
  postgres:17.4-bookworm

# (copy .so + .control + .sql files as compose does)

docker exec pgrdf-ta10 psql -U v -d v -c "CREATE EXTENSION pgrdf;"

docker exec pgrdf-ta10 psql -U v -d v -t -A -c "
  SELECT 'A_UNLOGGED_flat'        AS variant,
         (pgrdf.spike_ta11_batch_sweep(100000, 1000))->>'elapsed_ms' AS elapsed_ms
  UNION ALL SELECT 'B_LOGGED_flat',
         (pgrdf.spike_ta10_logged_flat(100000, 1000))->>'elapsed_ms'
  UNION ALL SELECT 'C_LOGGED_indexed',
         (pgrdf.spike_ta10_logged_indexed(100000, 1000))->>'elapsed_ms'
  UNION ALL SELECT 'D_LOGGED_partitioned',
         (pgrdf.spike_ta10_logged_partitioned(100000, 1000))->>'elapsed_ms'"
```
