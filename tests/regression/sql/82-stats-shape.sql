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
  jsonb_typeof(j->'plan_cache_local_size') = 'number'  AS pc_size_is_number,
  jsonb_typeof(j->'path_depth_truncations') = 'number' AS pdt_is_number
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
  (j->>'plan_cache_local_size')::bigint  >= 0    AS pc_size_nonneg,
  (j->>'path_depth_truncations')::bigint >= 0    AS pdt_nonneg
  FROM (SELECT pgrdf.stats() AS j) s;

-- Note: functional invariants (cache hit/miss counter behaviour
-- under load) are already covered by `51-plan-cache.sql` and
-- `52-bulk-ingest-perf.sql`. This file's contract is the
-- **schema shape only** — what fields exist, what types, what
-- ranges. Adding behavioural assertions here would duplicate
-- coverage and inflate the baseline diff without new signal.

-- ─── (added in slice #56) Schema-drift tripwire ────────────────────
-- The assertions above lock fields-that-SHOULD-be-there are there
-- (right keys, right types, plausible ranges). The block below
-- locks fields-that-SHOULDN'T-be-there ARE NOT there: pin the
-- exact key count, pin the canonical key list verbatim, and pin
-- that no key carries a JSON `null`. A silent new field added to
-- `src/storage/stats.rs::stats()` without a corresponding update
-- here trips on the exact-count assertion — forces a deliberate
-- test update rather than a silent shape extension that breaks
-- downstream consumers locked to the documented set.
--
-- Current canonical set (11 keys) per `src/storage/stats.rs::stats()`:
--   shmem_ready, shmem_slots, shmem_hits, shmem_misses,
--   shmem_inserts, shmem_evictions, plan_cache_hits,
--   plan_cache_misses, plan_cache_inserts, plan_cache_local_size,
--   path_depth_truncations.
-- (path_depth_truncations added by Phase E group E1, LLD v0.4 §7.2 —
--  the property-path depth-guard scaffold; always 0 until group E2
--  wires the recursive-CTE increment.)

-- (a) Exact field count — the bullseye tripwire.
SELECT (SELECT count(*) FROM jsonb_object_keys((SELECT pgrdf.stats())))
       = 11 AS exact_eleven_keys;

-- (b) No extra / no missing keys — array equality against the
--     canonical sorted list. Catches both additions and renames.
SELECT array_agg(k ORDER BY k)
       = ARRAY[
           'path_depth_truncations',
           'plan_cache_hits',
           'plan_cache_inserts',
           'plan_cache_local_size',
           'plan_cache_misses',
           'shmem_evictions',
           'shmem_hits',
           'shmem_inserts',
           'shmem_misses',
           'shmem_ready',
           'shmem_slots'
         ]::text[] AS keys_match_canonical
  FROM jsonb_object_keys((SELECT pgrdf.stats())) AS t(k);

-- (c) No JSON `null` values — every field carries a real value.
--     A refactor that defaults an uninitialised counter to `null`
--     instead of `0` would slip past the type-contract block above
--     (since `jsonb_typeof` of a real number is not 'null'), so
--     this assertion projects a single boolean across all keys.
SELECT bool_and(jsonb_typeof(value) != 'null') AS no_null_fields
  FROM jsonb_each((SELECT pgrdf.stats())) AS e(key, value);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
