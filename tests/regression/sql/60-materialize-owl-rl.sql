-- 60-materialize-owl-rl.sql
--
-- Phase 4 (LLD §2 / docs/04-inference.md) — pgrdf.materialize via the
-- reasonable OWL 2 RL forward-chain reasoner. Verifies:
--
--   1. The two-hop subClassOf chain
--        Engineer rdfs:subClassOf Person ; Person rdfs:subClassOf Agent
--      plus base assertions
--        ex:alice rdf:type ex:Engineer ; ex:bob rdf:type ex:Person
--      yields three application-level type entailments:
--        ex:alice rdf:type ex:Person   (1-hop)
--        ex:alice rdf:type ex:Agent    (2-hop)
--        ex:bob   rdf:type ex:Agent    (1-hop)
--   2. Idempotence: a second materialize call returns the same
--      inferred count and drops the prior run's output exactly.
--   3. Inverse-of: ex:owns owl:inverseOf ex:ownedBy entails
--      ex:store ex:ownedBy ex:owner from ex:owner ex:owns ex:store.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- ─── Block A: rdfs:subClassOf chain ─────────────────────────────
SELECT pgrdf.add_graph(9601);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.com/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Engineer rdfs:subClassOf ex:Person .
ex:Person   rdfs:subClassOf ex:Agent .
ex:alice    rdf:type        ex:Engineer .
ex:bob      rdf:type        ex:Person .
', 9601);

SELECT (j->>'base_triples')::int = 4 AS base_4
  FROM (SELECT pgrdf.materialize(9601) AS j) s;

-- Exactly the three application-level type entailments must appear.
SELECT count(*) = 3 AS three_app_entailments
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
 WHERE q.graph_id = 9601 AND q.is_inferred = TRUE
   AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
   AND s.lexical_value IN ('http://example.com/alice', 'http://example.com/bob')
   AND o.lexical_value IN ('http://example.com/Person', 'http://example.com/Agent');

-- The specific 2-hop entailment must be there.
SELECT EXISTS (
  SELECT 1 FROM pgrdf._pgrdf_quads q
   JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
   JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
   JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
  WHERE q.graph_id = 9601 AND q.is_inferred = TRUE
    AND s.lexical_value = 'http://example.com/alice'
    AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
    AND o.lexical_value = 'http://example.com/Agent'
) AS two_hop_entailment_present;

-- Base triples unchanged.
SELECT count(*) = 4 AS base_unchanged
  FROM pgrdf._pgrdf_quads WHERE graph_id = 9601 AND is_inferred = FALSE;

-- ─── Block B: idempotence ───────────────────────────────────────
SELECT (j->>'inferred_triples_written')::int AS first_written
  FROM (SELECT pgrdf.materialize(9601) AS j) s \gset

SELECT (j->>'inferred_triples_written')::int = :first_written AS same_count_2nd,
       (j->>'previous_inferred_dropped')::int = :first_written AS dropped_eq_first
  FROM (SELECT pgrdf.materialize(9601) AS j) s;

-- ─── Block C: owl:inverseOf ─────────────────────────────────────
SELECT pgrdf.add_graph(9602);
SELECT pgrdf.parse_turtle('
@prefix ex:  <http://example.com/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
ex:owns owl:inverseOf ex:ownedBy .
ex:owner ex:owns ex:store .
', 9602);

SELECT (j->>'inferred_triples_written')::int >= 1 AS at_least_one_inferred
  FROM (SELECT pgrdf.materialize(9602) AS j) s;

SELECT EXISTS (
  SELECT 1 FROM pgrdf._pgrdf_quads q
   JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
   JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
   JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
  WHERE q.graph_id = 9602 AND q.is_inferred = TRUE
    AND s.lexical_value = 'http://example.com/store'
    AND p.lexical_value = 'http://example.com/ownedBy'
    AND o.lexical_value = 'http://example.com/owner'
) AS inverseof_entailment_present;

-- ─── Cleanup ────────────────────────────────────────────────────
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
