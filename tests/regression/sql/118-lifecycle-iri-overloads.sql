-- 118-lifecycle-iri-overloads.sql
--
-- Phase G group G1 (slices 21-18, SPEC.pgRDF.LLD.v0.5-FUTURE §7) —
-- IRI-keyed overloads for the four v0.4 §5 lifecycle UDFs:
--
--   pgrdf.drop_graph(iri TEXT, cascade BOOLEAN DEFAULT TRUE) → BIGINT
--   pgrdf.clear_graph(iri TEXT)                              → BIGINT
--   pgrdf.copy_graph(src_iri TEXT, dst_iri TEXT)             → BIGINT
--   pgrdf.move_graph(src_iri TEXT, dst_iri TEXT)             → BIGINT
--
-- Semantics are IDENTICAL to the BIGINT overloads — each IRI
-- overload resolves `iri → graph_id` via `_pgrdf_graphs.iri` and
-- dispatches to the SAME Rust UDF (partition-DDL logic single-
-- sourced; the IRI overload is pure ergonomics, no duplication).
-- pgrx surfaces both signatures under one SQL name via
-- `#[pg_extern(name = "...")]` and Postgres dispatches on argument
-- types — the same pattern `add_graph` uses (slices 117/118).
--
-- The ONE intentional behavioural difference (§7.1 #2): an unbound
-- IRI is an ERROR with the stable prefix `<fn>: unknown iri`, NOT
-- the BIGINT overloads' no-op-returns-0 on an absent id.
--
-- Invariants locked by this file (all expected values hand-computed;
-- never ACCEPT=1 baselined):
--
--   G. drop_graph(iri) ≡ drop_graph(graph_id(iri)) — seed a named
--      graph, drop by IRI, assert the partition AND the binding are
--      gone; the return value is the pre-drop triple count. Resolve
--      agrees with pgrdf.graph_id(iri).
--   H. clear_graph(iri) empties the partition but KEEPS the binding
--      (mirrors the BIGINT clear_graph; partition stays attached).
--   I. copy_graph(src_iri,dst_iri) duplicates rows into dst, leaves
--      src intact; move_graph(src_iri,dst_iri) migrates rows then
--      unbinds src + leaves dst bound — both byte-identical to the
--      BIGINT §5 semantics (return = src row count at copy time).
--   J. Unknown-IRI errors: each of the four overloads on an unbound
--      IRI errors with the stable prefix `<fn>: unknown iri`
--      (drop/clear/copy/move). DISTINCT from the BIGINT overloads'
--      no-op-on-absent-id (re-asserted: drop_graph(99999) → 0, not
--      an error).
--   K. IRI overload composes with the v0.4 §4 SPARQL UPDATE
--      lifecycle algebra: drop a graph via the IRI overload, then a
--      `CREATE GRAPH <same-iri>` SPARQL UPDATE rebinds the IRI
--      cleanly to a fresh partition.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — re-declared locally (same shape as 88 / 99 / 117).
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

-- ─── Invariant G: drop_graph(iri) ≡ drop_graph(graph_id(iri)) ────
-- Bind a named graph, populate it with 4 triples, drop by IRI.
SELECT pgrdf.add_graph('http://example.org/g-drop') AS g_drop_id \gset
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
     VALUES (1,1,1,:g_drop_id), (2,2,2,:g_drop_id),
            (3,3,3,:g_drop_id), (4,4,4,:g_drop_id);

-- Resolution agrees with pgrdf.graph_id(iri).
SELECT pgrdf.graph_id('http://example.org/g-drop') = :g_drop_id
  AS drop_iri_resolves_to_same_id;

SELECT pgrdf.drop_graph('http://example.org/g-drop') AS dropped_by_iri;

-- Partition gone from pg_class.
SELECT EXISTS(
  SELECT 1 FROM pg_class
   WHERE relnamespace = 'pgrdf'::regnamespace
     AND relname = '_pgrdf_quads_g' || :g_drop_id::text
) AS drop_partition_still_exists;

-- Binding gone (graph_id lookup returns NULL).
SELECT pgrdf.graph_id('http://example.org/g-drop') IS NULL
  AS drop_binding_gone;

-- ─── Invariant H: clear_graph(iri) keeps the binding ────────────
SELECT pgrdf.add_graph('http://example.org/g-clear') AS g_clear_id \gset
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
     VALUES (1,1,1,:g_clear_id), (2,2,2,:g_clear_id), (3,3,3,:g_clear_id);

SELECT pgrdf.clear_graph('http://example.org/g-clear') AS cleared_by_iri;

SELECT count(*)::bigint = 0 AS clear_partition_emptied
  FROM pgrdf._pgrdf_quads WHERE graph_id = :g_clear_id;

SELECT pgrdf.graph_id('http://example.org/g-clear') = :g_clear_id
  AS clear_binding_preserved;

SELECT EXISTS(
  SELECT 1 FROM pg_class
   WHERE relnamespace = 'pgrdf'::regnamespace
     AND relname = '_pgrdf_quads_g' || :g_clear_id::text
) AS clear_partition_still_attached;

-- ─── Invariant I: copy_graph / move_graph (IRI) mirror BIGINT ───
SELECT pgrdf.add_graph('http://example.org/g-src') AS g_src_id \gset
SELECT pgrdf.add_graph('http://example.org/g-dst') AS g_dst_id \gset
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
     VALUES (1,1,1,:g_src_id), (2,2,2,:g_src_id), (3,3,3,:g_src_id);

-- copy: return = src row count; src intact; dst populated.
SELECT pgrdf.copy_graph('http://example.org/g-src',
                         'http://example.org/g-dst') AS copied_by_iri;
SELECT count(*)::bigint = 3 AS copy_src_intact
  FROM pgrdf._pgrdf_quads WHERE graph_id = :g_src_id;
SELECT count(*)::bigint = 3 AS copy_dst_populated
  FROM pgrdf._pgrdf_quads WHERE graph_id = :g_dst_id;

-- move: src → a fresh dst. After the move src is UNBOUND, dst BOUND,
-- dst holds the rows. Return = src row count at copy time (3).
SELECT pgrdf.add_graph('http://example.org/g-mv-src') AS g_mvsrc_id \gset
SELECT pgrdf.add_graph('http://example.org/g-mv-dst') AS g_mvdst_id \gset
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
     VALUES (5,5,5,:g_mvsrc_id), (6,6,6,:g_mvsrc_id), (7,7,7,:g_mvsrc_id);

SELECT pgrdf.move_graph('http://example.org/g-mv-src',
                         'http://example.org/g-mv-dst') AS moved_by_iri;
SELECT pgrdf.graph_id('http://example.org/g-mv-src') IS NULL
  AS move_src_unbound;
SELECT pgrdf.graph_id('http://example.org/g-mv-dst') = :g_mvdst_id
  AS move_dst_still_bound;
SELECT count(*)::bigint = 3 AS move_dst_has_rows
  FROM pgrdf._pgrdf_quads WHERE graph_id = :g_mvdst_id;

-- ─── Invariant J: unknown-IRI errors (all four overloads) ───────
SELECT _check_error(
  'drop_unknown_iri',
  'SELECT pgrdf.drop_graph(''http://example.org/nope-drop'')',
  'drop_graph: unknown iri'
);
SELECT _check_error(
  'clear_unknown_iri',
  'SELECT pgrdf.clear_graph(''http://example.org/nope-clear'')',
  'clear_graph: unknown iri'
);
SELECT _check_error(
  'copy_unknown_src_iri',
  'SELECT pgrdf.copy_graph(''http://example.org/nope-src'', ''http://example.org/nope-dst'')',
  'copy_graph: unknown iri'
);
SELECT _check_error(
  'move_unknown_src_iri',
  'SELECT pgrdf.move_graph(''http://example.org/nope-msrc'', ''http://example.org/nope-mdst'')',
  'move_graph: unknown iri'
);

-- The BIGINT overload's no-op-on-absent-id is UNCHANGED (distinct
-- semantics — re-assert it still returns 0, not an error).
SELECT pgrdf.drop_graph(99999::bigint) AS bigint_absent_still_noop;

-- ─── Invariant K: compose with v0.4 §4 SPARQL UPDATE algebra ────
-- Bind + populate a graph, drop it via the IRI overload, then a
-- SPARQL `CREATE GRAPH <same-iri>` UPDATE rebinds the IRI cleanly.
SELECT pgrdf.add_graph('http://example.org/g-compose') AS g_comp_id \gset
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
     VALUES (1,1,1,:g_comp_id), (2,2,2,:g_comp_id);

SELECT pgrdf.drop_graph('http://example.org/g-compose') AS compose_dropped;
SELECT pgrdf.graph_id('http://example.org/g-compose') IS NULL
  AS compose_unbound_after_drop;

-- CREATE GRAPH via SPARQL UPDATE rebinds the IRI to a fresh
-- partition (v0.4 §4 lifecycle algebra). form must be CREATE,
-- no row changes.
SELECT
  (j->'_update'->>'form')                     AS compose_create_form,
  (j->'_update'->>'triples_inserted')::bigint AS compose_create_inserted
FROM pgrdf.sparql('CREATE GRAPH <http://example.org/g-compose>') AS s(j);

SELECT pgrdf.graph_id('http://example.org/g-compose') IS NOT NULL
  AS compose_rebound_after_create;

-- The rebound graph is usable: INSERT DATA into it, then the IRI
-- overload clear_graph empties it again (full round-trip).
SELECT (j->'_update'->>'triples_inserted')::bigint AS compose_reinsert
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { GRAPH <http://example.org/g-compose> { ex:x ex:p "v" } }'
) AS s(j);

SELECT pgrdf.clear_graph('http://example.org/g-compose') AS compose_clear_after_recreate;

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
