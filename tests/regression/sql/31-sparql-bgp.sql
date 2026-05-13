-- 31-sparql-bgp — pgrdf.sparql executes single-pattern BGPs against
-- pre-loaded data. Empirically verifies BGP → SQL translation +
-- variable binding + JSONB output shape.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Pre-load a tiny FOAF graph.
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice  a foaf:Person ; foaf:name "Alice" .
   ex:bob    a foaf:Person ; foaf:name "Bob"   .
   ex:carol  a foaf:Person ; foaf:name "Carol" .
   ex:office a foaf:Organization .',
  310
);

-- 1. Generic 3-var pattern lists every triple. 4 subjects × varying
-- = 10 triples total.
SELECT count(*)::int AS all_triples
  FROM pgrdf.sparql('SELECT ?s ?p ?o WHERE { ?s ?p ?o }');

-- 2. Bound predicate (foaf:name) returns 3 rows.
SELECT count(*)::int AS by_foaf_name
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n WHERE { ?s foaf:name ?n }'
  );

-- 3. JSONB shape: each row is an object keyed by projected vars,
-- with the lexical value as the string value.
SELECT (sparql->>'n')::text AS alice_name
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?n WHERE { <http://example.com/alice> foaf:name ?n }'
  ) AS sparql
 LIMIT 1;

-- 4. Bound subject + variable predicate/object lists everything
-- known about that subject.
SELECT count(*)::int AS triples_for_alice
  FROM pgrdf.sparql(
    'SELECT ?p ?o WHERE { <http://example.com/alice> ?p ?o }'
  );

-- 5. Predicate that hasn't been loaded: zero rows, no error.
SELECT count(*)::int AS unknown_predicate_rows
  FROM pgrdf.sparql(
    'SELECT ?s ?o WHERE { ?s <http://nope.example/x> ?o }'
  );

-- 6. Both ends bound (constant subject + constant predicate +
-- variable object): one row with the foaf:name literal as ?n.
SELECT (sparql->>'n')::text AS bob_name
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?n WHERE { <http://example.com/bob> foaf:name ?n }'
  ) AS sparql;

-- 7. Object literal as constant: matches exactly that literal value.
-- 1 row expected (only Alice has foaf:name "Alice").
SELECT count(*)::int AS alice_lookups
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name "Alice" }'
  );

ROLLBACK;
