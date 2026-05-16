-- 120-parse-trig.sql
--
-- Phase G group G2 (slice 16, SPEC.pgRDF.LLD.v0.5-FUTURE §4) —
-- `pgrdf.parse_trig(content TEXT, default_graph_id BIGINT DEFAULT 0,
-- strict BOOLEAN DEFAULT FALSE) → JSONB`.
--
-- TriG = Turtle + inline `GRAPH <iri> { … }` blocks. Triples OUTSIDE
-- any GRAPH block land in `default_graph_id`; each `<iri>` graph
-- resolves via the v0.4 §3.2 IRI mapping (bound → its id; unbound →
-- `pgrdf.add_graph(iri)` auto-allocate by default, or REJECT under
-- `strict => TRUE` with `parse_trig: unknown graph iri <iri>`).
-- Reuses the v0.3 batched-insert path, partition-routed per resolved
-- graph_id.
--
-- v0.5 §4 acceptance criteria (binding) exercised here:
--
--   #1  A TriG doc declaring THREE inline named graphs loads into
--       three pgRDF graphs in a SINGLE call (per-graph quad count +
--       binding asserted).
--   #2  Unknown graph IRIs auto-allocate by default; under strict
--       they REJECT with the prefix and leave NO partial ingest.
--   #3  Round-trip: parse_trig(doc) then CONSTRUCT-of-each-graph
--       re-serialised back produces an isomorphic document. Realised
--       here as quad-set isomorphism PER GRAPH: `pgrdf.construct` of
--       each graph's `{ ?s ?p ?o }` reproduces exactly that graph's
--       triple set (same count + same (s,p,o) cells), which is the
--       spec's intent (no full-TriG re-serialiser UDF in v0.5).
--
-- Extra invariants: default-graph triples → default_graph_id; blank
-- nodes inside a graph block ingest correctly.
--
-- All expected values hand-computed; never ACCEPT=1.

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

-- #1 — three inline named graphs + a default-graph triple, all in
-- ONE call. g/1 = 2 quads, g/2 = 1, g/3 = 3, default = 1. Total 7.
SELECT (j->>'triples')::int AS one_triples,
       jsonb_array_length(j->'graphs') AS one_graphs_len
  FROM pgrdf.parse_trig(
'@prefix ex: <http://example.com/> .
 ex:dft ex:p "d" .
 GRAPH <http://example.com/g/1> { ex:a ex:p "1" . ex:a2 ex:p "1b" }
 GRAPH <http://example.com/g/2> { ex:b ex:p "2" }
 GRAPH <http://example.com/g/3> { ex:c ex:p "3" . ex:c2 ex:p "3b" . ex:c3 ex:p "3c" }',
    1200) AS j;

-- Per-graph quad counts + bindings (#1).
SELECT pgrdf.count_quads(pgrdf.graph_id('http://example.com/g/1'))::int AS one_g1;
SELECT pgrdf.count_quads(pgrdf.graph_id('http://example.com/g/2'))::int AS one_g2;
SELECT pgrdf.count_quads(pgrdf.graph_id('http://example.com/g/3'))::int AS one_g3;
-- The GRAPH-less triple landed in default_graph_id 1200.
SELECT pgrdf.count_quads(1200)::int AS one_default;

-- #3 — round-trip / quad-set isomorphism per graph. CONSTRUCT each
-- graph's triple set back out; the count must equal that graph's
-- ingested quad count, and the (s,p,o) cells must round-trip.
SELECT count(*)::int AS rt_g1_count
  FROM pgrdf.construct(
    'PREFIX ex: <http://example.com/>
     CONSTRUCT { ?s ?p ?o } WHERE { GRAPH <http://example.com/g/1> { ?s ?p ?o } }');
SELECT count(*)::int AS rt_g3_count
  FROM pgrdf.construct(
    'PREFIX ex: <http://example.com/>
     CONSTRUCT { ?s ?p ?o } WHERE { GRAPH <http://example.com/g/3> { ?s ?p ?o } }');
-- Spot-check a specific (s,p,o) cell survives the round-trip for g/2.
SELECT (row->'subject'->>'value') AS rt_g2_subject,
       (row->'object'->>'value')  AS rt_g2_object
  FROM pgrdf.construct(
    'PREFIX ex: <http://example.com/>
     CONSTRUCT { ?s ?p ?o } WHERE { GRAPH <http://example.com/g/2> { ?s ?p ?o } }')
  AS c(row);

-- #2 — unknown IRI auto-allocates by default (no strict).
SELECT (j->>'triples')::int AS two_auto_triples
  FROM pgrdf.parse_trig(
'GRAPH <http://example.com/auto> { <http://ex/s> <http://ex/p> "v" }', 0) AS j;
SELECT (pgrdf.graph_id('http://example.com/auto') IS NOT NULL) AS two_auto_bound;

-- #2 — strict => TRUE REJECTS an unknown inline GRAPH IRI with the
-- stable prefix AND leaves no partial ingest (the never-seen IRI is
-- not bound afterwards).
SELECT _check_error(
  'two_strict_reject',
  $$SELECT pgrdf.parse_trig(
      'GRAPH <http://example.com/nope> { <http://ex/s> <http://ex/p> "v" }', 0, TRUE)$$,
  'parse_trig: unknown graph iri http://example.com/nope');
SELECT (pgrdf.graph_id('http://example.com/nope') IS NULL) AS two_no_partial;

-- Blank nodes inside a graph block ingest (the bnode is a distinct
-- term in the dict; one quad lands in the auto-allocated graph).
SELECT (j->>'triples')::int AS bnode_triples
  FROM pgrdf.parse_trig(
'@prefix ex: <http://example.com/> .
 GRAPH <http://example.com/bn> { _:x ex:p "bv" }', 0) AS j;
SELECT pgrdf.count_quads(pgrdf.graph_id('http://example.com/bn'))::int AS bnode_in_graph;

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

ROLLBACK;
