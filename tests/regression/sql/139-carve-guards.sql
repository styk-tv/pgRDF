-- 139-carve-guards.sql — guard + edge-case coverage for pgrdf.carve_graph (#33)
--
-- 137/138 lock the happy-path COUNTS; this locks the GUARDS the code already
-- implements (graphs.rs: carve_graph @690 predicate overload, carve_graph @796
-- neighbourhood overload) so the #32 index-only EXTRACT rewrite can't silently
-- regress them. Both overloads covered.
--
-- Fixture: multiload-dedup-sample.nt (8 N-Triples; alice→bob→carol knows-chain
-- + foaf:name + ex:age) into graph 300; carve-blanknode-sample.nt (x→_:b0→y)
-- into graph 400 for the blank-node-frontier case.
--
-- NOT here (deferred per the issue): the plan-shape index-only assertion rides
-- with #32 (flaky at 8-row fixture scale); the C2 re-encode round-trip rides
-- with #19.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph(300);
SELECT pgrdf.load_turtle('/fixtures/regression/multiload-dedup-sample.nt', 300);
SELECT 'src_quads_8: ' || (count(*) = 8) FROM pgrdf._pgrdf_quads WHERE graph_id = 300;

-- Helper: run a carve call, assert it RAISES with the expected message fragment
-- (the guard panics abort the txn, so each runs in its own sub-block).
CREATE OR REPLACE FUNCTION _carve_raises(label TEXT, sql TEXT, frag TEXT)
RETURNS TEXT LANGUAGE plpgsql AS $$
DECLARE msg TEXT;
BEGIN
  BEGIN
    EXECUTE sql;
    RETURN label || ': !!! unexpected success !!!';
  EXCEPTION WHEN OTHERS THEN msg := SQLERRM;
  END;
  IF position(frag IN msg) > 0 THEN RETURN label || ': t';
  ELSE RETURN label || ': f (' || left(msg, 70) || ')'; END IF;
END $$;

-- ── GUARD: src == dst rejected (both overloads) ───────────────────────────
SELECT _carve_raises('pred_self_rejected',
  $q$SELECT pgrdf.carve_graph(300, 'http://xmlns.com/foaf/0.1/knows', 300)$q$,
  'src and dst must differ');
SELECT _carve_raises('nbhd_self_rejected',
  $q$SELECT pgrdf.carve_graph(300, ARRAY['http://example.org/alice']::text[], 300, 1)$q$,
  'src and dst must differ');

-- ── GUARD: negative graph_id rejected (both overloads) ────────────────────
SELECT _carve_raises('pred_neg_dst_rejected',
  $q$SELECT pgrdf.carve_graph(300, 'http://xmlns.com/foaf/0.1/knows', -1)$q$,
  'graph_id must be >= 0');
SELECT _carve_raises('nbhd_neg_src_rejected',
  $q$SELECT pgrdf.carve_graph(-5, ARRAY['http://example.org/alice']::text[], 301, 1)$q$,
  'graph_id must be >= 0');

-- ── EDGE: absent src → 0, and no dst partition created (both overloads) ────
SELECT 'pred_absent_src_0: ' || (pgrdf.carve_graph(999, 'http://xmlns.com/foaf/0.1/knows', 310) = 0);
SELECT 'pred_absent_no_dst_310: ' || (count(*) = 0)
  FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE c.relname = '_pgrdf_quads_g310' AND n.nspname = 'pgrdf';
SELECT 'nbhd_absent_src_0: ' || (pgrdf.carve_graph(999, ARRAY['http://example.org/alice']::text[], 311, 1) = 0);
SELECT 'nbhd_absent_no_dst_311: ' || (count(*) = 0)
  FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
  WHERE c.relname = '_pgrdf_quads_g311' AND n.nspname = 'pgrdf';

-- ── SEMANTICS: carve into an already-populated dst APPENDS (no UNIQUE on quads) ──
SELECT 'pred_knows_into_320_is_2: ' || (pgrdf.carve_graph(300, 'http://xmlns.com/foaf/0.1/knows', 320) = 2);
SELECT 'dst320_after_first_2: ' || (count(*) = 2) FROM pgrdf._pgrdf_quads WHERE graph_id = 320;
SELECT 'pred_knows_into_320_again_2: ' || (pgrdf.carve_graph(300, 'http://xmlns.com/foaf/0.1/knows', 320) = 2);
SELECT 'dst320_appended_to_4: ' || (count(*) = 4) FROM pgrdf._pgrdf_quads WHERE graph_id = 320;

-- ── EDGE: neighbourhood max_hops >> diameter terminates, no over-carve ─────
-- (2 hops already reaches the whole 8-quad graph per 138; 99 hops must equal it)
SELECT 'nbhd_overhops_99_stable_8: ' || (pgrdf.carve_graph(300, ARRAY['http://example.org/alice']::text[], 330, 99) = 8);

-- ── source + dictionary untouched by all the carves above ─────────────────
SELECT 'src_unchanged_8: ' || (count(*) = 8) FROM pgrdf._pgrdf_quads WHERE graph_id = 300;
SELECT 'dict_unchanged_14: ' || (count(*) = 14) FROM pgrdf._pgrdf_dictionary;

-- ── SEMANTICS: is_inferred carries forward into the slice ─────────────────
-- Insert an inferred 'alice knows carol' (novel triple, reuses interned ids so
-- the dict stays 14), then carve foaf:knows → 3, slice keeps the inferred flag.
INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred)
SELECT
  (SELECT id FROM pgrdf._pgrdf_dictionary WHERE term_type = 1 AND lexical_value = 'http://example.org/alice'),
  (SELECT id FROM pgrdf._pgrdf_dictionary WHERE term_type = 1 AND lexical_value = 'http://xmlns.com/foaf/0.1/knows'),
  (SELECT id FROM pgrdf._pgrdf_dictionary WHERE term_type = 1 AND lexical_value = 'http://example.org/carol'),
  300, true;
SELECT 'src300_now_9: ' || (count(*) = 9) FROM pgrdf._pgrdf_quads WHERE graph_id = 300;
SELECT 'pred_knows_with_inferred_3: ' || (pgrdf.carve_graph(300, 'http://xmlns.com/foaf/0.1/knows', 340) = 3);
SELECT 'inferred_carried_to_slice_1: ' || (count(*) = 1) FROM pgrdf._pgrdf_quads WHERE graph_id = 340 AND is_inferred = true;

-- ── EDGE: neighbourhood traverses a BLANK NODE in the frontier ────────────
SELECT pgrdf.add_graph(400);
SELECT pgrdf.load_turtle('/fixtures/regression/carve-blanknode-sample.nt', 400);
SELECT 'blanknode_src_2: ' || (count(*) = 2) FROM pgrdf._pgrdf_quads WHERE graph_id = 400;
-- 1 hop from x reaches _:b0 (blank) and through it the (_:b0 link y) quad → 2
SELECT 'nbhd_blanknode_1hop_2: ' || (pgrdf.carve_graph(400, ARRAY['http://example.org/x']::text[], 401, 1) = 2);
SELECT 'blanknode_slice_has_y: ' || (count(*) = 1)
  FROM pgrdf._pgrdf_quads q JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
  WHERE q.graph_id = 401 AND o.lexical_value = 'http://example.org/y';

DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
