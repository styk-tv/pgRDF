-- 64-plan-cache-clear.sql
--
-- Edge-case correctness regression — `pgrdf.plan_cache_clear()` MUST
-- return the literal count of prepared statements drained from THIS
-- backend's thread_local plan-cache HashMap. Continues the edge-case
-- track opened by `62-materialize-empty.sql` (62 → forward).
--
-- The implementation in `src/query/plan_cache.rs::plan_cache_clear`
-- reads `m.len()` BEFORE calling `m.clear()` and returns that as
-- `i64`. The cumulative shmem counters (`plan_cache_hits / misses /
-- inserts`) are untouched by this UDF — only the per-backend map is
-- drained. The return contract must therefore be:
--
--   * fresh backend → returned count = 0 (nothing to drop)
--   * after N structurally distinct queries → returned count = N
--   * immediately after clear → `plan_cache_local_size = 0`
--   * second consecutive clear → returns 0 (idempotent at zero)
--
-- A refactor that swapped `m.len()` for `0`, hard-coded a wrong
-- constant, or hoisted the `len()` call to AFTER the `clear()`
-- (always returning 0) would corrupt operator-facing telemetry —
-- this file catches that regression.
--
-- Note: `parse_turtle` internally prepares its `flush_batch` INSERT
-- SQL ("INSERT INTO _pgrdf_quads(...) SELECT ... FROM unnest(...)"),
-- so that plan ALSO lives in the same backend-local cache. With one
-- `parse_turtle` followed by three structurally distinct SELECT
-- queries `size_before` empirically lands at 4 (1 ingest + 3 SELECT)
-- on the current pgrx 0.16 / PG 17 build. We deliberately do NOT
-- pin that literal — the test locks the RELATION `drained ==
-- size_before AND size_after == 0 AND idempotent_clear == 0 AND
-- size_before > 0`. If a future refactor changes the ingest path so
-- `flush_batch` skips the plan cache, `size_before` drops to 3 and
-- the test still passes; the only failure mode is the actual
-- contract breaking.
--
-- Three booleans + a non-zero guard → four `t` rows in expected.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- ── Invariant 1: fresh backend → clear() returns 0 ────────────────
-- (a) The clear we run as part of cleanup at the END of every
-- regression file already keeps the backend hot-empty across files,
-- so a no-op clear right after CREATE EXTENSION reads as zero.
SELECT pgrdf.plan_cache_clear() AS fresh_clear \gset
SELECT :fresh_clear = 0 AS fresh_clear_is_zero;

-- ── Seed: one graph, one Turtle parse (primes flush_batch plan) ───
SELECT pgrdf.add_graph(9964);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:a ex:b ex:c .', 9964);

-- ── Three structurally distinct SPARQL shapes ─────────────────────
-- Each shape translates to a different parameterised SQL string,
-- so each populates a distinct plan-cache entry.
SELECT count(*) FROM pgrdf.sparql(
  'SELECT ?o WHERE { <http://example.org/a> <http://example.org/b> ?o }');
SELECT count(*) FROM pgrdf.sparql(
  'SELECT ?s WHERE { ?s <http://example.org/b> <http://example.org/c> }');
SELECT count(*) FROM pgrdf.sparql(
  'SELECT ?p WHERE { <http://example.org/a> ?p <http://example.org/c> }');

-- ── Snapshot local size, drain, snapshot again, drain again ───────
SELECT (pgrdf.stats()->>'plan_cache_local_size')::bigint AS size_before \gset
SELECT pgrdf.plan_cache_clear() AS drained \gset
SELECT (pgrdf.stats()->>'plan_cache_local_size')::bigint AS size_after \gset
SELECT pgrdf.plan_cache_clear() AS idempotent_clear \gset

-- ── Invariant 2: drained equals what was cached pre-clear ─────────
SELECT :drained = :size_before AS drained_matches_size_before;

-- ── Invariant 3: cache is empty immediately after clear ───────────
-- (Cumulative shmem counters are explicitly NOT compared — only the
-- per-backend `plan_cache_local_size` should fall to 0.)
SELECT :size_after = 0 AND :idempotent_clear = 0 AS empty_and_idempotent;

-- ── Sanity: size_before was strictly positive ─────────────────────
-- Guards against a regression where flush_batch + the 3 SELECT plans
-- silently stopped reaching the cache, which would make the
-- `drained = size_before` assertion vacuously pass at 0 == 0.
SELECT :size_before > 0 AS something_was_cached;

-- ── Cleanup ────────────────────────────────────────────────────
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
