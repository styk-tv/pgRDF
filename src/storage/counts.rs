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
    IF TG_LEVEL = 'STATEMENT' THEN RETURN NULL; END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
  END IF;
  RAISE EXCEPTION USING
    ERRCODE = '42501',
    MESSAGE = pg_catalog.format(
      'pgrdf: graph counts are written by the engine: direct %s refused', TG_OP),
    HINT = 'Read them with pgrdf.graph_inventory(); repair one with pgrdf.recount_graph(graph_id).';
END
$fn$;
CREATE TRIGGER pgrdf_counts_row BEFORE INSERT OR UPDATE OR DELETE ON _pgrdf_graph_counts
    FOR EACH ROW EXECUTE FUNCTION _refuse_count_write();
CREATE TRIGGER pgrdf_counts_truncate BEFORE TRUNCATE ON _pgrdf_graph_counts
    FOR EACH STATEMENT EXECUTE FUNCTION _refuse_count_write();
CREATE TRIGGER pgrdf_counts_row BEFORE UPDATE OR DELETE ON _pgrdf_graph_count_deltas
    FOR EACH ROW EXECUTE FUNCTION _refuse_count_write();
CREATE TRIGGER pgrdf_counts_truncate BEFORE TRUNCATE ON _pgrdf_graph_count_deltas
    FOR EACH STATEMENT EXECUTE FUNCTION _refuse_count_write();
CREATE TRIGGER pgrdf_counts_row BEFORE INSERT OR UPDATE OR DELETE ON _pgrdf_graph_count_bypass
    FOR EACH ROW EXECUTE FUNCTION _refuse_count_write();
CREATE TRIGGER pgrdf_counts_truncate BEFORE TRUNCATE ON _pgrdf_graph_count_bypass
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

-- A fresh install holds the default graph only, and it is empty.
INSERT INTO _pgrdf_graph_counts (graph_id, scanned_at, scan_snapshot, scan_xact)
SELECT graph_id, pg_catalog.clock_timestamp(), pg_catalog.pg_current_snapshot(),
       pg_catalog.pg_current_xact_id()
  FROM _pgrdf_graphs;
"#,
    name = "graph_counts_v0_6_42",
    requires = [
        "schema_v0_4_0_graphs",
        "set_index_v0_6_41",
        _count_delta_guard,
        _count_bypass
    ],
);

/// What each graph gained in one engine call: asserted, inferred.
type Gains = BTreeMap<i64, (i64, i64)>;

thread_local! {
    /// How many engine quad statements are running in this backend.
    static WRITING: Cell<u32> = const { Cell::new(0) };
    /// The open tally: nesting depth and what each graph gained.
    static TALLY: RefCell<(u32, Gains)> =
        const { RefCell::new((0, BTreeMap::new())) };
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
            (t.0 == 0).then(|| std::mem::take(&mut t.1))
        });
        if let Some(gained) = done {
            if std::thread::panicking() {
                return;
            }
            for (graph_id, (asserted, inferred)) in gained {
                write_delta(graph_id, asserted, inferred);
            }
        }
    }
}

/// Record what one write changed in `graph_id`: into the open tally, or as
/// a delta at once when no tally is open.
pub(crate) fn gained(graph_id: i64, asserted: i64, inferred: i64) {
    if asserted == 0 && inferred == 0 {
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

/// Insert one delta, as the caller, through the guard.
fn write_delta(graph_id: i64, asserted: i64, inferred: i64) {
    if (asserted == 0 && inferred == 0) || !present() {
        return;
    }
    engine_write(|| {
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_graph_count_deltas (graph_id, d_asserted, d_inferred) \
             VALUES ($1, $2, $3)",
            &[graph_id.into(), asserted.into(), inferred.into()],
        )
        .expect("recording the graph count delta failed");
    });
    maybe_fold(graph_id);
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
pub(crate) fn fold(graph_id: i64) -> i64 {
    as_storage_owner(|| {
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
pub(crate) const BYPASS_APPLIES: &str = "(b.graph_id = c.graph_id OR b.graph_id IS NULL) \
     AND CASE WHEN b.xact = c.scan_xact THEN b.id > c.scan_mark \
              ELSE NOT pg_visible_in_snapshot(b.xact, c.scan_snapshot) END";

/// The count columns of base row `c` as a reader sees them: asserted,
/// inferred, change_seq, verified, scanned_at. For a FROM list that has
/// `pgrdf._pgrdf_graph_counts c`.
pub(crate) fn read_columns() -> String {
    "c.asserted + d.a, c.inferred + d.i, c.change_seq + d.n + y.n, \
     c.scanned_at IS NOT NULL AND y.n = 0, c.scanned_at"
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

/// The stored count of `graph_id`, or `None` when it has no base row (a
/// graph restored from a dump, or an id that was never registered).
/// Read as the storage owner: the caller needs no privilege on the tables.
pub(crate) fn stored(graph_id: i64) -> Option<Stored> {
    if !present() {
        return None;
    }
    crate::storage::partition::as_storage_owner_reading(|| {
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

/// The asserted and inferred rows of `graph_id`, counted.
pub(crate) fn scanned(graph_id: i64) -> (i64, i64) {
    Spi::connect(|c| {
        let t = c
            .select(
                "SELECT count(*) FILTER (WHERE NOT is_inferred), \
                        count(*) FILTER (WHERE is_inferred) \
                   FROM pgrdf._pgrdf_quads WHERE graph_id = $1",
                None,
                &[graph_id.into()],
            )
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
const SCAN_STAMP: &str = "change_seq = c.change_seq + (SELECT count(*) FROM gone) + 1 \
       + (SELECT count(*) FROM pgrdf._pgrdf_graph_count_bypass b \
           WHERE (b.graph_id = c.graph_id OR b.graph_id IS NULL) \
             AND CASE WHEN b.xact = c.scan_xact THEN b.id > c.scan_mark \
                      ELSE NOT pg_visible_in_snapshot(b.xact, c.scan_snapshot) END), \
     scanned_at = clock_timestamp(), \
     scan_snapshot = pg_current_snapshot(), scan_xact = pg_current_xact_id(), \
     scan_mark = nextval('pgrdf._pgrdf_graph_count_bypass_seq'), \
     folded_below = greatest(c.folded_below, pg_snapshot_xmin(pg_current_snapshot()))";

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
/// is dropped; what the call gains after this point still counts.
pub(crate) fn set_by_scan(graph_id: i64) -> (i64, i64) {
    if !present() {
        return scanned(graph_id);
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
                              n AS (SELECT count(*) FILTER (WHERE NOT is_inferred) AS a, \
                                           count(*) FILTER (WHERE is_inferred) AS i \
                                      FROM pgrdf._pgrdf_quads WHERE graph_id = $1) \
                         UPDATE pgrdf._pgrdf_graph_counts c \
                            SET asserted = n.a, inferred = n.i, \
                                {SCAN_STAMP} \
                           FROM n WHERE c.graph_id = $1 \
                         RETURNING c.asserted, c.inferred"
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
/// scanned by definition.
pub(crate) fn created(graph_id: i64) {
    if !present() {
        return;
    }
    as_storage_owner(|| {
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_graph_counts \
                    (graph_id, scanned_at, scan_snapshot, scan_xact, scan_mark, folded_below) \
             VALUES ($1, clock_timestamp(), pg_current_snapshot(), pg_current_xact_id(), \
                     nextval('pgrdf._pgrdf_graph_count_bypass_seq'), \
                     pg_snapshot_xmin(pg_current_snapshot())) \
             ON CONFLICT (graph_id) DO NOTHING",
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
                        {SCAN_STAMP} \
                  WHERE c.graph_id = $1"
            ),
            &[graph_id.into(), asserted.into(), inferred.into()],
        )
        .expect("resetting the graph count failed");
        forget_spent_bypasses();
    });
}

/// The number of dictionary terms the engine holds a count for, or
/// `None` when it holds none.
pub(crate) fn dictionary_terms() -> Option<i64> {
    None
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
