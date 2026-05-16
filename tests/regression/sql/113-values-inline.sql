-- 113-values-inline.sql
--
-- Phase F group F1 (slices 34-31, LLD v0.4 §11) — `VALUES` inline
-- tables. spargebra surfaces `VALUES` as `GraphPattern::Values {
-- variables, bindings }` (top-level, or joined alongside a BGP).
-- v0.4 translates it to a `(VALUES (id,…),(id,…)) AS vN(cols)`
-- derived table whose constants are resolved to dictionary ids
-- ahead of execution; the surrounding BGP joins it on the shared
-- variables. `UNDEF` (spargebra `None`) is a NULL cell that places
-- NO constraint on that variable for that row (W3C §10).
--
-- Invariants (all expected values hand-computed; never ACCEPT=1):
--
--   H. Top-level single-column VALUES (?x) {(<a>)(<b>)} joined to
--      `?x ex:p ?y` — only the listed subjects that also have an
--      ex:p come back.
--   I. Multi-column VALUES (?x ?y) {(<a> 1)(<b> 2)} join — a row
--      survives only when BOTH the ?x and ?y BGP-bindings match a
--      VALUES tuple.
--   J. UNDEF in a binding row — that cell imposes no constraint, so
--      the row matches any value the var takes from the BGP.
--   K. VALUES + OPTIONAL together.
--   L. VALUES under GRAPH scoping.
--   M. typed / lang literals in VALUES rows round-trip with the
--      correct datatype (resolve to the right dict id).
--   N. pgrdf.construct + SPARQL UPDATE WHERE inherit VALUES.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Primary fixture in the DEFAULT graph (id 0) ONLY — the unscoped
-- H-K/M cases scan it without the dual-graph double-count an
-- unscoped BGP would incur if the same triples also lived in a
-- named graph (slice-112 semantics: an unscoped BGP scans ALL
-- graphs).
--   a ex:p 1 ; ex:size "M"
--   b ex:p 2
--   c ex:p 3
--   d ex:p 1
--   lit ex:tag "hello"@en ; ex:num 42^^xsd:integer
-- Wrapped in DO blocks so the volatile add_graph()/graph_id()
-- scalar returns (a sequence-allocated id that varies with suite
-- order) produce NO tuple output — every assertion below is
-- graph-id-agnostic.
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:a ex:p 1 ; ex:size "M" .
     ex:b ex:p 2 .
     ex:c ex:p 3 .
     ex:d ex:p 1 .
     ex:lit ex:tag "hello"@en .
     ex:lit ex:num "42"^^<http://www.w3.org/2001/XMLSchema#integer> .',
    0);
  -- Separate NAMED-graph dataset for the GRAPH-scoped L / N cases —
  -- distinct subjects (ga,gb,gc) so a scoped query is unambiguous
  -- and does not interact with the default-graph copy.
  PERFORM pgrdf.add_graph('http://example.com/g/v');
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:ga ex:p 1 .
     ex:gb ex:p 2 .
     ex:gc ex:p 3 .',
    pgrdf.graph_id('http://example.com/g/v'));
END $$;

-- H. top-level single-column VALUES joined on ?x. Only a, c listed
-- AND both have ex:p → 2 rows.
SELECT count(*)::int AS h_values_join
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x ?y
       WHERE { ?x ex:p ?y }
       VALUES (?x) { (ex:a) (ex:c) (ex:zzz) }'
  );
-- H. the surviving subjects, sorted.
SELECT string_agg(sparql->>'x', ',' ORDER BY sparql->>'x') AS h_subjects
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x ?y
       WHERE { ?x ex:p ?y }
       VALUES (?x) { (ex:a) (ex:c) (ex:zzz) }'
  ) AS sparql;

-- I. multi-column VALUES — a row survives only if BOTH ?x and ?y
-- match a tuple. (ex:a 1) matches a; (ex:b 2) matches b;
-- (ex:c 99) does NOT match c (c has ?y=3). = 2 rows.
SELECT count(*)::int AS i_multicol
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x ?y
       WHERE { ?x ex:p ?y }
       VALUES (?x ?y) { (ex:a 1) (ex:b 2) (ex:c 99) }'
  );

-- J. UNDEF — VALUES (?x ?y) { (ex:a UNDEF) (ex:zzz 2) }. Row
-- (ex:a UNDEF): ?x constrained to a, ?y UNCONSTRAINED → matches
-- a's actual ?y (=1). Row (ex:zzz 2): ?x=zzz has no ex:p → no
-- match. Net = 1 row (a, with ?y=1).
SELECT count(*)::int AS j_undef_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x ?y
       WHERE { ?x ex:p ?y }
       VALUES (?x ?y) { (ex:a UNDEF) (ex:zzz 2) }'
  );
SELECT (sparql->>'y')::text AS j_undef_y
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x ?y
       WHERE { ?x ex:p ?y }
       VALUES (?x ?y) { (ex:a UNDEF) (ex:zzz 2) }'
  ) AS sparql;

-- K. VALUES + OPTIONAL together. VALUES pins ?x to {a,b}; OPTIONAL
-- binds ?sz only for a (a ex:size "M"; b has none). 2 rows; ?sz
-- bound for 1.
SELECT count(*)::int AS k_values_optional_total
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x ?y ?sz
       WHERE { ?x ex:p ?y
               OPTIONAL { ?x ex:size ?sz } }
       VALUES (?x) { (ex:a) (ex:b) }'
  );
SELECT count(*)::int AS k_values_optional_sz_bound
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x ?y ?sz
       WHERE { ?x ex:p ?y
               OPTIONAL { ?x ex:size ?sz } }
       VALUES (?x) { (ex:a) (ex:b) }'
  ) WHERE sparql->'sz' != 'null'::jsonb;

-- L. VALUES under GRAPH scoping. Join scoped to the named graph
-- g/v whose subjects are ga/gb/gc; VALUES picks ga,gc → 2 rows.
SELECT count(*)::int AS l_values_graph
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x ?y
       WHERE { GRAPH <http://example.com/g/v> {
                 ?x ex:p ?y }
               VALUES (?x) { (ex:ga) (ex:gc) } }'
  );

-- M. typed / lang literals in VALUES rows. The lang-tagged value
-- "hello"@en and the xsd:integer 42 must resolve to the SAME dict
-- id the data carries, so the join matches.
SELECT count(*)::int AS m_lang_literal
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s
       WHERE { ?s ex:tag ?t }
       VALUES (?t) { ("hello"@en) }'
  );
SELECT count(*)::int AS m_typed_literal
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s
       WHERE { ?s ex:num ?n }
       VALUES (?n) { (42) }'
  );
-- M. a NON-matching lang tag must NOT join (datatype/lang aware).
SELECT count(*)::int AS m_wrong_lang
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s
       WHERE { ?s ex:tag ?t }
       VALUES (?t) { ("hello"@fr) }'
  );

-- N. pgrdf.construct inherits VALUES. CONSTRUCT { ?x ex:picked ?y }
-- gated by VALUES (?x) {(a)(c)} → 2 ex:picked triples.
SELECT count(*)::int AS n_construct_values
  FROM pgrdf.construct(
    'PREFIX ex: <http://example.com/>
     CONSTRUCT { ?x ex:picked ?y }
       WHERE { ?x ex:p ?y }
       VALUES (?x) { (ex:a) (ex:c) }'
  ) AS t(j)
  WHERE j->'predicate'->>'value' = 'http://example.com/picked';

-- N. SPARQL UPDATE INSERT ... WHERE inherits VALUES. Flag only the
-- VALUES-listed named-graph subjects {ga,gb} → 2 flags.
SELECT (j->'_update'->>'form') || '|'
       || (j->'_update'->>'triples_inserted') AS n_update_summary
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     INSERT { GRAPH <http://example.com/g/v> { ?x ex:chosen true } }
       WHERE { GRAPH <http://example.com/g/v> { ?x ex:p ?y }
               VALUES (?x) { (ex:ga) (ex:gb) } }'
  ) AS j;
SELECT count(*)::int AS n_update_values_flags
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x WHERE { GRAPH <http://example.com/g/v> {
       ?x <http://example.com/chosen> true } }'
  );

-- O. LLD §11 acceptance criterion: VALUES no longer appears in
-- pgrdf.sparql_parse's `unsupported_algebra` (it used to push
-- "Values (inline VALUES)"). The array is now empty for a
-- VALUES-bearing query; the declared column vars surface in
-- `variables`. DESCRIBE now ships (Phase F group F3, LLD §11) —
-- sparql_parse reports form="DESCRIBE", NOT flagged unsupported.
SELECT pgrdf.sparql_parse(
  'PREFIX ex: <http://example.com/>
   SELECT ?x ?y WHERE { ?x ex:p ?y } VALUES (?x) { (ex:a) (ex:b) }'
)->'unsupported_algebra' AS o_values_unsupported;
SELECT (pgrdf.sparql_parse(
  'PREFIX ex: <http://example.com/>
   SELECT ?x ?y WHERE { ?x ex:p ?y } VALUES (?x) { (ex:a) (ex:b) }'
)->'variables' ? 'x')::text AS o_values_var_x_present;
SELECT pgrdf.sparql_parse('DESCRIBE <http://example.com/a>')->>'form'
  AS o_describe_form;
SELECT pgrdf.sparql_parse('DESCRIBE <http://example.com/a>')
  ->'unsupported_algebra' AS o_describe_unsupported;

ROLLBACK;
