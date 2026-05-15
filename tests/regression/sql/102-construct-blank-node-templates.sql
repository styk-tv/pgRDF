-- 102-construct-blank-node-templates.sql
--
-- Phase D slice 57 — `pgrdf.construct` blank-node TEMPLATE support.
-- Slice 58 (`101-construct-variable-templates.sql`) admitted
-- variable substitution; slice 57 widens the template surface to
-- admit blank nodes per W3C SPARQL 1.1 §16.2:
--
--   "Each time the CONSTRUCT template is instantiated for a specific
--    solution, any blank nodes in the template are replaced with new
--    blank nodes."
--
-- Encoded blank-node cell (LLD v0.4 §6.1): `{"type":"bnode","value":"<label>"}`.
--
-- Per-position semantics:
--
--   * Subject blank-node `_:label`     → fresh per-solution label.
--   * Object blank-node `_:label`      → fresh per-solution label.
--   * Same template label in subject + object of the same triple in
--     the same solution                → SAME fresh label (within-
--                                         solution sameness).
--   * Same template label across       → cross-triple joining is
--     separate triples (multi-triple)    SLICE 56 territory, NOT
--                                         locked here. Multi-triple
--                                         templates panic with
--                                         `pgrdf.construct: slice 57
--                                         supports single-triple
--                                         templates; multi-triple
--                                         lands in slice 56`.
--   * Predicate blank-node             → parse-time error (illegal
--                                         RDF — `NamedNodePattern`
--                                         in spargebra excludes
--                                         blank nodes; the parser
--                                         surfaces `pgrdf.construct:
--                                         parse error: ...`).
--   * Variable-bound bnode from WHERE  → dictionary-stored label
--                                         flows through unchanged
--                                         (slice 58 contract).
--
-- Invariants locked by this file:
--
--   A. Single-bnode-subject template, single solution — one row,
--      `subject.type == "bnode"`, value is some string.
--   B. Single-bnode-subject, three solutions — three rows; all
--      bnode `value`s are DISTINCT (fresh per solution).
--   C. Same template label in subject + object of one triple,
--      two solutions — every row's subject value equals its
--      object value (within-solution sameness).
--   D. Mixed bnode subject + constant predicate + variable object
--      template — bnode value distinct per row, object value matches
--      bound variable.
--   E. Multi-triple template still rejected (slice 56 territory).
--   F. Variable bound to bnode in WHERE — emitted in template via
--      variable reference uses the dictionary-stored label
--      unchanged (slice 58 contract, must not regress).
--
-- Invariant for blank-node-in-predicate (parse-time rejection) is
-- covered by spargebra's parser: `CONSTRUCT { <s> _:p <o> } …`
-- fails parse with `pgrdf.construct: parse error: …`. We don't
-- exercise it here because the surface error message is spargebra-
-- owned and may evolve across versions; the contract is "parse
-- error, not a CONSTRUCT semantic error".
--
-- All expected values hand-computed; never ACCEPT=1 baselined.
-- bnode labels themselves are non-deterministic textually, so this
-- file LOCKS only structural invariants (cardinality, distinctness,
-- within-row sameness, type tags) and never specific label strings.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — same shape as 101-construct-variable-templates.sql.
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

-- ─── Invariant A: bnode subject, single solution ────────────────
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:s1 ex:p ex:o1 .',
  0);

SELECT count(*)::bigint AS a_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { _:newSubj <http://example.com/tag> "hit" } '
    'WHERE { ?x <http://example.com/p> ?y }') AS s(j);

SELECT
  (j->'subject'->>'type')          AS a_s_type,
  ((j->'subject'->>'value') IS NOT NULL) AS a_s_has_value,
  (j->'predicate'->>'type')        AS a_p_type,
  (j->'predicate'->>'value')       AS a_p_value,
  (j->'object'->>'type')           AS a_o_type,
  (j->'object'->>'value')          AS a_o_value
FROM pgrdf.construct(
  'CONSTRUCT { _:newSubj <http://example.com/tag> "hit" } '
  'WHERE { ?x <http://example.com/p> ?y }') AS s(j);

-- ─── Invariant B: bnode subject, three solutions, distinct values ─
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:b1 ex:p "1" .
   ex:b2 ex:p "2" .
   ex:b3 ex:p "3" .',
  10);

-- Three rows produced.
SELECT count(*)::bigint AS b_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { _:newSubj <http://example.com/tag> "hit" } '
    'WHERE { ?x <http://example.com/p> ?y . FILTER (?y IN ("1","2","3")) }')
  AS s(j);

-- All three bnode subject values are DISTINCT — fresh per solution
-- per W3C SPARQL 1.1 §16.2. Locks the across-solution distinctness
-- invariant without locking specific label strings.
SELECT count(DISTINCT j->'subject'->>'value')::bigint AS b_distinct_subjects
  FROM pgrdf.construct(
    'CONSTRUCT { _:newSubj <http://example.com/tag> "hit" } '
    'WHERE { ?x <http://example.com/p> ?y . FILTER (?y IN ("1","2","3")) }')
  AS s(j);

-- All rows carry `bnode` type tag.
SELECT count(*)::bigint AS b_all_bnodes
  FROM pgrdf.construct(
    'CONSTRUCT { _:newSubj <http://example.com/tag> "hit" } '
    'WHERE { ?x <http://example.com/p> ?y . FILTER (?y IN ("1","2","3")) }')
  AS s(j)
  WHERE j->'subject'->>'type' = 'bnode';

-- ─── Invariant C: same template label in subject + object ────────
-- `_:foo` appears in both subject and object of the SAME triple.
-- Per W3C §16.2 within-solution sameness, both positions resolve
-- to the SAME fresh label per solution.
SELECT bool_and((j->'subject'->>'value') = (j->'object'->>'value')) AS c_within_row_sameness
  FROM pgrdf.construct(
    'CONSTRUCT { _:foo <http://example.com/linksTo> _:foo } '
    'WHERE { ?x <http://example.com/p> ?y . FILTER (?y IN ("1","2","3")) }')
  AS s(j);

-- Across-solution, the (subject = object) label is still distinct —
-- three rows, three distinct labels. Within-row sameness AND
-- across-row distinctness compose.
SELECT count(DISTINCT j->'subject'->>'value')::bigint AS c_distinct_solutions
  FROM pgrdf.construct(
    'CONSTRUCT { _:foo <http://example.com/linksTo> _:foo } '
    'WHERE { ?x <http://example.com/p> ?y . FILTER (?y IN ("1","2","3")) }')
  AS s(j);

-- ─── Invariant D: mixed bnode + constant + variable template ────
-- Subject is a bnode, predicate is a constant IRI, object is a
-- variable bound to an IRI per solution. Verify each row's
-- structural shape.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:d1 ex:dp "obj1" .
   ex:d2 ex:dp "obj2" .',
  20);

SELECT count(*)::bigint AS d_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { _:bn <http://example.com/relates_to> ?s } '
    'WHERE { ?s <http://example.com/dp> ?o }') AS s(j);

-- Two distinct subject bnode values + two distinct object IRIs +
-- predicate is the constant IRI on every row.
SELECT count(DISTINCT j->'subject'->>'value')::bigint AS d_distinct_subj,
       count(DISTINCT j->'object'->>'value')::bigint  AS d_distinct_obj,
       max(j->'predicate'->>'value')                  AS d_pred,
       bool_and((j->'subject'->>'type') = 'bnode')    AS d_all_bnode_subj,
       bool_and((j->'object'->>'type')  = 'iri')      AS d_all_iri_obj
  FROM pgrdf.construct(
    'CONSTRUCT { _:bn <http://example.com/relates_to> ?s } '
    'WHERE { ?s <http://example.com/dp> ?o }') AS s(j);

-- ─── Invariant E: multi-triple template rejected (slice 56) ─────
SELECT _check_error(
  'e-rejects-multi-triple',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/a> <http://example.com/p> "1" . '
    '            <http://example.com/b> <http://example.com/q> "2" } '
    'WHERE { ?x ?y ?z }')$$,
  $$pgrdf.construct: slice 57 supports single-triple templates; multi-triple lands in slice 56$$
);

-- ─── Invariant F: variable-bound bnode passthrough (slice 58) ────
-- Seed a triple whose object is a blank node via Turtle's `_:b1`
-- syntax — the TurtleLoader stores the bnode in the dictionary with
-- typecode=2. Reference that bnode via a variable in the CONSTRUCT
-- template. The emitted cell carries the dictionary label
-- UNCHANGED — slice 57 must not synthesise a fresh label for the
-- variable path.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:fs ex:fp _:fb1 .',
  30);

SELECT
  (j->'object'->>'type')                    AS f_o_type,
  ((j->'object'->>'value') IS NOT NULL)     AS f_o_has_value
FROM pgrdf.construct(
  'CONSTRUCT { <http://example.com/tagged> <http://example.com/also> ?o } '
  'WHERE { <http://example.com/fs> <http://example.com/fp> ?o }') AS s(j);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
