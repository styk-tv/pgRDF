-- 29-update-with-graph-scope / setup.sql
--
-- Phase C slice 75 — pre-stage triples in a named graph
-- `http://example.org/store` so the subsequent
-- `WITH <store> INSERT … WHERE …` can both read and write under that
-- graph scope (per W3C §3.1.3 paragraph 3: WITH selects the default
-- graph for the WHERE evaluation in the absence of USING).
--
-- Seeded contents of `store`:
--
--   ex:item1 ex:hasPrice "10"
--   ex:item2 ex:hasPrice "20"
--   ex:item3 ex:hasPrice "30"

SELECT pgrdf.add_graph('http://example.org/store');
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
ex:item1 ex:hasPrice "10" .
ex:item2 ex:hasPrice "20" .
ex:item3 ex:hasPrice "30" .
', pgrdf.graph_id('http://example.org/store'));
