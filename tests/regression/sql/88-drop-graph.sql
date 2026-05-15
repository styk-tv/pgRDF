-- 88-drop-graph.sql
--
-- Phase B slice 99 — `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN
-- DEFAULT TRUE) → BIGINT` opens the lifecycle-UDF track (LLD v0.4
-- §5). The UDF removes the LIST partition `_pgrdf_quads_g<id>` from
-- the parent `_pgrdf_quads` (DETACH + DROP), deletes the matching
-- `_pgrdf_graphs` row, and returns the pre-drop triple count.
--
-- Invariants locked by this file:
--
--   1. Idempotent absent — dropping a graph_id whose partition does
--      not exist returns `0` and does NOT error.
--   2. Happy path — adding a graph, populating it with N triples,
--      and dropping it returns `N`. The partition disappears from
--      `pg_class`. The `_pgrdf_graphs` row disappears too.
--   3. Cascade-FALSE inferred guard — when the partition has any
--      `is_inferred = TRUE` row, `pgrdf.drop_graph(id, cascade =>
--      FALSE)` errors with the stable prefix `drop_graph:
--      inferred rows present`. Same input with `cascade => TRUE`
--      (the default) succeeds and returns the total row count.
--   4. Default partition guard — `pgrdf.drop_graph(0)` is rejected
--      with the stable prefix `drop_graph: cannot drop default
--      partition`. `_pgrdf_quads_default` is the catch-all bucket;
--      dropping it would orphan the partition router.
--   5. Negative-id guard — `pgrdf.drop_graph(-1)` is rejected with
--      the stable prefix `drop_graph: graph_id must be >= 0`.
--      Symmetric to the same guard on `pgrdf.add_graph(g BIGINT)`.
--
-- The error-path checks reuse the `_check_error(label, sql, frag)`
-- plpgsql helper introduced in `81-error-paths.sql`: it captures
-- `SQLERRM` from a wrapped EXECUTE, asserts the expected substring
-- is present, and emits `<label>: t` for the diff. Volatile tail
-- (oxiri-like Display tails, partition row counts in DEBUG, etc.)
-- stays out of the baseline.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — re-declared locally; sibling fixtures (`80`, `81`) do
-- the same so each pg_regress file stays self-contained.
CREATE OR REPLACE FUNCTION _check_error(label TEXT, sql TEXT, expected_fragment TEXT)
RETURNS TEXT
LANGUAGE plpgsql AS $$
DECLARE
  msg TEXT;
BEGIN
  BEGIN
    EXECUTE sql;
    RETURN format('%s: !!! unexpected success !!!', label);
  EXCEPTION WHEN OTHERS THEN
    msg := SQLERRM;
  END;
  IF position(expected_fragment IN msg) > 0 THEN
    RETURN format('%s: t', label);
  ELSE
    RETURN format('%s: f (got: %s)', label, left(msg, 80));
  END IF;
END
$$;

-- ─── Invariant 1: idempotent absent ──────────────────────────────
-- `_pgrdf_quads_g8801` does not exist. Drop returns 0 cleanly.
SELECT pgrdf.drop_graph(8801) AS absent_drop;

-- ─── Invariant 2: happy path ─────────────────────────────────────
-- Build a real graph, populate it, drop it. The return value is the
-- pre-drop triple count.
SELECT pgrdf.add_graph(8802) AS created_8802;
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
     VALUES (1, 1, 1, 8802), (2, 2, 2, 8802), (3, 3, 3, 8802), (4, 4, 4, 8802);
SELECT count(*)::bigint AS pre_drop_count
  FROM pgrdf._pgrdf_quads WHERE graph_id = 8802;
SELECT pgrdf.drop_graph(8802) AS dropped_8802;

-- Partition gone from `pg_class`.
SELECT EXISTS(
  SELECT 1 FROM pg_class
   WHERE relnamespace = 'pgrdf'::regnamespace
     AND relname = '_pgrdf_quads_g8802'
) AS partition_still_exists_8802;

-- `_pgrdf_graphs` binding gone.
SELECT count(*)::bigint AS binding_rows_8802
  FROM pgrdf._pgrdf_graphs WHERE graph_id = 8802;

-- Symmetric lookup miss post-drop.
SELECT pgrdf.graph_iri(8802::bigint) IS NULL AS iri_lookup_null_8802;

-- ─── Invariant 3a: cascade=FALSE blocks inferred ─────────────────
-- Build a graph with one base triple + one inferred triple. Strict
-- drop refuses; non-strict drop succeeds.
SELECT pgrdf.add_graph(8803) AS created_8803;
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred)
     VALUES (1, 1, 1, 8803, false),
            (2, 2, 2, 8803, true);

SELECT _check_error(
  'drop_8803_strict',
  'SELECT pgrdf.drop_graph(8803::bigint, cascade => false)',
  'drop_graph: inferred rows present'
);

-- The partition is still there (the panic rolled back the no-op
-- check, but no DDL happened in this transaction).
SELECT EXISTS(
  SELECT 1 FROM pg_class
   WHERE relnamespace = 'pgrdf'::regnamespace
     AND relname = '_pgrdf_quads_g8803'
) AS partition_still_exists_8803_after_strict;

-- ─── Invariant 3b: cascade=TRUE drops inferred + base ─────────────
SELECT pgrdf.drop_graph(8803, cascade => true) AS dropped_8803;
SELECT EXISTS(
  SELECT 1 FROM pg_class
   WHERE relnamespace = 'pgrdf'::regnamespace
     AND relname = '_pgrdf_quads_g8803'
) AS partition_still_exists_8803_after_cascade;

-- ─── Invariant 4: default partition guard ─────────────────────────
SELECT _check_error(
  'drop_default_partition',
  'SELECT pgrdf.drop_graph(0::bigint)',
  'drop_graph: cannot drop default partition'
);

-- ─── Invariant 5: negative-id guard ───────────────────────────────
SELECT _check_error(
  'drop_negative_id',
  'SELECT pgrdf.drop_graph(-1::bigint)',
  'drop_graph: graph_id must be >= 0'
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
