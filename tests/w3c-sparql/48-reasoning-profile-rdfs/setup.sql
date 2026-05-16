-- 48-reasoning-profile-rdfs / setup.sql
--
-- Phase G group G1 (SPEC.pgRDF.LLD.v0.5-FUTURE §3) — the
-- reasoning-profile selector on pgrdf.materialize, observed through
-- the SPARQL query surface. `pgrdf.materialize(g, 'rdfs')` runs the
-- strict RDFS forward-chain (rdfs2/3/5/7/9/11 only — a true subset of
-- OWL-RL). Here the rdfs9 (subClassOf application) + rdfs11
-- (subClassOf transitivity) rules entail every superclass type for
-- ex:tweety. The query then SELECTs all of tweety's types.
--
-- The harness default `data.ttl + add_graph + parse_turtle` path
-- can't call pgrdf.materialize, so this fixture uses setup.sql only
-- (no data.ttl). materialize()'s JSONB return is wrapped so it does
-- not leak a `{...}` line into the harness's solution-row filter.

SELECT pgrdf.add_graph(104800);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.org/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Sparrow rdfs:subClassOf ex:Bird .
ex:Bird    rdfs:subClassOf ex:Animal .
ex:tweety  rdf:type ex:Sparrow .
', 104800);

-- Materialise under the RDFS profile. rdfs11: Sparrow ⊑ Bird,
-- Bird ⊑ Animal ⇒ Sparrow ⊑ Animal. rdfs9: tweety a Sparrow +
-- (Sparrow ⊑ Bird, Sparrow ⊑ Animal) ⇒ tweety a Bird, tweety a
-- Animal. Wrapped so no JSON row leaks to stdout.
SELECT 'materialised' AS step FROM (SELECT pgrdf.materialize(104800, 'rdfs')) _m;
