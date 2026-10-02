-- pgRDF 0.6.37 -> 0.6.38 — clearing a graph as a non-owner (#145).
--
-- Additive only:
--
-- * New export: can_clear_graphs(), true when the current role holds
--   SELECT and DELETE on _pgrdf_quads (the rule clear_graph enforces).
--
-- Behaviour changes carried by the .so (no DDL):
-- * clear_graph (both overloads, and SPARQL CLEAR GRAPH) is authorised by
--   SELECT+DELETE on _pgrdf_quads; the partition TRUNCATE runs as the
--   storage owner under the restricted switch drop_graph uses. Locked
--   graphs still refuse 55P03 first.
-- * the empty-database fast paths of load_turtle(bulk_load => true) and
--   load_turtle_streaming take the standard path, with a NOTICE, for a
--   role that does not own _pgrdf_quads.

CREATE  FUNCTION "can_clear_graphs"() RETURNS bool /* bool */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'can_clear_graphs_wrapper';
