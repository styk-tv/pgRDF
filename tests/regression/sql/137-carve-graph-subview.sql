-- 137-carve-graph-subview.sql — pgrdf.carve_graph subview MVP (C1, issue #10).
--
-- carve_graph(src_graph, predicate, dst_graph) is a predicated copy_graph: it
-- carves every quad of `src` whose predicate is `predicate` into a NEW graph
-- `dst` in the same database (shared dictionary, no decode) — the SPEC.pgRDF.CARVE
-- §5.A fast sub-view. It returns the number of quads carved. The dictionary is
-- untouched (the slice shares the source term space); an unbound predicate
-- carves nothing and returns 0.
--
-- Fixture: multiload-dedup-sample.nt (8 N-Triples, 14 distinct terms):
--   foaf:knows  ×2  (alice→bob, bob→carol)
--   foaf:name   ×3  (Alice, Bob@en, Carol@en)
--   ex:age      ×3  (28, 30, 34)

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph(100);
SELECT pgrdf.load_turtle('/fixtures/regression/multiload-dedup-sample.nt', 100);
SELECT 'src_quads_8: ' || (count(*) = 8) FROM pgrdf._pgrdf_quads WHERE graph_id = 100;

-- ── carve foaf:knows (2 quads) into a new graph 101 ──────────────────────
SELECT 'carve_knows_returns_2: ' || (pgrdf.carve_graph(100, 'http://xmlns.com/foaf/0.1/knows', 101) = 2);
SELECT 'dst101_quads_2: ' || (count(*) = 2) FROM pgrdf._pgrdf_quads WHERE graph_id = 101;
-- the slice contains ONLY foaf:knows quads
SELECT 'dst101_only_knows: ' || (count(*) = 2)
  FROM pgrdf._pgrdf_quads q JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  WHERE q.graph_id = 101 AND p.lexical_value = 'http://xmlns.com/foaf/0.1/knows';
-- the source graph is untouched
SELECT 'src_unchanged_8: ' || (count(*) = 8) FROM pgrdf._pgrdf_quads WHERE graph_id = 100;

-- ── carve a different predicate (ex:age, 3 quads) into graph 102 ─────────
SELECT 'carve_age_returns_3: ' || (pgrdf.carve_graph(100, 'http://example.org/age', 102) = 3);
SELECT 'dst102_quads_3: ' || (count(*) = 3) FROM pgrdf._pgrdf_quads WHERE graph_id = 102;

-- ── an unbound predicate carves nothing ─────────────────────────────────
SELECT 'carve_unknown_returns_0: ' || (pgrdf.carve_graph(100, 'http://example.org/nonexistent', 103) = 0);

-- ── the dictionary is untouched by carving (shared term space) ──────────
SELECT 'dict_unchanged_14: ' || (count(*) = 14) FROM pgrdf._pgrdf_dictionary;

DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
