-- 101-construct-variable-templates.sql
--
-- Phase D slice 58 — `pgrdf.construct` template VARIABLE substitution.
-- Slice 59 (`100-construct-foundation.sql`) shipped the constant-only
-- foundation; slice 58 widens template positions to admit variables in
-- subject / predicate / object slots. Blank nodes in the template
-- still panic — slice 57 lifts that.
--
-- Per-position semantics (LLD v0.4 §6.1 / §6.2):
--
--   * Constant IRI / literal → encoded once, cloned per solution.
--   * Variable → looked up in the per-solution binding, resolved
--     through the dictionary into the structured term shape
--     `{type, value, datatype, [language]}`. Term types covered:
--     `iri`, `bnode`, `literal` (plain string, typed, language).
--   * Blank-node-in-template → panic with the slice-58 prefix.
--
-- Invariants locked by this file:
--
--   A. Variable in subject position. Three solutions, constant
--      predicate + object, three rows with different ?s IRIs.
--   B. Variable in object position. Each row carries the per-
--      solution object literal value with explicit xsd:string
--      datatype.
--   C. Variable in predicate position (legal RDF). Each row carries
--      a different predicate IRI.
--   D. Multi-variable single template triple `?s ?p ?o`. One row per
--      solution, every position structured.
--   E. Typed-literal binding. xsd:integer datatype IRI surfaces in
--      the encoded cell with the lexical value preserved.
--   F. Language-tagged literal binding. `language` field carries the
--      tag and the `datatype` carries rdf:langString (RDF 1.1 §3.3).
--   G. Unbound template variable rejection — `pgrdf.construct:
--      unbound template variable ?missing`.
--   H. Blank-node-in-template still rejected — slice-58 prefix.
--   I. Literal-in-subject still rejected — legal-RDF prefix.
--   J. Mixed constant + variable template (single triple form).
--      The constant positions encode identically to the slice-59
--      path; the variable position substitutes per solution.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — same shape as 100-construct-foundation.sql's helper.
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

-- ─── Invariant A: variable in subject position ──────────────────
-- Seed three subjects sharing a predicate; CONSTRUCT picks each
-- one and emits a tagged triple. Subjects collected in sorted
-- order for deterministic assertion.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:a ex:p "1" .
   ex:b ex:p "2" .
   ex:c ex:p "3" .',
  0);

SELECT count(*)::bigint AS a_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tag> "hit" } '
    'WHERE { ?s <http://example.com/p> ?o }') AS s(j);

SELECT string_agg(j->'subject'->>'value', ',' ORDER BY j->'subject'->>'value') AS a_subjects
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tag> "hit" } '
    'WHERE { ?s <http://example.com/p> ?o }') AS s(j);

-- The predicate + object stay constant across rows.
SELECT count(DISTINCT j->'predicate'->>'value') AS a_distinct_pred,
       count(DISTINCT j->'object'->>'value')    AS a_distinct_obj
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tag> "hit" } '
    'WHERE { ?s <http://example.com/p> ?o }') AS s(j);

-- ─── Invariant B: variable in object position ───────────────────
-- Same seed — every object literal substitutes per row. Plain
-- string literals carry xsd:string explicitly.
SELECT string_agg(j->'object'->>'value', ',' ORDER BY j->'object'->>'value') AS b_objects
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/src> <http://example.com/val> ?o } '
    'WHERE { ?s <http://example.com/p> ?o }') AS s(j);

SELECT count(DISTINCT j->'object'->>'datatype') AS b_distinct_dt,
       max(j->'object'->>'datatype')             AS b_dt
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/src> <http://example.com/val> ?o } '
    'WHERE { ?s <http://example.com/p> ?o }') AS s(j);

-- ─── Invariant C: variable in predicate position (legal RDF) ────
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:a2 ex:p1 "x" .
   ex:a2 ex:p2 "y" .',
  0);

SELECT string_agg(j->'predicate'->>'value', ',' ORDER BY j->'predicate'->>'value') AS c_preds
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/a2> ?p "tagged" } '
    'WHERE { <http://example.com/a2> ?p ?o }') AS s(j);

-- ─── Invariant D: all-variable template ─────────────────────────
-- Use the unique seed introduced for this invariant so the row
-- count is exactly 1 — keeps the assertion stable.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:d_s ex:d_p "d_o" .',
  0);

SELECT
  (j->'subject'->>'type')      AS d_s_type,
  (j->'subject'->>'value')     AS d_s_value,
  (j->'predicate'->>'type')    AS d_p_type,
  (j->'predicate'->>'value')   AS d_p_value,
  (j->'object'->>'type')       AS d_o_type,
  (j->'object'->>'value')      AS d_o_value,
  (j->'object'->>'datatype')   AS d_o_datatype
FROM pgrdf.construct(
  'CONSTRUCT { ?s ?p ?o } '
  'WHERE { ?s <http://example.com/d_p> ?o . '
  '        ?s ?p ?o }') AS s(j);

-- ─── Invariant E: typed-literal binding ─────────────────────────
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
   ex:eperson ex:age "42"^^xsd:integer .',
  0);

SELECT
  (j->'object'->>'value')      AS e_value,
  (j->'object'->>'datatype')   AS e_datatype
FROM pgrdf.construct(
  'CONSTRUCT { ?s <http://example.com/n> ?o } '
  'WHERE { ?s <http://example.com/age> ?o }') AS s(j);

-- ─── Invariant F: language-tagged literal binding ───────────────
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:falice ex:name "Alice"@en .',
  0);

SELECT
  (j->'object'->>'value')      AS f_value,
  (j->'object'->>'language')   AS f_language,
  (j->'object'->>'datatype')   AS f_datatype
FROM pgrdf.construct(
  'CONSTRUCT { ?s <http://example.com/label> ?o } '
  'WHERE { ?s <http://example.com/name> ?o }') AS s(j);

-- ─── Invariant G: unbound template variable rejection ───────────
SELECT _check_error(
  'g-rejects-unbound-template-var',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/t> ?missing } WHERE { ?s ?p ?o }')$$,
  $$pgrdf.construct: unbound template variable ?missing$$
);

-- ─── Invariant H: blank-node-in-template still rejected ─────────
SELECT _check_error(
  'h-rejects-bnode-in-template',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/s> <http://example.com/p> _:b } WHERE { ?s ?p ?o }')$$,
  $$pgrdf.construct: slice 58$$
);

-- ─── Invariant I: literal-in-subject still rejected ─────────────
SELECT _check_error(
  'i-rejects-literal-in-subject',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { "lit" <http://example.com/p> "x" } WHERE { ?s ?p ?o }')$$,
  $$pgrdf.construct: literal not allowed in subject/predicate position$$
);

-- ─── Invariant J: mixed constant + variable in one template ─────
-- Single-triple template (multi-triple lands in slice 56). The
-- subject is a variable, the predicate is a constant IRI, and the
-- object is a constant literal — verify each position encodes via
-- its respective slice (variable substitution vs slice-59 constant).
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:j1 ex:src "_" .
   ex:j2 ex:src "_" .',
  0);

SELECT count(*)::bigint AS j_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/p_const> "obj_const" } '
    'WHERE { ?s <http://example.com/src> ?o }') AS s(j);

SELECT string_agg(j->'subject'->>'value', ',' ORDER BY j->'subject'->>'value') AS j_subjects,
       max(j->'predicate'->>'value') AS j_pred,
       max(j->'object'->>'value')    AS j_obj,
       max(j->'object'->>'datatype') AS j_obj_dt
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/p_const> "obj_const" } '
    'WHERE { ?s <http://example.com/src> ?o }') AS s(j);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
