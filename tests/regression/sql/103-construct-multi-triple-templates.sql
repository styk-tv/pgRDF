-- 103-construct-multi-triple-templates.sql
--
-- Phase D slice 56 — `pgrdf.construct` MULTI-TRIPLE template support.
-- Slice 57 (`102-construct-blank-node-templates.sql`) narrowed the
-- template surface to a single triple; slice 56 widens to N-triple
-- templates per W3C SPARQL 1.1 §16.2 — the `{ … }` block carries N
-- triples separated by `.`, and for each solution the engine emits
-- N rows (one per template triple), each carrying that triple's
-- per-solution instantiation.
--
-- Cross-triple semantics (slice 56 contract):
--
--   * Cardinality: N template triples × M solutions → N×M emitted
--     rows.
--   * Blank-node labels are SHARED across all N triples WITHIN the
--     same solution. So `_:r` in triple-1 subject position and `_:r`
--     in triple-3 object position resolve to the SAME fresh label
--     for that solution.
--   * Across solutions, the same template label STILL mints a NEW
--     fresh label per solution (the W3C §16.2 fresh-per-solution
--     invariant is preserved).
--   * Distinct template labels (`_:a` vs `_:b`) within the same
--     solution mint DIFFERENT fresh labels — slice 56 does NOT
--     conflate distinct labels.
--   * Variable substitution carries identically per-position per
--     template triple — every template triple sees the same
--     per-solution binding.
--   * Empty template `{ }` rejects with `pgrdf.construct: empty
--     template`.
--
-- Invariants locked by this file:
--
--   A. 2-triple constant template, 1 solution → 2 rows. Predicates
--      distinguish the two template triples; object lexicals match
--      the template constants.
--   B. 2-triple template with variable, 3 solutions → 6 rows.
--      Per template-triple cardinality is 3 (one per solution).
--   C. 3-triple template with shared `_:r` across positions and
--      template-triples, 2 solutions → 6 rows. The set of all
--      bnode labels (subject of triples 0,1 + object of triple 2)
--      contains exactly 2 distinct values — one per solution.
--      Within a solution, all three bnode positions resolve to
--      the SAME label.
--   D. 2-triple template with `_:a` + `_:b` distinct labels, 2
--      solutions → 4 rows. The 4 subject bnode labels are all
--      DISTINCT — slice 56 does not conflate `_:a` and `_:b`,
--      and labels differ across solutions.
--   E. Mixed bnode + variable in 2-triple template, 2 solutions
--      → 4 rows. The bnode appearing in triple-0 object and
--      triple-1 subject resolves to the SAME label within each
--      solution, and differs across solutions. Variable bindings
--      flow through unchanged.
--   F. Empty template `CONSTRUCT { } …` rejects with
--      `pgrdf.construct: empty template`.
--   G. Single-triple path (slice-57 territory) still works — the
--      refactor must not regress same-label joining within one
--      triple of one solution.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.
-- bnode labels themselves are non-deterministic textually, so this
-- file LOCKS only structural invariants (cardinality, cross-row
-- equality, cross-solution distinctness, type tags) and never
-- specific label strings.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — same shape as 102-construct-blank-node-templates.sql.
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

-- ─── Invariant A: 2-triple constant template, 1 solution ────────
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:s1 ex:p ex:o1 .',
  0);

-- 2 template triples × 1 solution → 2 rows.
SELECT count(*)::bigint AS a_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/s1> <http://example.com/p1> "v1" . '
    '            <http://example.com/s2> <http://example.com/p2> "v2" } '
    'WHERE { ?x <http://example.com/p> ?y }') AS s(j);

-- Each template-triple's predicate appears exactly once. Locks per-
-- template-triple cardinality independent of insertion order.
SELECT count(*)::bigint AS a_pred_p1_count
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/s1> <http://example.com/p1> "v1" . '
    '            <http://example.com/s2> <http://example.com/p2> "v2" } '
    'WHERE { ?x <http://example.com/p> ?y }') AS s(j)
  WHERE j->'predicate'->>'value' = 'http://example.com/p1';

SELECT count(*)::bigint AS a_pred_p2_count
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/s1> <http://example.com/p1> "v1" . '
    '            <http://example.com/s2> <http://example.com/p2> "v2" } '
    'WHERE { ?x <http://example.com/p> ?y }') AS s(j)
  WHERE j->'predicate'->>'value' = 'http://example.com/p2';

-- Each triple carries its own subject + object pair (no
-- cross-contamination). For predicate p1 the subject MUST be s1 and
-- the object literal MUST be v1.
SELECT
  (j->'subject'->>'value')   AS a_p1_subj,
  (j->'object'->>'value')    AS a_p1_obj
FROM pgrdf.construct(
  'CONSTRUCT { <http://example.com/s1> <http://example.com/p1> "v1" . '
  '            <http://example.com/s2> <http://example.com/p2> "v2" } '
  'WHERE { ?x <http://example.com/p> ?y }') AS s(j)
WHERE j->'predicate'->>'value' = 'http://example.com/p1';

SELECT
  (j->'subject'->>'value')   AS a_p2_subj,
  (j->'object'->>'value')    AS a_p2_obj
FROM pgrdf.construct(
  'CONSTRUCT { <http://example.com/s1> <http://example.com/p1> "v1" . '
  '            <http://example.com/s2> <http://example.com/p2> "v2" } '
  'WHERE { ?x <http://example.com/p> ?y }') AS s(j)
WHERE j->'predicate'->>'value' = 'http://example.com/p2';

-- ─── Invariant B: 2-triple template + variable, 3 solutions ─────
-- Use a B-specific predicate (`ex:bp`) so the WHERE is isolated from
-- the other sections' seeds (slice 56's coverage seeds all live in
-- the default graph; without distinct predicates we'd cross-bind).
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:b1 ex:bp "b-1" .
   ex:b2 ex:bp "b-2" .
   ex:b3 ex:bp "b-3" .',
  10);

-- 2 template triples × 3 solutions → 6 rows total.
SELECT count(*)::bigint AS b_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tagA> "A" . '
    '            ?s <http://example.com/tagB> "B" } '
    'WHERE { ?s <http://example.com/bp> ?o }')
  AS s(j);

-- Per-template-triple cardinality is 3 (one per solution) on each side.
SELECT count(*)::bigint AS b_tagA_rows
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tagA> "A" . '
    '            ?s <http://example.com/tagB> "B" } '
    'WHERE { ?s <http://example.com/bp> ?o }')
  AS s(j)
  WHERE j->'predicate'->>'value' = 'http://example.com/tagA';

SELECT count(*)::bigint AS b_tagB_rows
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tagA> "A" . '
    '            ?s <http://example.com/tagB> "B" } '
    'WHERE { ?s <http://example.com/bp> ?o }')
  AS s(j)
  WHERE j->'predicate'->>'value' = 'http://example.com/tagB';

-- The set of distinct subjects across all 6 rows is exactly 3 — the
-- variable substitution carries identically across the two template
-- triples (slice 56 doesn't break per-position binding).
SELECT count(DISTINCT j->'subject'->>'value')::bigint AS b_distinct_subj
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tagA> "A" . '
    '            ?s <http://example.com/tagB> "B" } '
    'WHERE { ?s <http://example.com/bp> ?o }')
  AS s(j);

-- ─── Invariant C: shared `_:r` across triples within solution ───
-- 3-triple template with `_:r` in subject of triples 0+1 and object
-- of triple 2. 2 solutions × 3 template triples → 6 rows. The bnode
-- labels minted MUST collapse to 2 distinct values (one per solution)
-- across all 6 rows' bnode positions.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:r1 ex:has "v1" .
   ex:r2 ex:has "v2" .',
  20);

-- 6 rows total.
SELECT count(*)::bigint AS c_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { _:r <http://example.com/type> <http://example.com/Card> . '
    '            _:r <http://example.com/value> ?v . '
    '            <http://example.com/owner> <http://example.com/owns> _:r } '
    'WHERE { ?s <http://example.com/has> ?v }') AS s(j);

-- Across all rows where `_:r` shows up (subject of 2 triples + object
-- of 1 triple — 3 positions per solution × 2 solutions = 6 bnode
-- positions total), the set of DISTINCT labels collapses to exactly
-- 2 — one per solution. Within each solution all three bnode
-- positions resolve to the SAME label.
WITH r AS (
  SELECT * FROM pgrdf.construct(
    'CONSTRUCT { _:r <http://example.com/type> <http://example.com/Card> . '
    '            _:r <http://example.com/value> ?v . '
    '            <http://example.com/owner> <http://example.com/owns> _:r } '
    'WHERE { ?s <http://example.com/has> ?v }') AS t(j)
), labels AS (
  SELECT j->'subject'->>'value' AS lbl FROM r
    WHERE j->'subject'->>'type' = 'bnode'
  UNION ALL
  SELECT j->'object'->>'value' FROM r
    WHERE j->'object'->>'type' = 'bnode'
)
SELECT count(DISTINCT lbl)::bigint AS c_distinct_labels
  FROM labels;

-- Cardinality of bnode positions seen = 6 (3 per solution × 2 solutions).
WITH r AS (
  SELECT * FROM pgrdf.construct(
    'CONSTRUCT { _:r <http://example.com/type> <http://example.com/Card> . '
    '            _:r <http://example.com/value> ?v . '
    '            <http://example.com/owner> <http://example.com/owns> _:r } '
    'WHERE { ?s <http://example.com/has> ?v }') AS t(j)
)
SELECT (
  (SELECT count(*) FROM r WHERE j->'subject'->>'type' = 'bnode')
  +
  (SELECT count(*) FROM r WHERE j->'object'->>'type' = 'bnode')
)::bigint AS c_total_bnode_positions;

-- Within-solution sameness — group the 6 rows by the bnode label
-- (the per-solution fresh label) and verify each group has exactly
-- 3 rows (3 template-triple positions per solution). Locks
-- within-solution cross-triple joining without depending on label
-- strings or per-solution variable bindings.
--
-- Build a union of (label) cells across the 3 bnode positions per
-- solution, then GROUP BY label. Each label MUST appear in 3 rows
-- (subject of type-triple + subject of value-triple + object of
-- owns-triple). The aggregate `min(count) = max(count) = 3` locks
-- the within-solution grouping.
WITH r AS (
  SELECT * FROM pgrdf.construct(
    'CONSTRUCT { _:r <http://example.com/type> <http://example.com/Card> . '
    '            _:r <http://example.com/value> ?v . '
    '            <http://example.com/owner> <http://example.com/owns> _:r } '
    'WHERE { ?s <http://example.com/has> ?v }') AS t(j)
), labels AS (
  SELECT j->'subject'->>'value' AS lbl FROM r
    WHERE j->'subject'->>'type' = 'bnode'
  UNION ALL
  SELECT j->'object'->>'value' FROM r
    WHERE j->'object'->>'type' = 'bnode'
), grouped AS (
  SELECT lbl, count(*)::bigint AS n FROM labels GROUP BY lbl
)
SELECT min(n) = 3 AND max(n) = 3 AS c_within_solution_sameness
  FROM grouped;

-- ─── Invariant D: `_:a` vs `_:b` distinct within solution ───────
-- 2-triple template with distinct template labels. 2 solutions × 2
-- triples → 4 rows. All 4 subject bnode labels MUST be distinct —
-- slice 56 does not conflate `_:a` and `_:b`, and labels differ
-- across solutions.
-- D uses its own predicate `ex:dp` to isolate from B/C/E/G seeds.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:d1 ex:dp "d-1" .
   ex:d2 ex:dp "d-2" .',
  30);

SELECT count(*)::bigint AS d_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { _:a <http://example.com/type> <http://example.com/Foo> . '
    '            _:b <http://example.com/type> <http://example.com/Bar> } '
    'WHERE { ?s <http://example.com/dp> ?v }') AS s(j);

SELECT count(DISTINCT j->'subject'->>'value')::bigint AS d_distinct_subj
  FROM pgrdf.construct(
    'CONSTRUCT { _:a <http://example.com/type> <http://example.com/Foo> . '
    '            _:b <http://example.com/type> <http://example.com/Bar> } '
    'WHERE { ?s <http://example.com/dp> ?v }') AS s(j);

-- ─── Invariant E: mixed bnode + variable in 2-triple template ────
-- `?s <ex:tagged> _:tag . _:tag <ex:by> ?s`. 2 solutions × 2 triples
-- → 4 rows. Within each solution, the bnode in triple-0 object and
-- the bnode in triple-1 subject MUST share the same label; across
-- solutions, labels differ.
-- E uses `ex:hop` which is unique to E's seed.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:e1 ex:hop "e-1" .
   ex:e2 ex:hop "e-2" .',
  40);

SELECT count(*)::bigint AS e_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tagged> _:tag . '
    '            _:tag <http://example.com/by> ?s } '
    'WHERE { ?s <http://example.com/hop> ?v }') AS s(j);

-- For each solution-binding ?s, the bnode label in triple-0 object
-- (?s <tagged> _:tag) MUST equal the bnode label in triple-1 subject
-- (_:tag <by> ?s). Pair them via the shared ?s binding (which
-- appears as subject of triple-0 and object of triple-1). Two
-- solutions × 1 match each → 2 pairs; both pairs MUST satisfy
-- s_obj_lbl == s_subj_lbl. Locked via bool_and across the pairs.
WITH r AS (
  SELECT * FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.com/tagged> _:tag . '
    '            _:tag <http://example.com/by> ?s } '
    'WHERE { ?s <http://example.com/hop> ?v }') AS t(j)
), pairs AS (
  SELECT
    t.j->'subject'->>'value' AS s_iri,
    t.j->'object'->>'value'  AS tag_in_obj,
    (SELECT u.j->'subject'->>'value' FROM r u
       WHERE u.j->'predicate'->>'value' = 'http://example.com/by'
         AND u.j->'object'->>'value' = t.j->'subject'->>'value') AS tag_in_subj
  FROM r t
  WHERE t.j->'predicate'->>'value' = 'http://example.com/tagged'
)
SELECT
  bool_and(tag_in_obj = tag_in_subj) AS e_within_solution_sameness,
  count(DISTINCT tag_in_obj)::bigint AS e_distinct_across_solutions
FROM pairs;

-- ─── Invariant F: empty template `{ }` rejection ────────────────
SELECT _check_error(
  'f-rejects-empty-template',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { } WHERE { ?s ?p ?o }')$$,
  'pgrdf.construct: empty template'
);

-- ─── Invariant G: slice-57 single-triple path still works ───────
-- Same-label joining within ONE triple of one solution — locks the
-- single-triple path didn't regress under the slice-56 refactor.
-- G uses `ex:gp` predicate to isolate from earlier sections.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:g1 ex:gp "g-1" .
   ex:g2 ex:gp "g-2" .',
  50);

SELECT bool_and((j->'subject'->>'value') = (j->'object'->>'value')) AS g_within_row_sameness
  FROM pgrdf.construct(
    'CONSTRUCT { _:foo <http://example.com/linksTo> _:foo } '
    'WHERE { ?x <http://example.com/gp> ?y }')
  AS s(j);

SELECT count(DISTINCT j->'subject'->>'value')::bigint AS g_distinct_solutions
  FROM pgrdf.construct(
    'CONSTRUCT { _:foo <http://example.com/linksTo> _:foo } '
    'WHERE { ?x <http://example.com/gp> ?y }')
  AS s(j);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
