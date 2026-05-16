-- 49-reasoning-profile-owl-rl / setup.sql
--
-- Phase G group G1 (SPEC.pgRDF.LLD.v0.5-FUTURE §3) — the DEFAULT
-- 'owl-rl' profile on pgrdf.materialize, observed through the SPARQL
-- query surface, exercising an entailment the 'rdfs' profile does
-- NOT produce. `owl:inverseOf` is an OWL 2 RL rule (prp-inv), not an
-- RDFS rule — so this query result is profile-distinguishing: it
-- holds under 'owl-rl' (here, the default-arg form) and would NOT
-- hold under 'rdfs'. Pairs with 48-reasoning-profile-rdfs (same
-- harness shape, RDFS-only entailment) to show both profiles at the
-- query surface.
--
-- setup.sql-only (the harness default path can't call materialize);
-- materialize()'s JSONB return wrapped so no `{...}` row leaks.

SELECT pgrdf.add_graph(104900);
SELECT pgrdf.parse_turtle('
@prefix ex:  <http://example.org/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
ex:owns owl:inverseOf ex:ownedBy .
ex:alice ex:owns ex:book .
', 104900);

-- Default profile = 'owl-rl' (the v0.3/v0.4 surface unchanged). The
-- OWL-RL prp-inv rule entails the inverse triple
-- `ex:book ex:ownedBy ex:alice` from `ex:alice ex:owns ex:book` +
-- `ex:owns owl:inverseOf ex:ownedBy`. RDFS has no such rule.
SELECT 'materialised' AS step FROM (SELECT pgrdf.materialize(104900)) _m;
