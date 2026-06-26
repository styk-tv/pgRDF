-- 135-type-closure-lubm-patterns.sql — type-closure inclusion + exclusion
-- query patterns over the real LUBM class hierarchy (carve groundwork; see
-- issue "Carve completeness: transitive type (subclass) closure").
--
-- The carve's type filter must follow the subclass tree: "all X" has to include
-- entities typed to a SUBCLASS of X, and "exclude X" has to drop entities typed
-- to a subclass of X. This locks the supported, complete query patterns the carve
-- will emit, on real LUBM data loaded from a tracked fixture so a larger real
-- subset can swap into the same load path.
--
-- Surface constraints found while preparing this (v0.6.14):
--   * `FILTER NOT EXISTS { ... }` is NOT translatable (errors).
--   * `MINUS` is supported, but its right side must be a PLAIN BGP — a property
--     path inside MINUS errors ("MINUS right side must be a BGP").
-- Conclusion: do the closure via `pgrdf.materialize` (OWL-RL writes the type
-- entailments + subclass closure as direct edges), then INCLUSION is a plain
-- `?x rdf:type <Class>` and EXCLUSION is a plain `MINUS { ?x rdf:type <Class> }`.
-- Both are complete and use only supported SPARQL. The recursive property-path
-- form (`rdfs:subClassOf*`) remains the inclusion fallback on un-materialised data.
--
-- Fixture (fixtures/lubm-closure-sample.nt, N-Triples): instances typed to LEAF classes.
--   Students:  Alice, Bob, Eve   (Grad/Undergrad)
--   Employees: Carol, Dan, Frank (Full/Assistant/Associate Professor)
--
-- Invariants (all hand-computed AND verified on a live pgrdf v0.6.14 instance):
--   A. PRE-materialise inclusion DIRECT `?x rdf:type ub:Student` = 0 (the omission).
--   B. PRE-materialise inclusion PATH `?x rdf:type ?t . ?t rdfs:subClassOf* ub:Student`
--      = {Alice, Bob, Eve} — the un-materialised fallback already closes inclusion.
--   C. materialize writes type entailments (Carol rdf:type ub:Employee, is_inferred).
--   D. POST-materialise inclusion PLAIN `?x rdf:type ub:Student` = {Alice, Bob, Eve}.
--   E. POST-materialise inclusion PLAIN `?x rdf:type ub:Employee` = {Carol, Dan, Frank}.
--   F. POST-materialise exclusion PLAIN MINUS, employees-except-professors = 0
--      (symmetric: every employee here is transitively a professor).
--   G. control — employees-except-students = 3 (nothing wrongly removed).

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph(13500);
SELECT pgrdf.load_turtle('/fixtures/lubm-closure-sample.nt', 13500);

-- ─── A: PRE inclusion DIRECT — the omission ──────────────────────────────
SELECT count(*)::bigint AS pre_direct_students FROM pgrdf.sparql(
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   PREFIX ub:  <http://swat.cse.lehigh.edu/onto/univ-bench.owl#>
   SELECT ?x WHERE { ?x rdf:type ub:Student }'
);

-- ─── B: PRE inclusion via property path — fallback closes it ──────────────
SELECT (s.j->>'x') AS pre_path_student
FROM pgrdf.sparql(
  'PREFIX rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ub:   <http://swat.cse.lehigh.edu/onto/univ-bench.owl#>
   SELECT ?x WHERE { ?x rdf:type ?t . ?t rdfs:subClassOf* ub:Student } ORDER BY ?x'
) AS s(j);

-- ─── C: materialize writes the type entailments ──────────────────────────
SELECT (pgrdf.materialize(13500)->>'inferred_triples_written')::int >= 1
  AS materialise_wrote_inferred;

SELECT q.is_inferred AS carol_employee_inferred
FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
WHERE s.lexical_value = 'http://example.edu/Carol'
  AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
  AND o.lexical_value = 'http://swat.cse.lehigh.edu/onto/univ-bench.owl#Employee';

-- ─── D: POST inclusion PLAIN — the carve's primary inclusion path ─────────
SELECT (s.j->>'x') AS post_plain_student
FROM pgrdf.sparql(
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   PREFIX ub:  <http://swat.cse.lehigh.edu/onto/univ-bench.owl#>
   SELECT ?x WHERE { ?x rdf:type ub:Student } ORDER BY ?x'
) AS s(j);

-- ─── E: POST inclusion PLAIN — employees ─────────────────────────────────
SELECT (s.j->>'x') AS post_plain_employee
FROM pgrdf.sparql(
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   PREFIX ub:  <http://swat.cse.lehigh.edu/onto/univ-bench.owl#>
   SELECT ?x WHERE { ?x rdf:type ub:Employee } ORDER BY ?x'
) AS s(j);

-- ─── F: POST exclusion PLAIN MINUS — symmetric (employees minus professors)
SELECT count(*)::bigint AS employees_except_professors FROM pgrdf.sparql(
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   PREFIX ub:  <http://swat.cse.lehigh.edu/onto/univ-bench.owl#>
   SELECT ?x WHERE { ?x rdf:type ub:Employee MINUS { ?x rdf:type ub:Professor } }'
);

-- ─── G: control — employees minus students removes nothing ───────────────
SELECT count(*)::bigint AS employees_except_students FROM pgrdf.sparql(
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   PREFIX ub:  <http://swat.cse.lehigh.edu/onto/univ-bench.owl#>
   SELECT ?x WHERE { ?x rdf:type ub:Employee MINUS { ?x rdf:type ub:Student } }'
);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
