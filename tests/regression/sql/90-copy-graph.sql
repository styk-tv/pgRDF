-- 90-copy-graph.sql
--
-- Phase B slice 97 — `pgrdf.copy_graph(src BIGINT, dst BIGINT) →
-- BIGINT` lifecycle UDF (LLD v0.4 §5.1). Copies every row from
-- `pgrdf._pgrdf_quads_g<src>` into `pgrdf._pgrdf_quads_g<dst>` via
-- an `INSERT INTO … SELECT` against the per-graph LIST partitions,
-- returning the count copied. The destination partition is
-- auto-created via `pgrdf.add_graph(dst)` if it does not already
-- exist (so the synthetic `urn:pgrdf:graph:{dst}` IRI is bound
-- alongside).
--
-- Invariants locked by this file:
--
--   1. Copy from absent src returns 0, no error — dst partition is
--      NOT auto-created in this path (short-circuit on src-existence
--      check). LLD v0.4 §5.2 idempotency.
--   2. Loading N quads into src + copying to a fresh dst returns N.
--      The dst partition was auto-created and now contains every
--      copied row with `graph_id` rebound to dst. Source partition
--      is untouched.
--   3. Both `is_inferred = FALSE` and `is_inferred = TRUE` rows
--      carry forward verbatim — entailment state is preserved per
--      LLD v0.4 §5.2.
--   4. `pgrdf.graph_iri(dst)` resolves post-copy (the auto-create
--      path bound the synthetic IRI per slice 119).
--   5. Re-calling `copy_graph(src, dst)` against the same pair
--      duplicates rows — the function is NOT idempotent on the
--      dst side. Callers needing re-call idempotency clear dst
--      first.
--   6. `src == dst` panics with the stable
--      `copy_graph: src and dst must differ` prefix.
--   7. Negative ids panic with the stable
--      `copy_graph: graph_id must be >= 0` prefix.
--
-- All expected values hand-computed. The `_check_error` helper
-- from `81-error-paths.sql` is redefined locally so this file is
-- self-contained (regression files run in isolation).

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Local _check_error helper (mirrors 81-error-paths.sql) ──────
-- Runs `sql` inside try/catch; returns `<label>: t` if the panic
-- message contains `expected_fragment`, otherwise diagnostics.
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

-- ─── Invariant 1: copy from absent src returns 0 (idempotent) ────
-- 9701 has never had `add_graph(9701)` run for it, so no LIST
-- partition exists. The function detects the missing src partition
-- via the `pg_catalog.pg_class` existence check and returns 0
-- without erroring. The dst partition is NOT auto-created on this
-- short-circuit path — we verify that follow-up.
SELECT pgrdf.copy_graph(9701::bigint, 9702::bigint) AS copy_absent_src;

SELECT EXISTS(
  SELECT 1
    FROM pg_catalog.pg_class      c
    JOIN pg_catalog.pg_namespace  n ON n.oid = c.relnamespace
   WHERE c.relname = '_pgrdf_quads_g9702'
     AND n.nspname = 'pgrdf'
) AS dst_not_auto_created;

-- ─── Invariant 2-4: load src + copy to fresh dst ─────────────────
-- Create graph 200, drop three quads into it (two base, one
-- inferred), copy to fresh dst graph 201. Expect:
--   * copy returns 3 (the src row count)
--   * dst partition was auto-created (pg_class probe)
--   * dst holds 3 rows with rebound graph_id
--   * one dst row has is_inferred = TRUE (preserved)
--   * graph_iri(201) resolves to the synthetic IRI
--   * src is untouched (3 rows still resident)
SELECT pgrdf.add_graph(200::bigint) AS created_src;

INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred) VALUES
  (1001, 1002, 1003, 200, false),
  (2001, 2002, 2003, 200, false),
  (3001, 3002, 3003, 200, true);

SELECT count(*)::bigint AS pre_copy_src_count
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 200;

-- copy_graph: src 200 has 3 rows, dst 201 is fresh (no partition).
-- Return value is the count copied (3); the function auto-creates
-- _pgrdf_quads_g201 via pgrdf.add_graph(201).
SELECT pgrdf.copy_graph(200::bigint, 201::bigint) AS copied;

-- dst partition auto-created.
SELECT EXISTS(
  SELECT 1
    FROM pg_catalog.pg_class      c
    JOIN pg_catalog.pg_namespace  n ON n.oid = c.relnamespace
   WHERE c.relname = '_pgrdf_quads_g201'
     AND n.nspname = 'pgrdf'
) AS dst_auto_created;

-- dst now holds 3 rows, all under graph_id = 201.
SELECT count(*)::bigint AS dst_count
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 201;

-- is_inferred flag carried forward — exactly one inferred row in dst.
SELECT count(*)::bigint AS dst_inferred_count
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 201 AND is_inferred = true;

-- graph_iri(dst) resolves: the auto-create path bound the synthetic
-- IRI per slice 119's add_graph(id BIGINT) behaviour.
SELECT pgrdf.graph_iri(201::bigint) AS dst_iri;

-- src untouched — copy is non-destructive.
SELECT count(*)::bigint AS post_copy_src_count
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 200;

-- ─── Invariant 5: re-call duplicates ─────────────────────────────
-- copy_graph is NOT idempotent on dst — calling again with the
-- same (src, dst) appends another 3 rows to dst (total 6). To get
-- re-call idempotency, callers do `clear_graph(dst)` first; the
-- post-clear copy then yields exactly 3 again. This locks the
-- documented "callers handle clear-first" contract.
SELECT pgrdf.copy_graph(200::bigint, 201::bigint) AS recopied;

SELECT count(*)::bigint AS dst_after_recopy
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 201;

-- Clear + copy round trip returns to a 3-row dst.
SELECT pgrdf.clear_graph(201::bigint) AS cleared_for_recopy;
SELECT pgrdf.copy_graph(200::bigint, 201::bigint) AS clean_recopied;
SELECT count(*)::bigint AS dst_after_clean_recopy
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 201;

-- ─── Invariant 6: src == dst rejected ────────────────────────────
SELECT _check_error(
  'copy-graph-self',
  'SELECT pgrdf.copy_graph(200::bigint, 200::bigint)',
  'copy_graph: src and dst must differ (both = 200)'
);

-- ─── Invariant 7: negative ids rejected ──────────────────────────
SELECT _check_error(
  'copy-graph-negative-src',
  'SELECT pgrdf.copy_graph(-1::bigint, 200::bigint)',
  'copy_graph: graph_id must be >= 0, got src=-1, dst=200'
);

SELECT _check_error(
  'copy-graph-negative-dst',
  'SELECT pgrdf.copy_graph(200::bigint, -1::bigint)',
  'copy_graph: graph_id must be >= 0, got src=200, dst=-1'
);
