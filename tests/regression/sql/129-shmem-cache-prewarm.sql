-- 129-shmem-cache-prewarm.sql
--
-- TA-D2 spike correctness gate.
--
-- pgrdf.shmem_cache_prewarm(limit) walks _pgrdf_dictionary up to
-- `limit` rows and calls shmem_cache::insert_committed for each.
-- This regression locks the UDF's basic correctness.
--
-- **Cross-test isolation note:** the shmem cache is NOT
-- transactional — `insert_committed` writes directly to shmem and
-- survives any enclosing ROLLBACK. If this test were to prewarm
-- from rolled-back dictionary inserts, the shmem cache would
-- carry phantom (term, dict_id) pairs into subsequent tests, which
-- would then short-circuit dict lookups and return wrong ids. To
-- prevent that, this test calls `shmem_reset()` (wrapped in
-- side-effect DO blocks so it doesn't add empty-row output)
-- at BOTH the start and the end, and only queries against the
-- dictionary state that already exists at the moment the test
-- runs (not against ingested-and-rolled-back data).
--
-- Expected output: 2 boolean assertions all evaluating to `t`.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

DO $$ BEGIN PERFORM pgrdf.shmem_reset(); END $$;

-- ─── A: shmem_cache_prewarm returns count matching dict size ──
-- Use whatever rows already exist in _pgrdf_dictionary (from
-- earlier regression tests' committed state, if any; possibly 0
-- in a freshly-CREATE-EXTENSION'd database). Either way the
-- count must match.
SELECT (
  pgrdf.shmem_cache_prewarm(100000) =
  (SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary)
) AS a_prewarm_count_equals_dict_size;

DO $$ BEGIN PERFORM pgrdf.shmem_reset(); END $$;

-- ─── B: shmem_cache_prewarm respects the limit ──────────────
-- limit=0 must return 0 regardless of dict size.
SELECT (
  pgrdf.shmem_cache_prewarm(0) = 0
) AS b_prewarm_respects_zero_limit;

-- Cleanup: restore shmem to a known-clean state for subsequent
-- regression tests in the same compose database.
DO $$ BEGIN PERFORM pgrdf.shmem_reset(); END $$;
