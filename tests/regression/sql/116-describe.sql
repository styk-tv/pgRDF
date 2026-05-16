-- 116-describe.sql
--
-- Phase F group F3 (slices 26-24, LLD v0.4 §11) — SPARQL DESCRIBE.
-- `pgrdf.describe(q TEXT) → SETOF JSONB` is the sibling UDF to
-- `pgrdf.construct` (Phase D §6.1 sibling-UDF rationale: the caller
-- signals intent at the SQL boundary). The "description" is NOT a
-- CONSTRUCT template — there is no `{ template }`. Per W3C §16.4 +
-- the LLD §11 bullet the description is the *closure* of each
-- described resource:
--
--   For each described resource R (IRI or blank node — a literal
--   can't be a subject so it yields an empty description), every
--   triple `(R, ?p, ?o)`. Whenever an emitted object `?o` is itself
--   a blank node, recurse into that bnode's `(?o, ?p2, ?o2)`
--   triples, and keep following while the frontier object stays a
--   blank node ("transitively expanded one hop on blank nodes").
--   Recursion only ever traverses blank-node objects (never IRIs),
--   so it terminates on any finite graph; a visited-set of bnode
--   ids additionally guarantees termination on bnode cycles. The
--   output is the same {subject,predicate,object} structured-term
--   JSONB shape as `pgrdf.construct` (byte-identical encoders).
--
-- Invariants (all expected values hand-computed; never ACCEPT=1):
--
--   A. `DESCRIBE <iri>` constant, no WHERE → all (iri,?p,?o)
--      triples; deduped; structured-term shape matches construct.
--   B. `DESCRIBE <iri>` on an IRI with no triples → 0 rows (no
--      error).
--   C. `DESCRIBE ?x WHERE { ?x a ex:Thing }` → union of closures of
--      every ?x binding; deduped across bindings.
--   D. `DESCRIBE <a> <b> ?x WHERE { … }` mixed constant + variable.
--   E. `DESCRIBE *` describes all projected variable bindings.
--   F. Blank-node transitive one-hop expansion: a hand-built bnode
--      chain `<r> ex:p _:b1 . _:b1 ex:q _:b2 . _:b2 ex:r "leaf"`;
--      DESCRIBE <r> includes the <r> triple AND _:b1's AND _:b2's,
--      terminating at the literal. Plus a bnode cycle: assert
--      termination + the exact finite row set.
--   G. GRAPH-scoped DESCRIBE — closure computed within the named
--      graph; other graphs' triples about the same subject excluded.
--   H. LLD §11 acceptance: sparql_parse reports form:"DESCRIBE",
--      describe.kind, and does NOT flag DESCRIBE in
--      unsupported_algebra; F1/F2 forms still absent (not regressed).
--   I. DESCRIBE through pgrdf.sparql → clean redirect panic to
--      pgrdf.describe (mirrors the CONSTRUCT entry-point contract).
--   J. pgrdf.describe row shape byte-identical to pgrdf.construct
--      for an equivalent triple.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Default-graph (id 0) fixture.
--   ex:a ex:name "Alice" ; ex:age 30 ; ex:knows ex:b   (3 triples)
--   ex:b ex:name "Bob"                                  (1 triple)
--   ex:t1 a ex:Thing ; ex:p "p1"                        (2 triples)
--   ex:t2 a ex:Thing ; ex:p "p2"                        (2 triples)
--   ex:other a ex:NotThing ; ex:p "p3"                  (2 triples)
-- Bnode chain (F): ex:r ex:p _:c1 ; _:c1 ex:q _:c2 ; _:c2 ex:r "leaf"
-- Bnode cycle (F): ex:root ex:link _:y1 ; _:y1 ex:p _:y2 ;
--                  _:y2 ex:p _:y1
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     @prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
     ex:a ex:name "Alice" ; ex:age 30 ; ex:knows ex:b .
     ex:b ex:name "Bob" .
     ex:t1 rdf:type ex:Thing ; ex:p "p1" .
     ex:t2 rdf:type ex:Thing ; ex:p "p2" .
     ex:other rdf:type ex:NotThing ; ex:p "p3" .
     ex:r ex:p _:c1 .
     _:c1 ex:q _:c2 .
     _:c2 ex:r "leaf" .
     ex:root ex:link _:y1 .
     _:y1 ex:p _:y2 .
     _:y2 ex:p _:y1 .',
    0);
  -- Named graph for G — ex:a has DIFFERENT triples here so a scoped
  -- DESCRIBE must NOT pick up the default-graph triples.
  PERFORM pgrdf.add_graph('http://example.com/g1');
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:a ex:nick "Al" ; ex:city "NYC" .',
    pgrdf.graph_id('http://example.com/g1'));
END $$;

-- ─── A. DESCRIBE <iri> constant, no WHERE ───────────────────────
-- ex:a has exactly 3 triples (name, age, knows). ex:knows points to
-- ex:b (an IRI, NOT a blank node) so ex:b's triple is NOT included
-- (IRI objects are closure leaves). Unscoped → scans all graphs;
-- ex:a has no triples in g1 under those predicates (it has ex:nick /
-- ex:city there) so the all-graph closure of ex:a is the 3 default
-- + 2 g1 = 5 triples.
SELECT count(*)::int AS a_describe_count
  FROM pgrdf.describe('DESCRIBE <http://example.com/a>');

-- Every emitted subject is ex:a; sorted predicate localnames.
SELECT string_agg(
         replace(d->'predicate'->>'value','http://example.com/',''),
         ',' ORDER BY d->'predicate'->>'value')      AS a_predicates
  FROM pgrdf.describe('DESCRIBE <http://example.com/a>') AS d;

-- The literal-object triple is shaped exactly like construct.
SELECT
  d->'subject'->>'type'   AS a_s_type,
  d->'subject'->>'value'  AS a_s_value,
  d->'object'->>'type'    AS a_o_type,
  d->'object'->>'value'   AS a_o_value,
  d->'object'->>'datatype' AS a_o_dt
  FROM pgrdf.describe('DESCRIBE <http://example.com/a>') AS d
 WHERE d->'predicate'->>'value' = 'http://example.com/name';

-- ─── B. DESCRIBE <iri> with no triples → 0 rows, no error ───────
SELECT count(*)::int AS b_empty_count
  FROM pgrdf.describe('DESCRIBE <http://example.com/never-loaded>');

-- ─── C. DESCRIBE ?x WHERE { ?x a ex:Thing } — union + dedup ─────
-- ?x binds {t1,t2} (ex:other is a NotThing). Each has 2 triples
-- (rdf:type, ex:p) = 4 rows. ex:other excluded.
SELECT count(*)::int AS c_var_count
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE ?x WHERE { ?x a ex:Thing }');

SELECT string_agg(DISTINCT d->'subject'->>'value', ',' ORDER BY
                   d->'subject'->>'value')           AS c_subjects
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE ?x WHERE { ?x a ex:Thing }') AS d;

-- Dedup: ?x bound by TWO predicates still emits its closure once.
-- ex:t1 matched twice (ex:p AND rdf:type both bind ?x via the
-- WHERE below) → its 2-triple closure appears exactly once = 2 rows
-- for ex:t1 (not 4).
SELECT count(*)::int AS c_dedup_t1_rows
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE ?x WHERE { ?x ex:p ?v . ?x a ?type }') AS d
 WHERE d->'subject'->>'value' = 'http://example.com/t1';

-- ─── D. Mixed constant + variable terms ─────────────────────────
-- DESCRIBE <b> ?x WHERE { ?x a ex:Thing } → closure(b)=1 (name) +
-- closure(t1)=2 + closure(t2)=2 = 5 rows.
SELECT count(*)::int AS d_mixed_count
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE <http://example.com/b> ?x WHERE { ?x a ex:Thing }');

SELECT string_agg(DISTINCT d->'subject'->>'value', ',' ORDER BY
                   d->'subject'->>'value')           AS d_subjects
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE <http://example.com/b> ?x WHERE { ?x a ex:Thing }') AS d;

-- ─── E. DESCRIBE * ──────────────────────────────────────────────
-- DESCRIBE * WHERE { ex:a ex:knows ?x } binds ?x = ex:b. `*`
-- projects every in-scope var; here only ?x (ex:a is constant in
-- the BGP, not a projected term). closure(b) = 1 triple (ex:name).
SELECT count(*)::int AS e_star_count
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE * WHERE { <http://example.com/a> ex:knows ?x }');

SELECT DISTINCT d->'subject'->>'value' AS e_star_subject
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE * WHERE { <http://example.com/a> ex:knows ?x }') AS d;

-- ─── F. Blank-node transitive one-hop expansion ─────────────────
-- ex:r ex:p _:c1 ; _:c1 ex:q _:c2 ; _:c2 ex:r "leaf".
-- DESCRIBE <r> emits (r,p,_:c1) + (_:c1,q,_:c2) + (_:c2,r,"leaf")
-- = 3 rows. The closure follows bnode objects transitively until
-- the literal leaf (IRIs/literals are leaves).
SELECT count(*)::int AS f_chain_count
  FROM pgrdf.describe('DESCRIBE <http://example.com/r>');

-- Predicate localnames, sorted: p (r→_:c1), q (_:c1→_:c2),
-- r (_:c2→"leaf").
SELECT string_agg(
         replace(d->'predicate'->>'value','http://example.com/',''),
         ',' ORDER BY d->'predicate'->>'value')      AS f_chain_preds
  FROM pgrdf.describe('DESCRIBE <http://example.com/r>') AS d;

-- The terminal triple: a bnode subject, ex:r predicate, the literal
-- "leaf" object — proves the closure reached + stopped at the leaf.
SELECT
  d->'subject'->>'type'  AS f_leaf_s_type,
  d->'object'->>'type'   AS f_leaf_o_type,
  d->'object'->>'value'  AS f_leaf_o_value
  FROM pgrdf.describe('DESCRIBE <http://example.com/r>') AS d
 WHERE d->'predicate'->>'value' = 'http://example.com/r';

-- The first hop: ex:r (IRI subject) ex:p a bnode object.
SELECT
  d->'subject'->>'type'  AS f_head_s_type,
  d->'subject'->>'value' AS f_head_s_value,
  d->'object'->>'type'   AS f_head_o_type
  FROM pgrdf.describe('DESCRIBE <http://example.com/r>') AS d
 WHERE d->'predicate'->>'value' = 'http://example.com/p';

-- Bnode cycle: ex:root ex:link _:y1 ; _:y1 ex:p _:y2 ;
-- _:y2 ex:p _:y1. DESCRIBE <root> walks root→_:y1→_:y2→(_:y1 seen,
-- stop). Exactly 3 triples; MUST terminate.
SELECT count(*)::int AS f_cycle_count
  FROM pgrdf.describe('DESCRIBE <http://example.com/root>');

SELECT string_agg(
         replace(d->'predicate'->>'value','http://example.com/',''),
         ',' ORDER BY d->'predicate'->>'value')      AS f_cycle_preds
  FROM pgrdf.describe('DESCRIBE <http://example.com/root>') AS d;

-- ─── G. GRAPH-scoped DESCRIBE ───────────────────────────────────
-- Scoped to g1: ex:a there has ex:nick "Al" + ex:city "NYC" = 2
-- triples. The DEFAULT-graph ex:a triples (name/age/knows) MUST be
-- excluded — the closure is computed within the named graph.
SELECT count(*)::int AS g_scoped_count
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE <http://example.com/a>
       WHERE { GRAPH <http://example.com/g1> { ?s ?p ?o } }');

SELECT string_agg(
         replace(d->'predicate'->>'value','http://example.com/',''),
         ',' ORDER BY d->'predicate'->>'value')      AS g_scoped_preds
  FROM pgrdf.describe(
    'PREFIX ex: <http://example.com/>
     DESCRIBE <http://example.com/a>
       WHERE { GRAPH <http://example.com/g1> { ?s ?p ?o } }') AS d;

-- ─── H. LLD §11 acceptance — sparql_parse reports DESCRIBE ──────
SELECT pgrdf.sparql_parse(
         'DESCRIBE <http://example.com/a>')->>'form'  AS h_form_const;
SELECT pgrdf.sparql_parse(
         'DESCRIBE <http://example.com/a>')->'describe'->>'kind'
                                                       AS h_kind_const;
SELECT pgrdf.sparql_parse(
         'PREFIX ex: <http://example.com/>
          DESCRIBE ?x WHERE { ?x a ex:Thing }')->'describe'->>'kind'
                                                       AS h_kind_var;
SELECT pgrdf.sparql_parse(
         'PREFIX ex: <http://example.com/>
          DESCRIBE <http://example.com/a> ?x WHERE { ?x a ex:T }')
         ->'describe'->>'kind'                         AS h_kind_mixed;
-- DESCRIBE is NOT flagged unsupported (the §11 acceptance binding).
SELECT pgrdf.sparql_parse(
         'PREFIX ex: <http://example.com/>
          DESCRIBE ?x WHERE { ?x a ex:Thing }')
         ->>'unsupported_algebra'                      AS h_unsupported;
-- F1/F2 not regressed: OPTIONAL/VALUES/BIND/AGG-UNION still absent
-- from a SELECT's unsupported_algebra.
SELECT pgrdf.sparql_parse(
         'PREFIX ex: <http://example.com/>
          SELECT (COUNT(?v) AS ?n) WHERE {
            { ?s ex:p ?v } UNION { ?s ex:q ?v } }')
         ->>'unsupported_algebra'                      AS h_f2_not_regressed;

-- ─── I. DESCRIBE through pgrdf.sparql → redirect panic ──────────
-- Reuse the canonical `_check_error(label, sql, expected_fragment)`
-- helper shape (see 93-update-insert-data.sql): EXECUTE the SQL in a
-- subtransaction, catch the panic, match the stable fragment.
CREATE OR REPLACE FUNCTION _check_error(label TEXT, sql TEXT, expected_fragment TEXT)
RETURNS TEXT
LANGUAGE plpgsql AS $$
DECLARE
  msg TEXT;
BEGIN
  BEGIN
    EXECUTE sql;
    RETURN format('%s: !!! unexpected success !!!', label);
  EXCEPTION WHEN OTHERS THEN
    msg := SQLERRM;
  END;
  IF position(expected_fragment IN msg) > 0 THEN
    RETURN format('%s: t', label);
  ELSE
    RETURN format('%s: f (got: %s)', label, left(msg, 80));
  END IF;
END
$$;

-- A DESCRIBE through pgrdf.sparql redirects (mirrors how
-- pgrdf.construct is the CONSTRUCT entry point).
SELECT _check_error(
  'i-redirect',
  'SELECT count(*) FROM pgrdf.sparql(''DESCRIBE <http://example.com/a>'')',
  'use pgrdf.describe(q) for DESCRIBE queries'
);

-- pgrdf.describe rejects a non-DESCRIBE query (parallel to
-- pgrdf.construct's "not a CONSTRUCT query").
SELECT _check_error(
  'i-reject-select',
  'SELECT count(*) FROM pgrdf.describe(''SELECT ?s WHERE { ?s ?p ?o }'')',
  'pgrdf.describe: not a DESCRIBE query'
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- ─── J. Byte-identical row shape vs pgrdf.construct ─────────────
-- Both paths emit the (ex:b, ex:name, "Bob") triple. The JSONB
-- structure (keys + term encoding) must be identical.
SELECT
  (SELECT d::text FROM pgrdf.describe(
     'DESCRIBE <http://example.com/b>') AS d LIMIT 1)
  =
  (SELECT c::text FROM pgrdf.construct(
     'PREFIX ex: <http://example.com/>
      CONSTRUCT { <http://example.com/b> ex:name "Bob" }
      WHERE { <http://example.com/b> ex:name "Bob" }') AS c LIMIT 1)
  AS j_byte_identical;

ROLLBACK;

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
