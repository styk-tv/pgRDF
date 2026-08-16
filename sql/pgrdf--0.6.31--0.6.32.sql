-- pgRDF 0.6.31 -> 0.6.32 — the truth cut (#117, #109, #123).
--
-- #117: pgrdf.graph_digest(graph_id) — RDFC-1.0 canonical graph digest.
-- Identity of MEANING that survives reload: canonical blank-node
-- relabelling (W3C RDFC-1.0), canonical N-Triples, sha256, over the
-- graph's ASSERTED triples only. Algorithm label: rdfc-1.0-sha256 —
-- values are NOT comparable with byte digests or first-degree
-- structural pins, by design. Conformance proven against the W3C
-- rdf-canon suite byte-for-byte; adversarial (poison) structures RAISE
-- pgRDF#117 rather than degrade.
--
-- #109 and #123 ship in the .so as behaviour, not DDL: a VALUES binding
-- a graph variable now refuses (pgRDF#109) instead of silently
-- answering over every graph; the staged loader refuses in transaction
-- blocks (pgRDF#123) instead of hanging uncancellably, and load_turtle's
-- auto-dispatch falls back to the standard parser there.

CREATE  FUNCTION "graph_digest"(
	"graph_id" bigint /* i64 */
) RETURNS TEXT /* String */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_digest_wrapper';
