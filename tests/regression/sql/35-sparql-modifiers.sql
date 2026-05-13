-- 35-sparql-modifiers — solution-modifier surface: SELECT DISTINCT,
-- REDUCED, ORDER BY (ASC/DESC), LIMIT, OFFSET.
--
-- All four wrap the BGP+FILTER layer and land in the generated SQL
-- as DISTINCT, ORDER BY <lex_value> NULLS LAST, LIMIT N, OFFSET N.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture: 5 objects, 2 of which share the same literal "x".
--   ex:a ex:p "x"
--   ex:b ex:p "x"      (duplicate object literal)
--   ex:c ex:p "y"
--   ex:d ex:p "z"
--   ex:e ex:p "w"
-- 5 triples total. DISTINCT on ?o → 4 distinct ("x","y","z","w").
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:a ex:p "x" .
   ex:b ex:p "x" .
   ex:c ex:p "y" .
   ex:d ex:p "z" .
   ex:e ex:p "w" .',
  350
);

-- 1. Plain SELECT — 5 raw rows.
SELECT count(*)::int AS all_rows
  FROM pgrdf.sparql('SELECT ?o WHERE { ?s ?p ?o }');

-- 2. SELECT DISTINCT — 4 distinct ?o.
SELECT count(*)::int AS distinct_rows
  FROM pgrdf.sparql('SELECT DISTINCT ?o WHERE { ?s ?p ?o }');

-- 3. SELECT REDUCED — treated as DISTINCT (safe over-approximation).
SELECT count(*)::int AS reduced_rows
  FROM pgrdf.sparql('SELECT REDUCED ?o WHERE { ?s ?p ?o }');

-- 4. LIMIT 2 → first 2 rows of the unordered stream.
SELECT count(*)::int AS limit_2
  FROM pgrdf.sparql('SELECT ?s ?o WHERE { ?s ?p ?o } LIMIT 2');

-- 5. ORDER BY ?o ASC, LIMIT 1 → the alphabetically-first ?o
-- among w, x, y, z is "w".
SELECT (sparql->>'o')::text AS first_alphabetical
  FROM pgrdf.sparql('SELECT ?o WHERE { ?s ?p ?o } ORDER BY ?o LIMIT 1') AS sparql;

-- 6. ORDER BY DESC(?o), LIMIT 1 → alphabetically-last is "z".
SELECT (sparql->>'o')::text AS last_alphabetical
  FROM pgrdf.sparql('SELECT ?o WHERE { ?s ?p ?o } ORDER BY DESC(?o) LIMIT 1') AS sparql;

-- 7. OFFSET 3 LIMIT 2 in an ordered query → 4th and 5th rows
-- in ascending object-literal order, which are "y" and "z".
SELECT (sparql->>'o')::text AS offset_window
  FROM pgrdf.sparql('SELECT ?o WHERE { ?s ?p ?o } ORDER BY ?o OFFSET 3 LIMIT 2') AS sparql;

-- 8. DISTINCT + ORDER BY ?o → 4 distinct values in lexical order.
SELECT count(*)::int AS distinct_ordered
  FROM pgrdf.sparql('SELECT DISTINCT ?o WHERE { ?s ?p ?o } ORDER BY ?o');

-- 9. ORDER BY by a NON-projected variable. SELECT only ?o; sort
-- by ?s lexicographically. First row's ?o should be "x" (since
-- the alphabetically-first ?s among a..e is ex:a, whose ?o is "x").
SELECT (sparql->>'o')::text AS by_unprojected_subject
  FROM pgrdf.sparql('SELECT ?o WHERE { ?s ?p ?o } ORDER BY ?s LIMIT 1') AS sparql;

-- 10. OFFSET 0 LIMIT 0 → zero rows.
SELECT count(*)::int AS limit_zero
  FROM pgrdf.sparql('SELECT ?s ?o WHERE { ?s ?p ?o } LIMIT 0');

ROLLBACK;
