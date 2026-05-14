-- 50-shmem-dict-cache.sql
--
-- Phase 3 step 1 (LLD §4.1) acceptance test. The synth-100 fixture
-- contains 100 triples drawn from 115 distinct terms (10 subjects ×
-- 5 predicates × 100 distinct objects). Distinct-term arithmetic is
-- documented in fixtures/regression/synth-100.sh.
--
-- This file rolls the shmem cache from cold to hot across THREE
-- separate `pgrdf.load_turtle_verbose` calls inside the same psql
-- session. Implicit autocommit means each call's
-- `stage_for_commit` mappings publish to shmem on the row's own
-- commit, so by the second call every term is cached.
--
-- Expected results (hand-computed, NOT autobaselined):
--
--   Load 1 (cold shmem):
--     dict_cache_hits  = 185   per-call hashmap; 100*3 refs - 115 distinct
--     shmem_cache_hits = 0     shmem empty pre-load
--     dict_db_calls    = 115   one SELECT+INSERT per distinct term
--
--   Load 2 (warm shmem, fresh graph):
--     dict_cache_hits  = 185
--     shmem_cache_hits = 115   shmem hot from load 1
--     dict_db_calls    = 0     no dictionary table touch
--
--   Load 3 (still warm, fresh graph):
--     identical to load 2.
--
-- Cumulative `pgrdf.stats()` post-test asserts a delta of
-- ≥ 230 shmem_hits (load 2 + load 3) and ≥ 115 shmem_inserts
-- (load 1) versus the pre-test snapshot.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;

-- DROP+CREATE resets the dictionary id space but does NOT clear the
-- process-wide shmem cache. Bump the generation so any pre-existing
-- entries from prior sessions read as cold.
SELECT pgrdf.shmem_reset();

SELECT pgrdf.add_graph(7770);
SELECT pgrdf.add_graph(7771);
SELECT pgrdf.add_graph(7772);

-- Snapshot the cumulative counters so we can assert deltas later
-- in this session. Pre-test counter state depends on what other
-- statements ran in this postmaster, so we test the delta only.
SELECT (j->>'shmem_hits')::bigint    AS hits_before,
       (j->>'shmem_inserts')::bigint AS inserts_before
  FROM (SELECT pgrdf.stats() AS j) s \gset

-- ─── Load 1: cold shmem ─────────────────────────────────────────────
SELECT (j->>'triples')::int          = 100  AS triples_ok,
       (j->>'dict_cache_hits')::int  = 185  AS hashmap_hits_ok,
       (j->>'shmem_cache_hits')::int = 0    AS shmem_hits_zero,
       (j->>'dict_db_calls')::int    = 115  AS db_calls_match_distinct
  FROM (SELECT pgrdf.load_turtle_verbose(
                 '/fixtures/regression/synth-100.ttl', 7770) AS j) s;

-- ─── Load 2: hot shmem, identical terms, fresh graph ────────────────
SELECT (j->>'triples')::int          = 100  AS triples_ok,
       (j->>'dict_cache_hits')::int  = 185  AS hashmap_hits_ok,
       (j->>'shmem_cache_hits')::int = 115  AS shmem_hits_all_distinct,
       (j->>'dict_db_calls')::int    = 0    AS no_db_calls
  FROM (SELECT pgrdf.load_turtle_verbose(
                 '/fixtures/regression/synth-100.ttl', 7771) AS j) s;

-- ─── Load 3: same shape, confirm hot path is stable ─────────────────
SELECT (j->>'triples')::int          = 100  AS triples_ok,
       (j->>'shmem_cache_hits')::int = 115  AS shmem_hits_still_115,
       (j->>'dict_db_calls')::int    = 0    AS no_db_calls
  FROM (SELECT pgrdf.load_turtle_verbose(
                 '/fixtures/regression/synth-100.ttl', 7772) AS j) s;

-- ─── Delta assertions on cumulative counters ────────────────────────
SELECT (j->>'shmem_hits')::bigint    - :hits_before    >= 230 AS hits_delta_ok,
       (j->>'shmem_inserts')::bigint - :inserts_before >= 115 AS inserts_delta_ok
  FROM (SELECT pgrdf.stats() AS j) s;

-- ─── Sanity: all three graphs got their 100 quads each ─────────────
SELECT pgrdf.count_quads(7770), pgrdf.count_quads(7771), pgrdf.count_quads(7772);

-- ─── Cleanup ──────────────────────────────────────────────────────
-- Drop the graphs we created so the next regression run (and any
-- co-running global-count tests) sees a clean slate. Then bump the
-- shmem generation so cached entries from this run don't pollute
-- subsequent ones.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
