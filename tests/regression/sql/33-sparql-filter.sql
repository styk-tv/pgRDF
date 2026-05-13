-- 33-sparql-filter — FILTER expressions over BGPs. Covers identity
-- (=, !=, sameTerm via dict id), boolean composition (&&, ||, !),
-- term-type predicates (isIRI, isLiteral, isBlank), and BOUND.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture (graph 330): 7 triples spanning literal/IRI/self-loop shapes.
--   ex:alice foaf:name  "Alice"        -- literal object
--   ex:alice foaf:mbox  <mailto:a@x>   -- IRI object
--   ex:alice foaf:knows ex:bob         -- IRI object
--   ex:bob   foaf:name  "Bob"          -- literal object
--   ex:carol foaf:name  "Carol"        -- literal object
--   ex:carol foaf:mbox  <mailto:c@x>   -- IRI object
--   ex:self  foaf:knows ex:self        -- self-loop, IRI object
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:mbox <mailto:a@x> ; foaf:knows ex:bob .
   ex:bob   foaf:name "Bob"                                                .
   ex:carol foaf:name "Carol" ; foaf:mbox <mailto:c@x>                     .
   ex:self  foaf:knows ex:self .',
  330
);

-- 1. FILTER ?n = "Alice" — exactly one row (Alice).
SELECT count(*)::int AS filter_eq_lit
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n FILTER(?n = "Alice") }'
  );

-- 2. FILTER ?n != "Alice" — Bob + Carol.
SELECT count(*)::int AS filter_ne_lit
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n FILTER(?n != "Alice") }'
  );

-- 3. FILTER isIRI(?o) — mbox + knows triples = 4 rows.
SELECT count(*)::int AS filter_is_iri
  FROM pgrdf.sparql('SELECT ?s ?o WHERE { ?s ?p ?o FILTER(isIRI(?o)) }');

-- 4. FILTER isLiteral(?o) — name triples = 3 rows.
SELECT count(*)::int AS filter_is_literal
  FROM pgrdf.sparql('SELECT ?s ?o WHERE { ?s ?p ?o FILTER(isLiteral(?o)) }');

-- 5. FILTER ?s = ?o — the single self-loop.
SELECT count(*)::int AS filter_self_loop
  FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o FILTER(?s = ?o) }');

-- 6. FILTER isIRI(?o) && ?p = foaf:knows — the 2 knows triples.
SELECT count(*)::int AS filter_iri_and_predicate
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?o WHERE { ?s ?p ?o FILTER(isIRI(?o) && ?p = foaf:knows) }'
  );

-- 7. FILTER !isIRI(?o) — negation, matches the 3 literal-name triples.
SELECT count(*)::int AS filter_not_iri
  FROM pgrdf.sparql('SELECT ?s ?o WHERE { ?s ?p ?o FILTER(!isIRI(?o)) }');

-- 8. FILTER BOUND(?o) — trivially TRUE in BGP context, all 7 triples.
SELECT count(*)::int AS filter_bound
  FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o FILTER(BOUND(?o)) }');

-- 9. FILTER ?n = "DoesNotExist" — literal not in dict → 0 rows.
SELECT count(*)::int AS filter_unknown_lit
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s WHERE { ?s foaf:name ?n FILTER(?n = "DoesNotExist") }'
  );

ROLLBACK;
