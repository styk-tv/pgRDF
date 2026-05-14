-- 42-sparql-bind — BIND(expr AS ?var) projects a computed value.
-- Today's restriction: BIND output is visible in the SELECT
-- projection only; referencing a BIND var in a later FILTER/BGP
-- isn't supported in this slice.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture: 3 persons with name + age.
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:age 30 .
   ex:bob   foaf:name "Bob"   ; foaf:age 25 .
   ex:carol foaf:name "Carol" ; foaf:age 40 .',
  420
);

-- 1. BIND with UCASE → uppercase names.
SELECT (sparql->>'upper')::text AS upper_first
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?upper WHERE { ?s foaf:name ?n BIND(UCASE(?n) AS ?upper) }
     ORDER BY ?upper LIMIT 1'
  ) AS sparql;

-- 2. BIND with arithmetic: ?age + 5 → emitted as text.
SELECT count(*)::int AS row_count
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?older WHERE { ?s foaf:age ?a BIND(?a + 5 AS ?older) }'
  );

-- 3. BIND with CONCAT — greeting prefix.
SELECT (sparql->>'greeting')::text AS greeting_alice
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?greeting
       WHERE { <http://example.com/alice> foaf:name ?n
               BIND(CONCAT("Hello, ", ?n, "!") AS ?greeting) }'
  ) AS sparql;

-- 4. BIND with a literal constant.
SELECT (sparql->>'tag')::text AS tag_value
  FROM pgrdf.sparql(
    'SELECT ?s ?tag WHERE { ?s ?p ?o BIND("category-foo" AS ?tag) } LIMIT 1'
  ) AS sparql;

-- 5. BIND with STRLEN cast to text.
SELECT (sparql->>'len')::int AS first_name_length
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?n ?len
       WHERE { ?s foaf:name ?n BIND(STRLEN(?n) AS ?len) }
     ORDER BY ?n LIMIT 1'
  ) AS sparql;

-- 6. Two BINDs in one query.
SELECT count(*)::int AS two_binds_rows
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?upper ?double
       WHERE { ?s foaf:name ?n .
               ?s foaf:age  ?a
               BIND(UCASE(?n) AS ?upper)
               BIND(?a * 2 AS ?double) }'
  );

ROLLBACK;
