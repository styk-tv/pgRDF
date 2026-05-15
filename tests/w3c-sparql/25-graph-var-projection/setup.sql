-- 25-graph-var-projection / setup.sql
--
-- Phase A slice 111 — identical fixture to test 24 (two named graphs
-- with one ex:name triple each). Test 25 exercises the variable form
-- `GRAPH ?g { ... }` against the same data, so the harness can
-- compare scoping-by-IRI (24) vs IRI-as-projected-binding (25)
-- against the same source quads.

SELECT pgrdf.add_graph('http://example.org/g1');
SELECT pgrdf.add_graph('http://example.org/g2');
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:alice ex:name "Alice in g1" .
', pgrdf.graph_id('http://example.org/g1'));
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:bob ex:name "Bob in g2" .
', pgrdf.graph_id('http://example.org/g2'));
