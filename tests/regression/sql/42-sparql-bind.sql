-- 42-sparql-bind — BIND(expr AS ?var) projects a computed value.
-- This file locks the v0.3 BIND PROJECTION surface (it must keep
-- working — F2 invariant E, "no regression of v0.3 behaviour").
-- The v0.3 restriction "BIND output is projection-only" is LIFTED
-- by Phase F group F2 (LLD v0.4 §11): a BIND var is now usable in a
-- later FILTER, a BGP join, and a chained BIND. That downstream
-- surface is covered by tests/regression/sql/114-bind-downstream.sql;
-- the projection-only cases below are unchanged on purpose.

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
