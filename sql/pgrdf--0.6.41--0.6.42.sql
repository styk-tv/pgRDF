-- pgRDF 0.6.41 -> 0.6.42
--
-- No triple is rewritten by this script. graph_ids, partitions and consumer
-- schemas survive. It counts every graph and the dictionary once, under a
-- lock that keeps writers out until it commits.

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

-- #171, #178: the engine keeps a count per graph and one for the
-- dictionary. The counts are seeded below from one pass over the quad
-- table and one over the dictionary. A write that committed after a pass
-- read its table and before this script commits would be in no count, so
-- writers wait from here to the end of the upgrade. Readers do not.
--
-- Run the upgrade with writers quiet. The two locks are taken one after
-- the other, and no order suits every writer: a load takes the dictionary
-- before the quad table, materialize and a SPARQL DELETE ... INSERT take
-- them the other way round. A writer caught between the two can make the
-- upgrade fail with a deadlock (40P01). The upgrade is one transaction, so
-- nothing is left half-done: run it again.
LOCK TABLE _pgrdf_dictionary, _pgrdf_quads IN SHARE MODE;

-- The trigger functions the count tables stand on, and the one rule that
-- says which keys a count is kept for.
CREATE FUNCTION "_count_open"()
	RETURNS trigger
	LANGUAGE c
	AS 'MODULE_PATHNAME', '_count_open_wrapper';
CREATE  FUNCTION "_count_kept"(
	"graph_id" bigint /* i64 */
) RETURNS bool /* bool */
STABLE STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', '_count_kept_wrapper';
CREATE FUNCTION "_count_bypass"()
	RETURNS TRIGGER
	LANGUAGE c
	AS 'MODULE_PATHNAME', '_count_bypass_wrapper';

CREATE FUNCTION "_count_delta_guard"()
	RETURNS TRIGGER
	LANGUAGE c
	AS 'MODULE_PATHNAME', '_count_delta_guard_wrapper';

CREATE TABLE _pgrdf_graph_counts (
    graph_id     BIGINT PRIMARY KEY,
    asserted     BIGINT NOT NULL DEFAULT 0,
    inferred     BIGINT NOT NULL DEFAULT 0,
    change_seq   BIGINT NOT NULL DEFAULT 0,
    scanned_at   TIMESTAMPTZ,
    scan_snapshot pg_snapshot,
    scan_xact    xid8,
    scan_mark    BIGINT NOT NULL DEFAULT 0,
    folded_below xid8 NOT NULL DEFAULT '0'
) WITH (fillfactor = 50);

CREATE TABLE _pgrdf_graph_count_deltas (
    graph_id   BIGINT NOT NULL,
    xact       xid8 NOT NULL DEFAULT pg_catalog.pg_current_xact_id(),
    d_asserted BIGINT NOT NULL,
    d_inferred BIGINT NOT NULL
);
CREATE INDEX _pgrdf_graph_count_deltas_idx
    ON _pgrdf_graph_count_deltas (graph_id, xact);

-- A write to the quad table that did not come through the engine: one
-- row. graph_id NULL means "some graph": the statement named the parent.
CREATE SEQUENCE _pgrdf_graph_count_bypass_seq;
CREATE TABLE _pgrdf_graph_count_bypass (
    id       BIGINT NOT NULL DEFAULT pg_catalog.nextval('_pgrdf_graph_count_bypass_seq'),
    graph_id BIGINT,
    xact     xid8 NOT NULL DEFAULT pg_catalog.pg_current_xact_id()
);
CREATE INDEX _pgrdf_graph_count_bypass_idx
    ON _pgrdf_graph_count_bypass (graph_id, xact);
-- The sequence belongs to its column, so it follows the table when a
-- consumer hands the storage tables to another role.
ALTER SEQUENCE _pgrdf_graph_count_bypass_seq OWNED BY _pgrdf_graph_count_bypass.id;

REVOKE ALL ON _pgrdf_graph_counts, _pgrdf_graph_count_deltas,
              _pgrdf_graph_count_bypass FROM PUBLIC;
REVOKE ALL ON SEQUENCE _pgrdf_graph_count_bypass_seq FROM PUBLIC;
-- The delta rides in the engine's own statement, as the caller. The
-- guard trigger is what keeps a direct INSERT out.
GRANT INSERT ON _pgrdf_graph_count_deltas TO PUBLIC;

CREATE TRIGGER pgrdf_count_delta_guard BEFORE INSERT ON _pgrdf_graph_count_deltas
    FOR EACH ROW EXECUTE FUNCTION _count_delta_guard();

-- Everything else on the count tables is the engine's, which writes them
-- as the storage owner. A role that was granted the tables wholesale
-- (GRANT ... ON ALL TABLES IN SCHEMA pgrdf) could otherwise set a count,
-- or delete the mark that says a count is unverified.
CREATE FUNCTION _refuse_count_write() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
  IF pg_catalog.pg_has_role(
       current_user,
       (SELECT relowner FROM pg_catalog.pg_class WHERE oid = TG_RELID),
       'USAGE') THEN
    RETURN NULL;
  END IF;
  RAISE EXCEPTION USING
    ERRCODE = '42501',
    MESSAGE = pg_catalog.format(
      'pgrdf: graph counts are written by the engine: direct %s refused', TG_OP),
    HINT = 'Read them with pgrdf.graph_inventory(); repair one with pgrdf.recount_graph(graph_id).';
END
$fn$;
-- Per statement: the answer depends on the role and the table, never on a
-- row, and a fold or a recount retires many rows in one statement.
CREATE TRIGGER pgrdf_counts_guard
    BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON _pgrdf_graph_counts
    FOR EACH STATEMENT EXECUTE FUNCTION _refuse_count_write();
CREATE TRIGGER pgrdf_counts_guard
    BEFORE UPDATE OR DELETE OR TRUNCATE ON _pgrdf_graph_count_deltas
    FOR EACH STATEMENT EXECUTE FUNCTION _refuse_count_write();
CREATE TRIGGER pgrdf_counts_guard
    BEFORE INSERT OR UPDATE OR DELETE OR TRUNCATE ON _pgrdf_graph_count_bypass
    FOR EACH STATEMENT EXECUTE FUNCTION _refuse_count_write();

CREATE TRIGGER pgrdf_count_bypass
    BEFORE INSERT OR UPDATE OR DELETE ON _pgrdf_quads
    FOR EACH STATEMENT EXECUTE FUNCTION _count_bypass();
CREATE TRIGGER pgrdf_count_bypass_truncate
    BEFORE TRUNCATE ON _pgrdf_quads
    FOR EACH STATEMENT EXECUTE FUNCTION _count_bypass();
CREATE TRIGGER pgrdf_count_bypass
    BEFORE INSERT OR UPDATE OR DELETE ON _pgrdf_quads_default
    FOR EACH STATEMENT EXECUTE FUNCTION _count_bypass('0');
CREATE TRIGGER pgrdf_count_bypass_truncate
    BEFORE TRUNCATE ON _pgrdf_quads_default
    FOR EACH STATEMENT EXECUTE FUNCTION _count_bypass('0');
-- Every registered graph has a count row. The engine sets it when it
-- creates a graph; a registry row that arrives any other way, as all of
-- them do when a restore fills the registry, opens it here, not set by any
-- scan.
CREATE TRIGGER pgrdf_count_open
    AFTER INSERT ON _pgrdf_graphs
    FOR EACH ROW EXECUTE FUNCTION _count_open();
-- #178: the dictionary's count is kept under the key -1. A term added or
-- removed in plain SQL marks it unverified; an UPDATE cannot change how
-- many terms there are.
CREATE TRIGGER pgrdf_count_bypass
    BEFORE INSERT OR DELETE ON _pgrdf_dictionary
    FOR EACH STATEMENT EXECUTE FUNCTION _count_bypass('-1');
CREATE TRIGGER pgrdf_count_bypass_truncate
    BEFORE TRUNCATE ON _pgrdf_dictionary
    FOR EACH STATEMENT EXECUTE FUNCTION _count_bypass('-1');
-- The count tables belong to the storage owner, like every storage table:
-- the engine reads and writes them as that role. The bypass log's sequence
-- is owned by its column and moves with its table.
DO $pgrdf_counts_owner$
DECLARE
  storage_owner name := (SELECT pg_catalog.pg_get_userbyid(relowner) FROM pg_catalog.pg_class
                          WHERE oid = 'pgrdf._pgrdf_quads'::regclass);
BEGIN
  EXECUTE pg_catalog.format('ALTER TABLE pgrdf._pgrdf_graph_counts OWNER TO %I', storage_owner);
  EXECUTE pg_catalog.format('ALTER TABLE pgrdf._pgrdf_graph_count_deltas OWNER TO %I', storage_owner);
  EXECUTE pg_catalog.format('ALTER TABLE pgrdf._pgrdf_graph_count_bypass OWNER TO %I', storage_owner);
END
$pgrdf_counts_owner$;

-- Every existing graph partition gets the marker a new one is created
-- with: a write to it that does not come through the engine marks the
-- graph's count unverified. The graph id is read from the partition bound.
DO $pgrdf_count_markers$
DECLARE
  r record;
BEGIN
  FOR r IN SELECT c.relname,
                  (pg_catalog.regexp_match(pg_catalog.pg_get_expr(c.relpartbound, c.oid),
                                           '\(''?(-?[0-9]+)''?\)'))[1] AS graph_id
             FROM pg_catalog.pg_inherits i
             JOIN pg_catalog.pg_class c ON c.oid = i.inhrelid
            WHERE i.inhparent = 'pgrdf._pgrdf_quads'::regclass
              AND c.relname <> '_pgrdf_quads_default'
            ORDER BY c.relname
  LOOP
    CONTINUE WHEN r.graph_id IS NULL;
    EXECUTE pg_catalog.format(
      'CREATE TRIGGER pgrdf_count_bypass BEFORE INSERT OR UPDATE OR DELETE ON pgrdf.%I '
      'FOR EACH STATEMENT EXECUTE FUNCTION pgrdf._count_bypass(%L)', r.relname, r.graph_id);
    EXECUTE pg_catalog.format(
      'CREATE TRIGGER pgrdf_count_bypass_truncate BEFORE TRUNCATE ON pgrdf.%I '
      'FOR EACH STATEMENT EXECUTE FUNCTION pgrdf._count_bypass(%L)', r.relname, r.graph_id);
  END LOOP;
END
$pgrdf_count_markers$;

-- The seed: every graph the engine keeps a count for, counted now. One
-- pass over the quad table, about the cost of one graph_inventory() call
-- of 0.6.41. A registered graph without a partition of its own (a registry
-- row written by hand) gets no count: its rows live in the default
-- partition, no write to it is counted, and it is counted by scan. Nor
-- does a graph registered under a negative id: the keys below 0 are not
-- graphs', and -1 is the dictionary's, seeded just below.
INSERT INTO _pgrdf_graph_counts
       (graph_id, asserted, inferred, scanned_at, scan_snapshot, scan_xact, scan_mark, folded_below)
SELECT g.graph_id, coalesce(q.a, 0), coalesce(q.i, 0), pg_catalog.clock_timestamp(),
       pg_catalog.pg_current_snapshot(), pg_catalog.pg_current_xact_id(),
       pg_catalog.nextval('_pgrdf_graph_count_bypass_seq'),
       pg_catalog.pg_snapshot_xmin(pg_catalog.pg_current_snapshot())
  FROM _pgrdf_graphs g
  LEFT JOIN (SELECT graph_id,
                    count(*) FILTER (WHERE NOT is_inferred) AS a,
                    count(*) FILTER (WHERE is_inferred) AS i
               FROM _pgrdf_quads GROUP BY graph_id) q USING (graph_id)
 WHERE _count_kept(g.graph_id);
-- #178: and the dictionary, counted now: one pass over it.
INSERT INTO _pgrdf_graph_counts
       (graph_id, asserted, scanned_at, scan_snapshot, scan_xact, scan_mark, folded_below)
SELECT -1, pg_catalog.count(*), pg_catalog.clock_timestamp(),
       pg_catalog.pg_current_snapshot(), pg_catalog.pg_current_xact_id(),
       pg_catalog.nextval('_pgrdf_graph_count_bypass_seq'),
       pg_catalog.pg_snapshot_xmin(pg_catalog.pg_current_snapshot())
  FROM _pgrdf_dictionary;

-- Folding on request: what the engine does by itself after writes.
CREATE  FUNCTION "_fold_counts"(
	"graph_id" bigint /* i64 */
) RETURNS bigint /* i64 */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', '_fold_counts_wrapper';

-- #176: an act feed a poller can trust.
CREATE INDEX _pgrdf_graph_acts_xact_idx ON _pgrdf_graph_acts (xact, act_id);
CREATE INDEX _pgrdf_graph_acts_at_idx ON _pgrdf_graph_acts (at);

CREATE FUNCTION graph_act_feed(
    after xid8 DEFAULT NULL,
    only_graph BIGINT DEFAULT NULL,
    page TEXT DEFAULT NULL,
    max_acts INTEGER DEFAULT 1000)
RETURNS TABLE (
    complete_below xid8, next_page TEXT,
    act_id BIGINT, graph_id BIGINT, graph_iri TEXT, act TEXT, at TIMESTAMPTZ, xact xid8,
    session_role TEXT, acting_role TEXT, application_name TEXT, requester TEXT,
    workspace TEXT, agent TEXT, thread TEXT, on_behalf_of TEXT, harness TEXT, detail JSONB)
LANGUAGE sql STABLE
SET search_path = pg_catalog, pg_temp
AS $fn$
  -- One statement, so the watermark and the acts come from one snapshot.
  WITH tok AS (
    SELECT CASE WHEN graph_act_feed.page IS NULL
                THEN pg_snapshot_xmin(pg_current_snapshot())
                ELSE split_part(graph_act_feed.page, ':', 1)::xid8 END AS w,
           CASE WHEN graph_act_feed.page IS NOT NULL
                THEN split_part(graph_act_feed.page, ':', 2)::xid8 END AS px,
           CASE WHEN graph_act_feed.page IS NOT NULL
                THEN split_part(graph_act_feed.page, ':', 3)::bigint END AS pa,
           pg_snapshot_xmax(pg_current_snapshot()) AS xmax
  ), picked AS (
    SELECT a.*
      FROM pgrdf._pgrdf_graph_acts a, tok
     WHERE (graph_act_feed.after IS NULL OR a.xact >= graph_act_feed.after)
       AND (graph_act_feed.only_graph IS NULL OR a.graph_id = graph_act_feed.only_graph)
       -- The caller's own open transaction: the only visible acts that
       -- can still roll back.
       AND a.xact IS DISTINCT FROM pg_current_xact_id_if_assigned()
       -- A visible act of this server has a transaction id below the
       -- snapshot's xmax. One at or above it came with a restore from
       -- another server; it is history, delivered to a reader that starts
       -- from the beginning and to no cursor.
       AND (graph_act_feed.after IS NULL OR a.xact < tok.xmax)
       AND (tok.px IS NULL OR (a.xact, a.act_id) > (tok.px, tok.pa))
     ORDER BY a.xact, a.act_id
     LIMIT greatest(graph_act_feed.max_acts, 1) + 1
  ), shown AS (
    SELECT * FROM picked ORDER BY xact, act_id LIMIT greatest(graph_act_feed.max_acts, 1)
  ), tail AS (
    SELECT (SELECT count(*) FROM picked) > greatest(graph_act_feed.max_acts, 1) AS more,
           (SELECT s.xact FROM shown s ORDER BY s.xact DESC, s.act_id DESC LIMIT 1) AS lx,
           (SELECT s.act_id FROM shown s ORDER BY s.xact DESC, s.act_id DESC LIMIT 1) AS la
  )
  SELECT tok.w,
         CASE WHEN tail.more THEN tok.w::text || ':' || tail.lx::text || ':' || tail.la::text END,
         p.act_id, p.graph_id, p.graph_iri, p.act, p.at, p.xact,
         p.session_role, p.acting_role, p.application_name, p.requester,
         p.workspace, p.agent, p.thread, p.on_behalf_of, p.harness, p.detail
    FROM tok CROSS JOIN tail LEFT JOIN shown p ON true
   ORDER BY p.xact, p.act_id
$fn$;
-- #171, #175: graph_inventory() reads the stored counts, lists by role,
-- and gains four columns and two one-graph forms. A changed result type
-- needs the function dropped first.
DROP FUNCTION graph_inventory();
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
	"locked_by" TEXT,  /* Option < String > */
	"change_seq" bigint,  /* i64 */
	"counts_verified" bool,  /* bool */
	"counts_scanned_at" timestamp with time zone,  /* Option < pgrx :: datum :: TimestampWithTimeZone > */
	"last_act_id" bigint  /* Option < i64 > */
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_inventory_wrapper';

CREATE  FUNCTION "graph_inventory"(
	"graph_id" bigint /* i64 */
) RETURNS TABLE (
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
	"locked_by" TEXT,  /* Option < String > */
	"change_seq" bigint,  /* i64 */
	"counts_verified" bool,  /* bool */
	"counts_scanned_at" timestamp with time zone,  /* Option < pgrx :: datum :: TimestampWithTimeZone > */
	"last_act_id" bigint  /* Option < i64 > */
)
STRICT 
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_inventory_by_id_wrapper';

CREATE  FUNCTION "graph_inventory"(
	"iri" TEXT /* & str */
) RETURNS TABLE (
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
	"locked_by" TEXT,  /* Option < String > */
	"change_seq" bigint,  /* i64 */
	"counts_verified" bool,  /* bool */
	"counts_scanned_at" timestamp with time zone,  /* Option < pgrx :: datum :: TimestampWithTimeZone > */
	"last_act_id" bigint  /* Option < i64 > */
)
STRICT 
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_inventory_by_iri_wrapper';

CREATE  FUNCTION "recount_graph"(
	"graph_id" bigint DEFAULT NULL /* Option < i64 > */
) RETURNS TABLE (
	"graph_id" bigint,  /* i64 */
	"asserted" bigint,  /* i64 */
	"inferred" bigint,  /* i64 */
	"was_asserted" bigint,  /* Option < i64 > */
	"was_inferred" bigint,  /* Option < i64 > */
	"was_verified" bool  /* bool */
)
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'recount_graph_wrapper';

CREATE  FUNCTION "graph_totals"() RETURNS TABLE (
	"scope" TEXT,  /* String */
	"graphs_listed" bigint,  /* i64 */
	"graphs_total" bigint,  /* i64 */
	"asserted" bigint,  /* i64 */
	"inferred" bigint,  /* i64 */
	"unverified_graphs" bigint,  /* i64 */
	"terms" bigint,  /* Option < i64 > */
	"terms_verified" bool  /* Option < bool > */
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_totals_wrapper';

-- #178: the exact number of terms on purpose, and the cure for a term
-- count that is not verified.
CREATE  FUNCTION "count_terms"() RETURNS bigint /* i64 */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'count_terms_wrapper';

CREATE  FUNCTION "recount_terms"() RETURNS TABLE (
	"terms" bigint,  /* i64 */
	"was_terms" bigint,  /* Option < i64 > */
	"was_verified" bool  /* bool */
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'recount_terms_wrapper';

-- #175: the listing follows the role from here on. A graph with no recorded
-- owner (created before 0.6.41) is listed to admins only.
DO $pgrdf_ownerless$
DECLARE
  n bigint := (SELECT pg_catalog.count(*) FROM pgrdf._pgrdf_graphs
                WHERE owner IS NULL AND graph_id <> 0);
BEGIN
  IF n > 0 THEN
    RAISE WARNING 'pgrdf: % graph(s) have no recorded owner. Since 0.6.42 pgrdf.graph_inventory() lists a graph to its owner and to admins (a superuser, a role holding the storage owner''s privileges, or one holding pg_read_all_data); these are listed to admins only. Assign each with pgrdf.set_graph_owner(graph_id, role).', n;
  END IF;
END
$pgrdf_ownerless$;
