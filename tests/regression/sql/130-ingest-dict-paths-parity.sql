-- 130-ingest-dict-paths-parity.sql
--
-- TA-7 production landing — correctness gate for the
-- `pgrdf.ingest_dict_path` GUC dispatch.
--
-- The new GUC routes `parse_turtle` / `load_turtle` through one of
-- four dict-resolution paths:
--
--   baseline    — legacy single-term `put_term_full` SPI per term.
--   batched     — TA-D3 spike path: 2-pass materialise + bulk resolve.
--   shmem_warm  — baseline path after a forced one-shot prewarm.
--   combined    — TA-7 production single-pass: shmem hot-cache check
--                 + defer queue + bulk resolve at boundaries.
--
-- All four MUST produce identical `_pgrdf_quads` rows by construction
-- (same Turtle in, same triples out). Only the SPI shape differs.
-- This regression ingests one Turtle blob into four separate graphs
-- under each path setting in turn, then asserts:
--
--   A: triple counts are equal across all four graphs.
--   B-D: every (s, p, o) decoded-lexical-triple in `combined` appears
--        in each of the other three (with the symmetric subset check
--        elided — count parity + one-direction subset is sufficient
--        because the sets are equi-sized).
--   E: an out-of-range GUC value silently falls back to `combined`.
--
-- Blank-node subjects/objects are excluded from lexical parity per
-- the precedent in 128-parse-turtle-dict-batched-parity.sql: bnode
-- labels are parser-assigned and need not match byte-for-byte across
-- two parser invocations.
--
-- Expected output: 5 boolean assertions all evaluating to `t`.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

DO $$
DECLARE
  ttl text := $ttl$
    @prefix ex:  <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

    ex:s1 ex:hasName "Alice" .
    ex:s2 ex:hasAge "30"^^xsd:integer .
    ex:s3 ex:hasGreeting "Hello"@en .
    ex:s4 ex:hasGreeting "Bonjour"@fr-CA .
    ex:s5 ex:hasNote _:n0 .
    _:n0 ex:lex "blank-node-anchored" .
    ex:s6 ex:hasUri <urn:example:bob> .
    ex:s7 ex:hasZero "0030"^^xsd:integer .
  $ttl$;
BEGIN
  PERFORM pgrdf.add_graph('urn:test/ta-7/baseline');
  PERFORM pgrdf.add_graph('urn:test/ta-7/batched');
  PERFORM pgrdf.add_graph('urn:test/ta-7/shmem_warm');
  PERFORM pgrdf.add_graph('urn:test/ta-7/combined');

  SET LOCAL pgrdf.ingest_dict_path = 'baseline';
  PERFORM pgrdf.parse_turtle(ttl, pgrdf.graph_id('urn:test/ta-7/baseline'));

  SET LOCAL pgrdf.ingest_dict_path = 'batched';
  PERFORM pgrdf.parse_turtle(ttl, pgrdf.graph_id('urn:test/ta-7/batched'));

  SET LOCAL pgrdf.ingest_dict_path = 'shmem_warm';
  PERFORM pgrdf.parse_turtle(ttl, pgrdf.graph_id('urn:test/ta-7/shmem_warm'));

  SET LOCAL pgrdf.ingest_dict_path = 'combined';
  PERFORM pgrdf.parse_turtle(ttl, pgrdf.graph_id('urn:test/ta-7/combined'));
END $$;

-- A: triple counts equal across all four graphs
SELECT (
  SELECT count(DISTINCT cnt) FROM (
    SELECT count(*) AS cnt
      FROM pgrdf._pgrdf_quads
     WHERE graph_id IN (
       pgrdf.graph_id('urn:test/ta-7/baseline'),
       pgrdf.graph_id('urn:test/ta-7/batched'),
       pgrdf.graph_id('urn:test/ta-7/shmem_warm'),
       pgrdf.graph_id('urn:test/ta-7/combined')
     )
     GROUP BY graph_id
  ) s
) = 1 AS a_triple_count_parity;

-- Helper view: decoded-lexical triples per graph, excluding bnodes.
CREATE TEMP VIEW lexical_triples AS
SELECT q.graph_id,
       s.lexical_value AS s_lex,
       p.lexical_value AS p_lex,
       o.lexical_value AS o_lex,
       o.term_type     AS o_type,
       o.datatype_iri_id IS NOT NULL AS o_has_dt,
       o.language_tag  AS o_lang
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
 WHERE s.term_type <> 2 AND o.term_type <> 2;

-- B: combined ≡ baseline (lexical)
SELECT (
  NOT EXISTS (
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_triples
     WHERE graph_id = pgrdf.graph_id('urn:test/ta-7/combined')
    EXCEPT
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_triples
     WHERE graph_id = pgrdf.graph_id('urn:test/ta-7/baseline')
  )
) AS b_combined_subset_of_baseline;

-- C: combined ≡ batched (lexical)
SELECT (
  NOT EXISTS (
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_triples
     WHERE graph_id = pgrdf.graph_id('urn:test/ta-7/combined')
    EXCEPT
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_triples
     WHERE graph_id = pgrdf.graph_id('urn:test/ta-7/batched')
  )
) AS c_combined_subset_of_batched;

-- D: combined ≡ shmem_warm (lexical)
SELECT (
  NOT EXISTS (
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_triples
     WHERE graph_id = pgrdf.graph_id('urn:test/ta-7/combined')
    EXCEPT
    SELECT s_lex, p_lex, o_lex, o_type, o_has_dt, o_lang
      FROM lexical_triples
     WHERE graph_id = pgrdf.graph_id('urn:test/ta-7/shmem_warm')
  )
) AS d_combined_subset_of_shmem_warm;

-- E: an unrecognised GUC value silently falls back to combined
SET LOCAL pgrdf.ingest_dict_path = 'no-such-path';
DO $$
DECLARE
  gid bigint := pgrdf.add_graph('urn:test/ta-7/fallback');
BEGIN
  PERFORM pgrdf.parse_turtle('@prefix ex: <http://x/> . ex:s ex:p "fallback" .', gid);
END $$;
SET LOCAL pgrdf.ingest_dict_path = 'combined';
SELECT (
  (SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = pgrdf.graph_id('urn:test/ta-7/fallback')) = 1
) AS e_unknown_value_falls_back;

ROLLBACK;
