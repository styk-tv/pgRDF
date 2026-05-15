-- 78-sparql-graph-literal-iri.sql
--
-- Phase A slice 114 — SPARQL `GRAPH <iri> { … }` (literal-IRI form)
-- translation. The executor's pattern walk now handles
-- `GraphPattern::Graph { NamedNode(iri), inner }` by resolving the
-- IRI to a `graph_id` via `_pgrdf_graphs.iri` at translate time and
-- adding `qN.graph_id = <id>` to every triple alias inside the
-- GRAPH block. Unresolved IRIs bind to `-1` — a sentinel id no
-- real partition uses — which is the same trick the constant-term
-- path uses for unknown dictionary entries: produces zero rows,
-- spec-correct "no solutions".
--
-- This file locks the per-graph scoping invariants:
--
--   1. `GRAPH <g1>` returns ONLY triples in g1; never bleeds in g2.
--   2. `GRAPH <g2>` returns ONLY triples in g2; symmetric to (1).
--   3. A bare BGP outside any GRAPH continues to scan every
--      partition — the pre-existing v0.3 semantics are preserved.
--   4. Unresolved-IRI `GRAPH <nonexistent>` returns zero rows
--      without raising an error (spec-correct "no solutions"
--      rather than translate-time panic).
--   5. `pgrdf.sparql_parse` no longer flags literal-IRI GRAPH as
--      `unsupported_algebra` — the parser walks `inner` so its BGP
--      triples are still counted, but the slice now drops the
--      "Graph (named graph clause)" tag.
--
-- Variable form `GRAPH ?g { … }` remains 🚧 until slice 113 —
-- locked separately in `tests/regression/sql/80-unsupported-shapes.sql`.
-- Composition with OPTIONAL / UNION / MINUS that span different
-- GRAPH scopes is slice 112.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- Two named graphs with different triples.
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

-- ─── Per-graph scoping ───────────────────────────────────────────
-- GRAPH <iri> { … } returns ONLY triples in that graph.
SELECT bool_and((s.sparql->>'name') = 'Alice in g1') AS only_g1_in_g1
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?name WHERE { GRAPH <http://example.org/g1> { ?s ex:name ?name } }'
) AS s(sparql);

SELECT bool_and((s.sparql->>'name') = 'Bob in g2') AS only_g2_in_g2
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?name WHERE { GRAPH <http://example.org/g2> { ?s ex:name ?name } }'
) AS s(sparql);

-- ─── No-graph-scope preservation ─────────────────────────────────
-- Without GRAPH, both triples surface (existing v0.3 behaviour).
SELECT count(*) AS total_without_graph FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?name WHERE { ?s ex:name ?name }'
);

-- ─── Unresolved-IRI zero-rows semantics ──────────────────────────
-- An IRI not bound in _pgrdf_graphs resolves to the sentinel -1;
-- no partition has graph_id = -1, so the result is empty — no
-- error raised. Spec-correct "no solutions".
SELECT count(*) AS unresolved_iri_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?name WHERE { GRAPH <http://example.org/nonexistent> { ?s ex:name ?name } }'
);

-- ─── sparql_parse stops flagging literal-IRI GRAPH ───────────────
-- The parser's `unsupported_algebra` list no longer carries the
-- "Graph (named graph clause)" tag for the literal-IRI form.
SELECT jsonb_array_length(
  COALESCE(pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/>
     SELECT ?name WHERE { GRAPH <http://example.org/g1> { ?s ex:name ?name } }'
  )->'unsupported_algebra', '[]'::jsonb)
) AS unsupported_count;

-- Cleanup so the next regression file starts from a clean slate.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
