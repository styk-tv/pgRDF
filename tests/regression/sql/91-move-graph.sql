-- 91-move-graph.sql
--
-- Phase B slice 96 — `pgrdf.move_graph(src BIGINT, dst BIGINT) →
-- BIGINT` lifecycle UDF (LLD v0.4 §5.1). Migrates every quad in
-- graph `src` to graph `dst` and removes `src`, returning the count
-- of triples moved.
--
-- v0.4.2 implementation strategy is a compose over the sibling
-- primitives: `pgrdf.copy_graph(src, dst)` (slice 97) then
-- `pgrdf.drop_graph(src, cascade => TRUE)` (slice 99). The §5.2
-- "metadata-only DETACH/ATTACH rebind" claim is aspirational; the
-- compose approach is correctness-preserving but not constant-time
-- (a v0.5 perf optimisation).
--
-- Invariants locked by this file:
--
--   1. Happy path — N quads in `src`, run move, return value is N;
--      `src` partition gone; `dst` partition holds N rows; the
--      `_pgrdf_graphs` row for `src` is removed, the one for `dst`
--      is allocated with the synthetic IRI.
--   2. Idempotent absent — move with a non-existent `src` returns
--      0 without erroring (no call into `copy_graph` is made; the
--      function short-circuits on the existence check).
--   3. src == dst rejected — `pgrdf.move_graph(N, N)` panics with
--      `move_graph: src and dst must differ`. A self-move would
--      be destructive (copy-then-drop the dst).
--   4. dst-has-data rejected — when the dst partition already holds
--      rows, `move_graph` panics with `move_graph: dst graph_id <N>
--      already has data` rather than silently appending. Callers
--      who want merge semantics use `copy_graph` directly.
--   5. Negative-id rejected — `pgrdf.move_graph(-1, N)` panics with
--      `move_graph: graph_id must be >= 0`. Same shape as the
--      sibling lifecycle UDFs.
--
-- **Runtime dependency.** Invariant 1 (happy path) calls
-- `pgrdf.copy_graph` under the covers. Slice 97 lands `copy_graph`
-- in parallel; this regression file is correct in shape but will
-- fail in the slice-96 worktree until slice 97 merges. The
-- standalone invariants (2 / 3 / 4 / 5) run green in this worktree.
--
-- The error-path checks reuse the `_check_error(label, sql, frag)`
-- plpgsql helper introduced in `81-error-paths.sql`, redefined
-- locally so this file stays self-contained.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Local _check_error helper (mirrors 81-error-paths.sql) ──────
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

-- ─── Invariant 1: happy path ─────────────────────────────────────
-- Build 9101 with four rows (three base + one inferred). Move to
-- 9102. The return value is 4; 9101 is gone; 9102 holds the rows.
SELECT pgrdf.add_graph(9101) AS created_9101;
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred)
     VALUES (1, 1, 1, 9101, false),
            (2, 2, 2, 9101, false),
            (3, 3, 3, 9101, false),
            (4, 4, 4, 9101, true);

SELECT pgrdf.move_graph(9101::bigint, 9102::bigint) AS moved_9101_to_9102;

SELECT EXISTS(
  SELECT 1 FROM pg_class
   WHERE relnamespace = 'pgrdf'::regnamespace
     AND relname = '_pgrdf_quads_g9101'
) AS src_partition_still_exists;

SELECT count(*)::bigint AS dst_row_count
  FROM pgrdf._pgrdf_quads WHERE graph_id = 9102;

SELECT pgrdf.graph_iri(9101::bigint) IS NULL AS src_iri_unbound;
SELECT pgrdf.graph_iri(9102::bigint) AS dst_iri_after_move;

-- ─── Invariant 2: idempotent absent ──────────────────────────────
-- 9103 partition does not exist. Move returns 0 without erroring.
-- No call to `copy_graph` is made (existence-check short circuit),
-- so this invariant is independent of slice 97.
SELECT pgrdf.move_graph(9103::bigint, 9104::bigint) AS absent_move;

-- ─── Invariant 3: src == dst rejected ────────────────────────────
SELECT _check_error(
  'move_self',
  'SELECT pgrdf.move_graph(9105::bigint, 9105::bigint)',
  'move_graph: src and dst must differ'
);

-- ─── Invariant 4: dst-has-data rejected ──────────────────────────
SELECT pgrdf.add_graph(9106) AS created_9106;
SELECT pgrdf.add_graph(9107) AS created_9107;
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
     VALUES (10, 10, 10, 9106),
            (20, 20, 20, 9107);

SELECT _check_error(
  'move_dst_has_data',
  'SELECT pgrdf.move_graph(9106::bigint, 9107::bigint)',
  'move_graph: dst graph_id 9107 already has data'
);

-- ─── Invariant 5: negative-id rejected ───────────────────────────
SELECT _check_error(
  'move_negative_id',
  'SELECT pgrdf.move_graph(-1::bigint, 9108::bigint)',
  'move_graph: graph_id must be >= 0'
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
