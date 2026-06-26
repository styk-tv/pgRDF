-- 138-carve-graph-neighbourhood.sql — pgrdf.carve_graph(src, seeds[], dst, max_hops)
-- the neighbourhood carve (#30, v0.6.17): the §4b.1 common case. Carves the K-hop
-- neighbourhood of a seed set into a new graph (shared dictionary, id-space BFS).
--
-- Fixture: multiload-dedup-sample.nt (8 N-Triples) — a knows-chain + name/age:
--   alice knows bob ; bob knows carol
--   alice/bob/carol each have a foaf:name and an ex:age literal
--
-- Hand-computed neighbourhoods of seed = alice (subject<->object edges):
--   0 hops → nodes {alice}                          → alice's own 3 quads
--   1 hop  → {alice, bob, "Alice", "28"}            → alice's 3 + bob's 3 = 6
--   2 hops → reaches carol + all literals           → all 8 quads
--   seeds {alice,carol} 0 hops → quads touching alice OR carol (incl. bob→carol) = 6

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph(200);
SELECT pgrdf.load_turtle('/fixtures/regression/multiload-dedup-sample.nt', 200);
SELECT 'src_quads_8: ' || (count(*) = 8) FROM pgrdf._pgrdf_quads WHERE graph_id = 200;

-- ── 0 hops from alice → 3 (alice's own quads) ────────────────────────────
SELECT 'carve_0hop_returns_3: ' || (pgrdf.carve_graph(200, ARRAY['http://example.org/alice']::text[], 201, 0) = 3);
SELECT 'dst201_quads_3: ' || (count(*) = 3) FROM pgrdf._pgrdf_quads WHERE graph_id = 201;

-- ── 1 hop from alice → 6 (alice + bob) ───────────────────────────────────
SELECT 'carve_1hop_returns_6: ' || (pgrdf.carve_graph(200, ARRAY['http://example.org/alice']::text[], 202, 1) = 6);
SELECT 'dst202_quads_6: ' || (count(*) = 6) FROM pgrdf._pgrdf_quads WHERE graph_id = 202;

-- ── 2 hops from alice → 8 (reaches the whole graph) ──────────────────────
SELECT 'carve_2hop_returns_8: ' || (pgrdf.carve_graph(200, ARRAY['http://example.org/alice']::text[], 203, 2) = 8);

-- ── unknown seed → 0 ─────────────────────────────────────────────────────
SELECT 'carve_unknown_returns_0: ' || (pgrdf.carve_graph(200, ARRAY['http://example.org/nobody']::text[], 204, 1) = 0);

-- ── multi-seed {alice, carol}, 0 hops → 6 (incl. bob→carol via the object) ─
SELECT 'carve_multiseed_returns_6: ' || (pgrdf.carve_graph(200, ARRAY['http://example.org/alice','http://example.org/carol']::text[], 205, 0) = 6);

-- ── default max_hops (1) reachable as a 3-arg call ───────────────────────
SELECT 'carve_default_hop_returns_6: ' || (pgrdf.carve_graph(200, ARRAY['http://example.org/alice']::text[], 206) = 6);

-- ── source untouched + dictionary untouched (shared term space) ──────────
SELECT 'src_unchanged_8: ' || (count(*) = 8) FROM pgrdf._pgrdf_quads WHERE graph_id = 200;
SELECT 'dict_unchanged_14: ' || (count(*) = 14) FROM pgrdf._pgrdf_dictionary;

DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
