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

-- A graph is a set of triples: each partition of the quad table carries a
-- unique index on (subject, predicate, object), which is what stops a
-- repeated triple from being stored twice (#155).
--
-- This step never rewrites data. The parent table's subject-predicate-object
-- index is dropped (a parent-level unique index would have to include
-- graph_id, and could not exist beside a partition that holds repeats), and
-- each partition gets its own: the unique one where its rows allow it, a
-- plain one on the same key where they do not, so queries keep their plan.
-- A partition left without the unique index is named in a WARNING (PostgreSQL
-- hides a NOTICE raised inside an extension script from the client), listed by
-- duplicate_report(), still refuses further repeats (writes check for the
-- row themselves), and is cured with dedup_graph(graph_id) (#157).
--
-- Cost: one index build per partition, about what the index it replaces
-- cost to build.
CREATE FUNCTION _set_index(part regclass, dedup boolean) RETURNS bigint
LANGUAGE plpgsql AS $fn$
DECLARE
  rel  text := (SELECT c.relname FROM pg_catalog.pg_class c WHERE c.oid = part);
  cols text := CASE WHEN rel = '_pgrdf_quads_default'
                    THEN 'graph_id, subject_id, predicate_id, object_id'
                    ELSE 'subject_id, predicate_id, object_id' END;
  removed bigint := 0;
BEGIN
  -- Dropped only where present, so a caller sees no "does not exist,
  -- skipping" notice for the one that is not there.
  IF pg_catalog.to_regclass(pg_catalog.format('pgrdf.%I', rel || '_spo_uniq')) IS NOT NULL THEN
    EXECUTE pg_catalog.format('DROP INDEX pgrdf.%I', rel || '_spo_uniq');
  END IF;
  IF pg_catalog.to_regclass(pg_catalog.format('pgrdf.%I', rel || '_spo_idx')) IS NOT NULL THEN
    EXECUTE pg_catalog.format('DROP INDEX pgrdf.%I', rel || '_spo_idx');
  END IF;
  BEGIN
    EXECUTE pg_catalog.format('CREATE UNIQUE INDEX %I ON pgrdf.%I (%s) INCLUDE (is_inferred)',
                              rel || '_spo_uniq', rel, cols);
    RETURN 0;
  EXCEPTION WHEN unique_violation THEN
    IF NOT dedup THEN
      EXECUTE pg_catalog.format('CREATE INDEX %I ON pgrdf.%I (%s) INCLUDE (is_inferred)',
                                rel || '_spo_idx', rel, cols);
      RETURN -1;
    END IF;
  END;
  EXECUTE pg_catalog.format(
    'WITH gone AS (
       DELETE FROM pgrdf.%1$I q
        USING (SELECT ctid,
                      pg_catalog.row_number() OVER (PARTITION BY %2$s ORDER BY is_inferred, ctid) AS rn
                 FROM pgrdf.%1$I) r
        WHERE q.ctid = r.ctid AND r.rn > 1
       RETURNING 1)
     SELECT pg_catalog.count(*) FROM gone', rel, cols)
    INTO removed;
  EXECUTE pg_catalog.format('CREATE UNIQUE INDEX %I ON pgrdf.%I (%s) INCLUDE (is_inferred)',
                            rel || '_spo_uniq', rel, cols);
  RETURN removed;
END
$fn$;

DROP INDEX IF EXISTS _pgrdf_idx_spo;

DO $pgrdf_set_index$
DECLARE
  r record;
BEGIN
  FOR r IN SELECT c.oid::regclass AS part, c.relname
             FROM pg_catalog.pg_inherits i
             JOIN pg_catalog.pg_class c ON c.oid = i.inhrelid
            WHERE i.inhparent = 'pgrdf._pgrdf_quads'::regclass
            ORDER BY c.relname
  LOOP
    IF pgrdf._set_index(r.part, false) = -1 THEN
      RAISE WARNING 'pgrdf: % holds repeated triples; it is left as it is, without the unique index. See pgrdf.duplicate_report(); pgrdf.dedup_graph(graph_id) cures it.', r.relname;
    END IF;
  END LOOP;
END
$pgrdf_set_index$;

CREATE  FUNCTION "duplicate_report"() RETURNS TABLE (
	"graph_id" bigint,  /* Option < i64 > */
	"iri" TEXT,  /* Option < String > */
	"rows" bigint,  /* i64 */
	"distinct_rows" bigint,  /* i64 */
	"surplus" bigint,  /* i64 */
	"groups" bigint,  /* i64 */
	"max_multiplicity" bigint,  /* i64 */
	"set_enforced" bool,  /* Option < bool > */
	"locked" bool,  /* Option < bool > */
	"locked_digest_engine" TEXT,  /* Option < String > */
	"note" TEXT  /* String */
)
STRICT STABLE
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'duplicate_report_wrapper';

CREATE  FUNCTION "dedup_graph"(
	"graph_id" bigint /* i64 */
) RETURNS bigint /* i64 */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'dedup_graph_wrapper';
-- Whole-graph reads in bounded memory: export_graph, graph_manifest,
-- graph_digest and structural_digest no longer hold the graph in backend
-- memory. _nt_line formats one stored triple as one line; PostgreSQL sorts
-- the lines (on disk past work_mem) and the functions read them through a
-- cursor. It refuses a term containing U+0000 with 22021, so no such term
-- is exported or digested; nul_term_report() lists the stored terms that
-- contain one and the graphs using them.
CREATE  FUNCTION "_nt_line"(
	"style" INT, /* i32 */
	"graph_id" bigint, /* i64 */
	"s_id" bigint, /* i64 */
	"s_type" smallint, /* i16 */
	"s_value" TEXT, /* & str */
	"p_id" bigint, /* i64 */
	"p_value" TEXT, /* & str */
	"o_id" bigint, /* i64 */
	"o_type" smallint, /* i16 */
	"o_value" TEXT, /* & str */
	"o_datatype" TEXT, /* Option < & str > */
	"o_language" TEXT /* Option < & str > */
) RETURNS TEXT /* Option < String > */
IMMUTABLE PARALLEL SAFE 
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'nt_line_wrapper';

CREATE  FUNCTION "nul_term_report"() RETURNS TABLE (
	"term_id" bigint,  /* i64 */
	"term_type" smallint,  /* i16 */
	"preview" TEXT,  /* String */
	"graph_ids" bigint[]  /* Vec < i64 > */
)
STRICT STABLE
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'nul_term_report_wrapper';

-- #158: graph_digest is a function of the SET of triples. A locked graph's
-- digest is cached in locked_digest; locked_digest_engine records the
-- engine version that computed it, NULL for a cache written before 0.6.41.
-- Existing caches are left untouched: a locked graph keeps answering the
-- digest recorded for it until it is unlocked.
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS locked_digest_engine TEXT;
