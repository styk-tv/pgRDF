-- pgRDF 0.6.27 -> 0.6.28 — lock custody moves into the engine (#107).
--
-- Before this release the checkpoint "lock" lived in pgrdf_mcp.ledger and
-- only the MCP door consulted it: SQL clear_graph emptied a LOCKED graph
-- and the door then refused the repair. The lock now lives in the engine
-- and every SQL write path refuses on it.
--
-- SCOPE, stated where the DDL lands: this is a COORDINATION primitive,
-- not a security boundary — anyone who can write the graph can lock or
-- unlock it, with a mandatory reason both ways. Security remains grants.

ALTER TABLE pgrdf._pgrdf_graphs ADD COLUMN IF NOT EXISTS locked      BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE pgrdf._pgrdf_graphs ADD COLUMN IF NOT EXISTS lock_reason TEXT;
ALTER TABLE pgrdf._pgrdf_graphs ADD COLUMN IF NOT EXISTS locked_at   TIMESTAMPTZ;

CREATE FUNCTION "lock_graph"("graph_id" bigint, "reason" TEXT) RETURNS bool
STRICT
LANGUAGE c
AS 'MODULE_PATHNAME', 'lock_graph_wrapper';

CREATE FUNCTION "unlock_graph"("graph_id" bigint, "reason" TEXT) RETURNS bool
STRICT
LANGUAGE c
AS 'MODULE_PATHNAME', 'unlock_graph_wrapper';
