-- 82-stats-shape.sql
--
-- Locks the shape contract for `pgrdf.stats()` — the operator-facing
-- observability UDF. Downstream tooling (CloudNativePG operators,
-- client libraries, CI dashboards) wires against this JSONB shape; a
-- silent field rename or drop would break those consumers without
-- any pgRDF-side test firing. This file pins the field set + JSON
-- types + plausible value ranges so any change must be intentional
-- (the baseline diff catches the schema drift).
--
-- Out of scope: exact field VALUES — they depend on what ran in the
-- postmaster's lifetime. Only shape + type + non-negativity.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Field set: every expected key is present ─────────────────────
-- jsonb_object_keys returns one row per key; sort + array_agg for a
-- deterministic, easily-eyeballable list.
SELECT array_agg(k ORDER BY k) AS keys
  FROM jsonb_object_keys((SELECT pgrdf.stats())) AS t(k);

-- ─── Type contract: every numeric field is `number` (integer or
--     non-negative); the boolean is `boolean`. jsonb_typeof returns
--     'string'/'number'/'boolean'/'null'/'object'/'array'.
SELECT
  jsonb_typeof(j->'shmem_ready')           = 'boolean' AS ready_is_bool,
  jsonb_typeof(j->'shmem_slots')           = 'number'  AS slots_is_number,
  jsonb_typeof(j->'shmem_hits')            = 'number'  AS sh_hits_is_number,
  jsonb_typeof(j->'shmem_misses')          = 'number'  AS sh_misses_is_number,
  jsonb_typeof(j->'shmem_inserts')         = 'number'  AS sh_inserts_is_number,
  jsonb_typeof(j->'shmem_evictions')       = 'number'  AS sh_evict_is_number,
  jsonb_typeof(j->'plan_cache_hits')       = 'number'  AS pc_hits_is_number,
  jsonb_typeof(j->'plan_cache_misses')     = 'number'  AS pc_misses_is_number,
  jsonb_typeof(j->'plan_cache_inserts')    = 'number'  AS pc_inserts_is_number,
  jsonb_typeof(j->'plan_cache_local_size') = 'number'  AS pc_size_is_number
  FROM (SELECT pgrdf.stats() AS j) s;

-- ─── Value-range contract: every counter is non-negative; slots is
--     a positive power-of-two-ish capacity (the shmem cache table
--     size). The exact slot count is an implementation detail (16384
--     today, see src/storage/shmem_cache.rs::SLOTS) so we assert
--     >= 1024 to give room for future tuning without forcing this
--     file's update.
SELECT
  (j->>'shmem_ready')::boolean           AS shmem_ready_true,
  (j->>'shmem_slots')::bigint            >= 1024 AS slots_at_least_1024,
  (j->>'shmem_hits')::bigint             >= 0    AS hits_nonneg,
  (j->>'shmem_misses')::bigint           >= 0    AS misses_nonneg,
  (j->>'shmem_inserts')::bigint          >= 0    AS inserts_nonneg,
  (j->>'shmem_evictions')::bigint        >= 0    AS evict_nonneg,
  (j->>'plan_cache_hits')::bigint        >= 0    AS pc_hits_nonneg,
  (j->>'plan_cache_misses')::bigint      >= 0    AS pc_misses_nonneg,
  (j->>'plan_cache_inserts')::bigint     >= 0    AS pc_inserts_nonneg,
  (j->>'plan_cache_local_size')::bigint  >= 0    AS pc_size_nonneg
  FROM (SELECT pgrdf.stats() AS j) s;

-- Note: functional invariants (cache hit/miss counter behaviour
-- under load) are already covered by `51-plan-cache.sql` and
-- `52-bulk-ingest-perf.sql`. This file's contract is the
-- **schema shape only** — what fields exist, what types, what
-- ranges. Adding behavioural assertions here would duplicate
-- coverage and inflate the baseline diff without new signal.

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
