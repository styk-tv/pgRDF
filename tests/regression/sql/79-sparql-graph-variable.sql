-- 79-sparql-graph-variable.sql
--
-- Phase A slice 113 — SPARQL `GRAPH ?g { … }` (variable form)
-- translation. The executor's pattern walk now handles
-- `GraphPattern::Graph { Variable(?g), inner }` by recording the
-- variable name in `ParsedSelect.graph_var` (or `UnionBranch.graph_var`)
-- and threading it into `build_from_and_where`, which:
--
--   1. JOINs `pgrdf._pgrdf_graphs g0 ON g0.graph_id = q1.graph_id`
--      (exactly one JOIN per inner BGP) — INNER matches W3C SPARQL
--      1.1 §13.3: only graphs present in the IRI mapping bind ?g.
--   2. Constrains every additional triple alias inside the GRAPH
--      block (qN, N≥2) to `q1.graph_id` so a multi-triple inner BGP
--      cannot stitch triples from different graphs together.
--   3. Projects `?g` from `g0.iri` (NOT the integer id) — the JSONB
--      row value is the IRI string.
--
-- The parser's `unsupported_algebra` list no longer carries the
-- "Graph (variable IRI; slice 113)" tag — slice 113 walks `inner`
-- like the literal-IRI form.
--
-- Composition with OPTIONAL / UNION / MINUS that span DIFFERENT
-- GRAPH scopes is slice 112; today the entire single-branch BGP
-- shares one constraint.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- Two named graphs, each with one triple.
SELECT pgrdf.add_graph('http://example.org/g1');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:alice ex:name "Alice in g1" .',
  pgrdf.graph_id('http://example.org/g1')
);
SELECT pgrdf.add_graph('http://example.org/g2');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:bob ex:name "Bob in g2" .',
  pgrdf.graph_id('http://example.org/g2')
);

-- ─── Per-row IRI projection ──────────────────────────────────────
-- `SELECT ?g ?name WHERE { GRAPH ?g { ?s ex:name ?name } }` returns
-- 2 rows; ?g is the IRI string from `_pgrdf_graphs.iri`, not the
-- integer id.
SELECT count(*) AS two_rows FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?name WHERE { GRAPH ?g { ?s ex:name ?name } }'
);

-- The graph IRI projects as TEXT (the IRI string) — bool_and over
-- both expected pairings confirms binding shape.
SELECT bool_and(
  ((s.sparql->>'g') = 'http://example.org/g1' AND (s.sparql->>'name') = 'Alice in g1')
  OR
  ((s.sparql->>'g') = 'http://example.org/g2' AND (s.sparql->>'name') = 'Bob in g2')
) AS rows_well_paired
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?name WHERE { GRAPH ?g { ?s ex:name ?name } }'
) AS s(sparql);

-- ─── COUNT + GROUP BY ?g ─────────────────────────────────────────
-- Aggregates over a graph-bound ?g group by IRI. Two graphs, one
-- triple each ⇒ two rows of (?g, 1).
SELECT count(*) AS distinct_graph_groups FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ex:name ?name } } GROUP BY ?g'
);

-- Every group has exactly the expected per-graph count (1 here).
SELECT bool_and((s.sparql->>'n')::int = 1) AS each_group_count_one
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ex:name ?name } } GROUP BY ?g'
) AS s(sparql);

-- ─── Multi-triple inner BGP: shared-graph constraint ─────────────
-- Inside the GRAPH block, every triple anchor must share the same
-- graph_id. Add a second triple to each graph so a multi-triple
-- BGP can actually match — and verify it never accidentally joins
-- a subject in g1 with a name in g2.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:alice ex:age "30" .',
  pgrdf.graph_id('http://example.org/g1')
);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:bob ex:age "25" .',
  pgrdf.graph_id('http://example.org/g2')
);

-- `GRAPH ?g { ?s ex:name ?n . ?s ex:age ?a }` — both triples MUST
-- come from the same graph. Alice has name in g1 and age in g1 ⇒ 1
-- row for g1; Bob symmetric in g2 ⇒ 1 row for g2. Total = 2. If the
-- shared-graph constraint were missing, the multi-graph subject
-- shape would surface cross-graph stitches.
SELECT count(*) AS multi_triple_same_graph_rows FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?n ?a WHERE { GRAPH ?g { ?s ex:name ?n . ?s ex:age ?a } }'
);

-- Cross-graph pollution check: a row tying Alice''s name to Bob''s
-- age (or vice versa) would mean we accidentally joined ?s across
-- partitions. Confirm no such row exists.
SELECT count(*) AS cross_graph_rows FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?n ?a WHERE { GRAPH ?g { ?s ex:name ?n . ?s ex:age ?a } }'
) AS s(sparql)
WHERE
  ((s.sparql->>'g') = 'http://example.org/g1' AND (s.sparql->>'n') = 'Bob in g2')
  OR
  ((s.sparql->>'g') = 'http://example.org/g2' AND (s.sparql->>'n') = 'Alice in g1');

-- ─── sparql_parse stops flagging variable-IRI GRAPH ──────────────
-- The parser's `unsupported_algebra` list no longer carries the
-- "Graph (variable IRI; slice 113)" tag — like the literal-IRI form,
-- it walks `inner` so the contained BGP triples are still counted.
SELECT jsonb_array_length(
  COALESCE(pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/>
     SELECT ?g ?name WHERE { GRAPH ?g { ?s ex:name ?name } }'
  )->'unsupported_algebra', '[]'::jsonb)
) AS unsupported_count;

-- Cleanup so the next regression file starts from a clean slate.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
