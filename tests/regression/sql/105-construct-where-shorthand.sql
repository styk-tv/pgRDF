-- 105-construct-where-shorthand.sql
--
-- Phase D slice 54 — `pgrdf.construct` `CONSTRUCT WHERE { pattern }`
-- shorthand form, per W3C SPARQL 1.1 §16.2.4 ("Querying for the
-- Solutions"). The shorthand omits the template block; the WHERE
-- pattern itself BECOMES the template. spargebra populates the AST's
-- `template` field by cloning the BGP triples (parser.rs
-- `ConstructQuery` rule, `template: c.clone()`), so the executor's
-- existing slice-56 multi-triple emission path handles the shorthand
-- form without any new emit logic — slice 54 reduces to:
--
--   1. Detecting the shorthand form via an ASCII probe of the input
--      query string (the post-parse AST is otherwise indistinguishable
--      from the explicit `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }`
--      form).
--   2. Enforcing the two W3C restrictions:
--        a. The pattern must be a pure BGP — no OPTIONAL / UNION /
--           MINUS / FILTER / GRAPH / BIND / VALUES wrapper. spargebra's
--           grammar enforces this at parse time (the shorthand rule
--           only accepts `TriplesTemplate`), so wrapping composites
--           surface as `pgrdf.construct: parse error: …` and our
--           semantic guard is defensive (in case spargebra evolves).
--        b. No blank nodes anywhere in the pattern. spargebra's
--           `TriplesTemplate` admits blank nodes freely, so the slice
--           enforces this semantically with the W3C-citing prefix
--           `pgrdf.construct: WHERE-shorthand prohibits blank nodes
--           in the pattern (W3C SPARQL 1.1 §16.2.4)`.
--
-- Invariants locked by this file:
--
--   A. Single-triple shorthand, 3 solutions → 3 rows.
--   B. Equivalence with the explicit form (same row count, same
--      structured-term row shapes) — the W3C equivalence proof.
--   C. Multi-triple shorthand: 2-triple BGP × N solutions → 2N rows,
--      with the join preserved (both template triples agree on the
--      shared variable per solution).
--   D. Constants in the shorthand BGP: `CONSTRUCT WHERE
--      { <s1> ?p ?o }` scopes to the constant subject only.
--   E. Composite-FILTER rejected at parse time. Assert the parse-error
--      prefix.
--   F. Composite-OPTIONAL rejected at parse time. Same prefix.
--   G. Blank node in shorthand pattern rejected semantically with the
--      W3C-citing message.
--   H. GRAPH-wrapper in shorthand rejected at parse time. Same prefix.
--   I. Regression preservation — the explicit `CONSTRUCT { } WHERE
--      { … }` empty-template form STILL panics with the slice-56
--      `pgrdf.construct: empty template` prefix. The shorthand
--      detection MUST NOT swallow this case.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — mirrors 101 / 102 / 103 / 104.
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
    RETURN format('%s: f (got: %s)', label, left(msg, 120));
  END IF;
END
$$;

-- ─── Invariant A: single-triple shorthand, 3 solutions ──────────────
-- Three seed quads, single-triple shorthand emits 3 rows, one per
-- matched solution. Each row's structured-term shape matches the
-- per-solution dict resolve.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:a ex:p "1" .
   ex:b ex:p "2" .
   ex:c ex:p "3" .',
  0);

SELECT count(*)::bigint AS a_row_count
  FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s <http://example.org/p> ?o }');

-- Subject IRIs come back sorted (deterministic assertion).
SELECT string_agg(j->'subject'->>'value', ',' ORDER BY j->'subject'->>'value') AS a_subjects
  FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s <http://example.org/p> ?o }') AS s(j);

-- Object literals carry datatype + value (plain string → xsd:string).
SELECT count(DISTINCT j->'object'->>'value')::bigint AS a_distinct_obj_values,
       max(j->'object'->>'datatype') AS a_obj_datatype
  FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s <http://example.org/p> ?o }') AS s(j);

-- ─── Invariant B: equivalence with the explicit form ───────────────
-- The shorthand and explicit forms must emit IDENTICAL row sets.
-- We compare via array_agg ORDER BY j::text — same row count, same
-- ordered serialised JSONB equates to equivalent solution sets.
WITH
  s AS (
    SELECT j FROM pgrdf.construct(
      'CONSTRUCT WHERE { ?s <http://example.org/p> ?o }') AS t(j)
  ),
  e AS (
    SELECT j FROM pgrdf.construct(
      'CONSTRUCT { ?s <http://example.org/p> ?o } '
      'WHERE { ?s <http://example.org/p> ?o }') AS t(j)
  )
SELECT
  (SELECT count(*)::bigint FROM s) AS b_short_count,
  (SELECT count(*)::bigint FROM e) AS b_explicit_count,
  ((SELECT array_agg(j::text ORDER BY j::text) FROM s)
   = (SELECT array_agg(j::text ORDER BY j::text) FROM e)) AS b_equivalent;

-- ─── Invariant C: multi-triple shorthand BGP ───────────────────────
-- Two predicates per subject; shorthand 2-triple BGP emits 2 rows per
-- matched solution. We seed 3 subjects each carrying both predicates;
-- expect 6 rows (3 solutions × 2 template triples).
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:c1 ex:p1 "x1" .
   ex:c1 ex:p2 "y1" .
   ex:c2 ex:p1 "x2" .
   ex:c2 ex:p2 "y2" .
   ex:c3 ex:p1 "x3" .
   ex:c3 ex:p2 "y3" .',
  0);

SELECT count(*)::bigint AS c_row_count
  FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s <http://example.org/p1> ?o1 . '
    '                  ?s <http://example.org/p2> ?o2 }');

-- Each template-triple emits one row per solution → 3 of each.
SELECT count(*)::bigint AS c_p1_rows
  FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s <http://example.org/p1> ?o1 . '
    '                  ?s <http://example.org/p2> ?o2 }') AS s(j)
  WHERE j->'predicate'->>'value' = 'http://example.org/p1';

SELECT count(*)::bigint AS c_p2_rows
  FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s <http://example.org/p1> ?o1 . '
    '                  ?s <http://example.org/p2> ?o2 }') AS s(j)
  WHERE j->'predicate'->>'value' = 'http://example.org/p2';

-- Join preservation: for each subject ?s, both emitted rows share the
-- same ?s — verified by pairing the p1-row and p2-row of the same
-- subject. We expect 3 paired groups (one per solution), each with
-- exactly 1 p1 row and 1 p2 row.
WITH r AS (
  SELECT * FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s <http://example.org/p1> ?o1 . '
    '                  ?s <http://example.org/p2> ?o2 }') AS t(j)
)
SELECT
  count(DISTINCT j->'subject'->>'value')::bigint AS c_distinct_subjects,
  bool_and(per_subject_pairs = 2) AS c_pair_per_subject
FROM (
  SELECT
    j,
    count(*) OVER (PARTITION BY j->'subject'->>'value') AS per_subject_pairs
  FROM r
) z;

-- ─── Invariant D: constants in shorthand BGP ───────────────────────
-- Constant subject narrows the result to that subject only — 1 row.
SELECT count(*)::bigint AS d_row_count
  FROM pgrdf.construct(
    'CONSTRUCT WHERE { <http://example.org/c1> <http://example.org/p1> ?o }');

SELECT max(j->'subject'->>'value') AS d_subject
  FROM pgrdf.construct(
    'CONSTRUCT WHERE { <http://example.org/c1> <http://example.org/p1> ?o }') AS s(j);

-- ─── Invariant E: composite-FILTER rejected at parse time ─────────
-- spargebra's shorthand rule accepts `TriplesTemplate` only — a
-- FILTER inside the braces is a parse error. The surfaced prefix is
-- `pgrdf.construct: parse error: …`.
SELECT _check_error(
  'e-rejects-filter-in-shorthand',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s ?p ?o FILTER(?o > 0) }')$$,
  'pgrdf.construct: parse error'
);

-- ─── Invariant F: composite-OPTIONAL rejected at parse time ───────
SELECT _check_error(
  'f-rejects-optional-in-shorthand',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s <http://ex/p> ?o OPTIONAL { ?s <http://ex/q> ?o2 } }')$$,
  'pgrdf.construct: parse error'
);

-- ─── Invariant G: blank node in shorthand pattern rejected ────────
-- spargebra DOES admit the blank node at parse; the slice-54 semantic
-- guard catches it and panics with the W3C-citing message.
SELECT _check_error(
  'g-rejects-bnode-in-shorthand',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT WHERE { ?s ?p _:b }')$$,
  'pgrdf.construct: WHERE-shorthand prohibits blank nodes'
);

-- ─── Invariant H: GRAPH-wrapper in shorthand rejected at parse ────
SELECT _check_error(
  'h-rejects-graph-wrapper-in-shorthand',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT WHERE { GRAPH <http://ex/g> { ?s ?p ?o } }')$$,
  'pgrdf.construct: parse error'
);

-- ─── Invariant I: explicit empty-template still panics ────────────
-- Regression-preservation: the slice-56 `pgrdf.construct: empty
-- template` panic MUST still fire for the EXPLICIT empty-template
-- form. The shorthand detection branch only fires for actual
-- shorthand syntax, never swallowing this case.
SELECT _check_error(
  'i-explicit-empty-template-still-panics',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { } WHERE { ?s ?p ?o }')$$,
  'pgrdf.construct: empty template'
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup so the next regression file starts from a clean slate.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
