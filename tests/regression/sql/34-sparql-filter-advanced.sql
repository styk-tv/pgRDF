-- 34-sparql-filter-advanced — numeric ordering, REGEX, IN.
--
-- These complete the Phase 3 step 2 surface. Numeric ordering is
-- type-safe (non-numeric rows compare NULL and drop). REGEX uses
-- Postgres ~ / ~* operators. IN is dict-id set membership.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture: 6 typed values + 4 named persons.
--   ex:a ex:age 25    (xsd:integer)
--   ex:b ex:age 35
--   ex:c ex:age 45
--   ex:d ex:age 55
--   ex:e ex:age "fifty"  (xsd:string — not numeric)
--   ex:f ex:age 50.5     (xsd:decimal)
--   ex:alice foaf:name "Alice"
--   ex:adam  foaf:name "Adam"
--   ex:bob   foaf:name "Bob"
--   ex:carol foaf:name "Carol"
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:a ex:age 25 .
   ex:b ex:age 35 .
   ex:c ex:age 45 .
   ex:d ex:age 55 .
   ex:e ex:age "fifty" .
   ex:f ex:age "50.5"^^xsd:decimal .
   ex:alice foaf:name "Alice" .
   ex:adam  foaf:name "Adam"  .
   ex:bob   foaf:name "Bob"   .
   ex:carol foaf:name "Carol" .',
  340
);

-- 1. ?age > 30 → b, c, d, f (35, 45, 55, 50.5).
SELECT count(*)::int AS filter_gt
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/age> ?age FILTER(?age > 30) }'
  );

-- 2. ?age >= 35 && ?age <= 45 → b (35), c (45). 50.5 is > 45.
SELECT count(*)::int AS filter_range
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/age> ?age FILTER(?age >= 35 && ?age <= 45) }'
  );

-- 3. ?age < 30 → a (25). "fifty" string-typed → NULL → dropped.
SELECT count(*)::int AS filter_lt
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/age> ?age FILTER(?age < 30) }'
  );

-- 4. ?age >= 0 → 5 numeric rows (a,b,c,d,f). e="fifty" is xsd:string,
--    not numeric — CASE drops it to NULL → row excluded.
SELECT count(*)::int AS filter_ge_zero
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/age> ?age FILTER(?age >= 0) }'
  );

-- 5. REGEX case-sensitive: starts with A → Alice, Adam.
SELECT count(*)::int AS filter_regex_caps_A
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n FILTER(REGEX(?n, "^A")) }'
  );

-- 6. REGEX case-insensitive: ar | Ar matches → Carol, (no Mark).
SELECT count(*)::int AS filter_regex_i
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n FILTER(REGEX(?n, "ar", "i")) }'
  );

-- 7. REGEX with STR() wrapper — same result as without it.
SELECT count(*)::int AS filter_regex_str_wrap
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n FILTER(REGEX(STR(?n), "^A")) }'
  );

-- 8. IN set membership: ?s IN (alice, carol) → 2 rows.
SELECT count(*)::int AS filter_in
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n FILTER(?s IN (<http://example.com/alice>, <http://example.com/carol>)) }'
  );

-- 9. IN with a single literal: ?n IN ("Alice").
SELECT count(*)::int AS filter_in_literal
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n FILTER(?n IN ("Alice")) }'
  );

-- 10. Composed: ?age > 20 && REGEX would be over the name var — but
-- those are different patterns. Use a 2-pattern BGP to combine.
-- Find persons (have a name) whose age is also > 30. No data
-- crosses, so this is 0.
SELECT count(*)::int AS filter_no_cross
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n .
                       ?s <http://example.com/age> ?age
                       FILTER(?age > 30 && REGEX(?n, "^A")) }'
  );

ROLLBACK;
