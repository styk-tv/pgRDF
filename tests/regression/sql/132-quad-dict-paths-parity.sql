-- 132-quad-dict-paths-parity.sql
--
-- TA-6 correctness gate. v0.5.40 routes the quad-stream ingest
-- UDFs `pgrdf.parse_nquads` + `pgrdf.parse_trig` through the
-- `pgrdf.ingest_dict_path` dispatch (baseline / batched /
-- shmem_warm / combined) — the same four-path switch TA-7 added
-- for `parse_turtle`. The combined quad path
-- (`ingest_quads_combined`) threads the shmem hot-cache + defer
-- queue through the per-graph `GraphBatches` routing.
--
-- All four paths MUST produce byte-identical dict rows + quad
-- counts for the same input. This test ingests the same N-Quads
-- blob under each path value into a per-path-distinct default
-- graph (so runs don't stack), plus one named-graph line per run
-- (graph IRI made distinct per run) to exercise the per-graph
-- drain through the combined path. Then it asserts:
--
--   A: quad counts equal across all four runs.
--   B-D: decoded-lexical quads (s,p,o,o_type,o_has_dt,o_lang) for
--        `combined` ≡ each of baseline / batched / shmem_warm.
--   E: parse_trig also routes through the combined path without
--      error and lands the expected quad count.
--
-- Blank-node subjects/objects are excluded from lexical parity
-- per the precedent in 130-ingest-dict-paths-parity.sql.
--
-- Expected output: 5 boolean assertions all evaluating to `t`.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

DO $$
DECLARE
  -- N-Quads template. The first four lines are 3-position (route
  -- to default_graph_id); the fifth is a 4-position line whose
  -- graph IRI carries the %s path tag so named-graph routing is
  -- distinct per run (no cross-run stacking). The s/p/o TERMS are
  -- identical across runs — only the routing differs — so the
  -- dict-term parity comparison is apples-to-apples.
  nq_tmpl text :=
    '<http://ex/s1> <http://ex/p> "Alice" .' || E'\n' ||
    '<http://ex/s2> <http://ex/p> "30"^^<http://www.w3.org/2001/XMLSchema#integer> .' || E'\n' ||
    '<http://ex/s3> <http://ex/p> "Hello"@en .' || E'\n' ||
    '<http://ex/s4> <http://ex/p> <http://ex/o4> .' || E'\n' ||
    '<http://ex/s5> <http://ex/p> "named"^^<http://www.w3.org/2001/XMLSchema#string> <http://ex/g/%s> .';
  paths text[] := ARRAY['baseline','batched','shmem_warm','combined'];
  pth text;
  dgid bigint;
BEGIN
  FOREACH pth IN ARRAY paths LOOP
    EXECUTE format('SET LOCAL pgrdf.ingest_dict_path = %L', pth);
    dgid := pgrdf.add_graph('urn:test/ta-6/default/' || pth);
    PERFORM pgrdf.parse_nquads(format(nq_tmpl, pth), dgid);
  END LOOP;
END $$;

-- A: quad counts equal across all four runs (each run = 5 quads:
-- 4 to the per-path default graph + 1 to its per-path named graph).
SELECT (
  SELECT count(DISTINCT cnt) FROM (
    SELECT
      (SELECT count(*) FROM pgrdf._pgrdf_quads
        WHERE graph_id = pgrdf.graph_id('urn:test/ta-6/default/' || pth)
           OR graph_id = pgrdf.graph_id('http://ex/g/' || pth)) AS cnt
    FROM unnest(ARRAY['baseline','batched','shmem_warm','combined']) AS pth
  ) s
) = 1 AS a_quad_count_parity;

-- Helper view: decoded-lexical quads per run tag, excluding bnodes.
-- A run's quads live in either its default graph or its named graph;
-- the `run` column extracts the path tag from whichever graph IRI
-- the quad landed in.
CREATE TEMP VIEW lexical_quads AS
SELECT
  CASE
    WHEN g.iri LIKE 'urn:test/ta-6/default/%' THEN substring(g.iri from 'default/(.*)$')
    WHEN g.iri LIKE 'http://ex/g/%'           THEN substring(g.iri from 'g/(.*)$')
  END AS run,
  s.lexical_value AS s_lex,
  p.lexical_value AS p_lex,
  o.lexical_value AS o_lex,
  o.term_type     AS o_type,
  o.datatype_iri_id IS NOT NULL AS o_has_dt,
  o.language_tag  AS o_lang
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_graphs g     ON g.graph_id = q.graph_id
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
 WHERE s.term_type <> 2 AND o.term_type <> 2
   AND g.iri ~ '(ta-6/default/|http://ex/g/)';

-- B: combined ≡ baseline (lexical, run tag stripped)
SELECT (
  NOT EXISTS (
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_quads WHERE run = 'combined'
    EXCEPT
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_quads WHERE run = 'baseline'
  )
) AS b_combined_eq_baseline;

-- C: combined ≡ batched
SELECT (
  NOT EXISTS (
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_quads WHERE run = 'combined'
    EXCEPT
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_quads WHERE run = 'batched'
  )
) AS c_combined_eq_batched;

-- D: combined ≡ shmem_warm
SELECT (
  NOT EXISTS (
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_quads WHERE run = 'combined'
    EXCEPT
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_quads WHERE run = 'shmem_warm'
  )
) AS d_combined_eq_shmem_warm;

-- E: parse_trig routes through the combined path too. Two graphs in
-- one TriG document; expect 3 quads total (2 in g1 + 1 default).
SET LOCAL pgrdf.ingest_dict_path = 'combined';
DO $$
BEGIN
  PERFORM pgrdf.add_graph('http://ex/trig/g1');
  PERFORM pgrdf.parse_trig($trig$
    @prefix ex: <http://ex/trig/> .
    ex:a ex:p "default-graph-triple" .
    GRAPH <http://ex/trig/g1> {
      ex:b ex:p "named-1" .
      ex:c ex:p "named-2" .
    }
  $trig$, pgrdf.add_graph('urn:test/ta-6/trig-default'));
END $$;
SELECT (
  (SELECT count(*) FROM pgrdf._pgrdf_quads
    WHERE graph_id = pgrdf.graph_id('urn:test/ta-6/trig-default')) = 1
  AND
  (SELECT count(*) FROM pgrdf._pgrdf_quads
    WHERE graph_id = pgrdf.graph_id('http://ex/trig/g1')) = 2
) AS e_trig_combined_routes;

ROLLBACK;
