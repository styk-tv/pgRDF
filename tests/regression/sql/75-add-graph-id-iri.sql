-- 75-add-graph-id-iri.sql
--
-- Phase A slice 117 — `pgrdf.add_graph(id BIGINT, iri TEXT) → BIGINT`
-- explicit-binding overload (LLD v0.4 §3.2). Caller specifies both
-- halves; idempotent on matching pairs; errors on conflicting
-- bindings (id bound to a different IRI, or IRI bound to a different
-- id); UPDATEs in place when the existing IRI for `id` is the
-- synthetic placeholder `urn:pgrdf:graph:{id}` (the upgrade path
-- covering `add_graph(42)` → `add_graph(42, 'http://…/g42')`).
--
-- Invariants locked by this file:
--
--   1. Fresh pair `add_graph(50, 'http://example.org/g50')` returns
--      `50`, INSERTs the row, and creates the LIST partition.
--   2. Idempotent re-call against the same pair returns `50` again,
--      no duplicate row.
--   3. Synthetic-upgrade path: `add_graph(60)` first seeds
--      `urn:pgrdf:graph:60`; a subsequent
--      `add_graph(60, 'http://example.org/g60')` UPDATEs the row in
--      place, single row remains, IRI is now the user-supplied one.
--   4. Id conflict: `add_graph(70, …)` then a second call with the
--      same id but a different IRI panics with the stable
--      `add_graph: graph_id 70 is bound to a different IRI` prefix.
--   5. IRI conflict: `add_graph(80, 'http://example.org/shared')`
--      then `add_graph(81, 'http://example.org/shared')` panics
--      with the stable
--      `add_graph: iri http://example.org/shared is bound to a
--      different graph_id` prefix.
--   6. Negative `id` panics with the stable
--      `add_graph: graph_id must be >= 0` prefix (matches the
--      integer-overload check at `hexastore.rs:51`).
--   7. Empty `iri` panics with the stable
--      `add_graph: iri must be non-empty` prefix (shared with the
--      slice-118 IRI-keyed overload).
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Fresh pair: add_graph(50, 'http://example.org/g50') ──────────
SELECT pgrdf.add_graph(50::bigint, 'http://example.org/g50') AS fresh_add;
SELECT iri AS bound_50 FROM pgrdf._pgrdf_graphs WHERE graph_id = 50;

-- ─── Idempotent re-call against the same pair ────────────────────
SELECT pgrdf.add_graph(50::bigint, 'http://example.org/g50') AS repeat_add;
SELECT count(*)::bigint AS row_count_50
  FROM pgrdf._pgrdf_graphs WHERE graph_id = 50;

-- ─── Synthetic upgrade: add_graph(60) then add_graph(60, '…') ────
SELECT pgrdf.add_graph(60::bigint) AS seed_synthetic;
SELECT iri AS pre_upgrade_iri_60
  FROM pgrdf._pgrdf_graphs WHERE graph_id = 60;
SELECT pgrdf.add_graph(60::bigint, 'http://example.org/g60') AS upgrade_add;
SELECT iri AS post_upgrade_iri_60
  FROM pgrdf._pgrdf_graphs WHERE graph_id = 60;
SELECT count(*)::bigint AS row_count_60
  FROM pgrdf._pgrdf_graphs WHERE graph_id = 60;

-- ─── Error-message contract checks ───────────────────────────────
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

-- Id conflict: bind 70 to one IRI, then try to bind it to another.
SELECT pgrdf.add_graph(70::bigint, 'http://example.org/g70') AS seed_70;
SELECT _check_error(
  'add_graph id conflict',
  $$SELECT pgrdf.add_graph(70::bigint, 'http://example.org/different')$$,
  'add_graph: graph_id 70 is bound to a different IRI'
);

-- IRI conflict: bind 80 to a shared IRI, then try to bind 81 to the
-- same IRI.
SELECT pgrdf.add_graph(80::bigint, 'http://example.org/shared') AS seed_80;
SELECT _check_error(
  'add_graph iri conflict',
  $$SELECT pgrdf.add_graph(81::bigint, 'http://example.org/shared')$$,
  'add_graph: iri http://example.org/shared is bound to a different graph_id'
);

-- Negative id rejected.
SELECT _check_error(
  'add_graph negative id',
  $$SELECT pgrdf.add_graph(-1::bigint, 'http://example.org/x')$$,
  'add_graph: graph_id must be >= 0'
);

-- Empty IRI rejected (shared prefix with slice 118).
SELECT _check_error(
  'add_graph empty iri',
  $$SELECT pgrdf.add_graph(90::bigint, '')$$,
  'add_graph: iri must be non-empty'
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
