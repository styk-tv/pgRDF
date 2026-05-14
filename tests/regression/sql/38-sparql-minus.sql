-- 38-sparql-minus — MINUS { ?s :p ?o } translates to WHERE NOT
-- EXISTS (SELECT 1 FROM ...) keyed on shared variables. SPARQL
-- semantics: MINUS with no shared variables is a no-op.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture (graph 380):
--   alice: foaf:name "Alice", foaf:mbox <a@x>, foaf:age 30
--   bob:   foaf:name "Bob",                    foaf:age 25
--   carol: foaf:name "Carol", foaf:mbox <c@x>
--   dave:  foaf:name "Dave"                                  (just a name)
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:mbox <mailto:a@x> ; foaf:age 30 .
   ex:bob   foaf:name "Bob"   ; foaf:age 25 .
   ex:carol foaf:name "Carol" ; foaf:mbox <mailto:c@x> .
   ex:dave  foaf:name "Dave"  .',
  380
);

-- 1. Simple MINUS — drop subjects that have an mbox.
--    Persons with name: 4. With mbox: alice, carol. Survives: bob, dave → 2.
SELECT count(*)::int AS minus_basic
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n
       WHERE { ?s foaf:name ?n
               MINUS { ?s foaf:mbox ?m } }'
  );

-- 2. No shared variable → MINUS is a no-op (4 rows preserved).
SELECT count(*)::int AS minus_no_shared_vars
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n
       WHERE { ?s foaf:name ?n
               MINUS { ?x foaf:knows ?y } }'
  );

-- 3. Chained MINUSes — both subtract.
--    Persons with name: 4. With mbox: 2 (alice, carol). With age: 2 (alice, bob).
--    Survives: dave only → 1.
SELECT count(*)::int AS minus_chained
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s
       WHERE { ?s foaf:name ?n
               MINUS { ?s foaf:mbox ?m }
               MINUS { ?s foaf:age  ?a } }'
  );

-- 4. MINUS + outer FILTER. Names > "B" AND no mbox.
--    Without mbox: alice, bob, dave. > "B": Carol(no), Dave. Actually:
--    alice (A no), bob (B borderline — REGEX ^[C-Z] excludes B), dave (D yes).
--    → 1 row (dave).
--    Wait: alice is excluded by the FILTER (^[C-Z] requires C..Z). bob also.
--    So among without-mbox {alice, bob, dave}, FILTER keeps only dave. 1 row.
SELECT count(*)::int AS minus_with_filter
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s
       WHERE { ?s foaf:name ?n
               MINUS { ?s foaf:mbox ?m }
               FILTER(REGEX(?n, "^[C-Z]")) }'
  );

-- 5. MINUS scoped by ?s only (the MINUS triple uses a constant
-- predicate). Same as #1 in shape.
SELECT (sparql->>'n')::text AS surviving_name
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n
       WHERE { ?s foaf:name ?n
               MINUS { ?s foaf:mbox ?m }
               MINUS { ?s foaf:age  ?a } }
     ORDER BY ?n LIMIT 1'
  ) AS sparql;

-- 6. MINUS with a SHARED non-subject variable.
--    Names of persons whose name does NOT also appear as someone
--    else's nick. (Constructed query — there are no foaf:nick triples
--    in the fixture so MINUS subtracts nothing → all 4 names.)
SELECT count(*)::int AS minus_shared_n
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n
       WHERE { ?s foaf:name ?n
               MINUS { ?other foaf:nick ?n } }'
  );

ROLLBACK;
