-- 43-sparql-minus-multi — multi-triple MINUS sub-patterns.
-- Subtracts subjects whose ENTIRE sub-pattern matches (logical AND
-- of all triples), not subjects matching any one triple.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture (graph 430):
--   alice: name, mbox, age
--   bob:   name, mbox
--   carol: name, age
--   dave:  name (only)
--   eve:   name, mbox, age, knows (most complete)
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:mbox <mailto:a@x> ; foaf:age 30 .
   ex:bob   foaf:name "Bob"   ; foaf:mbox <mailto:b@x> .
   ex:carol foaf:name "Carol" ; foaf:age 25 .
   ex:dave  foaf:name "Dave"  .
   ex:eve   foaf:name "Eve"   ; foaf:mbox <mailto:e@x> ; foaf:age 40 ; foaf:knows ex:alice .',
  430
);

-- 1. Multi-triple MINUS: subjects with BOTH mbox AND age. Subtracts
-- alice + eve. Survives: bob, carol, dave → 3.
SELECT count(*)::int AS minus_both
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n
                       MINUS { ?s foaf:mbox ?m . ?s foaf:age ?a } }'
  );

-- 2. Multi-triple MINUS with three triples: subjects with mbox AND
-- age AND a foaf:knows relation. Only eve qualifies. Subtracts eve.
-- Survives: 4 (alice, bob, carol, dave).
SELECT count(*)::int AS minus_triple
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n
                       MINUS { ?s foaf:mbox ?m .
                               ?s foaf:age  ?a .
                               ?s foaf:knows ?k } }'
  );

-- 3. Chained multi-triple MINUSes — each independent.
-- MINUS { mbox AND age } drops alice + eve.
-- MINUS { knows } drops eve (but already dropped).
-- Net survivors: bob, carol, dave → 3.
SELECT count(*)::int AS chained_multi_minus
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n
                       MINUS { ?s foaf:mbox ?m . ?s foaf:age ?a }
                       MINUS { ?s foaf:knows ?k } }'
  );

-- 4. The single-triple form still works (back-compat).
-- MINUS { ?s foaf:mbox ?m } drops alice, bob, eve. Survives: 2.
SELECT count(*)::int AS single_triple_minus
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n MINUS { ?s foaf:mbox ?m } }'
  );

ROLLBACK;
