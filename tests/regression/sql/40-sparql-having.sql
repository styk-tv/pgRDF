-- 40-sparql-having — HAVING (post-aggregate filter) + GROUP_CONCAT
-- + SAMPLE aggregates.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture (graph 400): heterogeneous predicate usage.
--   ex:p → 5 objects
--   ex:q → 2 objects
--   ex:r → 1 object
-- 8 triples total.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:a ex:p "1" . ex:b ex:p "2" . ex:c ex:p "3" . ex:d ex:p "4" . ex:e ex:p "5" .
   ex:f ex:q "x" . ex:g ex:q "y" .
   ex:h ex:r "z" .',
  400
);

-- 1. HAVING (?n > 2) — only ex:p has more than 2 objects.
SELECT count(*)::int AS having_gt_2
  FROM pgrdf.sparql(
    'SELECT ?p (COUNT(?o) AS ?n)
       WHERE { ?s ?p ?o }
     GROUP BY ?p HAVING (?n > 2)'
  );

-- 2. HAVING (?n = 1) — only ex:r has exactly one object.
SELECT (sparql->>'p')::text AS singleton_predicate
  FROM pgrdf.sparql(
    'SELECT ?p (COUNT(?o) AS ?n)
       WHERE { ?s ?p ?o }
     GROUP BY ?p HAVING (?n = 1)'
  ) AS sparql;

-- 3. HAVING combined: COUNT > 1 AND COUNT < 5.
-- ex:p (5) excluded, ex:q (2) included, ex:r (1) excluded.
SELECT (sparql->>'p')::text AS mid_predicate
  FROM pgrdf.sparql(
    'SELECT ?p (COUNT(?o) AS ?n)
       WHERE { ?s ?p ?o }
     GROUP BY ?p HAVING (?n > 1 && ?n < 5)'
  ) AS sparql;

-- 4. GROUP_CONCAT with separator — concatenate all ex:p objects.
-- Sort the result via Postgres so the test is deterministic.
SELECT (
  SELECT string_agg(part, ',')
    FROM (
      SELECT unnest(string_to_array(sparql->>'vals', ',')) AS part ORDER BY part
    ) ordered_parts
  )::text AS sorted_concat
  FROM pgrdf.sparql(
    'SELECT (GROUP_CONCAT(?o; SEPARATOR=",") AS ?vals)
       WHERE { ?s <http://example.com/p> ?o }'
  ) AS sparql;

-- 5. GROUP_CONCAT default separator (single space per spec).
SELECT (
  SELECT count(*)::int FROM regexp_split_to_table(sparql->>'vals', '[[:space:]]')
)::int AS default_sep_words
  FROM pgrdf.sparql(
    'SELECT (GROUP_CONCAT(?o) AS ?vals)
       WHERE { ?s <http://example.com/p> ?o }'
  ) AS sparql;

-- 6. SAMPLE — picks one value from the group. Just verify the value
-- is in the expected set (deterministically the lex-min "1" via MIN).
SELECT (sparql->>'one')::text AS sample_pick
  FROM pgrdf.sparql(
    'SELECT (SAMPLE(?o) AS ?one) WHERE { ?s <http://example.com/p> ?o }'
  ) AS sparql;

-- 7. HAVING on SUM aggregate — all values in this fixture are xsd:string
-- literals (quoted "1", "2", …), so the numeric-aware SUM yields NULL for
-- every group → HAVING drops all rows. Result: 0. This demonstrates the
-- SUM numeric-awareness rule.
SELECT count(*)::int AS sum_having_strings
  FROM pgrdf.sparql(
    'SELECT ?p (SUM(?v) AS ?t)
       WHERE { ?s ?p ?v }
     GROUP BY ?p HAVING (?t > 10)'
  );

-- 8. HAVING on SUM with real numeric data. Load some numeric triples
-- in a second graph and re-aggregate. Both graphs are in scope (no
-- GRAPH clause yet).
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:m ex:n 10 . ex:m ex:n 20 . ex:m ex:n 30 .
   ex:m ex:o 5 . ex:m ex:o 5 .',
  401
);

-- ex:n sum = 60 (passes > 10), ex:o sum = 10 (fails > 10).
-- Plus all the string predicates from graph 400 with NULL sums.
-- So 1 row passes.
SELECT count(*)::int AS sum_having_numeric
  FROM pgrdf.sparql(
    'SELECT ?p (SUM(?v) AS ?t)
       WHERE { ?s ?p ?v }
     GROUP BY ?p HAVING (?t > 10)'
  );

ROLLBACK;
