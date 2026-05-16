-- 50-trig-graph-scoped / setup.sql
--
-- Phase G group G2 (SPEC.pgRDF.LLD.v0.5-FUTURE §4) —
-- `pgrdf.parse_trig` ingest observed through the SPARQL GRAPH
-- surface. A TriG document with two inline `GRAPH <iri> { … }`
-- blocks loads into two pgRDF graphs (auto-allocated via the v0.4
-- §3.2 IRI map); the query then SELECTs per-graph with `GRAPH ?g`,
-- proving the inline TriG graph scoping is observable end-to-end at
-- the query surface.
--
-- setup.sql-only — the harness default `add_graph + parse_turtle`
-- path is single-graph and Turtle-only; parse_trig's JSONB stats
-- return is wrapped so no `{...}` row leaks into the solution-row
-- filter.

SELECT 'ingested' AS step FROM (SELECT pgrdf.parse_trig('
@prefix ex: <http://example.org/> .
GRAPH <http://example.org/g1> {
  ex:alice ex:name "Alice" .
}
GRAPH <http://example.org/g2> {
  ex:bob ex:name "Bob" .
  ex:carol ex:name "Carol" .
}
')) _t;
