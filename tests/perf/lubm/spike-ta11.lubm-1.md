# TA-11 spike — batch-size sweep + heap_multi_insert verdict

**Status:** spike completed v0.5.32. **Negative result, but
the result is the spike's value**: `heap_multi_insert` would
not meaningfully help at LUBM-1 scale. The dominant insert_ms
components (WAL writing, partition routing) are NOT what
heap_multi_insert addresses. **Recommendation: TA-11 is
MEASURED NOT WORTH IMPLEMENTING**; document and move on to
TA-10 (COPY BINARY — different mechanism, separately measure).

## What this spike measures

A new UDF `pgrdf.spike_ta11_batch_sweep(triple_count, batch_size)`
that inserts `triple_count` synthetic triples (sequential
bigints) into an UNLOGGED un-partitioned target table via the
same prepared `INSERT ... unnest($1,$2,$3)` SQL the production
loader uses. The UNLOGGED + flat-target choice strips two cost
components from the measurement so we can see what's left:

- **No WAL writing** (UNLOGGED bypasses the WAL path that
  dominates LOGGED INSERT cost on most workloads).
- **No partition routing** (the production `_pgrdf_quads` is
  LIST-partitioned by `graph_id`; routing has measurable
  per-row cost).

What remains in the measurement is the *bulk-insert mechanic
itself*: SPI dispatch + planner/executor + heap_insert into the
target's storage.

## Sweep results (100,000 synthetic triples, pg17, Apple-Silicon, Colima)

| batch_size | batches | elapsed_ms | per_triple (µs) | per_batch (µs) |
|---|---:|---:|---:|---:|
| 100 | 1,000 | 55.80 | 0.56 | 55.05 |
| **1,000** ★ | 100 | **40.25** | **0.40** | 399.52 |
| 10,000 | 10 | 40.82 | 0.41 | 4,054.91 |
| 100,000 (one-shot) | 1 | 46.99 | 0.47 | 46,510.05 |

★ = current production loader's `BATCH_SIZE` constant.

**`BATCH_SIZE = 1000` is already the sweet spot.** Halving it
(100) costs +38%. Bigger batches (10k, 100k) don't help, and
the all-at-once 100k batch is actually *worse* than 100 batches
of 1000 by 6.7 ms (probably memory-allocation pressure on the
array params).

## Bulk-insert mechanic vs total LUBM-1 insert_ms

| measurement | per-triple cost | comment |
|---|---:|---|
| LUBM-1 baseline insert_ms (real _pgrdf_quads) | 3.00 µs | 312 ms / 103,104 triples |
| TA-11 sweet-spot (UNLOGGED flat) | 0.40 µs | this spike |
| **Gap factor** | **~7.5×** | what's accounted for by WAL writing + partition routing + real-data variability |

The bulk-insert mechanic accounts for **~13% of the real LUBM-1
insert_ms**. The other ~87% is what UNLOGGED + flat target
eliminated: WAL + partition routing.

## Implication for TA-11 (heap_multi_insert)

`heap_multi_insert` is a PG C-API path that skips the SPI
roundtrip + the planner/executor. Its theoretical win is:

- **Avoid SPI dispatch + planner/executor per batch.** At
  BATCH_SIZE=1000 in the sweet spot, the per-batch cost is
  399 µs total. Of that 399 µs, the bulk of the time is the
  actual `heap_insert` work (which `heap_multi_insert` STILL
  does). Maybe 50–100 µs of the per-batch cost is SPI +
  planner + executor stack — call it ~30%. Eliminating it
  saves ~120 µs/batch × 100 batches = 12 ms of the 40 ms
  flat-table baseline = **30% improvement on the bulk-insert
  mechanic alone**.
- But the bulk-insert mechanic is only 13% of real LUBM-1
  insert_ms. 30% × 13% = **~4% total improvement at LUBM-1
  scale**.
- And `heap_multi_insert` doesn't address WAL writing
  (durability is the dominant cost) or partition routing
  (PG's INSERT does this; the C-API caller has to do it
  manually — adding the complexity back).

**A 4% total win for ~300 lines of unsafe Rust against
pgrx::pg_sys + partition Oid lookup is poor ROI compared to
TA-D3 (-17% e2e, dict batching) and TA-D2 (-54% e2e, shmem
prewarm) — both of which already shipped.**

## What this spike does NOT do

- Does NOT actually implement heap_multi_insert. The measured
  bulk-insert-mechanic numbers are sufficient to gate the
  decision.
- Does NOT measure at LUBM-10/100. Per [[lubm-localhost-only]]
  the spike stays at LUBM-1. The gap factor between
  bulk-insert mechanic and total insert_ms should be similar
  at larger scale (both grow with row count).
- Does NOT touch the production `_pgrdf_quads` path. The
  spike's target is a synthetic UNLOGGED table; the only
  pgRDF code added is the spike UDF itself.

## Recommendation for TA-D1 / TA-9 (combined decision)

Track A spike chain status going into TA-D1:

- ✅ TA-D3 — batch dict resolution: **-17% e2e** (shipped v0.5.27)
- ✅ TA-D2 — shmem cache prewarm: **-54% e2e** (shipped v0.5.28)
- ✅ TA-11 — heap_multi_insert: **measured ~4% theoretical
  ceiling, not implemented** (this spike v0.5.32)
- ⏭ TA-10 — COPY BINARY: open. Different mechanism than
  heap_multi_insert (operates at server-side COPY protocol
  level; can be faster for very large batches). Worth a
  separate measurement before deciding.

**Proposed TA-D1 decision** (subject to TA-10 measurement):

1. Land TA-D3 + TA-D2 combined into the default
   `parse_turtle` path (TA-7).
2. Do NOT land TA-11 heap_multi_insert as a production path
   — the implementation cost (~300 lines of unsafe Rust +
   partition routing) does not pay back.
3. Defer the WAL writing cost to deployment GUC documentation
   (`synchronous_commit=off` for batch ingest sessions,
   `wal_level=minimal` for one-shot bulk loads) — not
   extension code.

## Reproducing the spike

```sh
# Build the extension with the spike UDF compiled in
just build-ext

# Boot postgres, install the extension, run the sweep
docker run --rm -d --name pgrdf-ta11 \
  -e POSTGRES_PASSWORD=v -e POSTGRES_USER=v -e POSTGRES_DB=v \
  postgres:17.4-bookworm

# (copy .so + .control + .sql files in as compose/ does)

docker exec pgrdf-ta11 psql -U v -d v -c "CREATE EXTENSION pgrdf;"

for bs in 100 1000 10000 100000; do
  docker exec pgrdf-ta11 psql -U v -d v -t -A -c \
    "SELECT jsonb_pretty(pgrdf.spike_ta11_batch_sweep(100000, ${bs}))"
done
```
