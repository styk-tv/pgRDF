-- 104-construct-graph-scoped-where.sql
--
-- Phase D slice 55 — `pgrdf.construct` GRAPH-scoped WHERE patterns.
-- Slices 59 → 56 widened the template surface (constant → variable
-- → blank-node → multi-triple). Slice 55 widens the WHERE side: the
-- pattern can now wrap its BGP in `GRAPH <iri> { … }` or
-- `GRAPH ?g { … }`, scoping the solutions to a named graph and
-- (in the variable form) projecting the source graph IRI into the
-- template via `?g`.
--
-- Two SQL paths land in `build_from_and_where` (executor.rs):
--
--   * Literal-IRI GRAPH (`GRAPH <iri>`): the BGP triples carry a
--     `q{N}.graph_id = $K` constraint resolved at translate time
--     via `lookup_graph_id`. Constraint piggybacks on the construct
--     path's existing variable resolution — no new column.
--   * Variable GRAPH (`GRAPH ?g`): the BGP triples carry a
--     `g{S}.graph_id = q{anchor}.graph_id` constraint via the
--     `_pgrdf_graphs g{S}` JOIN. The graph IRI is projected as
--     `g{S}.iri AS "g"` (TEXT, not a dict id — graph IRIs are NOT
--     entered in `_pgrdf_dictionary`); the construct row-iteration
--     layer reads it as a String and emits an IRI term directly via
--     `encode_iri_term`. This is the slice-55 bug-fix: the earlier
--     scalar-subselect rewrite resolved through the dictionary,
--     returned NULL for every named-graph row (graph IRIs aren't
--     dict-stored), and the unbound-check dropped the entire
--     template triple.
--
-- W3C SPARQL 1.1 §13.3: `GRAPH ?g { … }` ranges over the NAMED
-- graphs only — the default graph (graph_id = 0) never binds `?g`.
-- Slice 55 enforces this via `AND g{S}.graph_id <> 0` on the
-- `_pgrdf_graphs` JOIN's ON clause. This also corrects the
-- pre-existing SELECT-side bleed (see slice-55 changelog entry).
--
-- Invariants locked by this file:
--
--   A. Literal-GRAPH WHERE: `GRAPH <g1>` returns only g1 subjects.
--      Default + g2 quads excluded.
--   B. Variable-GRAPH WHERE: `?g` binds per-solution; 4 rows total
--      (2 from g1 + 2 from g2 in a seed with 4 named + 1 default
--      quads). Each row's `?g` IRI matches the source graph.
--   C. Skipped (FROM <iri> dataset clauses out of scope).
--   D. Multi-triple template + GRAPH-scoped WHERE: 2N rows per
--      solution × N template triples; within-solution `?g`
--      consistency across emitted triples.
--   E. Blank-node template + GRAPH-scoped WHERE: bnode label
--      shared across within-solution rows, distinct across
--      solutions; `?g` IRI is correct per solution.
--   F. Default-graph excluded from `GRAPH ?g`: seed only default-
--      graph quads, run a variable-GRAPH CONSTRUCT, expect 0 rows.
--   G. Empty named graph: a graph created via `add_graph` with no
--      quads inserted yields 0 solutions, hence 0 emitted rows.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Seed: 2 quads in g1, 2 in g2, 1 in default. Same predicate so the
-- BGP `?s ?p ?o` matches all five seed quads — the GRAPH wrapper is
-- the only filter at play.
SELECT pgrdf.add_graph('http://example.org/g1');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:alice ex:p "v1-g1" .
   ex:bob   ex:p "v2-g1" .',
  pgrdf.graph_id('http://example.org/g1')
);
SELECT pgrdf.add_graph('http://example.org/g2');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:carol ex:p "v1-g2" .
   ex:dave  ex:p "v2-g2" .',
  pgrdf.graph_id('http://example.org/g2')
);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:def ex:p "v-def" .',
  0
);

-- ─── Invariant A: literal-GRAPH WHERE — scope filters ──────────────
-- `GRAPH <g1>` matches only g1 quads → 2 solutions → 2 emitted rows.
SELECT count(*)::bigint AS a_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/tag> "from-g1" } '
    'WHERE { GRAPH <http://example.org/g1> { ?s ?p ?o } }');

-- Every row's subject is one of {alice, bob} (g1's subjects only).
-- A default- or g2-subject in the output would mean GRAPH didn't
-- scope the solutions.
SELECT count(*)::bigint AS a_g1_subjects
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/tag> "from-g1" } '
    'WHERE { GRAPH <http://example.org/g1> { ?s ?p ?o } }') AS s(j)
  WHERE j->'subject'->>'value'
        IN ('http://example.org/alice', 'http://example.org/bob');

-- Negative-shape: no g2 or default-graph subjects bleed through.
SELECT count(*)::bigint AS a_g1_bleed
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/tag> "from-g1" } '
    'WHERE { GRAPH <http://example.org/g1> { ?s ?p ?o } }') AS s(j)
  WHERE j->'subject'->>'value'
        IN ('http://example.org/carol',
            'http://example.org/dave',
            'http://example.org/def');

-- ─── Invariant B: variable-GRAPH WHERE — IRI flows in ──────────────
-- `GRAPH ?g` matches all named-graph quads (4 of them); ?g binds to
-- the source graph IRI per solution; default-graph quads excluded.
SELECT count(*)::bigint AS b_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/from_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }');

-- The object column carries the source graph IRI as an IRI term.
-- Every row's object value is one of {g1, g2}; default-graph (urn:pgrdf:graph:0)
-- MUST NOT appear (W3C §13.3 — variable-GRAPH ranges over named
-- graphs only).
SELECT count(*)::bigint AS b_named_objects
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/from_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS s(j)
  WHERE j->'object'->>'value'
        IN ('http://example.org/g1', 'http://example.org/g2');

-- Per-graph cardinality: g1 contributes 2 rows; g2 contributes 2.
SELECT count(*)::bigint AS b_g1_rows
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/from_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS s(j)
  WHERE j->'object'->>'value' = 'http://example.org/g1';

SELECT count(*)::bigint AS b_g2_rows
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/from_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS s(j)
  WHERE j->'object'->>'value' = 'http://example.org/g2';

-- Subjects pair correctly with their source graph (no cross-graph
-- contamination). g1 subjects MUST pair with g1 IRI; same for g2.
SELECT bool_and(
  ((j->'subject'->>'value') IN ('http://example.org/alice', 'http://example.org/bob')
    AND (j->'object'->>'value') = 'http://example.org/g1')
  OR
  ((j->'subject'->>'value') IN ('http://example.org/carol', 'http://example.org/dave')
    AND (j->'object'->>'value') = 'http://example.org/g2')
) AS b_pairing_correct
FROM pgrdf.construct(
  'CONSTRUCT { ?s <http://example.org/from_graph> ?g } '
  'WHERE { GRAPH ?g { ?s ?p ?o } }') AS s(j);

-- The `?g`-bound object emits as an IRI term (LLD §6.1 shape).
SELECT bool_and(j->'object'->>'type' = 'iri') AS b_g_is_iri
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/from_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS s(j);

-- ─── Invariant D: multi-triple template + GRAPH-scoped WHERE ──────
-- 2-triple template × 4 named-graph solutions → 8 emitted rows.
-- Within each solution, both emitted rows MUST agree on the
-- ?g binding (no cross-row drift).
SELECT count(*)::bigint AS d_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.org/export> <http://example.org/contains> ?s . '
    '            ?s <http://example.org/source_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }');

-- Each template-triple emits exactly 4 rows (one per solution).
SELECT count(*)::bigint AS d_contains_rows
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.org/export> <http://example.org/contains> ?s . '
    '            ?s <http://example.org/source_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS s(j)
  WHERE j->'predicate'->>'value' = 'http://example.org/contains';

SELECT count(*)::bigint AS d_source_rows
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.org/export> <http://example.org/contains> ?s . '
    '            ?s <http://example.org/source_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS s(j)
  WHERE j->'predicate'->>'value' = 'http://example.org/source_graph';

-- Within-solution consistency: for every subject ?s, the source_graph
-- row's object MUST equal the per-solution ?g binding the contains
-- row carries. We pair them via ?s and verify the source IRI is g1
-- for alice/bob and g2 for carol/dave (no cross-graph drift).
WITH r AS (
  SELECT * FROM pgrdf.construct(
    'CONSTRUCT { <http://example.org/export> <http://example.org/contains> ?s . '
    '            ?s <http://example.org/source_graph> ?g } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS t(j)
), pairs AS (
  SELECT
    (j->'object'->>'value') AS subj_iri,
    (SELECT u.j->'object'->>'value' FROM r u
       WHERE (u.j->'predicate'->>'value') = 'http://example.org/source_graph'
         AND (u.j->'subject'->>'value') = (r.j->'object'->>'value')) AS source_iri
  FROM r
  WHERE (j->'predicate'->>'value') = 'http://example.org/contains'
)
SELECT bool_and(
  (subj_iri IN ('http://example.org/alice', 'http://example.org/bob')
    AND source_iri = 'http://example.org/g1')
  OR
  (subj_iri IN ('http://example.org/carol', 'http://example.org/dave')
    AND source_iri = 'http://example.org/g2')
) AS d_within_solution_consistency
FROM pairs;

-- ─── Invariant E: blank-node template + GRAPH-scoped WHERE ────────
-- `_:bn <ex:source> ?g . _:bn <ex:contains> ?s` over `GRAPH ?g { ?s ?p ?o }`.
-- 4 named-graph solutions × 2 template triples → 8 rows.
-- Per-solution `_:bn` label is shared across the 2 emitted rows;
-- across solutions the labels differ.
SELECT count(*)::bigint AS e_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { _:bn <http://example.org/source> ?g . '
    '            _:bn <http://example.org/contains> ?s } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }');

-- The set of distinct bnode labels seen across all 8 rows is exactly
-- 4 — one per solution. Within each solution both emitted bnode
-- positions share a label.
SELECT count(DISTINCT j->'subject'->>'value')::bigint AS e_distinct_bn_labels
  FROM pgrdf.construct(
    'CONSTRUCT { _:bn <http://example.org/source> ?g . '
    '            _:bn <http://example.org/contains> ?s } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS s(j);

-- For each bnode label, both emitted rows carry the SAME ?g binding
-- (within-solution sameness) — locked via min == max per-label group.
-- And the source IRI must be one of g1/g2 (named graphs only).
WITH r AS (
  SELECT * FROM pgrdf.construct(
    'CONSTRUCT { _:bn <http://example.org/source> ?g . '
    '            _:bn <http://example.org/contains> ?s } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }') AS t(j)
), source_rows AS (
  SELECT (j->'subject'->>'value') AS bn_label,
         (j->'object'->>'value')  AS source_iri
  FROM r
  WHERE (j->'predicate'->>'value') = 'http://example.org/source'
), contains_rows AS (
  SELECT (j->'subject'->>'value') AS bn_label,
         (j->'object'->>'value')  AS subj_iri
  FROM r
  WHERE (j->'predicate'->>'value') = 'http://example.org/contains'
), paired AS (
  SELECT s.bn_label, s.source_iri, c.subj_iri
  FROM source_rows s
  JOIN contains_rows c ON c.bn_label = s.bn_label
)
SELECT bool_and(
  (subj_iri IN ('http://example.org/alice', 'http://example.org/bob')
    AND source_iri = 'http://example.org/g1')
  OR
  (subj_iri IN ('http://example.org/carol', 'http://example.org/dave')
    AND source_iri = 'http://example.org/g2')
) AS e_pair_consistency,
count(*)::bigint AS e_pair_count
FROM paired;

-- ─── Invariant F: default-graph excluded from `GRAPH ?g` ──────────
-- Seed ONLY default-graph quads (drop the prior seed first), run a
-- variable-GRAPH CONSTRUCT — must yield 0 rows.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:def1 ex:p "v1-def" .
   ex:def2 ex:p "v2-def" .',
  0
);

SELECT count(*)::bigint AS f_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/tag> "x" } '
    'WHERE { GRAPH ?g { ?s ?p ?o } }');

-- ─── Invariant G: empty named graph yields zero solutions ─────────
-- `add_graph` registers the IRI but inserts no quads → 0 rows.
SELECT pgrdf.add_graph('http://example.org/empty');

SELECT count(*)::bigint AS g_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { ?s <http://example.org/tag> "x" } '
    'WHERE { GRAPH <http://example.org/empty> { ?s ?p ?o } }');

-- Cleanup so the next regression file starts from a clean slate.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
