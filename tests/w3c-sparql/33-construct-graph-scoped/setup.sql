-- 33-construct-graph-scoped / setup.sql
--
-- Phase D slice 51 — variable-GRAPH CONSTRUCT, W3C §13.3 scoping.
-- The default `data.ttl + add_graph(gid) + parse_turtle` path can
-- only populate ONE graph; this fixture needs TWO named graphs PLUS
-- a default-graph triple to prove `GRAPH ?g` binds named graphs ONLY
-- (the default graph must NOT bleed through). No data.ttl — this
-- setup.sql is the sole input source (run.sh slice-111 extension).
--
-- Seeded contents:
--   http://example.org/g1  →  ex:alice ex:name "Alice"
--   http://example.org/g2  →  ex:bob   ex:name "Bob"
--   DEFAULT (graph 0)      →  ex:carol ex:name "Carol"   (must NOT appear)

SELECT pgrdf.add_graph('http://example.org/g1');
SELECT pgrdf.add_graph('http://example.org/g2');
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:alice ex:name "Alice" .
', pgrdf.graph_id('http://example.org/g1'));
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:bob ex:name "Bob" .
', pgrdf.graph_id('http://example.org/g2'));
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:carol ex:name "Carol" .
', 0);
