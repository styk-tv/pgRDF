-- 63-shmem-reset-invalidation.sql
--
-- Edge-case correctness regression — `pgrdf.shmem_reset()` MUST
-- invalidate the process-wide shmem dict cache. Continues the
-- edge-case track opened by `62-materialize-empty.sql`.
--
-- The cache implementation in `src/storage/shmem_cache.rs` keys
-- entries by `(generation, fingerprint)` where `GENERATION` is a
-- monotonic `PgAtomic<AtomicU64>` bumped by `reset()`. A lookup
-- whose slot generation != current generation reads as cold. This
-- file locks the behaviour contract: a refactor that forgets to
-- bump the generation in `reset()` would leave stale dict ids
-- visible across a `DROP EXTENSION; CREATE EXTENSION` cycle (where
-- the dict id space resets), corrupting fresh inserts with rotting
-- cached ids. That bug must surface as a regression failure here.
--
-- Three invariants locked:
--
--   1. Re-parsing the same Turtle terms while shmem is hot drives
--      cumulative `shmem_hits` upward (sanity — the cache is in
--      fact hot before reset).
--   2. After `pgrdf.shmem_reset()`, parsing those same terms once
--      more does NOT drive `shmem_hits` further upward — the
--      pre-reset entries are no longer visible. `shmem_hits` after
--      the post-reset parse equals `shmem_hits` at the moment of
--      reset.
--   3. The post-reset parse drives `shmem_inserts` upward — the
--      fresh inserts that replace the invalidated entries are
--      observable as new inserts (not silent reuse).
--
-- Counter VALUES are not pinned (sensitive to cumulative state
-- from prior tests in the same psql session and from upstream
-- internal churn). Each assertion projects a single boolean
-- comparing deltas captured via `\gset`, so expected output stays
-- `t`-flat regardless of absolute counter movement.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

SELECT pgrdf.add_graph(9963);

-- Warm shmem with three distinct terms (ex:a, ex:b, ex:c). The
-- first parse is the "cold" pass; its inserts publish to shmem on
-- the implicit-autocommit boundary of this very SELECT.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:a ex:b ex:c .', 9963);

-- ─── Snapshot baseline, then re-parse hot, then snapshot again ────
SELECT (pgrdf.stats()->>'shmem_hits')::bigint    AS h_pre,
       (pgrdf.stats()->>'shmem_inserts')::bigint AS i_pre
  FROM (SELECT 1) _ \gset

SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:a ex:b ex:c .', 9963);

SELECT (pgrdf.stats()->>'shmem_hits')::bigint    AS h_hot,
       (pgrdf.stats()->>'shmem_inserts')::bigint AS i_hot
  FROM (SELECT 1) _ \gset

-- Invariant 1: re-parse drove shmem_hits up (cache was hot).
SELECT :h_hot > :h_pre AS hits_went_up_when_hot;

-- ─── Reset generation, snapshot, re-parse, snapshot again ─────────
SELECT pgrdf.shmem_reset();

SELECT (pgrdf.stats()->>'shmem_hits')::bigint    AS h_at_reset,
       (pgrdf.stats()->>'shmem_inserts')::bigint AS i_at_reset
  FROM (SELECT 1) _ \gset

SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:a ex:b ex:c .', 9963);

SELECT (pgrdf.stats()->>'shmem_hits')::bigint    AS h_post_reset,
       (pgrdf.stats()->>'shmem_inserts')::bigint AS i_post_reset
  FROM (SELECT 1) _ \gset

-- Invariant 2: hits stayed flat across the post-reset parse.
SELECT :h_post_reset = :h_at_reset AS hits_flat_after_reset;

-- Invariant 3: inserts strictly increased after the reset.
SELECT :i_post_reset > :i_at_reset AS inserts_went_up_after_reset;

-- ─── Cleanup ────────────────────────────────────────────────────
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
