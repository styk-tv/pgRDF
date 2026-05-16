-- 112-optional-multi-triple.sql
--
-- Phase F group F1 (slices 34-31, LLD v0.4 §11) — multi-triple
-- OPTIONAL. The v0.3 OPTIONAL handler accepted only a SINGLE-triple
-- right side; v0.4 lifts that: an `OPTIONAL { ?s :p ?n . ?s :q ?ag }`
-- N-triple group translates to a LATERAL-style derived table inside
-- the LEFT JOIN, so the group binds ATOMICALLY (all-or-nothing per
-- W3C §6.1) — every inner variable binds together, or every one
-- comes back NULL. Nested OPTIONAL, OPTIONAL-internal FILTER, the
-- optional-var outer FILTER, GRAPH scoping, property-path-in-required
-- composition, and pgrdf.construct / SPARQL-UPDATE-WHERE inheritance
-- all compose on the same machinery.
--
-- Invariants (all expected values hand-computed; never ACCEPT=1):
--
--   A. 2-triple OPTIONAL over `?s a ex:Person`. alice (name+age) +
--      carol (name+age) match → ?n,?ag bound. bob has name but NO
--      age → 2-triple group fails atomically → ?n AND ?ag BOTH NULL
--      (the all-or-nothing proof — bob's ?n is NULL even though he
--      HAS a name). dave has neither → both NULL. 4 rows preserved;
--      2 with ?ag bound; 2 with ?n bound.
--   B. Optional var in an outer FILTER (`!bound(?n) || ?n="Alice"`)
--      keeps bob/dave (NULL ?n) + alice; drops carol = 3 rows.
--   C. Nested OPTIONAL (OPTIONAL inside OPTIONAL): ?n bound for
--      alice/bob/carol (3); ?ag bound for alice/carol (2); 4 rows.
--   D. OPTIONAL-internal FILTER `FILTER(?ag >= 18)` — carol age 17
--      fails → her whole 2-triple optional unmatched (?n,?ag NULL);
--      only alice (30) binds. 4 rows; 1 with ?ag bound.
--   E. OPTIONAL composed with a `+` property path in the required
--      part. dog->mammal->animal; only dog labelled. 3 base rows;
--      label bound on the 2 dog rows.
--   F. GRAPH <iri> and GRAPH ?g scoping the whole pattern.
--   G. pgrdf.construct + SPARQL UPDATE INSERT WHERE inherit it.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture: a named graph g (registered IRI) with 4 persons of
-- varying completeness. Wrapped in a DO block so the volatile
-- add_graph()/graph_id() scalar returns (a sequence-allocated id
-- that varies with suite order) produce NO tuple output — the test
-- assertions below are graph-id-agnostic. (An unscoped BGP scans
-- all graphs, so the single named-graph copy is found by the
-- unscoped A-D cases too.)
--   alice: name "Alice", age 30
--   bob:   name "Bob"             (no age)
--   carol: name "Carol", age 17
--   dave:  (no name, no age)
DO $$
BEGIN
  PERFORM pgrdf.add_graph('http://example.com/g/people');
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:alice a ex:Person ; ex:name "Alice" ; ex:age 30 .
     ex:bob   a ex:Person ; ex:name "Bob"  .
     ex:carol a ex:Person ; ex:name "Carol" ; ex:age 17 .
     ex:dave  a ex:Person .',
    pgrdf.graph_id('http://example.com/g/people'));
END $$;

-- A. 2-triple OPTIONAL — left-side count preserved (4 persons).
SELECT count(*)::int AS a_total_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } }'
  );

-- A. rows where ?ag IS NOT NULL — alice + carol = 2.
SELECT count(*)::int AS a_ag_bound
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } }'
  ) WHERE sparql->'ag' != 'null'::jsonb;

-- A. atomic: rows where ?n IS NOT NULL — also exactly 2. bob HAS a
-- name but the 2-triple group failed (no age) so ?n is NULL too.
SELECT count(*)::int AS a_n_bound_atomic
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } }'
  ) WHERE sparql->'n' != 'null'::jsonb;

-- A. the alice row carries the name "Alice".
SELECT (sparql->>'n')::text AS a_alice_name
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } }'
  ) AS sparql
  WHERE sparql->>'s' = 'http://example.com/alice';

-- B. optional var in an outer FILTER. !bound(?n) keeps bob+dave
-- (?n NULL); ?n = "Alice" keeps alice; carol dropped. = 3.
SELECT count(*)::int AS b_outer_filter
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag }
               FILTER(!bound(?n) || ?n = "Alice") }'
  );

-- C. nested OPTIONAL. Outer binds ?n for alice/bob/carol; inner
-- binds ?ag for alice/carol; dave has no name → ?n,?ag NULL.
SELECT count(*)::int AS c_nested_total
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n
                          OPTIONAL { ?s ex:age ?ag } } }'
  );
SELECT count(*)::int AS c_nested_n_bound
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n
                          OPTIONAL { ?s ex:age ?ag } } }'
  ) WHERE sparql->'n' != 'null'::jsonb;
SELECT count(*)::int AS c_nested_ag_bound
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n
                          OPTIONAL { ?s ex:age ?ag } } }'
  ) WHERE sparql->'ag' != 'null'::jsonb;
-- C. bob's row: ?n="Bob", ?ag NULL (inner optional missed; bob's
-- outer ?n stays bound — nested compose, not atomic across levels).
SELECT (sparql->>'n')::text AS c_bob_name
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n
                          OPTIONAL { ?s ex:age ?ag } } }'
  ) AS sparql
  WHERE sparql->>'s' = 'http://example.com/bob'
    AND sparql->'ag' = 'null'::jsonb;

-- D. 2-triple OPTIONAL with an internal FILTER. carol age 17 fails
-- FILTER(?ag >= 18) → her optional unmatched → ?n,?ag NULL. alice
-- (30) binds both. 4 rows; ?ag bound only for alice = 1.
SELECT count(*)::int AS d_inner_filter_total
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag
                          FILTER(?ag >= 18) } }'
  );
SELECT count(*)::int AS d_inner_filter_ag_bound
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag
                          FILTER(?ag >= 18) } }'
  ) WHERE sparql->'ag' != 'null'::jsonb;

-- E. OPTIONAL composed with a `+` property path in the required
-- part. Class hierarchy dog -> mammal -> animal; only dog labelled.
--   ?s ex:subClassOf+ ?base :  dog→{mammal,animal} (2), mammal→
--   {animal} (1)  = 3 required rows. ?l bound on the 2 dog rows.
DO $$
BEGIN
  PERFORM pgrdf.add_graph('http://example.com/g/tax');
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:dog    ex:subClassOf ex:mammal .
     ex:mammal ex:subClassOf ex:animal .
     ex:dog    ex:label "Dog" .',
    pgrdf.graph_id('http://example.com/g/tax'));
END $$;
SELECT count(*)::int AS e_path_plus_optional_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?base ?l
       WHERE { GRAPH <http://example.com/g/tax> {
                 ?s ex:subClassOf+ ?base
                 OPTIONAL { ?s ex:label ?l } } }'
  );
SELECT count(*)::int AS e_path_label_bound
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?base ?l
       WHERE { GRAPH <http://example.com/g/tax> {
                 ?s ex:subClassOf+ ?base
                 OPTIONAL { ?s ex:label ?l } } }'
  ) WHERE sparql->'l' != 'null'::jsonb;

-- F. GRAPH <iri> scoping the whole pattern → same A counts.
SELECT count(*)::int AS f_graph_literal_total
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { GRAPH <http://example.com/g/people> {
                 ?s a ex:Person
                 OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } } }'
  );
SELECT count(*)::int AS f_graph_literal_ag_bound
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?n ?ag
       WHERE { GRAPH <http://example.com/g/people> {
                 ?s a ex:Person
                 OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } } }'
  ) WHERE sparql->'ag' != 'null'::jsonb;

-- F. GRAPH ?g scoping the whole pattern. ?g binds the named graph;
-- the OPTIONAL inner triples inherit + correlate to the mandatory
-- ?g anchor. The only graph carrying this data is g/people, so
-- ?g = <http://example.com/g/people> on every row. 4 rows; ?ag
-- bound for 2; every row's ?g is the people graph IRI.
SELECT count(*)::int AS f_graph_var_total
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?g ?s ?n ?ag
       WHERE { GRAPH ?g {
                 ?s a ex:Person
                 OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } } }'
  );
SELECT count(*)::int AS f_graph_var_ag_bound
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?g ?s ?n ?ag
       WHERE { GRAPH ?g {
                 ?s a ex:Person
                 OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } } }'
  ) WHERE sparql->'ag' != 'null'::jsonb;
SELECT count(DISTINCT sparql->>'g')::int AS f_graph_var_distinct_g
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?g ?s ?n ?ag
       WHERE { GRAPH ?g {
                 ?s a ex:Person
                 OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } } }'
  ) AS sparql;
SELECT (sparql->>'g')::text AS f_graph_var_g_iri
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?g ?s ?n ?ag
       WHERE { GRAPH ?g {
                 ?s a ex:Person
                 OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } } }'
  ) AS sparql
  WHERE sparql->>'s' = 'http://example.com/alice';

-- G. pgrdf.construct inherits multi-triple OPTIONAL. CONSTRUCT a
-- ex:summary triple gated by the optional name+age match → only
-- alice + carol (atomic) = 2 ex:summary triples.
SELECT count(*)::int AS g_construct_summary
  FROM pgrdf.construct(
    'PREFIX ex: <http://example.com/>
     CONSTRUCT { ?s ex:summary ?ag }
       WHERE { ?s a ex:Person
               OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } }'
  ) AS t(j)
  WHERE j->'predicate'->>'value' = 'http://example.com/summary';

-- G. SPARQL UPDATE INSERT ... WHERE inherits multi-triple OPTIONAL.
-- Flag every person whose name+age 2-triple optional matched
-- (alice, carol). bob/dave (unmatched) get no flag → 2 flags.
SELECT (j->'_update'->>'form') || '|'
       || (j->'_update'->>'triples_inserted') AS g_update_summary
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     INSERT { GRAPH <http://example.com/g/people> { ?s ex:fullProfile true } }
       WHERE { GRAPH <http://example.com/g/people> {
                 ?s a ex:Person
                 OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } }
               FILTER(bound(?ag)) }'
  ) AS j;
SELECT count(*)::int AS g_update_where_flags
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s WHERE { GRAPH <http://example.com/g/people> {
       ?s <http://example.com/fullProfile> true } }'
  );

-- O. LLD §11 acceptance criterion: a multi-triple OPTIONAL no
-- longer appears in pgrdf.sparql_parse's `unsupported_algebra`
-- (it never panics at parse; the executor now translates it). The
-- array is empty for this shape.
SELECT pgrdf.sparql_parse(
  'PREFIX ex: <http://example.com/>
   SELECT ?s ?n ?ag WHERE { ?s a ex:Person
     OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } }'
)->'unsupported_algebra' AS o_optional_unsupported;
-- O. DESCRIBE is still NOT shipped (Phase F group F3) — it reports
-- form=DESCRIBE supported=false (a separate field; DESCRIBE was
-- never an `unsupported_algebra` entry).
SELECT pgrdf.sparql_parse('DESCRIBE <http://example.com/alice>')->>'supported'
  AS o_describe_supported;

ROLLBACK;
