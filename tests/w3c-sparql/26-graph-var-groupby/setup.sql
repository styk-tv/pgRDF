-- 26-graph-var-groupby / setup.sql
--
-- Phase A slice 111 — two named graphs with 3 + 2 triples
-- respectively. Exercises `GRAPH ?g { ... }` composed with
-- aggregate `COUNT(*)` + `GROUP BY ?g` (W3C SPARQL 1.1 §11) and
-- `ORDER BY ?g` (§15.1) — verifies that `?g` projects as the IRI
-- and that GROUP BY keys on the IRI value (not the integer
-- `graph_id`).

SELECT pgrdf.add_graph('http://example.org/g1');
SELECT pgrdf.add_graph('http://example.org/g2');
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:a ex:p "p1" .
ex:b ex:p "p2" .
ex:c ex:p "p3" .
', pgrdf.graph_id('http://example.org/g1'));
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:d ex:p "p4" .
ex:e ex:p "p5" .
', pgrdf.graph_id('http://example.org/g2'));
