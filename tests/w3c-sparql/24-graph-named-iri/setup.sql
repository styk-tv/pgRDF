-- 24-graph-named-iri / setup.sql
--
-- Phase A slice 111 — two named graphs with disjoint triples. The
-- harness's default `data.ttl + add_graph(gid) + parse_turtle(...)`
-- path can only populate ONE graph; this setup.sql replaces that
-- path with two `add_graph(iri)` + per-graph `parse_turtle` calls so
-- the W3C §13.3 `GRAPH <iri>` scoping invariant is observable.

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
