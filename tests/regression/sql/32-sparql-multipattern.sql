-- 32-sparql-multipattern — N-pattern BGPs where shared variables
-- across patterns become INNER joins. The translator aliases each
-- pattern as q1, q2, … and emits equality predicates against the
-- first-occurrence anchor for each variable.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- FOAF graph: 2 persons fully described, 1 person without mbox.
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice  a foaf:Person ; foaf:name "Alice"  ; foaf:mbox <mailto:a@x>  ; foaf:knows ex:bob   .
   ex:bob    a foaf:Person ; foaf:name "Bob"                                                   .
   ex:carol  a foaf:Person ; foaf:name "Carol"  ; foaf:mbox <mailto:c@x>                       .',
  320
);

-- 1. Two-pattern BGP sharing ?p — only Alice + Carol qualify (Bob
--    has no mbox). 2 rows.
SELECT count(*)::int AS persons_with_name_and_mbox
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?p ?n ?m WHERE { ?p foaf:name ?n . ?p foaf:mbox ?m }'
  );

-- 2. Three-pattern chain following foaf:knows. Alice → Bob is the
--    only "knows" edge, and both have names. 1 row, ?an = "Alice",
--    ?bn = "Bob".
WITH r AS (
  SELECT pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?an ?bn
       WHERE { ?a foaf:knows ?b .
               ?a foaf:name  ?an .
               ?b foaf:name  ?bn }'
  ) AS j
)
SELECT (j->>'an')::text AS alice_name, (j->>'bn')::text AS bob_name
  FROM r;

-- 3. Self-join on the SAME variable position. `?s ?p ?s` requires
--    subject == object on the same triple. We have none of these in
--    the fixture → 0 rows.
SELECT count(*)::int AS self_loops
  FROM pgrdf.sparql('SELECT ?s ?p WHERE { ?s ?p ?s }');

-- 4. Mixing a bound-subject pattern with a follow-up var: list
--    everything Alice has, then for each predicate-bound triple
--    against ex:alice, look up the mbox via a second pattern.
SELECT count(*)::int AS alice_with_mbox
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?n ?m
       WHERE { <http://example.com/alice> foaf:name ?n .
               <http://example.com/alice> foaf:mbox ?m }'
  );

-- 5. Bound predicate AND bound object — both anchors and selects.
--    Asks "who is named Alice?" → 1 row.
SELECT (j->>'p')::text AS alice_iri
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?p WHERE { ?p foaf:name "Alice" . ?p a foaf:Person }'
  ) AS j;

ROLLBACK;
