-- 119-parse-nquads.sql
--
-- Phase G group G2 (slice 17, SPEC.pgRDF.LLD.v0.5-FUTURE §4) —
-- `pgrdf.parse_nquads(content TEXT, default_graph_id BIGINT DEFAULT
-- 0, strict BOOLEAN DEFAULT FALSE) → JSONB`.
--
-- N-Quads is the 4-position line format: a 4th-position graph IRI
-- routes the quad to that graph (resolved via the v0.4 §3.2 IRI
-- mapping — `pgrdf.graph_id(iri)` if bound, else
-- `pgrdf.add_graph(iri)` auto-allocates by default). A 3-position
-- line (no 4th term) falls to `default_graph_id`. Under `strict =>
-- TRUE` an unknown graph IRI is REJECTED with the stable prefix
-- `parse_nquads: unknown graph iri <iri>` (no auto-allocate, no
-- partial ingest — resolution happens before the quad is buffered).
-- Reuses the v0.3 batched-insert path, partition-routed per resolved
-- graph_id; verbose JSONB stats mirror parse_turtle_verbose plus a
-- `graphs` array.
--
-- Invariants (all expected values hand-computed; never ACCEPT=1):
--
--   A. 4-position parse + default-graph fallback. Two quads to a
--      named graph, one 3-position line to default_graph_id.
--   B. Unknown 4th-position IRI auto-allocates by default; the new
--      graph is bound + reachable via pgrdf.graph_id.
--   C. strict => TRUE rejects an unknown IRI with the stable prefix
--      AND leaves no partial rows (transactional: nothing for that
--      call persisted).
--   D. Typed + language-tagged literals round-trip into the dict.
--   E. Batched-insert stats shape: triples / quad_batches / graphs.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

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

-- A. 4-position parse + 3-position default-graph fallback.
-- Two quads to <http://nq/g1> (auto-allocated), one 3-position line
-- to default_graph_id 1190.
SELECT (j->>'triples')::int AS a_triples,
       (j->>'quad_batches')::int AS a_batches
  FROM pgrdf.parse_nquads(
'<http://nq/a> <http://nq/p> "x" <http://nq/g1> .
<http://nq/b> <http://nq/p> "y" <http://nq/g1> .
<http://nq/c> <http://nq/p> "z" .',
    1190) AS j;

-- The two named-graph quads landed in g1; the 3-position line in
-- default_graph_id 1190.
SELECT pgrdf.count_quads(pgrdf.graph_id('http://nq/g1'))::int AS a_in_g1;
SELECT pgrdf.count_quads(1190)::int AS a_in_default;

-- B. The unknown IRI auto-allocated and is now bound + reachable.
SELECT (pgrdf.graph_id('http://nq/g1') IS NOT NULL) AS b_g1_bound;

-- C. strict => TRUE rejects an unknown IRI with the stable prefix
-- and leaves no partial rows. Use a SAVEPOINT so the rejection's
-- statement rollback doesn't poison the surrounding transaction;
-- assert the never-seen graph IRI was NOT bound (no partial ingest).
SELECT _check_error(
  'c_strict_reject',
  $$SELECT pgrdf.parse_nquads(
      '<http://nq/s> <http://nq/p> "v" <http://nq/never> .', 0, TRUE)$$,
  'parse_nquads: unknown graph iri http://nq/never');
SELECT (pgrdf.graph_id('http://nq/never') IS NULL) AS c_no_partial_bind;

-- D. Typed + language-tagged literals round-trip into the dict.
SELECT (j->>'triples')::int AS d_triples
  FROM pgrdf.parse_nquads(
'<http://nq/n> <http://nq/age> "42"^^<http://www.w3.org/2001/XMLSchema#integer> <http://nq/g1> .
<http://nq/n> <http://nq/label> "hi"@en <http://nq/g1> .',
    0) AS j;
SELECT (EXISTS (SELECT 1 FROM pgrdf._pgrdf_dictionary
                 WHERE term_type = 1
                   AND lexical_value = 'http://www.w3.org/2001/XMLSchema#integer'))
  AS d_xsd_integer_interned;
SELECT (EXISTS (SELECT 1 FROM pgrdf._pgrdf_dictionary
                 WHERE term_type = 3 AND lexical_value = 'hi' AND language_tag = 'en'))
  AS d_lang_literal_interned;

-- E. Batched-insert stats shape: a single named graph → exactly one
-- flush batch; the `graphs` array carries the resolved id.
SELECT (j->>'quad_batches')::int AS e_one_batch,
       jsonb_array_length(j->'graphs') AS e_graphs_len
  FROM pgrdf.parse_nquads(
'<http://nq/x> <http://nq/p> "1" <http://nq/g1> .
<http://nq/y> <http://nq/p> "2" <http://nq/g1> .',
    0) AS j;

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

ROLLBACK;
