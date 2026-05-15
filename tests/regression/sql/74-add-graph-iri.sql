-- 74-add-graph-iri.sql
--
-- Phase A slice 118 — `pgrdf.add_graph(iri TEXT) → BIGINT` overload
-- (LLD v0.4 §3.2). Auto-allocates the next `graph_id` (smallest
-- unused positive integer via `COALESCE(MAX(graph_id), 0) + 1`),
-- binds the IRI verbatim in `_pgrdf_graphs`, and creates the matching
-- LIST partition of `_pgrdf_quads`. Idempotent on the IRI: a second
-- call with the same IRI returns the existing id, no new partition,
-- no duplicate row.
--
-- Invariants locked by this file:
--
--   1. Baseline after `CREATE EXTENSION` is exactly one row — the
--      seed `(0, 'urn:pgrdf:graph:0')`.
--   2. First IRI-keyed `add_graph('http://example.org/g1')` returns
--      `1` (the smallest unused positive integer).
--   3. A repeat call with `'http://example.org/g1'` returns the same
--      `1` — idempotent, no new row.
--   4. A distinct IRI `'http://example.org/g2'` returns `2`.
--   5. The user-supplied IRI is bound verbatim — NOT the synthetic
--      `urn:pgrdf:graph:{id}` shape that the integer overload uses.
--      The pre-INSERT inside the IRI overload pre-empts the
--      slice-119 synthetic-IRI insert path on `add_graph(id BIGINT)`.
--   6. Mixing surfaces is safe: `pgrdf.add_graph(100)` (integer
--      overload) binds the synthetic IRI; a subsequent IRI-keyed
--      `add_graph('http://example.org/g3')` allocates `101`
--      (one past the current `MAX(graph_id)`).
--   7. Empty-string IRI panics with the stable
--      `add_graph: iri must be non-empty` prefix.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Baseline — only the seed row ─────────────────────────────────
SELECT count(*)::bigint AS baseline FROM pgrdf._pgrdf_graphs;

-- ─── First IRI-keyed add returns the auto-allocated id (1) ────────
SELECT pgrdf.add_graph('http://example.org/g1') AS first_add;

-- ─── Idempotent re-call returns the same id (1) ───────────────────
SELECT pgrdf.add_graph('http://example.org/g1') AS repeat_add;

-- ─── Distinct IRI gets the next id (2) ────────────────────────────
SELECT pgrdf.add_graph('http://example.org/g2') AS distinct_add;

-- ─── User-supplied IRIs persist verbatim, ordered by graph_id ─────
SELECT graph_id, iri
  FROM pgrdf._pgrdf_graphs
 ORDER BY graph_id;

-- ─── Mixing with the integer-keyed surface ────────────────────────
SELECT pgrdf.add_graph(100) AS int_add_100;
SELECT iri AS synthetic_iri_for_100
  FROM pgrdf._pgrdf_graphs
 WHERE graph_id = 100;

-- ─── Next IRI-keyed add allocates 101 (one past current MAX) ──────
SELECT pgrdf.add_graph('http://example.org/g3') AS add_after_int;

-- ─── Empty IRI errors with the stable prefix ──────────────────────
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

SELECT _check_error(
  'add_graph empty iri',
  $$SELECT pgrdf.add_graph('')$$,
  'add_graph: iri must be non-empty'
);

SELECT _check_error(
  'add_graph whitespace-only iri',
  $$SELECT pgrdf.add_graph('   ')$$,
  'add_graph: iri must be non-empty'
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
