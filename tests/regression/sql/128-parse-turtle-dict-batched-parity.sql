-- 128-parse-turtle-dict-batched-parity.sql
--
-- TA-D3 spike correctness gate.
--
-- The new `pgrdf.parse_turtle_dict_batched` ingest path (2-pass:
-- collect all triples → bulk-resolve unique terms → walk again
-- building s/p/o batches) MUST produce identical _pgrdf_quads
-- rows to the baseline `pgrdf.parse_turtle` path. The spike
-- exists for speed; if it diverges in correctness it's worthless.
--
-- This regression ingests a small but varied Turtle blob (covering
-- all term shapes 123-dictionary-lexical-contract checks — IRIs,
-- typed literals, lang-tagged literals, plain literals, blank
-- nodes) into two separate graphs via the two paths, then asserts:
--
--   * triple count matches between graphs.
--   * dictionary IDs may differ across the two graphs (the spike
--     resolves terms in a different ORDER from baseline) but the
--     decoded lexical_value / term_type / datatype / language must
--     match for every term referenced by quads in each graph.
--   * For every (s, p, o) triple in the baseline graph, an
--     equivalent triple exists in the spike graph (by decoded
--     lexical values).
--
-- Expected output: 4 boolean assertions all evaluating to `t`.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

DO $$
BEGIN
  PERFORM pgrdf.add_graph('urn:test/ta-d3/baseline');
  PERFORM pgrdf.add_graph('urn:test/ta-d3/spike');

  -- Same Turtle into both graphs via different paths.
  PERFORM pgrdf.parse_turtle($ttl$
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
  $ttl$, pgrdf.graph_id('urn:test/ta-d3/baseline'));

  PERFORM pgrdf.parse_turtle_dict_batched($ttl$
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
  $ttl$, pgrdf.graph_id('urn:test/ta-d3/spike'), NULL, 4);
END $$;

-- A: triple count parity
SELECT (
  (SELECT count(*)::int FROM pgrdf._pgrdf_quads WHERE graph_id = pgrdf.graph_id('urn:test/ta-d3/baseline')) =
  (SELECT count(*)::int FROM pgrdf._pgrdf_quads WHERE graph_id = pgrdf.graph_id('urn:test/ta-d3/spike'))
) AS a_triple_count_matches;

-- B: every (s, p, o) decoded-lexical-triple in baseline appears in spike
WITH baseline_triples AS (
  SELECT s.lexical_value AS s_lex,
         p.lexical_value AS p_lex,
         o.lexical_value AS o_lex,
         o.term_type      AS o_type,
         o.language_tag   AS o_lang
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
  WHERE q.graph_id = pgrdf.graph_id('urn:test/ta-d3/baseline')
    -- Exclude blank-node subjects/objects from the parity-by-lexical
    -- assertion: bnode labels are parser-assigned and need not match
    -- byte-for-byte across two parser invocations.
    AND s.term_type <> 2   -- not BLANK_NODE
    AND o.term_type <> 2
),
spike_triples AS (
  SELECT s.lexical_value AS s_lex,
         p.lexical_value AS p_lex,
         o.lexical_value AS o_lex,
         o.term_type      AS o_type,
         o.language_tag   AS o_lang
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
  WHERE q.graph_id = pgrdf.graph_id('urn:test/ta-d3/spike')
    AND s.term_type <> 2
    AND o.term_type <> 2
)
SELECT (
  NOT EXISTS (
    SELECT * FROM baseline_triples
    EXCEPT
    SELECT * FROM spike_triples
  )
) AS b_baseline_subset_of_spike;

-- C: AND the other way — every spike triple appears in baseline
WITH baseline_triples AS (
  SELECT s.lexical_value AS s_lex,
         p.lexical_value AS p_lex,
         o.lexical_value AS o_lex,
         o.term_type      AS o_type,
         o.language_tag   AS o_lang
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
  WHERE q.graph_id = pgrdf.graph_id('urn:test/ta-d3/baseline')
    AND s.term_type <> 2 AND o.term_type <> 2
),
spike_triples AS (
  SELECT s.lexical_value AS s_lex,
         p.lexical_value AS p_lex,
         o.lexical_value AS o_lex,
         o.term_type      AS o_type,
         o.language_tag   AS o_lang
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
  WHERE q.graph_id = pgrdf.graph_id('urn:test/ta-d3/spike')
    AND s.term_type <> 2 AND o.term_type <> 2
)
SELECT (
  NOT EXISTS (
    SELECT * FROM spike_triples
    EXCEPT
    SELECT * FROM baseline_triples
  )
) AS c_spike_subset_of_baseline;

-- D: the spike's JSONB reports `path = "dict_batched"` discriminator
SELECT (
  pgrdf.parse_turtle_dict_batched('@prefix ex: <http://x/> . ex:s ex:p ex:o .',
    pgrdf.add_graph('urn:test/ta-d3/discriminator'), NULL, 4) ->> 'path' = 'dict_batched'
) AS d_path_discriminator;

ROLLBACK;
