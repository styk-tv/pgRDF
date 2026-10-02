-- pgRDF 0.6.36 -> 0.6.37 — SPEC.pgRDF.v0.6.37: query custody under the
-- pinned join order, set-semantics paths, graph DDL authority.
--
-- Everything here is additive, plus one signature change:
--
-- * graph_inventory() gains source_sha256, source_loads and
--   identity_digest. A RETURNS TABLE change cannot be CREATE OR
--   REPLACEd, so the function is dropped and recreated; its callers
--   (SELECT ... FROM pgrdf.graph_inventory()) are unaffected, and
--   nothing in the extension depends on it.
-- * _pgrdf_graphs gains locked_digest: a locked graph's cached rdfc-1.0
--   digest, filled lazily by graph_digest and cleared by lock/unlock.
-- * _refuse_locked_write() is the trigger function lock_graph installs
--   on a locked graph's partition (#142). Graphs locked BEFORE this
--   upgrade get their triggers here, so a lock taken on 0.6.36 refuses
--   direct SQL on 0.6.37 exactly like a new one.
-- * New exports: can_create_graphs(), can_drop_graphs(), create_graph(),
--   graph_diff(), graph_diff_summary(); internal _path_walk().
--
-- Behaviour changes carried by the .so (no DDL):
-- * creating/dropping a graph is authorised by SELECT+INSERT / SELECT+
--   DELETE on _pgrdf_quads and _pgrdf_graphs, not by ownership (#137)
-- * + and * property paths walk breadth-first from the bound end; depth
--   is shortest distance, truncation exact; new GUC pgrdf.path_max_pairs
--   (#138)
-- * OPTIONAL/MINUS groups chain their joins; OPTIONAL is fenced, so the
--   SQL sparql_sql() shows for OPTIONAL changes (#139)
-- * blank nodes in query patterns are hidden variables; rdf:rest*/
--   rdf:first lists execute; gated paths refuse 0A000 (#140)
-- * server-path loaders require pg_read_server_files; the staged lane is
--   owner-only and its workers run as the caller
--
-- No data migration. graph_ids, partitions and consumer schemas survive.

ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS locked_digest TEXT;

CREATE FUNCTION _refuse_locked_write() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
  RAISE EXCEPTION USING
    ERRCODE = '55P03',
    MESSAGE = format(
      'pgrdf: graph %s is locked (%s): direct SQL %s refused. Unlock with pgrdf.unlock_graph(%s, ''<reason>'').',
      TG_ARGV[0], TG_ARGV[1], TG_OP, TG_ARGV[0]),
    HINT = format('pgrdf.unlock_graph(%s, ''<reason>'')', TG_ARGV[0]);
END
$fn$;

-- graph_inventory: signature change (three new columns).
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
	"identity_digest" TEXT  /* Option < String > */
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_inventory_wrapper';

CREATE  FUNCTION "can_create_graphs"() RETURNS bool /* bool */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'can_create_graphs_wrapper';

CREATE  FUNCTION "can_drop_graphs"() RETURNS bool /* bool */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'can_drop_graphs_wrapper';

CREATE  FUNCTION "create_graph"(
	"iri" TEXT /* & str */
) RETURNS bigint /* i64 */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'create_graph_wrapper';

CREATE  FUNCTION "graph_diff"(
	"a" bigint, /* i64 */
	"b" bigint, /* i64 */
	"side" TEXT DEFAULT NULL, /* Option < & str > */
	"predicate" TEXT DEFAULT NULL, /* Option < & str > */
	"class" TEXT DEFAULT NULL /* Option < & str > */
) RETURNS TABLE (
	"side" TEXT,  /* String */
	"subject" TEXT,  /* String */
	"predicate" TEXT,  /* String */
	"object" TEXT,  /* String */
	"component" TEXT  /* Option < String > */
)
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_diff_wrapper';

CREATE  FUNCTION "graph_diff_summary"(
	"a" bigint, /* i64 */
	"b" bigint /* i64 */
) RETURNS jsonb /* pgrx :: JsonB */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_diff_summary_wrapper';

CREATE  FUNCTION "_path_walk"(
	"preds" bigint[], /* Vec < i64 > */
	"graph_id" bigint, /* Option < i64 > */
	"seed" bigint, /* Option < i64 > */
	"follow_inverse" bool, /* bool */
	"max_depth" INT, /* i32 */
	"path_id" INT, /* i32 */
	"per_graph" bool /* bool */
) RETURNS TABLE (
	"start" bigint,  /* i64 */
	"reached" bigint,  /* i64 */
	"gid" bigint  /* Option < i64 > */
)

LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'path_walk_wrapper';

-- Graphs already locked on 0.6.36: install the refuse-triggers now, so
-- lock custody holds below the engine for them too (#142).
DO $pgrdf_0637$
DECLARE g record;
BEGIN
  FOR g IN
    SELECT gr.graph_id, gr.lock_reason
      FROM _pgrdf_graphs gr
     WHERE gr.locked
       AND EXISTS (SELECT 1 FROM pg_class c
                    WHERE c.relnamespace = 'pgrdf'::regnamespace
                      AND c.relname = format('_pgrdf_quads_g%s', gr.graph_id))
  LOOP
    EXECUTE format(
      'CREATE TRIGGER pgrdf_lock_row BEFORE INSERT OR UPDATE OR DELETE ON pgrdf.%I '
      'FOR EACH ROW EXECUTE FUNCTION pgrdf._refuse_locked_write(%L, %L)',
      format('_pgrdf_quads_g%s', g.graph_id), g.graph_id::text, coalesce(g.lock_reason, ''));
    EXECUTE format(
      'CREATE TRIGGER pgrdf_lock_truncate BEFORE TRUNCATE ON pgrdf.%I '
      'FOR EACH STATEMENT EXECUTE FUNCTION pgrdf._refuse_locked_write(%L, %L)',
      format('_pgrdf_quads_g%s', g.graph_id), g.graph_id::text, coalesce(g.lock_reason, ''));
  END LOOP;
END
$pgrdf_0637$;
