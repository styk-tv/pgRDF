-- 35-construct-round-trip / setup.sql
--
-- Phase D slice 51 — CONSTRUCT round-trip at W3C-shape level. The
-- harness runs exactly ONE `query.rq` through `pgrdf.construct`, so
-- it cannot express a multi-statement
-- construct → put_construct_rows → re-query script on its own. We
-- perform the construct + re-ingest leg HERE in setup.sql; `query.rq`
-- then re-queries the destination graph, and `expected.jsonl` is the
-- re-queried destination state. Equivalence to the source graph is
-- the load-bearing assertion: if the round-trip lost a term, dropped
-- a datatype, or mangled a language tag, the re-queried rows would
-- diverge from the hand-computed source triples below.
--
-- Harness limitation (documented per slice-51 scope): a true
-- "assert dst == src in one query" would need a two-query fixture
-- shape the W3C runner does not have. The engine-side bidirectional
-- EXCEPT equivalence + idempotency + bnode-join + typed/lang-literal
-- preservation invariants are fully locked in
-- `tests/regression/sql/106-construct-round-trip.sql` (invariants
-- A–J). This fixture is the conformance-harness cross-check that the
-- construct → put_construct_rows → construct loop preserves graph
-- state end-to-end through the structured-term shaper.
--
-- Source graph (http://example.org/src) seeded contents:
--   ex:a ex:p ex:b                        (IRI object)
--   ex:b ex:q "hello"@en                  (language-tagged literal)
--   ex:c ex:r "7"^^xsd:integer            (typed literal)

SELECT pgrdf.add_graph('http://example.org/src');
SELECT pgrdf.parse_turtle('
@prefix ex:  <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
ex:a ex:p ex:b .
ex:b ex:q "hello"@en .
ex:c ex:r "7"^^xsd:integer .
', pgrdf.graph_id('http://example.org/src'));

-- Round-trip leg: capture the source graph via CONSTRUCT and
-- re-ingest into a fresh destination graph through the slice-53
-- ingest pair. `query.rq` then re-queries the destination.
SELECT pgrdf.add_graph('http://example.org/dst');
SELECT pgrdf.put_construct_rows(
  (SELECT array_agg(j)
     FROM pgrdf.construct(
       'CONSTRUCT { ?s ?p ?o } '
       'WHERE { GRAPH <http://example.org/src> { ?s ?p ?o } }') AS t(j)),
  pgrdf.graph_id('http://example.org/dst'));
