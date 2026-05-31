# TA-D2 spike — shmem cache pre-warm

**Status:** spike completed v0.5.28. Strongly validates theory
(-54% e2e, -72.7% dict_ms). Larger win than TA-D3 (-17% e2e).
Decision deferred to TA-D1 — recommend combining TA-D2 with
TA-D3 (the wins are largely additive).

## What this spike implements

A new UDF `pgrdf.shmem_cache_prewarm(limit BIGINT DEFAULT 100000)`
that walks `_pgrdf_dictionary` ordered by `id` (oldest first —
core predicates) and calls `shmem_cache::insert_committed` for
each row, pre-warming the cross-backend shmem cache. Returns
count of rows actually pre-warmed.

The existing `shmem_cache::insert_committed` is exposed via this
UDF for the first time — previously only the per-write
`stage_for_commit → flush_pending` flow populated the cache.

## Measurement scenario (3-way, same backend session)

1. **Cold ingest** of LUBM-1 with fresh shmem cache (just-
   booted backend, `shared_preload_libraries=pgrdf`,
   `shmem_reset()` before to be explicit). All terms SPI'd via
   `put_term_full`. Records baseline dict_ms.
2. **`shmem_reset()`** — drops cache contents. Dictionary
   survives (cache is process-state; dict is on-disk).
3. **`shmem_cache_prewarm(100000)`** — reads all 26,473 dict
   rows back into the shmem cache.
4. **Warm ingest** of LUBM-1 into a different graph. Now every
   term either hits per-call HashMap (already true for repeats
   in single ingest) or the warmed shmem cache.

## Numbers (LUBM-1 / pg17 / Apple-Silicon / Colima / pgRDF 0.5.27)

| metric              | cold    | warm-after-prewarm | delta |
|---------------------|---------|--------------------|-------|
| triples             | 100,573 | 100,573            | ✅ identical |
| elapsed_ms          | 1517.82 | 697.86             | **-54.0%** |
| dict_ms             | 1115.00 | 304.45             | **-72.7%** |
| parse_ms            | 70.75   | 63.82              | -9.8% |
| insert_ms           | 324.14  | 322.14             | -0.6% (unchanged, as expected) |
| **dict_db_calls**   | **26,473** | **10,178**      | **-61.6%** |
| **shmem_cache_hits** | 0      | **16,295**         | (16k cache absorptions) |
| dict_cache_hits     | 308,325 | 308,325           | identical (per-call HashMap unchanged) |
| pre-warmed rows     | —       | 26,473             | (full dict snapshot) |

## Interpretation

**The win is structural.** Pre-warming the shmem cache from the
dictionary eliminates 16,295 of the 26,473 would-have-been-SPI
calls — a 61.5% absorption rate. The dict phase drops 73% in
absolute terms (-810 ms), translating to a 54% reduction in
total ingest time.

The shmem cache architecture is **already in pgRDF** (LLD §4.1);
TA-D2 didn't change it. The new contribution is the *prewarm
trigger* — making the warming explicit + on-demand rather than
relying on the per-write `stage_for_commit → flush_pending`
path which only fills as side-effect of new writes.

**Why are 10,178 db_calls remaining?** The shmem cache is bounded:
`SLOTS = 16,384` slots × `PROBE_DEPTH = 8` open-addressing
window. With 26,473 entries trying to fit, collision pressure
evicts the older entries via probe-depth overflow. The 16,295
shmem hits = entries that survived insertion; the 10,178 misses
= entries the cache couldn't hold. Increasing `SLOTS` would
capture more (a future optimization, not part of this spike).

## Combined with TA-D3 (estimated)

TA-D3 (-17% e2e, batched dict resolution) and TA-D2 (-54% e2e,
shmem prewarm) optimize different phases of the same dict path:

- TA-D3 reduces *SPI roundtrip cost* on the calls that DO hit
  the database. From 26k individual SPIs → ~106 batch SPIs.
- TA-D2 reduces *how many database hits happen at all* via the
  shmem cache.

The two are largely **additive**. Combined estimate: dict_ms
could drop from 1115ms → maybe 150-200ms (rough). E2E:
1517 → maybe 600-650ms (~58% total improvement). TA-D1
decision quantifies this.

## Use cases for shmem_cache_prewarm in production

- **Boot a fresh backend** connecting to a database with an
  existing dictionary. Without prewarm, the first ingest of
  every new backend hits SPI for every term.
- **Post `DROP/CREATE EXTENSION`** to re-establish cache state
  in one call.
- **Warm-and-forget on `_PG_init`** — could be wired so the
  extension auto-prewarms on backend boot. Currently manual.

## What this spike does NOT do

- Does NOT auto-prewarm on `_PG_init`. Manual UDF call is the
  trigger. (Auto-prewarm is a TA-7 production landing choice
  with its own tradeoffs — boot cost vs first-ingest cost.)
- Does NOT change SLOTS / PROBE_DEPTH (production tunable).
- Does NOT measure at LUBM-10/100. TA-D1 / TA-9 will.

## Companion: TA-D1 decision

TA-D1 weighs both TA-D3 (batch dict) and TA-D2 (shmem prewarm).
**Recommendation**: ship BOTH. They're not exclusive — TA-D3
applies inside the batch-resolve path; TA-D2 reduces what needs
to go through that path. Combined the spike numbers suggest a
significant total reduction; the production landing in TA-7
should validate combined at LUBM-10.
