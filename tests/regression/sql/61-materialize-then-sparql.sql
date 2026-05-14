-- 61-materialize-then-sparql.sql
--
-- Integration test for Phase 4 (inference) × Phase 3 SPARQL surface.
-- Verifies that triples *inferred* by `pgrdf.materialize` are visible
-- to subsequent `pgrdf.sparql` queries — i.e. the two engines
-- compose correctly across the `is_inferred` column.
--
-- The existing 60-materialize-owl-rl.sql covers the materialize side
-- but only queries `_pgrdf_quads` directly. The SPARQL surface sees
-- both base + inferred rows by default (no `is_inferred` filter in
-- the executor's WHERE clause); this test pins that contract.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

SELECT pgrdf.add_graph(9610);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.com/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Engineer rdfs:subClassOf ex:Person .
ex:Person   rdfs:subClassOf ex:Agent .
ex:alice    rdf:type        ex:Engineer .
', 9610);

-- ─── Before materialize: SPARQL sees only the explicit assertion ──
SELECT count(*) = 1 AS one_type_before
  FROM pgrdf.sparql(
    'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
     PREFIX ex:  <http://example.com/>
     SELECT ?c WHERE { ex:alice rdf:type ?c }'
  );

-- ─── Materialize, then query again ────────────────────────────────
-- Strip the non-deterministic elapsed_ms field before asserting.
SELECT (j->>'base_triples')::int = 3 AS three_base,
       (j->>'inferred_triples_written')::int >= 2 AS at_least_two_inferred
  FROM (SELECT pgrdf.materialize(9610) - 'elapsed_ms' AS j) s;

-- Now SPARQL should see all three application-level classes:
-- Engineer (base), Person (1-hop entailment), Agent (2-hop).
-- OWL 2 RL also adds owl:Thing axiomatic-ly to every typed
-- subject, so a fourth class is expected. The test asserts that
-- the three application-level entailments are PRESENT — not exact
-- equality — so the reasonable axiomatic set can evolve without
-- breaking this gate.
WITH classes AS (
  SELECT sparql->>'c' AS c
    FROM pgrdf.sparql(
      'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
       PREFIX ex:  <http://example.com/>
       SELECT ?c WHERE { ex:alice rdf:type ?c }'
    )
)
SELECT bool_and(EXISTS (SELECT 1 FROM classes WHERE c = expected.c))
       AS all_three_present
  FROM (VALUES
    ('http://example.com/Engineer'),
    ('http://example.com/Person'),
    ('http://example.com/Agent')
  ) AS expected(c);

-- ─── Second materialize is idempotent — same class set, same
--     count. Capture both numbers and assert equality.
SELECT count(*) AS class_count_first
  FROM pgrdf.sparql(
    'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
     PREFIX ex:  <http://example.com/>
     SELECT ?c WHERE { ex:alice rdf:type ?c }'
  ) \gset

-- Wrap the call so its non-deterministic elapsed_ms doesn't leak
-- into the captured output.
SELECT 'materialize' AS step FROM (SELECT pgrdf.materialize(9610)) _;

SELECT count(*) = :class_count_first AS idempotent_class_count
  FROM pgrdf.sparql(
    'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
     PREFIX ex:  <http://example.com/>
     SELECT ?c WHERE { ex:alice rdf:type ?c }'
  );

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
