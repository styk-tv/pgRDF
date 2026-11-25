//! Stored graph counts (0.6.42, #171).
//!
//! Before this module every count was a scan: `graph_inventory()` ran one
//! aggregate over the whole quad table on every call, so knowing the
//! graphs cost seconds on a large store. Here the engine keeps the counts
//! as it writes.
//!
//! Storage is two tables:
//!
//! * `_pgrdf_graph_counts` — one base row per graph: asserted, inferred,
//!   a change counter, and when and under which snapshot a scan last set
//!   the numbers;
//! * `_pgrdf_graph_count_deltas` — insert-only rows, one per engine write
//!   that changed a graph;
//! * `_pgrdf_graph_count_bypass` — insert-only rows, one per write that
//!   did not come through the engine.
//!
//! A graph's count is its base plus the deltas a reader can see at or
//! above the base row's `folded_below`. Writers only ever insert a delta,
//! so no engine writer waits on another for a count, and two transactions
//! that write the same graphs in opposite order cannot deadlock on one. A
//! fold moves finished deltas into the base and raises `folded_below` to
//! the fold's snapshot `xmin`; every transaction below that has ended, so
//! every delta below it is folded in or aborted, and a reader's index
//! range scan starts above the rows a fold deleted.
//!
//! Two triggers stand on one backend-local mark, [`Writing`], which only
//! the engine's own statements raise and nothing in SQL can set:
//!
//! * the delta table accepts an INSERT only while the mark is up, so a
//!   caller cannot choose what is recorded;
//! * a write to the quad table while the mark is down is a bypass: it
//!   leaves a row in the bypass log, and the graph's count is unverified
//!   until a scan that saw that write.
//!
//! Nothing here waits. Deltas and bypass marks are inserts; a fold skips a
//! base row somebody else holds. A bypass applies to a graph when the
//! scan that last set the graph's count could not have seen it: its
//! transaction was not visible in that scan's snapshot, or it came later
//! in the scan's own transaction.
//!
//! The dictionary's count (#178) lives in the same tables under the key
//! [`DICTIONARY`], a negative number. Its `asserted` is the number of
//! terms. Everything above holds for it unchanged; its bypass trigger is
//! on the dictionary table. The quad side never reaches that key:
//! `put_quad` and the numeric-id loaders accept a negative graph id, so
//! the graph entries here ([`gained`], [`stored`], [`counted`]) answer
//! "no count" for every negative id, and the dictionary has entries of
//! its own ([`terms_gained`], [`stored_terms`]).
//!
//! Which graph ids are counted (#180): the default graph, and every id
//! that has a partition of its own ([`counted`]). A row written into an
//! id without one lands in the default partition and records no delta.
//! Readers apply the same rule through `_count_kept`, so a count the
//! writers do not keep is never called verified, whatever its row says.
//!
//! A count row is opened by whoever registers the graph: the engine when
//! it creates one ([`created`]), and a trigger on the registry for a row
//! that arrives any other way, which is how every graph gets its row when
//! a restore fills the registry. A writer never opens one. It would hold
//! the new row until it commits, and the next writer of that graph would
//! wait for it.

use crate::storage::partition::as_storage_owner;
use pgrx::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

pgrx::extension_sql!(
    r#"
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
-- scan. Deltas then have a row to fold into before the graph's recount.
CREATE TRIGGER pgrdf_count_open
    AFTER INSERT ON _pgrdf_graphs
    FOR EACH ROW EXECUTE FUNCTION _count_open();

-- The dictionary's count is kept under the key -1. A term added or removed
-- in plain SQL marks it unverified; an UPDATE cannot change how many terms
-- there are.
CREATE TRIGGER pgrdf_count_bypass
    BEFORE INSERT OR DELETE ON _pgrdf_dictionary
    FOR EACH STATEMENT EXECUTE FUNCTION _count_bypass('-1');
CREATE TRIGGER pgrdf_count_bypass_truncate
    BEFORE TRUNCATE ON _pgrdf_dictionary
    FOR EACH STATEMENT EXECUTE FUNCTION _count_bypass('-1');

-- A fresh install holds the default graph only, and it is empty.
INSERT INTO _pgrdf_graph_counts (graph_id, scanned_at, scan_snapshot, scan_xact)
SELECT graph_id, pg_catalog.clock_timestamp(), pg_catalog.pg_current_snapshot(),
       pg_catalog.pg_current_xact_id()
  FROM _pgrdf_graphs;
-- And the dictionary, which is empty too.
INSERT INTO _pgrdf_graph_counts (graph_id, asserted, scanned_at, scan_snapshot, scan_xact)
SELECT -1, pg_catalog.count(*), pg_catalog.clock_timestamp(),
       pg_catalog.pg_current_snapshot(), pg_catalog.pg_current_xact_id()
  FROM _pgrdf_dictionary;
"#,
    name = "graph_counts_v0_6_42",
    requires = [
        "schema_v0_4_0_graphs",
        "set_index_v0_6_41",
        _count_delta_guard,
        _count_bypass,
        _count_open
    ],
);

/// The key the dictionary's count is kept under. `asserted` is the number
/// of terms; `inferred` stays 0. Not a graph id: the graph entries of this
/// module answer "no count" for it.
pub(crate) const DICTIONARY: i64 = -1;

/// Does the engine keep a count for this graph id? For the default graph,
/// and for any other id that has a partition of its own, which is what
/// registering a graph creates. Never for a negative id.
///
/// Asked of the system cache on every write, so the writer reads no
/// table, and never remembered: a backend that remembered "has one" after
/// the graph was dropped would record deltas nothing folds, which is #180
/// itself. A relation that carries the partition's name and is not a
/// partition (one that was detached) does not count: rows of the id go to
/// the default partition then.
///
/// This is the one rule. Readers reach it through [`_count_kept`].
pub(crate) fn counted(graph_id: i64) -> bool {
    if graph_id == 0 {
        return true;
    }
    if graph_id < 0 {
        return false;
    }
    let name = std::ffi::CString::new(format!("_pgrdf_quads_g{graph_id}"))
        .expect("a partition name holds no NUL");
    unsafe {
        let ns = pgrx::pg_sys::get_namespace_oid(c"pgrdf".as_ptr(), true);
        if ns == pgrx::pg_sys::InvalidOid {
            return false;
        }
        let rel = pgrx::pg_sys::get_relname_relid(name.as_ptr(), ns);
        rel != pgrx::pg_sys::InvalidOid && pgrx::pg_sys::get_rel_relispartition(rel)
    }
}

/// Is a count kept under this key of the count tables: the dictionary's
/// key, or a graph id the writers count ([`counted`])? The readers' side
/// of the one rule: a count row is called verified only when this holds,
/// so a row the writers do not keep up (a registered graph without a
/// partition of its own) never reads as verified.
#[pg_extern(stable)]
fn _count_kept(key: i64) -> bool {
    key == DICTIONARY || counted(key)
}

/// AFTER INSERT row trigger on the graph registry: open the graph's count
/// row, not set by any scan, unless it has one. The engine's own
/// [`created`] sets the row of a graph it creates; this is for registry
/// rows that arrive any other way, above all a restore's.
#[pg_trigger]
fn _count_open<'a>(
    trigger: &'a pgrx::PgTrigger<'a>,
) -> Result<Option<PgHeapTuple<'a, AllocatedByPostgres>>, pgrx::PgTriggerError> {
    let graph = trigger
        .new()
        .and_then(|row| row.get_by_name::<i64>("graph_id").ok().flatten());
    if let (Some(graph_id), true) = (graph, present()) {
        as_storage_owner(|| {
            Spi::run_with_args(
                "INSERT INTO pgrdf._pgrdf_graph_counts (graph_id) VALUES ($1) \
                 ON CONFLICT (graph_id) DO NOTHING",
                &[graph_id.into()],
            )
        })
        .expect("opening the graph count failed");
    }
    Ok(None)
}

/// What each graph gained in one engine call: asserted, inferred.
type Gains = BTreeMap<i64, (i64, i64)>;

thread_local! {
    /// How many engine quad statements are running in this backend.
    static WRITING: Cell<u32> = const { Cell::new(0) };
    /// The open tally: nesting depth, what each graph gained, and how
    /// many terms the dictionary gained.
    static TALLY: RefCell<(u32, Gains, i64)> =
        const { RefCell::new((0, BTreeMap::new(), 0)) };
    /// State of the fold dice.
    static DICE: Cell<u64> = const { Cell::new(0) };
}

/// The mark that says "the engine is writing quads". Raise it around an
/// engine-built statement that changes the quad table, and around nothing
/// else: never around SPI that runs text a caller supplied.
pub(crate) struct Writing(());

impl Writing {
    pub(crate) fn begin() -> Self {
        WRITING.with(|w| w.set(w.get() + 1));
        Writing(())
    }
}

impl Drop for Writing {
    fn drop(&mut self) {
        WRITING.with(|w| w.set(w.get().saturating_sub(1)));
    }
}

fn writing() -> bool {
    WRITING.with(|w| w.get() > 0)
}

/// Run `f` with the mark up.
pub(crate) fn engine_write<R>(f: impl FnOnce() -> R) -> R {
    let _mark = Writing::begin();
    f()
}

/// BEFORE INSERT row trigger on the delta table: only the engine's own
/// statement may add a delta.
#[pg_trigger]
fn _count_delta_guard<'a>(
    trigger: &'a pgrx::PgTrigger<'a>,
) -> Result<Option<PgHeapTuple<'a, AllocatedByPostgres>>, pgrx::PgTriggerError> {
    if writing() {
        return Ok(trigger.new());
    }
    crate::refuse_with_hint(
        PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
        "pgrdf: graph counts are written by the engine: direct INSERT refused".to_string(),
        "Read them with pgrdf.graph_inventory(); repair one with pgrdf.recount_graph(graph_id)."
            .to_string(),
    )
}

/// BEFORE statement trigger on the quad table and on each partition. A
/// statement that reaches it with the mark down did not come through the
/// engine's writer: the count of the graph it names (a partition's trigger
/// carries its graph id) is unverified from here on, or the count of every
/// graph when the statement names the parent, whose trigger cannot know
/// which graphs the statement touches.
#[pg_trigger]
fn _count_bypass<'a>(
    trigger: &'a pgrx::PgTrigger<'a>,
) -> Result<Option<PgHeapTuple<'a, AllocatedByPostgres>>, pgrx::PgTriggerError> {
    if writing() {
        return Ok(None);
    }
    let graph = trigger
        .extra_args()?
        .first()
        .and_then(|a| a.parse::<i64>().ok());
    // An insert, so a bypass waits on nothing and blocks nothing. Whether
    // it applies to a graph is decided when the count is read, against
    // the snapshot of the scan that last set that count.
    as_storage_owner(|| {
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_graph_count_bypass (graph_id) VALUES ($1)",
            &[graph.into()],
        )
    })
    .expect("marking the graph count unverified failed");
    Ok(None)
}

/// Is the delta table there? It arrives with the 0.6.42 SQL; a library
/// newer than the installed SQL writes no delta until `ALTER EXTENSION
/// pgrdf UPDATE`. Asked through the syscache on every write and never
/// remembered as "absent": a backend that did would go on writing
/// uncounted rows after the upgrade committed.
pub(crate) fn present() -> bool {
    thread_local! { static SEEN: Cell<bool> = const { Cell::new(false) }; }
    if SEEN.with(|s| s.get()) {
        return true;
    }
    let there = unsafe {
        let ns = pgrx::pg_sys::get_namespace_oid(c"pgrdf".as_ptr(), true);
        ns != pgrx::pg_sys::InvalidOid
            && pgrx::pg_sys::get_relname_relid(c"_pgrdf_graph_count_deltas".as_ptr(), ns)
                != pgrx::pg_sys::InvalidOid
    };
    if there {
        SEEN.with(|s| s.set(true));
    }
    there
}

/// One engine call. While a tally is open the writer adds what it stored
/// to it instead of writing a delta per statement; when the outermost
/// tally closes, each graph gets one delta. A call that unwinds writes
/// nothing: its rows roll back with it.
pub(crate) struct Tally(());

impl Tally {
    pub(crate) fn open() -> Self {
        TALLY.with(|t| t.borrow_mut().0 += 1);
        Tally(())
    }
}

impl Drop for Tally {
    fn drop(&mut self) {
        let done = TALLY.with(|t| {
            let mut t = t.borrow_mut();
            t.0 = t.0.saturating_sub(1);
            (t.0 == 0).then(|| (std::mem::take(&mut t.1), std::mem::take(&mut t.2)))
        });
        if let Some((gained, terms)) = done {
            if std::thread::panicking() {
                return;
            }
            for (graph_id, (asserted, inferred)) in gained {
                write_delta(graph_id, asserted, inferred);
            }
            write_terms_delta(terms);
        }
    }
}

/// Is a tally open? Then a writer hands its gains to [`gained`] and must
/// not use a statement that carries its own delta.
pub(crate) fn tally_open() -> bool {
    TALLY.with(|t| t.borrow().0 > 0)
}

/// Record what one write changed in `graph_id`: into the open tally, or as
/// a delta at once when no tally is open.
///
/// A negative graph id has no count: nothing is recorded for it, here or
/// in [`write_delta`]. The dictionary's key is negative, and a write into
/// "graph -1" must never be taken for terms.
pub(crate) fn gained(graph_id: i64, asserted: i64, inferred: i64) {
    if graph_id < 0 || (asserted == 0 && inferred == 0) {
        return;
    }
    let open = TALLY.with(|t| {
        let mut t = t.borrow_mut();
        if t.0 == 0 {
            return false;
        }
        let e = t.1.entry(graph_id).or_insert((0, 0));
        e.0 += asserted;
        e.1 += inferred;
        true
    });
    if !open {
        write_delta(graph_id, asserted, inferred);
    }
}

/// Record that the dictionary gained `n` terms: into the open tally, or as
/// a delta at once when no tally is open. The dictionary's side of
/// [`gained`], with a slot of its own in the tally.
pub(crate) fn terms_gained(n: i64) {
    if n == 0 {
        return;
    }
    let open = TALLY.with(|t| {
        let mut t = t.borrow_mut();
        if t.0 == 0 {
            return false;
        }
        t.2 += n;
        true
    });
    if !open {
        write_terms_delta(n);
    }
}

/// Insert one delta of a graph, as the caller, through the guard.
fn write_delta(graph_id: i64, asserted: i64, inferred: i64) {
    if (asserted == 0 && inferred == 0) || !present() || !counted(graph_id) {
        return;
    }
    insert_delta(graph_id, asserted, inferred);
}

/// Insert one delta of the dictionary, as the caller, through the guard.
fn write_terms_delta(n: i64) {
    if n == 0 || !present() {
        return;
    }
    insert_delta(DICTIONARY, n, 0);
}

fn insert_delta(key: i64, asserted: i64, inferred: i64) {
    engine_write(|| {
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_graph_count_deltas (graph_id, d_asserted, d_inferred) \
             VALUES ($1, $2, $3)",
            &[key.into(), asserted.into(), inferred.into()],
        )
        .expect("recording the count delta failed");
    });
    maybe_fold(key);
}

/// One writer in [`FOLD_ONE_IN`] folds the graph it just wrote.
const FOLD_ONE_IN: u64 = 64;

/// After a delta: fold now and then, so pending deltas stay few whatever
/// the pattern of sessions. Only in READ COMMITTED, where the fold's
/// row lock and its snapshot cannot raise a serialization failure in the
/// caller's transaction.
pub(crate) fn maybe_fold(graph_id: i64) {
    let roll = DICE.with(|d| {
        let mut x = d.get();
        if x == 0 {
            x = 0x9E37_79B9_7F4A_7C15
                ^ (std::process::id() as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93);
        }
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        d.set(x);
        x
    });
    if !roll.is_multiple_of(FOLD_ONE_IN) {
        return;
    }
    let read_committed =
        unsafe { pgrx::pg_sys::XactIsoLevel } == pgrx::pg_sys::XACT_READ_COMMITTED as i32;
    if read_committed {
        fold(graph_id);
    }
}

/// Move the finished deltas of `graph_id` into its base row. The sums
/// come from what the DELETE itself returns: after a read-committed
/// re-check only that says what this fold removed. A folder that cannot
/// get the row leaves it to the one that has it.
///
/// A missing base row is skipped like a locked one. Opening it here would
/// put an uncommitted row in a writer's transaction, and the next writer
/// of that graph would wait on it; two transactions writing two such
/// graphs in opposite order would deadlock.
pub(crate) fn fold(graph_id: i64) -> i64 {
    as_storage_owner(|| {
        // The DELETE below runs on every fold, also when its range is
        // empty, which under a long-held transaction id is every time: it
        // must be the index range and never a pass over the whole table.
        plan_by_index();
        Spi::get_one_with_args::<i64>(
            "WITH b AS (SELECT graph_id, folded_below FROM pgrdf._pgrdf_graph_counts \
                         WHERE graph_id = $1 FOR UPDATE SKIP LOCKED), \
                  w AS (SELECT pg_snapshot_xmin(pg_current_snapshot()) AS w), \
                  gone AS (DELETE FROM pgrdf._pgrdf_graph_count_deltas d USING b, w \
                            WHERE d.graph_id = b.graph_id \
                              AND d.xact >= b.folded_below AND d.xact < w.w \
                           RETURNING d.d_asserted, d.d_inferred), \
                  s AS (SELECT coalesce(sum(d_asserted), 0)::bigint AS a, \
                               coalesce(sum(d_inferred), 0)::bigint AS i, \
                               count(*) AS n FROM gone), \
                  up AS (UPDATE pgrdf._pgrdf_graph_counts c \
                            SET asserted = c.asserted + s.a, inferred = c.inferred + s.i, \
                                change_seq = c.change_seq + s.n, \
                                folded_below = greatest(c.folded_below, w.w) \
                           FROM b, w, s \
                          WHERE c.graph_id = b.graph_id AND w.w > b.folded_below \
                         RETURNING s.n) \
             SELECT coalesce((SELECT n FROM up), 0)::bigint",
            &[graph_id.into()],
        )
        .expect("folding the graph count failed")
        .unwrap_or(0)
    })
}

/// Fold the finished deltas of one graph into its base row now, and say
/// how many were folded. The engine does this by itself now and then
/// after a write; this is the same act on request, for maintenance and
/// for the gates.
#[pg_extern]
fn _fold_counts(graph_id: i64) -> i64 {
    if !present() {
        return 0;
    }
    fold(graph_id)
}

/// What the engine holds for one graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Stored {
    pub asserted: i64,
    pub inferred: i64,
    pub change_seq: i64,
    /// Set by a scan, and every change since came through the engine.
    pub verified: bool,
}

/// The bypass rows that apply to base row `c`: the scan that set `c`
/// could not have seen them. A bypass of another transaction applies when
/// that transaction was not visible in the scan's snapshot. One of the
/// scan's own transaction applies when it came after the scan: a snapshot
/// does not list the transaction that took it, so the order inside one
/// transaction is kept by the row ids, which only grow.
///
/// A bypass with no graph id is a statement on the quad parent: it can
/// have touched any graph and no term, so it applies to every graph row
/// and not to the dictionary's.
pub(crate) const BYPASS_APPLIES: &str = "(b.graph_id = c.graph_id OR (b.graph_id IS NULL AND c.graph_id >= 0)) \
     AND CASE WHEN b.xact = c.scan_xact THEN b.id > c.scan_mark \
              ELSE NOT pg_visible_in_snapshot(b.xact, c.scan_snapshot) END";

/// The count columns of base row `c` as a reader sees them: asserted,
/// inferred, change_seq, verified, scanned_at. For a FROM list that has
/// `pgrdf._pgrdf_graph_counts c`.
pub(crate) fn read_columns() -> String {
    "c.asserted + d.a, c.inferred + d.i, c.change_seq + d.n + y.n, \
     c.scanned_at IS NOT NULL AND y.n = 0 AND pgrdf._count_kept(c.graph_id), c.scanned_at"
        .to_string()
}

/// The two lateral subqueries [`read_columns`] stands on.
pub(crate) fn read_laterals() -> String {
    format!(
        "LATERAL (SELECT coalesce(sum(x.d_asserted), 0)::bigint AS a, \
                         coalesce(sum(x.d_inferred), 0)::bigint AS i, \
                         count(*) AS n \
                    FROM pgrdf._pgrdf_graph_count_deltas x \
                   WHERE x.graph_id = c.graph_id AND x.xact >= c.folded_below) d ON true \
         LEFT JOIN LATERAL (SELECT count(*) AS n FROM pgrdf._pgrdf_graph_count_bypass b \
                             WHERE {BYPASS_APPLIES}) y ON true"
    )
}

/// For the rest of the enclosing owner switch, plan without sequential
/// scans. The delta and bypass tables are read by index range and by
/// nothing else, and the planner must not be talked out of it: once
/// statistics say that every waiting delta belongs to one graph, a
/// per-graph subquery over the whole table looks cheaper than the range,
/// and the listing then reads every delta once per graph. Local to the
/// switch's GUC level, so it ends with it. Tables too small to have an
/// index path worth taking are unaffected in any way that matters: the
/// registry holds one row per graph. The count of an unverified graph
/// ([`scanned`]) is unaffected too: a partition has no index that can
/// answer it, and the planner still scans the partition (measured).
pub(crate) fn plan_by_index() {
    unsafe {
        pgrx::pg_sys::set_config_option(
            c"enable_seqscan".as_ptr(),
            c"off".as_ptr(),
            pgrx::pg_sys::GucContext::PGC_USERSET,
            pgrx::pg_sys::GucSource::PGC_S_SESSION,
            pgrx::pg_sys::GucAction::GUC_ACTION_LOCAL,
            true,
            0,
            false,
        );
    }
}

/// The stored count of graph `graph_id`, or `None` when it has no base
/// row (an id that was never registered) and for every negative id: the
/// dictionary's key is not a graph, and `count_quads(-1)` must count the
/// rows of that id, not the terms. Read as the storage owner: the caller
/// needs no privilege on the tables.
pub(crate) fn stored(graph_id: i64) -> Option<Stored> {
    if graph_id < 0 {
        return None;
    }
    stored_key(graph_id)
}

/// The stored number of dictionary terms (`asserted`), or `None` when the
/// dictionary has no count row.
pub(crate) fn stored_terms() -> Option<Stored> {
    stored_key(DICTIONARY)
}

fn stored_key(graph_id: i64) -> Option<Stored> {
    if !present() {
        return None;
    }
    crate::storage::partition::as_storage_owner_reading(|| {
        plan_by_index();
        Spi::connect(|c| {
            let t = c
                .select(
                    &format!(
                        "SELECT {} FROM pgrdf._pgrdf_graph_counts c LEFT JOIN {} \
                          WHERE c.graph_id = $1",
                        read_columns(),
                        read_laterals()
                    ),
                    None,
                    &[graph_id.into()],
                )
                .expect("reading the graph count failed");
            t.into_iter().next().map(|row| Stored {
                asserted: row.get::<i64>(1).unwrap().unwrap_or(0),
                inferred: row.get::<i64>(2).unwrap().unwrap_or(0),
                change_seq: row.get::<i64>(3).unwrap().unwrap_or(0),
                verified: row.get::<bool>(4).unwrap().unwrap_or(false),
            })
        })
    })
}

/// The statement that counts what `$1` stands for: the asserted and
/// inferred rows of a graph, or the terms of the dictionary.
fn scan_of(id: i64) -> &'static str {
    if id == DICTIONARY {
        "SELECT count(*) AS a, 0::bigint AS i FROM pgrdf._pgrdf_dictionary"
    } else {
        "SELECT count(*) FILTER (WHERE NOT is_inferred) AS a, \
                count(*) FILTER (WHERE is_inferred) AS i \
           FROM pgrdf._pgrdf_quads WHERE graph_id = $1"
    }
}

/// The asserted and inferred rows of `graph_id`, counted (for
/// [`DICTIONARY`]: its terms, and 0).
pub(crate) fn scanned(graph_id: i64) -> (i64, i64) {
    Spi::connect(|c| {
        let t = c
            .select(scan_of(graph_id), None, &[graph_id.into()])
            .expect("counting the graph failed");
        let row = t.first();
        (
            row.get::<i64>(1).unwrap().unwrap_or(0),
            row.get::<i64>(2).unwrap().unwrap_or(0),
        )
    })
}

/// What a scan leaves on the base row besides the numbers: when, under
/// which snapshot, in which transaction, and where in that transaction.
/// `change_seq` takes the retired deltas (`gone`, a CTE of the statement)
/// and the bypass marks that applied until now with it, so it never goes
/// back: a reader added both to it a moment ago.
fn scan_stamp() -> String {
    format!("change_seq = {}, {}", seq_after_scan(), stamp_of_scan())
}

/// `change_seq` after a scan set the row: past the retired deltas and the
/// bypass marks that applied, and one more for the scan.
fn seq_after_scan() -> String {
    format!(
        "c.change_seq + (SELECT count(*) FROM gone) + 1 \
           + (SELECT count(*) FROM pgrdf._pgrdf_graph_count_bypass b WHERE {BYPASS_APPLIES})"
    )
}

/// When, under which snapshot, in which transaction and where in it.
fn stamp_of_scan() -> &'static str {
    "scanned_at = clock_timestamp(), \
     scan_snapshot = pg_current_snapshot(), scan_xact = pg_current_xact_id(), \
     scan_mark = nextval('pgrdf._pgrdf_graph_count_bypass_seq'), \
     folded_below = greatest(c.folded_below, pg_snapshot_xmin(pg_current_snapshot()))"
}

/// Drop the bypass rows no graph needs any more. Run as the storage owner
/// after a scan stamped a base row.
fn forget_spent_bypasses() {
    Spi::run(&format!(
        "DELETE FROM pgrdf._pgrdf_graph_count_bypass b \
          WHERE NOT EXISTS (SELECT 1 FROM pgrdf._pgrdf_graph_counts c WHERE {BYPASS_APPLIES})"
    ))
    .expect("clearing spent bypass marks failed");
}

/// Set the count of `graph_id` from a scan and mark it verified. Returns
/// the count it found.
///
/// Lock first, scan second. A fold holds the base row from its statement
/// to the end of its transaction. If this scanned first and waited for
/// the row afterwards, a fold that committed in between could have taken
/// in the delta of a write this scan did not see, and the scan's older
/// number would then be stored over it: rows in the graph and in no
/// count. With the row held before the scan's snapshot is taken, no fold
/// can commit in between (folds skip a locked row). In REPEATABLE READ
/// the lock raises a serialization failure when the row has moved on,
/// which is the honest answer there.
///
/// The scan sees every row this transaction wrote, so what the open
/// tally has gathered for the graph so far is in the number already and
/// is dropped; what the call gains after this point still counts. The
/// same holds for the dictionary's key and the tally's terms.
pub(crate) fn set_by_scan(graph_id: i64) -> (i64, i64) {
    if !present() {
        return scanned(graph_id);
    }
    TALLY.with(|t| {
        let mut t = t.borrow_mut();
        if graph_id == DICTIONARY {
            t.2 = 0;
        } else {
            t.1.remove(&graph_id);
        }
    });
    as_storage_owner(|| {
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_graph_counts (graph_id) VALUES ($1) \
             ON CONFLICT (graph_id) DO NOTHING",
            &[graph_id.into()],
        )
        .expect("opening the graph count failed");
        Spi::run_with_args(
            "SELECT 1 FROM pgrdf._pgrdf_graph_counts WHERE graph_id = $1 FOR UPDATE",
            &[graph_id.into()],
        )
        .expect("locking the graph count failed");
        let found = Spi::connect_mut(|c| {
            let t = c
                .update(
                    &format!(
                        "WITH gone AS (DELETE FROM pgrdf._pgrdf_graph_count_deltas \
                                        WHERE graph_id = $1 RETURNING 1), \
                              n AS ({scan}) \
                         UPDATE pgrdf._pgrdf_graph_counts c \
                            SET asserted = n.a, inferred = n.i, \
                                {stamp} \
                           FROM n WHERE c.graph_id = $1 \
                         RETURNING c.asserted, c.inferred",
                        stamp = scan_stamp(),
                        scan = scan_of(graph_id),
                    ),
                    None,
                    &[graph_id.into()],
                )
                .expect("setting the graph count failed");
            let row = t.first();
            (
                row.get::<i64>(1).unwrap().unwrap_or(0),
                row.get::<i64>(2).unwrap().unwrap_or(0),
            )
        });
        forget_spent_bypasses();
        found
    })
}

/// A graph was created: it is empty, and an empty new partition is
/// scanned by definition. Whatever an earlier life of the id left behind
/// goes: a count row (reset) and every delta. A row the registry opened a
/// moment ago, in this transaction, is set the same way and keeps its
/// `change_seq`, so a new graph starts at 0 whichever came first, its
/// registry row or its partition. The deltas matter. An id can
/// take writes while it is unregistered, lose those rows to a DELETE in
/// plain SQL, which records no negative delta, and be registered
/// afterwards; a row merely inserted beside the old deltas would then read
/// them on top of zero, verified. PostgreSQL creates the partition only
/// when the default partition holds no row of the id and no writer of it
/// is in flight (the DDL waits for them), so every delta of the id is
/// finished and visible here. In READ COMMITTED, that is. A caller in
/// REPEATABLE READ deletes by its transaction's snapshot, and a delta
/// committed after that snapshot survives; it takes a drop that raced a
/// writer and a plain-SQL delete of that writer's row inside the window to
/// have such a delta, and that is accepted.
pub(crate) fn created(graph_id: i64) {
    if !present() {
        return;
    }
    TALLY.with(|t| t.borrow_mut().1.remove(&graph_id));
    as_storage_owner(|| {
        Spi::run_with_args(
            &format!(
                "WITH gone AS (DELETE FROM pgrdf._pgrdf_graph_count_deltas \
                                WHERE graph_id = $1 RETURNING 1) \
                 INSERT INTO pgrdf._pgrdf_graph_counts AS c \
                        (graph_id, scanned_at, scan_snapshot, scan_xact, scan_mark, folded_below) \
                 VALUES ($1, clock_timestamp(), pg_current_snapshot(), pg_current_xact_id(), \
                         nextval('pgrdf._pgrdf_graph_count_bypass_seq'), \
                         pg_snapshot_xmin(pg_current_snapshot())) \
                 ON CONFLICT (graph_id) DO UPDATE SET asserted = 0, inferred = 0, \
                        change_seq = CASE WHEN c.scanned_at IS NULL THEN c.change_seq \
                                          ELSE {seq} END, \
                        {stamp}",
                seq = seq_after_scan(),
                stamp = stamp_of_scan()
            ),
            &[graph_id.into()],
        )
        .expect("opening the graph count failed");
    });
}

/// A graph was dropped: its count goes with it.
pub(crate) fn dropped(graph_id: i64) {
    if !present() {
        return;
    }
    TALLY.with(|t| t.borrow_mut().1.remove(&graph_id));
    as_storage_owner(|| {
        for table in [
            "_pgrdf_graph_count_deltas",
            "_pgrdf_graph_count_bypass",
            "_pgrdf_graph_counts",
        ] {
            Spi::run_with_args(
                &format!("DELETE FROM pgrdf.{table} WHERE graph_id = $1"),
                &[graph_id.into()],
            )
            .expect("dropping the graph count failed");
        }
    });
}

/// Every row of a graph was replaced in one act (a TRUNCATE, or a
/// partition swapped for a new one holding `asserted` rows): the count is
/// known without a scan, and whatever deltas describe the old rows go.
///
/// No lock-first step as in [`set_by_scan`]: the caller holds ACCESS
/// EXCLUSIVE on the graph's partition, so no writer of this graph is in
/// flight, and the value stored is absolute, not a scan a fold could have
/// overtaken.
pub(crate) fn replaced(graph_id: i64, asserted: i64, inferred: i64) {
    if !present() {
        return;
    }
    TALLY.with(|t| t.borrow_mut().1.remove(&graph_id));
    as_storage_owner(|| {
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_graph_counts (graph_id) VALUES ($1) \
             ON CONFLICT (graph_id) DO NOTHING",
            &[graph_id.into()],
        )
        .expect("opening the graph count failed");
        Spi::run_with_args(
            &format!(
                "WITH gone AS (DELETE FROM pgrdf._pgrdf_graph_count_deltas \
                                WHERE graph_id = $1 RETURNING 1) \
                 UPDATE pgrdf._pgrdf_graph_counts c \
                    SET asserted = $2, inferred = $3, \
                        {stamp} \
                  WHERE c.graph_id = $1",
                stamp = scan_stamp()
            ),
            &[graph_id.into(), asserted.into(), inferred.into()],
        )
        .expect("resetting the graph count failed");
        forget_spent_bypasses();
    });
}

/// Install the bypass marker on a graph's own partition. The caller runs
/// this as the storage owner, where it creates or attaches the partition.
pub(crate) fn guard_partition(part_name: &str, graph_id: i64) {
    if !present() {
        return;
    }
    for (name, events) in [
        ("pgrdf_count_bypass", "INSERT OR UPDATE OR DELETE"),
        ("pgrdf_count_bypass_truncate", "TRUNCATE"),
    ] {
        Spi::run(&format!(
            "CREATE OR REPLACE TRIGGER {name} BEFORE {events} ON pgrdf.{part_name} \
             FOR EACH STATEMENT EXECUTE FUNCTION pgrdf._count_bypass('{graph_id}')"
        ))
        .expect("installing the count bypass marker failed");
    }
}

#[cfg(any(test, feature = "pg_test"))]
pub(crate) mod testing {
    use super::*;

    /// `Err` with what is wrong unless the stored count of `g` is verified
    /// and equal to a scan.
    pub(crate) fn truth(g: i64) -> Result<Stored, String> {
        let stored = super::stored(g).ok_or_else(|| format!("graph {g}: no stored count"))?;
        let (a, i) = super::scanned(g);
        if !stored.verified {
            return Err(format!("graph {g}: not verified ({stored:?})"));
        }
        if (stored.asserted, stored.inferred) != (a, i) {
            return Err(format!(
                "graph {g}: stored {}+{}, scanned {a}+{i}",
                stored.asserted, stored.inferred
            ));
        }
        Ok(stored)
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use super::testing::truth;
    use pgrx::prelude::*;

    fn one(q: &str) -> Option<i64> {
        Spi::get_one::<i64>(q).unwrap()
    }

    fn try_fn() {
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.q_try(q text) RETURNS text LANGUAGE plpgsql AS $$ \
             BEGIN EXECUTE q; RETURN 'ok'; EXCEPTION WHEN OTHERS THEN RETURN SQLSTATE; END $$",
        )
        .unwrap();
    }

    fn attempt(q: &str) -> String {
        Spi::get_one_with_args::<String>("SELECT pg_temp.q_try($1)", &[q.into()])
            .unwrap()
            .unwrap()
    }

    #[pg_test]
    fn a_new_graph_has_a_verified_count_of_zero() {
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c1:new')").unwrap();
        let s = truth(g).unwrap();
        assert_eq!((s.asserted, s.inferred), (0, 0));
    }

    #[pg_test]
    fn a_load_is_counted_and_a_repeat_of_it_moves_nothing() {
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c1:load')").unwrap();
        let ttl = "<urn:c1:a> <urn:c1:p> <urn:c1:b> . <urn:c1:a> <urn:c1:p> <urn:c1:c> .";
        Spi::run(&format!("SELECT pgrdf.parse_turtle('{ttl}', {g})")).unwrap();
        let first = truth(g).unwrap();
        assert_eq!((first.asserted, first.inferred), (2, 0));
        assert!(
            first.change_seq > 0,
            "a load that stored rows moves change_seq"
        );
        Spi::run(&format!("SELECT pgrdf.parse_turtle('{ttl}', {g})")).unwrap();
        let second = truth(g).unwrap();
        assert_eq!(second, first, "a load that stored nothing changes nothing");
    }

    #[pg_test]
    fn a_single_row_write_is_counted() {
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c1:row')").unwrap();
        Spi::run(&format!("SELECT pgrdf.put_quad(91, 92, 93, {g})")).unwrap();
        Spi::run(&format!("SELECT pgrdf.put_quad(91, 92, 94, {g})")).unwrap();
        Spi::run(&format!("SELECT pgrdf.put_quad(91, 92, 94, {g})")).unwrap();
        let s = truth(g).unwrap();
        assert_eq!((s.asserted, s.inferred, s.change_seq), (2, 0, 2));
    }

    /// A write that fails takes its count with it.
    #[pg_test]
    fn a_write_that_rolls_back_leaves_no_count() {
        try_fn();
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c1:undo')").unwrap();
        let before = truth(g).unwrap();
        let r = attempt(&format!(
            "DO $do$ BEGIN PERFORM pgrdf.put_quad(81, 82, 83, {g}); \
             RAISE EXCEPTION 'undo' USING ERRCODE = 'P0001'; END $do$"
        ));
        assert_eq!(r, "P0001");
        assert_eq!(truth(g).unwrap(), before);
    }

    /// Inside the writing transaction nothing is finished, so a fold has
    /// nothing to move, and the count stays right.
    #[pg_test]
    fn a_fold_changes_no_count() {
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c1:fold')").unwrap();
        for o in 1..=5 {
            Spi::run(&format!("SELECT pgrdf.put_quad(71, 72, {o}, {g})")).unwrap();
        }
        let before = truth(g).unwrap();
        crate::storage::counts::fold(g);
        assert_eq!(truth(g).unwrap(), before);
    }

    /// Every engine route that removes, moves or derives triples leaves
    /// the stored count verified and equal to a scan. (The routes that add
    /// triples are walked by `quads::every_write_route_stores_a_set`.)
    #[pg_test]
    fn every_engine_route_keeps_the_count_true() {
        let mk = |iri: &str| one(&format!("SELECT pgrdf.add_graph('{iri}')")).unwrap();
        let a = mk("urn:tdd:c2:a");
        let b = mk("urn:tdd:c2:b");
        let c = mk("urn:tdd:c2:c");
        let d = mk("urn:tdd:c2:d");
        let ttl = "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> . \
                   <urn:c2:Dog> rdfs:subClassOf <urn:c2:Animal> . \
                   <urn:c2:rex> a <urn:c2:Dog> . <urn:c2:rex> <urn:c2:name> \"Rex\" . \
                   <urn:c2:tom> <urn:c2:name> \"Tom\" .";
        let mut wrong: Vec<String> = Vec::new();
        let mut check = |step: &str, graphs: &[i64]| {
            for g in graphs {
                if let Err(e) = truth(*g) {
                    wrong.push(format!("after {step}: {e}"));
                }
            }
        };
        let run = |q: &str| Spi::run(q).unwrap_or_else(|e| panic!("{q}: {e}"));

        run(&format!("SELECT pgrdf.parse_turtle($t${ttl}$t$, {a})"));
        check("parse_turtle", &[a]);
        run(&format!("SELECT pgrdf.materialize({a}, 'rdfs')"));
        check("materialize", &[a]);
        assert!(
            truth(a).map(|s| s.inferred > 0).unwrap_or(false),
            "materialize derived rows"
        );
        run(&format!("SELECT pgrdf.materialize({a}, 'rdfs')"));
        check("materialize again", &[a]);
        // An asserted copy of an inferred triple: one row changes sides.
        run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:c2:rex> a <urn:c2:Animal> .', {a})"
        ));
        check("asserting an inferred triple", &[a]);
        run(
            "SELECT * FROM pgrdf.sparql('DELETE DATA { GRAPH <urn:tdd:c2:a> { <urn:c2:tom> <urn:c2:name> \"Tom\" } }')",
        );
        check("DELETE DATA", &[a]);
        run(
            "SELECT * FROM pgrdf.sparql('INSERT { GRAPH <urn:tdd:c2:b> { ?s <urn:c2:label> ?n } } WHERE { GRAPH <urn:tdd:c2:a> { ?s <urn:c2:name> ?n } }')",
        );
        check("INSERT WHERE", &[a, b]);
        run(
            "SELECT * FROM pgrdf.sparql('DELETE WHERE { GRAPH <urn:tdd:c2:b> { ?s <urn:c2:label> ?n } }')",
        );
        check("DELETE WHERE", &[b]);
        run(&format!("SELECT pgrdf.copy_graph({a}, {b})"));
        check("copy_graph", &[a, b]);
        run(&format!(
            "SELECT pgrdf.carve_graph({a}, 'urn:c2:name', {c})"
        ));
        check("carve_graph", &[a, c]);
        run(&format!("SELECT pgrdf.move_graph({b}, {d})"));
        check("move_graph", &[d]);
        run("SELECT * FROM pgrdf.sparql('COPY GRAPH <urn:tdd:c2:a> TO GRAPH <urn:tdd:c2:c>')");
        // COPY drops its destination and writes it anew: a new graph id.
        let c = one("SELECT pgrdf.graph_id('urn:tdd:c2:c')").unwrap();
        check("SPARQL COPY", &[a, c]);
        assert!(
            truth(c).map(|s| s.asserted > 0).unwrap_or(false),
            "COPY wrote rows"
        );
        run("SELECT * FROM pgrdf.sparql('CLEAR GRAPH <urn:tdd:c2:c>')");
        check("SPARQL CLEAR", &[c]);
        run(&format!("SELECT pgrdf.clear_graph({d})"));
        check("clear_graph", &[d]);
        assert_eq!(truth(d).map(|s| (s.asserted, s.inferred)), Ok((0, 0)));
        run(&format!("SELECT pgrdf.drop_graph({d})"));
        assert!(
            crate::storage::counts::stored(d).is_none(),
            "a dropped graph keeps no count"
        );
        assert!(wrong.is_empty(), "counts that went wrong: {wrong:#?}");
    }

    // ── the term count (#178) ──────────────────────────────────────────

    /// (terms, terms_verified, the dictionary counted) as this
    /// transaction sees them.
    fn terms_state() -> (Option<i64>, Option<bool>, i64) {
        Spi::connect(|c| {
            let row = c
                .select(
                    "SELECT terms, terms_verified, \
                            (SELECT count(*) FROM pgrdf._pgrdf_dictionary) \
                       FROM pgrdf.graph_totals()",
                    None,
                    &[],
                )
                .unwrap()
                .first();
            (
                row.get::<i64>(1).unwrap(),
                row.get::<bool>(2).unwrap(),
                row.get::<i64>(3).unwrap().unwrap(),
            )
        })
    }

    /// `Err` with what is wrong unless the stored term count is verified
    /// and equal to the dictionary.
    fn terms_truth(step: &str) -> Result<i64, String> {
        let (terms, verified, counted) = terms_state();
        if verified != Some(true) {
            return Err(format!("after {step}: terms not verified ({verified:?})"));
        }
        if terms != Some(counted) {
            return Err(format!(
                "after {step}: stored {terms:?}, the dictionary holds {counted}"
            ));
        }
        Ok(counted)
    }

    #[pg_test]
    fn a_new_term_is_counted_and_a_known_one_is_not() {
        let before = terms_truth("start").unwrap();
        Spi::run("SELECT pgrdf.put_term('urn:t178:one', 1::smallint)").unwrap();
        assert_eq!(terms_truth("a new term"), Ok(before + 1));
        Spi::run("SELECT pgrdf.put_term('urn:t178:one', 1::smallint)").unwrap();
        assert_eq!(terms_truth("the same term again"), Ok(before + 1));
        assert_eq!(one("SELECT pgrdf.count_terms()"), Some(before + 1));
    }

    /// Every route that can add a term leaves the stored term count
    /// verified and equal to the dictionary, and each route did add terms.
    #[pg_test]
    fn every_route_that_adds_terms_keeps_the_term_count_true() {
        let path = format!("/tmp/pgrdf-t178-{}.nt", std::process::id());
        // Fresh terms per route: an IRI subject, predicate and object, a
        // typed literal (its datatype IRI is a term too), a tagged one.
        let nt = |i: usize| {
            format!(
                "<urn:t178:r{i}:a> <urn:t178:r{i}:p> <urn:t178:r{i}:b> .\n\
                 <urn:t178:r{i}:a> <urn:t178:r{i}:q> \"n{i}\"^^<urn:t178:r{i}:dt> .\n\
                 <urn:t178:r{i}:a> <urn:t178:r{i}:q> \"w{i}\"@en .\n"
            )
        };
        let jrow = |i: usize| {
            format!(
                "'{{\"subject\": {{\"type\": \"iri\", \"value\": \"urn:t178:r{i}:a\"}}, \
                   \"predicate\": {{\"type\": \"iri\", \"value\": \"urn:t178:r{i}:p\"}}, \
                   \"object\": {{\"type\": \"literal\", \"value\": \"j{i}\"}}}}'::jsonb"
            )
        };
        type Route = Box<dyn Fn(usize, i64, &str) -> String>;
        let p = path.clone();
        let p2 = path.clone();
        let p3 = path.clone();
        let routes: Vec<(&str, Route)> = vec![
            (
                "put_term",
                Box::new(|i, _, _| {
                    format!("SELECT pgrdf.put_term('urn:t178:r{i}:lone', 1::smallint)")
                }),
            ),
            (
                "parse_turtle",
                Box::new(move |i, g, _| format!("SELECT pgrdf.parse_turtle($t${}$t$, {g})", nt(i))),
            ),
            (
                "parse_turtle_verbose",
                Box::new(move |i, g, _| {
                    format!("SELECT pgrdf.parse_turtle_verbose($t${}$t$, {g})", nt(i))
                }),
            ),
            (
                "parse_turtle_dict_batched",
                Box::new(move |i, g, _| {
                    format!(
                        "SELECT pgrdf.parse_turtle_dict_batched($t${}$t$, {g})",
                        nt(i)
                    )
                }),
            ),
            (
                "parse_nquads",
                Box::new(move |i, g, _| format!("SELECT pgrdf.parse_nquads($t${}$t$, {g})", nt(i))),
            ),
            (
                "parse_trig",
                Box::new(move |i, _, iri| {
                    format!("SELECT pgrdf.parse_trig($t$<{iri}> {{ {} }}$t$)", nt(i))
                }),
            ),
            (
                "load_turtle",
                Box::new(move |_, g, _| {
                    format!("SELECT pgrdf.load_turtle('{p}', {g}, 'http://base.example/')")
                }),
            ),
            (
                "load_turtle bulk",
                Box::new(move |_, g, _| {
                    format!("SELECT pgrdf.load_turtle('{p2}', {g}, NULL, true)")
                }),
            ),
            (
                "load_turtle_dict_batched",
                Box::new(move |_, g, _| {
                    format!("SELECT pgrdf.load_turtle_dict_batched('{p3}', {g})")
                }),
            ),
            (
                "put_construct_rows",
                Box::new(move |i, g, _| {
                    format!("SELECT pgrdf.put_construct_rows(ARRAY[{}], {g})", jrow(i))
                }),
            ),
            (
                "sparql INSERT DATA",
                Box::new(|i, _, iri| {
                    format!(
                        "SELECT * FROM pgrdf.sparql('INSERT DATA {{ GRAPH <{iri}> {{ \
                         <urn:t178:r{i}:a> <urn:t178:r{i}:p> \"s{i}\" }} }}')"
                    )
                }),
            ),
        ];
        let mut wrong = Vec::new();
        let mut last = terms_truth("start").unwrap();
        for (i, (name, sql)) in routes.iter().enumerate() {
            let iri = format!("urn:tdd:t178:route{i}");
            let g = one(&format!("SELECT pgrdf.add_graph('{iri}')")).unwrap();
            std::fs::write(&path, nt(i)).unwrap();
            for round in 1..=2 {
                Spi::run(&sql(i, g, &iri)).unwrap_or_else(|e| panic!("{name}: {e}"));
                match terms_truth(&format!("{name}, call {round}")) {
                    Err(e) => wrong.push(e),
                    Ok(now) => {
                        if round == 1 && now <= last {
                            wrong.push(format!("{name}: the route added no term"));
                        }
                        if round == 2 && now != last {
                            wrong.push(format!("{name}: a repeat added terms"));
                        }
                        last = now;
                    }
                }
            }
        }
        let _ = std::fs::remove_file(&path);
        assert!(wrong.is_empty(), "term counts that went wrong: {wrong:#?}");
    }

    /// A term that rolls back takes its count with it.
    #[pg_test]
    fn a_term_that_rolls_back_leaves_no_count() {
        try_fn();
        let before = terms_truth("start").unwrap();
        let r = attempt(
            "DO $do$ BEGIN PERFORM pgrdf.put_term('urn:t178:undone', 1::smallint); \
             RAISE EXCEPTION 'undo' USING ERRCODE = 'P0001'; END $do$",
        );
        assert_eq!(r, "P0001");
        assert_eq!(terms_truth("a rolled-back term"), Ok(before));
    }

    /// A term written in plain SQL: the totals say the term count is not
    /// verified and give no number, `count_terms()` gives the exact one,
    /// and `recount_terms()` restores the stored count.
    #[pg_test]
    fn a_direct_dictionary_write_leaves_the_term_count_unverified() {
        let before = terms_truth("start").unwrap();
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_dictionary (term_type, lexical_value) \
             VALUES (1, 'urn:t178:direct')",
        )
        .unwrap();
        // The totals never scan the dictionary: no stored count to trust,
        // no number. The exact one is asked for on purpose.
        assert_eq!(
            terms_state(),
            (None, Some(false), before + 1),
            "after a direct INSERT"
        );
        assert_eq!(one("SELECT pgrdf.count_terms()"), Some(before + 1));
        let cured = Spi::get_one::<String>(
            "SELECT concat_ws('|', terms, was_terms, was_verified) FROM pgrdf.recount_terms()",
        )
        .unwrap();
        assert_eq!(cured, Some(format!("{}|{}|f", before + 1, before)));
        assert_eq!(terms_truth("recount_terms"), Ok(before + 1));
        Spi::run("DELETE FROM pgrdf._pgrdf_dictionary WHERE lexical_value = 'urn:t178:direct'")
            .unwrap();
        assert_eq!(
            terms_state(),
            (None, Some(false), before),
            "after a direct DELETE"
        );
        assert_eq!(one("SELECT pgrdf.count_terms()"), Some(before));
        Spi::run("SELECT pgrdf.recount_terms()").unwrap();
        assert_eq!(terms_truth("the second recount"), Ok(before));
        // An UPDATE cannot change how many terms there are.
        Spi::run("SELECT pgrdf.put_term('urn:t178:renamed', 1::smallint)").unwrap();
        Spi::run(
            "UPDATE pgrdf._pgrdf_dictionary SET lexical_value = 'urn:t178:renamed-2' \
              WHERE lexical_value = 'urn:t178:renamed'",
        )
        .unwrap();
        assert_eq!(terms_truth("a direct UPDATE"), Ok(before + 1));
    }

    /// A plain-SQL write to the quad table says nothing about the
    /// dictionary: the graphs go unverified, the term count does not.
    #[pg_test]
    fn a_direct_quad_write_leaves_the_term_count_verified() {
        let g = one("SELECT pgrdf.add_graph('urn:tdd:t178:quads')").unwrap();
        let before = terms_truth("start").unwrap();
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id) \
             VALUES (5, 6, 7, {g})"
        ))
        .unwrap();
        assert!(one("SELECT unverified_graphs FROM pgrdf.graph_totals()").unwrap() >= 1);
        assert_eq!(terms_truth("a direct quad write"), Ok(before));
    }

    // ── deltas for an id without a count (#180) ────────────────────────

    fn deltas_of(g: i64) -> i64 {
        one(&format!(
            "SELECT count(*) FROM pgrdf._pgrdf_graph_count_deltas WHERE graph_id = {g}"
        ))
        .unwrap()
    }

    /// A graph id that was never registered has no count, so its writes
    /// record none: nothing would ever fold or remove those rows.
    #[pg_test]
    fn a_write_into_an_unregistered_id_records_no_delta() {
        const G: i64 = 918_001;
        Spi::run(&format!("SELECT pgrdf.put_quad(61, 62, 63, {G})")).unwrap();
        Spi::run(&format!("SELECT pgrdf.put_quad(61, 62, 64, {G})")).unwrap();
        assert_eq!(
            one(&format!("SELECT pgrdf.count_quads({G})")),
            Some(2),
            "the rows are stored and counted by scan"
        );
        assert_eq!(deltas_of(G), 0, "no delta for an id without a count");
        // The default graph shares that partition and is counted as ever.
        Spi::run("SELECT pgrdf.put_quad(61, 62, 65, 0)").unwrap();
        truth(0).unwrap();
    }

    /// A registry row opens its count row, unverified: that is how every
    /// graph gets one when a restore fills the registry. No writer ever
    /// has to open one.
    #[pg_test]
    fn a_registry_row_opens_its_count_row() {
        // The engine's own graphs first: they take the partition gate, and
        // every caller takes it before the registry.
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c180:fresh')").unwrap();
        let h = 918_012;
        Spi::run(&format!("SELECT pgrdf.add_graph({h})")).unwrap();
        // The row a new graph gets from its registry insert does not undo
        // the stamp its creation set, and its change counter starts at 0
        // whichever of the two came first.
        for id in [g, h] {
            let s = truth(id).unwrap();
            assert_eq!((s.asserted, s.inferred, s.change_seq), (0, 0, 0));
        }
        const G: i64 = 918_011;
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) VALUES ({G}, 'urn:tdd:c180:restored')"
        ))
        .unwrap();
        assert_eq!(
            Spi::get_one::<String>(&format!(
                "SELECT concat_ws('|', asserted, inferred, scanned_at IS NULL) \
                   FROM pgrdf._pgrdf_graph_counts WHERE graph_id = {G}"
            ))
            .unwrap()
            .as_deref(),
            Some("0|0|t"),
            "the count row is there, and no scan has set it"
        );
    }

    /// A fold never opens a count row. A writer that inserted one would
    /// hold it until it commits, and the next writer of that graph would
    /// wait for it: two transactions writing two such graphs in opposite
    /// order would deadlock.
    #[pg_test]
    fn a_fold_skips_a_missing_count_row() {
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c180:rowless')").unwrap();
        Spi::run(&format!("SELECT pgrdf.put_quad(51, 52, 53, {g})")).unwrap();
        Spi::run(&format!(
            "DELETE FROM pgrdf._pgrdf_graph_counts WHERE graph_id = {g}"
        ))
        .unwrap();
        assert_eq!(crate::storage::counts::fold(g), 0);
        assert_eq!(
            one(&format!(
                "SELECT count(*) FROM pgrdf._pgrdf_graph_counts WHERE graph_id = {g}"
            )),
            Some(0)
        );
        Spi::run(&format!("SELECT pgrdf.recount_graph({g})")).unwrap();
        assert_eq!(truth(g).map(|s| s.asserted), Ok(1));
    }

    /// The writers count an id that has a partition of its own. A
    /// registered graph without one (a registry row written in plain SQL,
    /// or left behind by a drop) gets no delta, so no reader may call its
    /// count verified, whatever its count row says.
    #[pg_test]
    fn a_graph_without_its_own_partition_is_never_verified() {
        const G: i64 = 918_010;
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) VALUES ({G}, 'urn:tdd:c180:nopart')"
        ))
        .unwrap();
        // A stamped count row, as a recount (or an upgrade's seed) leaves.
        crate::storage::counts::set_by_scan(G);
        Spi::run(&format!("SELECT pgrdf.put_quad(31, 32, 33, {G})")).unwrap();
        assert_eq!(
            Spi::get_one::<String>(&format!(
                "SELECT concat_ws('|', asserted, counts_verified) FROM pgrdf.graph_inventory({G})"
            ))
            .unwrap()
            .as_deref(),
            Some("1|f")
        );
        assert_eq!(one(&format!("SELECT pgrdf.count_quads({G})")), Some(1));
    }

    /// A table that was detached keeps the partition's name and is not a
    /// partition: rows of the id go to the default partition from then
    /// on. The writer records nothing for them and the reader says
    /// unverified. The two decide by one rule ([`counted`]).
    #[pg_test]
    fn a_detached_partition_is_not_counted() {
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c180:detached')").unwrap();
        Spi::run(&format!("SELECT pgrdf.put_quad(21, 22, 23, {g})")).unwrap();
        assert!(crate::storage::counts::counted(g));
        Spi::run(&format!(
            "ALTER TABLE pgrdf._pgrdf_quads DETACH PARTITION pgrdf._pgrdf_quads_g{g}"
        ))
        .unwrap();
        assert!(!crate::storage::counts::counted(g));
        let before = deltas_of(g);
        Spi::run(&format!("SELECT pgrdf.put_quad(21, 22, 24, {g})")).unwrap();
        assert_eq!(
            deltas_of(g),
            before,
            "no delta for a row in the default partition"
        );
        assert_eq!(
            Spi::get_one::<String>(&format!(
                "SELECT concat_ws('|', asserted, counts_verified) FROM pgrdf.graph_inventory({g})"
            ))
            .unwrap()
            .as_deref(),
            Some("1|f"),
            "the one row the engine can see, and not verified"
        );
    }

    /// The dictionary's count has a key of its own. A graph id of -1,
    /// which `put_quad` and the numeric-id loaders accept, must not reach
    /// it from the quad side: not as a delta, and not as a read.
    #[pg_test]
    fn a_negative_graph_id_never_reaches_the_term_count() {
        let before = terms_truth("start").unwrap();
        // The load first: it takes the registry before the quad table, as
        // every engine call does. A single-row write before it in the same
        // transaction would take them the other way round.
        Spi::run("SELECT pgrdf.parse_turtle('<urn:neg:a> <urn:neg:b> <urn:neg:c> .', -1)").unwrap();
        assert_eq!(
            terms_truth("a load into graph -1"),
            Ok(before + 3),
            "three terms, and the triple is not one of them"
        );
        assert_eq!(
            one("SELECT pgrdf.count_quads(-1)"),
            Some(1),
            "count_quads(-1) counts the rows of that id, not the terms"
        );
        Spi::run("SELECT pgrdf.put_quad(41, 42, 43, -1)").unwrap();
        assert_eq!(terms_truth("put_quad into graph -1"), Ok(before + 3));
        assert_eq!(one("SELECT pgrdf.count_quads(-1)"), Some(2));
        assert!(crate::storage::counts::stored(-1).is_none());
    }

    /// Registering a graph sets its count, whatever an earlier life of
    /// the id left in the delta table.
    #[pg_test]
    fn registering_a_graph_resets_its_count() {
        const G: i64 = 918_003;
        crate::storage::counts::engine_write(|| {
            Spi::run(&format!(
                "INSERT INTO pgrdf._pgrdf_graph_count_deltas (graph_id, d_asserted, d_inferred) \
                 VALUES ({G}, 7, 0)"
            ))
            .unwrap();
        });
        Spi::run(&format!("SELECT pgrdf.add_graph({G})")).unwrap();
        assert_eq!(truth(G).map(|s| (s.asserted, s.inferred)), Ok((0, 0)));
        assert_eq!(deltas_of(G), 0);
        // The same when a count row is left over, whatever it says.
        const H: i64 = 918_004;
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_graph_counts (graph_id, asserted, inferred) VALUES ({H}, 9, 4)"
        ))
        .unwrap();
        Spi::run(&format!("SELECT pgrdf.add_graph({H})")).unwrap();
        assert_eq!(truth(H).map(|s| (s.asserted, s.inferred)), Ok((0, 0)));
    }

    #[pg_test]
    fn a_direct_delta_is_refused() {
        try_fn();
        let g = one("SELECT pgrdf.add_graph('urn:tdd:c1:forge')").unwrap();
        let r = attempt(&format!(
            "INSERT INTO pgrdf._pgrdf_graph_count_deltas (graph_id, d_asserted, d_inferred) \
             VALUES ({g}, 1000, 0)"
        ));
        assert_eq!(r, "42501");
        assert_eq!(truth(g).unwrap().asserted, 0);
    }
}
