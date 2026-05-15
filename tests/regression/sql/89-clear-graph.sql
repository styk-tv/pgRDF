-- 89-clear-graph.sql
--
-- Phase B slice 98 — `pgrdf.clear_graph(id BIGINT) → BIGINT`
-- lifecycle UDF (LLD v0.4 §5.1). `TRUNCATE ONLY`s the per-graph
-- LIST partition `_pgrdf_quads_g<id>`, wiping every row (base +
-- inferred) and returning the pre-clear row count. The partition
-- shell + the matching `_pgrdf_graphs` IRI binding survive, so
-- subsequent inserts route normally and `graph_iri(id)` still
-- resolves.
--
-- Invariants locked by this file:
--
--   1. Clearing a never-created graph is idempotent — returns 0,
--      no error. (LLD v0.4 §5.2 idempotency.)
--   2. Loading N quads into a graph + clearing returns N. The
--      partition relation still exists in `pg_class` under the
--      `pgrdf` schema afterward, and `_pgrdf_quads` has 0 rows
--      for that `graph_id`. Both base + inferred rows are
--      wiped (the function is not `is_inferred`-discriminating).
--   3. The `_pgrdf_graphs` row for the cleared id stays put — so
--      `graph_iri(id)` keeps resolving to the bound IRI (the
--      distinction from `drop_graph(id)`, which removes the
--      binding).
--   4. Clearing the same graph twice returns 0 the second time
--      (already empty).
--   5. `clear_graph(0)` is permitted — wipes the default
--      catch-all partition. Unlike `drop_graph(0)` (which would
--      destroy the catch-all bucket every unrouted insert
--      depends on), clearing it is harmless.
--   6. Negative id panics with the stable
--      `clear_graph: graph_id must be >= 0, got <N>` prefix.
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

-- ─── Invariant 1: clear absent graph returns 0 (idempotent) ──────
-- 9999 has never had `add_graph(9999)` run for it, so no LIST
-- partition exists. The function detects the missing partition
-- via the `pg_catalog.pg_class` existence check and returns 0
-- without erroring.
SELECT pgrdf.clear_graph(9999::bigint) AS clear_absent;

-- ─── Invariant 2: load + clear returns row count ─────────────────
-- Create graph 100, drop three quads into it (two base, one
-- inferred), then clear. Pre-clear count is 3; clear returns 3;
-- post-clear count is 0. Partition shell stays attached.
SELECT pgrdf.add_graph(100::bigint) AS created;

-- Direct INSERTs with hand-picked term ids — the dictionary
-- doesn't need to know about them because `clear_graph` operates
-- at the partition level, not via the term-resolving UDF surface.
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred) VALUES
  (1001, 1002, 1003, 100, false),
  (2001, 2002, 2003, 100, false),
  (3001, 3002, 3003, 100, true);

SELECT count(*)::bigint AS pre_clear_count
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 100;

SELECT pgrdf.clear_graph(100::bigint) AS removed;

SELECT count(*)::bigint AS post_clear_count
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 100;

-- Partition relation still exists in pg_class — the shell was
-- preserved by `TRUNCATE ONLY`.
SELECT EXISTS(
  SELECT 1
    FROM pg_catalog.pg_class      c
    JOIN pg_catalog.pg_namespace  n ON n.oid = c.relnamespace
   WHERE c.relname = '_pgrdf_quads_g100'
     AND n.nspname = 'pgrdf'
) AS partition_still_attached;

-- ─── Invariant 3: _pgrdf_graphs binding survives clear ───────────
-- `graph_iri(100)` still resolves to the synthetic IRI bound when
-- `add_graph(100)` ran. (The drop_graph slice removes this row;
-- clear_graph keeps it.)
SELECT pgrdf.graph_iri(100::bigint) AS iri_after_clear;

-- ─── Invariant 4: second clear on empty partition returns 0 ──────
SELECT pgrdf.clear_graph(100::bigint) AS second_clear;

-- ─── Invariant 5: clear_graph(0) is permitted ────────────────────
-- LLD v0.4 §5.1 carries `clear_graph(0)` as a legal call — unlike
-- `drop_graph(0)` (sibling slice 99) which rejects the default-
-- partition id outright. Our implementation operates on
-- `_pgrdf_quads_g0` specifically (not on the schema-level default
-- partition `_pgrdf_quads_default`), so we first explicitly create
-- the `g0` LIST partition via `add_graph(0)`. Subsequent INSERTs
-- with `graph_id = 0` route into `_pgrdf_quads_g0` (the explicit
-- partition wins over the default), then `clear_graph(0)` wipes it.
-- The `_pgrdf_graphs` seed row `(0, 'urn:pgrdf:graph:0')` stays put
-- — its IRI binding is untouched by partition TRUNCATE.
SELECT pgrdf.add_graph(0::bigint) AS created_g0;

INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id) VALUES
  (5001, 5002, 5003, 0),
  (5004, 5005, 5006, 0);

SELECT pgrdf.clear_graph(0::bigint) AS cleared_default;

SELECT count(*)::bigint AS post_default_count
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 0;

SELECT pgrdf.graph_iri(0::bigint) AS default_iri_after_clear;

-- ─── Invariant 6: negative id panics with stable prefix ──────────
SELECT _check_error(
  'clear-graph-negative',
  'SELECT pgrdf.clear_graph(-1::bigint)',
  'clear_graph: graph_id must be >= 0, got -1'
);
