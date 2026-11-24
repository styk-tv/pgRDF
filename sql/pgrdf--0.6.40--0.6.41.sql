-- pgRDF 0.6.40 -> 0.6.41
--
-- No data is rewritten by this script. graph_ids, partitions and consumer
-- schemas survive.

-- Every upgrade script repeats this alignment (0.6.40, #153): an upgrade
-- must leave the engine's own allocator usable by the storage owner.
DO $pgrdf_153$
DECLARE
  storage_owner oid := (SELECT relowner FROM pg_catalog.pg_class
                         WHERE oid = 'pgrdf._pgrdf_quads'::regclass);
BEGIN
  IF (SELECT relowner FROM pg_catalog.pg_class
       WHERE oid = 'pgrdf._pgrdf_graph_id_seq'::regclass) <> storage_owner THEN
    EXECUTE pg_catalog.format('ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO %I',
                              pg_catalog.pg_get_userbyid(storage_owner));
  END IF;
END
$pgrdf_153$;

-- #161: functions that act on state shared by every database on the server
-- are not PUBLIC. stats() is opened with GRANT EXECUTE ON FUNCTION
-- pgrdf.stats() TO <role>; last_call_stats() stays PUBLIC.
REVOKE EXECUTE ON FUNCTION shmem_reset() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION shmem_cache_prewarm(bigint) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION stats() FROM PUBLIC;

-- #152: the registry is dumped without its seed row (graph_id 0). A restore
-- creates that row itself, and the dumped copy made the registry COPY fail,
-- which lost every graph's registry row. Calling the function again replaces
-- the filter.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graphs', 'WHERE graph_id <> 0');

-- #152: the default partition's rows (the default graph, graph_id 0, and any
-- graph id without its own partition) were not dumped at all.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_quads_default', '');

-- #152: the dictionary's rows were dumped but its id allocator was not, so a
-- restored database handed out dictionary ids from 1 again and the first
-- load of a new term failed 23505 on the primary key.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_dictionary_id_seq', '');

-- Graph ownership and the act record. A graph created from now on records
-- the role that created it (owner, a role name) and the act that created
-- it; graphs that exist today keep owner NULL: "not recorded", never a
-- guess. _pgrdf_graph_acts is the act record: one row per committed
-- graph-level act, written by the engine as the storage owner. Its two
-- triggers refuse every direct write unless the writer holds the table
-- owner's privileges, so a role granted on ALL TABLES cannot forge rows.
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS owner TEXT;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS created_act BIGINT;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS locked_act BIGINT;

CREATE SEQUENCE _pgrdf_graph_act_id_seq MINVALUE 1 START 1;
CREATE TABLE _pgrdf_graph_acts (
    act_id           BIGINT PRIMARY KEY,
    graph_id         BIGINT NOT NULL,
    graph_iri        TEXT,
    act              TEXT NOT NULL,
    at               TIMESTAMPTZ NOT NULL,
    xact             xid8 NOT NULL,
    session_role     TEXT NOT NULL,
    acting_role      TEXT NOT NULL,
    application_name TEXT,
    requester        TEXT,
    workspace        TEXT,
    agent            TEXT,
    thread           TEXT,
    on_behalf_of     TEXT,
    harness          TEXT,
    detail           JSONB
);
CREATE INDEX _pgrdf_graph_acts_graph_idx ON _pgrdf_graph_acts (graph_id, act_id);
GRANT SELECT ON _pgrdf_graph_acts TO PUBLIC;
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graph_acts', '');
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graph_act_id_seq', '');

CREATE FUNCTION _refuse_act_write() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
  IF pg_catalog.pg_has_role(
       current_user,
       (SELECT relowner FROM pg_catalog.pg_class WHERE oid = TG_RELID),
       'USAGE') THEN
    IF TG_LEVEL = 'STATEMENT' THEN RETURN NULL; END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
  END IF;
  RAISE EXCEPTION USING
    ERRCODE = '42501',
    MESSAGE = pg_catalog.format(
      'pgrdf: the act record is written by the engine: direct %s refused', TG_OP),
    HINT = 'Read it with pgrdf.graph_acts().';
END
$fn$;
CREATE TRIGGER pgrdf_acts_row BEFORE INSERT OR UPDATE OR DELETE ON _pgrdf_graph_acts
  FOR EACH ROW EXECUTE FUNCTION _refuse_act_write();
CREATE TRIGGER pgrdf_acts_truncate BEFORE TRUNCATE ON _pgrdf_graph_acts
  FOR EACH STATEMENT EXECUTE FUNCTION _refuse_act_write();

-- This script runs as a superuser; the engine writes the act record as the
-- storage owner (the owner of _pgrdf_quads). Hand both new relations to that
-- role, the way the graph-id sequence is aligned above.
DO $pgrdf_acts$
DECLARE
  storage_owner name := (SELECT pg_catalog.pg_get_userbyid(relowner) FROM pg_catalog.pg_class
                          WHERE oid = 'pgrdf._pgrdf_quads'::regclass);
BEGIN
  EXECUTE pg_catalog.format('ALTER TABLE pgrdf._pgrdf_graph_acts OWNER TO %I', storage_owner);
  EXECUTE pg_catalog.format('ALTER SEQUENCE pgrdf._pgrdf_graph_act_id_seq OWNER TO %I', storage_owner);
END
$pgrdf_acts$;

CREATE FUNCTION graph_acts(graph_id BIGINT DEFAULT NULL, since TIMESTAMPTZ DEFAULT NULL)
RETURNS SETOF _pgrdf_graph_acts
LANGUAGE sql STABLE
AS $fn$
  SELECT a.* FROM pgrdf._pgrdf_graph_acts a
   WHERE ($1 IS NULL OR a.graph_id = $1)
     AND ($2 IS NULL OR a.at >= $2)
   ORDER BY a.act_id
$fn$;

CREATE  FUNCTION "graph_activity"(
	"graph_id" bigint DEFAULT NULL /* Option < i64 > */
) RETURNS TABLE (
	"pid" INT,  /* i32 */
	"graph_id" bigint,  /* Option < i64 > */
	"act" TEXT,  /* Option < String > */
	"started" timestamp with time zone,  /* Option < pgrx :: datum :: TimestampWithTimeZone > */
	"part" TEXT,  /* Option < String > */
	"session_role" TEXT,  /* Option < String > */
	"acting_role" TEXT,  /* Option < String > */
	"application_name" TEXT,  /* Option < String > */
	"requester" TEXT,  /* Option < String > */
	"workspace" TEXT,  /* Option < String > */
	"agent" TEXT,  /* Option < String > */
	"thread" TEXT,  /* Option < String > */
	"on_behalf_of" TEXT,  /* Option < String > */
	"harness" TEXT  /* Option < String > */
)
STABLE
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_activity_wrapper';

CREATE  FUNCTION "set_graph_owner"(
	"graph_id" bigint, /* i64 */
	"role" TEXT /* & str */
) RETURNS TEXT /* String */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'set_graph_owner_wrapper';

-- graph_inventory: signature change (owner, created_at, who created the
-- graph, who holds the lock). A RETURNS TABLE change cannot be CREATE OR
-- REPLACEd; nothing in the extension depends on the function.
DROP FUNCTION "graph_inventory"();
CREATE  FUNCTION "graph_inventory"() RETURNS TABLE (
	"graph_id" bigint,  /* i64 */
	"iri" TEXT,  /* Option < String > */
	"asserted" bigint,  /* i64 */
	"inferred" bigint,  /* i64 */
	"locked" bool,  /* bool */
	"lock_reason" TEXT,  /* Option < String > */
	"materialization" TEXT,  /* String */
	"source_sha256" TEXT,  /* Option < String > */
	"source_loads" INT,  /* Option < i32 > */
	"identity_digest" TEXT,  /* Option < String > */
	"owner" TEXT,  /* Option < String > */
	"created_at" timestamp with time zone,  /* Option < pgrx :: datum :: TimestampWithTimeZone > */
	"created_workspace" TEXT,  /* Option < String > */
	"created_agent" TEXT,  /* Option < String > */
	"created_thread" TEXT,  /* Option < String > */
	"locked_by" TEXT  /* Option < String > */
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_inventory_wrapper';
