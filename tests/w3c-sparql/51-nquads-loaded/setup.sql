-- 51-nquads-loaded / setup.sql
--
-- Phase G group G2 (SPEC.pgRDF.LLD.v0.5-FUTURE §4) —
-- `pgrdf.parse_nquads` ingest observed through the SPARQL GRAPH
-- surface. N-Quads is the 4-position line format; the fourth
-- position is the graph IRI, resolved through the v0.4 §3.2 IRI map
-- (auto-allocated when unbound). The query then reads one of those
-- graphs back with `GRAPH <iri>`, proving the per-line N-Quads graph
-- assignment is observable end-to-end.
--
-- setup.sql-only — the harness default path is single-graph
-- Turtle-only; parse_nquads's JSONB stats wrapped so no `{...}` row
-- leaks into the solution-row filter.

SELECT 'ingested' AS step FROM (SELECT pgrdf.parse_nquads(
'<http://example.org/alice> <http://example.org/name> "Alice" <http://example.org/people> .
<http://example.org/bob> <http://example.org/name> "Bob" <http://example.org/people> .
<http://example.org/widget> <http://example.org/kind> "gadget" <http://example.org/things> .
')) _n;
