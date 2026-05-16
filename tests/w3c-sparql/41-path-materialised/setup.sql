-- 41-path-materialised / setup.sql
--
-- Phase E group E4 — the materialised-closure no-CTE fallback must
-- be SEMANTICS-PRESERVING: a `*` query over a graph whose closure
-- has been materialised (pgrdf.materialize wrote is_inferred rows)
-- returns the IDENTICAL solution set as the non-materialised case.
-- This fixture exercises the user-visible equivalence; the no-CTE
-- EXPLAIN assertion lives in pg_regress
-- (111-property-path-materialised-closure.sql) where EXPLAIN
-- scraping is expressible.
--
-- The harness's default `data.ttl + add_graph(gid) + parse_turtle`
-- path can't call pgrdf.materialize, so this fixture uses setup.sql
-- exclusively (no data.ttl). We seed a subClassOf chain, then
-- materialize so the executed query takes the no-CTE fast path. The
-- materialize() call is wrapped in `SELECT 'x' FROM (...)` so its
-- JSONB return does not emit a `{...}` line the harness's
-- JSON-line filter would mistake for a solution row.

SELECT pgrdf.add_graph(104100);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.org/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Sparrow rdfs:subClassOf ex:Bird .
ex:Bird    rdfs:subClassOf ex:Vertebrate .
ex:Vertebrate rdfs:subClassOf ex:Animal .
ex:tweety  rdf:type ex:Sparrow .
', 104100);

-- Materialise the OWL-RL closure. The is_inferred subClassOf rows
-- this writes are what trigger the no-CTE direct-match fallback in
-- the subsequent query.rq. Wrapped so no JSON row leaks to stdout.
SELECT 'materialised' AS step FROM (SELECT pgrdf.materialize(104100)) _m;
