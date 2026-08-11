//! Engine-owned graph locks (#107, v0.6.28).
//!
//! Before this module the checkpoint "lock" lived in `pgrdf_mcp.ledger`
//! and was consulted only by the MCP server's own writes — measured:
//! `SELECT pgrdf.clear_graph(...)` emptied a LOCKED graph and the door
//! then refused the repair, leaving it locked and empty. The one
//! boundary that reported protection without enforcing it.
//!
//! Custody now lives here: lock state is three columns on
//! `_pgrdf_graphs`, and [`require_unlocked`] is called by **every**
//! engine write path — `clear_graph`, `drop_graph`, `move_graph`,
//! `copy_graph`/`carve_graph` (as destination), `put_quad`,
//! `put_construct_row(s)`, every `parse_*`/`load_*` ingest, and
//! `materialize` (it writes inferred rows). Reads are NEVER blocked —
//! a lock is a write fence, not a read fence.
//!
//! HONEST SCOPE (so #107 does not recur one level up): this lock is a
//! COORDINATION primitive, not a security boundary. Anyone who can
//! write the graph can lock or unlock it, with a mandatory reason both
//! ways. Security remains table grants — a lock that claimed to stop a
//! hostile writer would be the same over-promise #107 was filed about.

use pgrx::prelude::*;

// The DDL rides as an idempotent ALTER so the same block is correct on
// a fresh install (after the graphs table exists) and in the generated
// full-install SQL. The upgrade path ships the identical statements in
// sql/pgrdf--0.6.27--0.6.28.sql.
pgrx::extension_sql!(
    r#"
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS locked      BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS lock_reason TEXT;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS locked_at   TIMESTAMPTZ;
"#,
    name = "graph_lock_columns_v0_6_28",
    requires = ["schema_v0_4_0_graphs"],
);

/// The stable error prefix every refusal carries. Tests and callers
/// match on this; changing it is a contract change.
const LOCK_PREFIX: &str = "pgrdf: graph";

/// Refuse `verb` if `graph_id` is locked. Called at the top of every
/// engine write path. A graph with no `_pgrdf_graphs` row (e.g. the
/// implicit default graph 0 before any registration) cannot be locked
/// and passes.
pub(crate) fn require_unlocked(graph_id: i64, verb: &str) {
    let row = Spi::get_two_with_args::<bool, String>(
        "SELECT locked, COALESCE(lock_reason, 'checkpointed') \
         FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
        &[graph_id.into()],
    );
    if let Ok((Some(true), reason)) = row {
        let reason = reason.unwrap_or_else(|| "checkpointed".to_string());
        pgrx::error!(
            "{LOCK_PREFIX} {graph_id} is locked ({reason}): {verb} refused. \
             Unlock with pgrdf.unlock_graph({graph_id}, '<reason>')."
        );
    }
}

/// Lock a graph against every engine write path, with a mandatory
/// reason. Locking an already-locked graph refuses (explicit state
/// machine — re-locking silently would swallow the standing reason).
#[pg_extern]
fn lock_graph(graph_id: i64, reason: &str) -> bool {
    if reason.trim().is_empty() {
        pgrx::error!("lock_graph: a non-empty reason is required — the reason IS the record");
    }
    let existing = Spi::get_two_with_args::<bool, String>(
        "SELECT locked, COALESCE(lock_reason, '') FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
        &[graph_id.into()],
    );
    match existing {
        Ok((Some(true), prior)) => {
            let prior = prior.unwrap_or_default();
            pgrx::error!(
                "lock_graph: graph {graph_id} is already locked ({prior}). \
                 Unlock first — a silent re-lock would swallow the standing reason."
            );
        }
        Ok((Some(false), _)) => {}
        _ => pgrx::error!("lock_graph: no graph with id {graph_id} (see pgrdf._pgrdf_graphs)"),
    }
    Spi::run_with_args(
        "UPDATE pgrdf._pgrdf_graphs \
         SET locked = true, lock_reason = $2, locked_at = now() \
         WHERE graph_id = $1",
        &[graph_id.into(), reason.into()],
    )
    .expect("lock_graph: update failed");
    true
}

/// Unlock a graph, with a mandatory reason. Unlocking an unlocked
/// graph refuses — the caller's model of the state is wrong and that
/// is worth hearing about.
#[pg_extern]
fn unlock_graph(graph_id: i64, reason: &str) -> bool {
    if reason.trim().is_empty() {
        pgrx::error!("unlock_graph: a non-empty reason is required — the reason IS the record");
    }
    let existing = Spi::get_one_with_args::<bool>(
        "SELECT locked FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
        &[graph_id.into()],
    );
    match existing {
        Ok(Some(true)) => {}
        Ok(Some(false)) => {
            pgrx::error!("unlock_graph: graph {graph_id} is not locked — nothing to unlock")
        }
        _ => pgrx::error!("unlock_graph: no graph with id {graph_id} (see pgrdf._pgrdf_graphs)"),
    }
    Spi::run_with_args(
        "UPDATE pgrdf._pgrdf_graphs \
         SET locked = false, lock_reason = NULL, locked_at = NULL \
         WHERE graph_id = $1",
        &[graph_id.into(), reason.into()],
    )
    .expect("unlock_graph: update failed");
    // The unlock reason goes to the log — the row's reason column
    // belongs to the (now absent) lock.
    pgrx::log!("pgrdf: graph {graph_id} unlocked: {reason}");
    true
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    fn setup(gid: i64) {
        Spi::run(&format!("SELECT pgrdf.add_graph({gid})")).unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:l:s> <urn:l:p> \"v\" .', {gid})"
        ))
        .unwrap();
        Spi::run(&format!("SELECT pgrdf.lock_graph({gid}, 'test lock')")).unwrap();
    }

    /// Every write path refuses on a locked graph with the stable
    /// prefix. One test, all paths — a path missing from this list is
    /// a path the lock does not cover, which is #107 again.
    #[pg_test]
    fn lock_refuses_every_write_path() {
        setup(980_001);
        Spi::run("SELECT pgrdf.add_graph(980002)").unwrap(); // unlocked peer
        let iri = Spi::get_one::<String>("SELECT pgrdf.graph_iri(980001)")
            .unwrap()
            .unwrap();
        let writes: Vec<String> = vec![
            "SELECT pgrdf.clear_graph(980001)".into(),
            format!("SELECT pgrdf.clear_graph('{iri}')"),
            "SELECT pgrdf.drop_graph(980001)".into(),
            format!("SELECT pgrdf.drop_graph('{iri}')"),
            "SELECT pgrdf.move_graph(980002, 980001)".into(),
            "SELECT pgrdf.move_graph(980001, 980002)".into(), // src is cleared too
            "SELECT pgrdf.copy_graph(980002, 980001)".into(),
            "SELECT pgrdf.carve_graph(980002, 'urn:l:p', 980001)".into(),
            "SELECT pgrdf.put_quad(1, 1, 1, 980001)".into(),
            "SELECT pgrdf.put_construct_row('{\"s\":\"urn:l:s2\",\"p\":\"urn:l:p\",\"o\":\"urn:l:o\"}'::jsonb, 980001)".into(),
            "SELECT pgrdf.parse_turtle('<urn:l:s3> <urn:l:p> \"w\" .', 980001)".into(),
            "SELECT pgrdf.materialize(980001)".into(),
        ];
        // Each probe runs inside a PL/pgSQL exception block: the
        // subtransaction rolls back cleanly on the expected refusal,
        // so probe N+1 tests the LOCK and not an aborted transaction.
        // (catch_unwind here would leave SPI aborted after probe 1 and
        // every later assertion would pass for the wrong reason.)
        for sql in writes {
            let stmt = sql.replace('\'', "''");
            Spi::run(&format!(
                "DO $probe$ BEGIN \
                   EXECUTE '{stmt}'; \
                   RAISE EXCEPTION 'UNEXPECTED: write succeeded on a locked graph: %', '{stmt}'; \
                 EXCEPTION WHEN OTHERS THEN \
                   IF SQLERRM LIKE 'pgrdf: graph%is locked%' THEN NULL; \
                   ELSE RAISE; END IF; \
                 END $probe$"
            ))
            .unwrap_or_else(|e| panic!("probe failed for {sql}: {e}"));
        }
    }

    /// Reads are never blocked: SPARQL, counts and the integrity probe
    /// all answer against a locked graph. A lock is a write fence.
    #[pg_test]
    fn lock_never_blocks_reads() {
        setup(980_010);
        let n = Spi::get_one::<i64>("SELECT pgrdf.count_quads(980010)")
            .unwrap()
            .unwrap();
        assert_eq!(n, 1);
        let clean = Spi::get_one::<pgrx::JsonB>("SELECT pgrdf.graph_integrity(980010)")
            .unwrap()
            .unwrap();
        assert_eq!(clean.0["clean"], serde_json::json!(true));
    }

    /// unlock-with-reason restores every path; the state machine is
    /// explicit at both ends.
    #[pg_test]
    fn unlock_restores_writes() {
        setup(980_020);
        Spi::run("SELECT pgrdf.unlock_graph(980020, 'test done')").unwrap();
        Spi::run("SELECT pgrdf.parse_turtle('<urn:l:s4> <urn:l:p> \"x\" .', 980020)").unwrap();
        assert_eq!(
            Spi::get_one::<i64>("SELECT pgrdf.count_quads(980020)").unwrap(),
            Some(2)
        );
    }

    #[pg_test(error = "lock_graph: a non-empty reason is required — the reason IS the record")]
    fn lock_requires_reason() {
        Spi::run("SELECT pgrdf.add_graph(980030)").unwrap();
        Spi::run("SELECT pgrdf.lock_graph(980030, '  ')").unwrap();
    }

    #[pg_test(error = "unlock_graph: graph 980031 is not locked — nothing to unlock")]
    fn unlock_unlocked_refuses() {
        Spi::run("SELECT pgrdf.add_graph(980031)").unwrap();
        Spi::run("SELECT pgrdf.unlock_graph(980031, 'why')").unwrap();
    }

    #[pg_test]
    fn double_lock_refuses_and_keeps_reason() {
        Spi::run("SELECT pgrdf.add_graph(980032)").unwrap();
        Spi::run("SELECT pgrdf.lock_graph(980032, 'first holder')").unwrap();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            Spi::run("SELECT pgrdf.lock_graph(980032, 'second holder')").unwrap();
        }));
        assert!(r.is_err(), "double lock must refuse");
    }
}
