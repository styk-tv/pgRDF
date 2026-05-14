-- 52-bulk-ingest-perf.sql
--
-- Phase 3 step 3 (LLD §4.3, phase A) verification. The batched-INSERT
-- SQL string is constant across every flush; the per-backend
-- plan_cache now prepares it exactly once and reuses the
-- `OwnedPreparedStatement` for the rest of the backend's life.
--
-- Verifies behaviour, not wall-clock — exact ms numbers are flaky
-- across runtimes. The assertion is on the cache-counter arithmetic
-- only.
--
-- Fixture: fixtures/regression/synth-10k.ttl (10 000 triples =
-- 10 full + 1 zero-trailing batch at BATCH_SIZE = 1000 → 10 flushes
-- per load).
--
-- Hand-computed expectations after `plan_cache_clear()`:
--
--   Load 1 (cold INSERT plan):
--     plan_cache_misses += 1  (one prepare on first flush)
--     plan_cache_inserts += 1
--     plan_cache_hits   += (n_batches - 1)  (other flushes reuse)
--
--   Loads 2 & 3:
--     plan_cache_misses unchanged
--     plan_cache_inserts unchanged
--     plan_cache_hits += n_batches each

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph(9201);
SELECT pgrdf.add_graph(9202);
SELECT pgrdf.add_graph(9203);

-- snapshot 0 ─────────────────────────────────────────────────────
SELECT (j->>'plan_cache_hits')::bigint    AS h0,
       (j->>'plan_cache_misses')::bigint  AS m0,
       (j->>'plan_cache_inserts')::bigint AS i0
  FROM (SELECT pgrdf.stats() AS j) s \gset

-- Load 1 — cold INSERT plan. Capture the batch count.
SELECT (j->>'triples')::int      = 10000 AS triples_ok,
       (j->>'quad_batches')::int >= 10   AS at_least_10_batches
  FROM (SELECT pgrdf.load_turtle_verbose(
                 '/fixtures/regression/synth-10k.ttl', 9201) AS j) s;

SELECT (j->>'quad_batches')::int AS nb1
  FROM (SELECT pgrdf.load_turtle_verbose(
                 '/fixtures/regression/synth-10k.ttl', 9202) AS j) s \gset

-- Snapshot delta after two loads should match expected counter math.
SELECT (j->>'plan_cache_misses')::bigint  - :m0 = 1 AS exactly_one_prepare,
       (j->>'plan_cache_inserts')::bigint - :i0 = 1 AS exactly_one_insert
  FROM (SELECT pgrdf.stats() AS j) s;

-- Both loads together produced ≥ 20 batches; nearly all should hit.
-- (First flush of load 1 is the miss; everything else hits.)
SELECT (j->>'plan_cache_hits')::bigint   - :h0 >= 19 AS hits_ge_19_after_two_loads
  FROM (SELECT pgrdf.stats() AS j) s;

-- Load 3 — every flush should hit; no new inserts.
SELECT (j->>'plan_cache_inserts')::bigint AS i_before_load3
  FROM (SELECT pgrdf.stats() AS j) s \gset

SELECT (j->>'triples')::int = 10000 AS l3_triples_ok
  FROM (SELECT pgrdf.load_turtle_verbose(
                 '/fixtures/regression/synth-10k.ttl', 9203) AS j) s;

SELECT (j->>'plan_cache_inserts')::bigint - :i_before_load3 = 0 AS l3_no_new_inserts
  FROM (SELECT pgrdf.stats() AS j) s;

-- Sanity: 30 000 triples total in the three graphs.
SELECT pgrdf.count_quads(9201) + pgrdf.count_quads(9202) + pgrdf.count_quads(9203) = 30000
       AS three_loads_30000_triples;

-- ── Cleanup ─────────────────────────────────────────────────────
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
