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

// F5 (SPEC 0.6.37, #142): lock custody below the engine. `lock_graph`
// installs two triggers on the graph's own partition that call this
// function, so a direct SQL write — through the parent or on the
// partition, row-level or TRUNCATE — refuses exactly like the engine
// fence. Unlocked partitions carry no trigger, so ordinary writes pay
// nothing. The graph id and reason travel as trigger arguments: no table
// read at fire time, so the refusal needs no privilege the writer lacks.
// Message shape and HINT match `require_unlocked`.
pgrx::extension_sql!(
    r#"
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
"#,
    name = "lock_refuse_trigger_v0_6_37",
    requires = ["graph_lock_columns_v0_6_28"],
);

// 0.6.37: the rdfc-1.0 digest of a locked graph, cached by graph_digest on
// first computation and cleared by lock_graph / unlock_graph. Valid only
// while the graph is locked and its lock custody holds.
pgrx::extension_sql!(
    r#"
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS locked_digest TEXT;
"#,
    name = "graph_locked_digest_v0_6_37",
    requires = ["graph_lock_columns_v0_6_28"],
);

/// The partition that holds `graph_id`'s quads, when it has its own.
fn dedicated_partition(graph_id: i64) -> Option<String> {
    let name = format!("_pgrdf_quads_g{graph_id}");
    let exists = Spi::get_one_with_args::<bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_class \
         WHERE relnamespace = 'pgrdf'::regnamespace AND relname = $1)",
        &[name.as_str().into()],
    )
    .expect("lock: partition lookup failed")
    .unwrap_or(false);
    exists.then_some(name)
}

/// Install the refuse-triggers on the graph's partition (as the storage
/// owner — trigger DDL needs table ownership). A graph without a
/// dedicated partition keeps the engine fence only.
fn install_lock_triggers(graph_id: i64, reason: &str) {
    let Some(part) = dedicated_partition(graph_id) else {
        return;
    };
    let reason = reason.replace('\'', "''");
    crate::storage::partition::as_storage_owner(|| {
        Spi::run(&format!(
            "CREATE TRIGGER pgrdf_lock_row BEFORE INSERT OR UPDATE OR DELETE \
             ON pgrdf.{part} FOR EACH ROW \
             EXECUTE FUNCTION pgrdf._refuse_locked_write('{graph_id}', '{reason}')"
        ))
        .expect("lock_graph: row trigger install failed");
        Spi::run(&format!(
            "CREATE TRIGGER pgrdf_lock_truncate BEFORE TRUNCATE \
             ON pgrdf.{part} FOR EACH STATEMENT \
             EXECUTE FUNCTION pgrdf._refuse_locked_write('{graph_id}', '{reason}')"
        ))
        .expect("lock_graph: truncate trigger install failed");
    });
}

fn drop_lock_triggers(graph_id: i64) {
    let Some(part) = dedicated_partition(graph_id) else {
        return;
    };
    crate::storage::partition::as_storage_owner(|| {
        Spi::run(&format!(
            "DROP TRIGGER IF EXISTS pgrdf_lock_row ON pgrdf.{part}"
        ))
        .expect("unlock_graph: row trigger drop failed");
        Spi::run(&format!(
            "DROP TRIGGER IF EXISTS pgrdf_lock_truncate ON pgrdf.{part}"
        ))
        .expect("unlock_graph: truncate trigger drop failed");
    });
}

/// How many of the two lock triggers the graph's partition carries
/// (None = no dedicated partition). graph_integrity compares it with
/// the `locked` flag.
pub(crate) fn lock_trigger_count(graph_id: i64) -> Option<i64> {
    let part = dedicated_partition(graph_id)?;
    Spi::get_one_with_args::<i64>(
        "SELECT count(*) FROM pg_trigger \
         WHERE tgrelid = format('pgrdf.%I', $1::text)::regclass \
           AND tgname IN ('pgrdf_lock_row', 'pgrdf_lock_truncate')",
        &[part.as_str().into()],
    )
    .expect("lock: trigger count failed")
}

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
        // E0 (SPEC.pgRDF.LIB.v0.6.34): a deliberate refusal is a RESULT and
        // must not reach clients as XX000 internal_error. 55P03
        // lock_not_available is the standard class for "declined because a
        // lock is held". Mechanism unchanged (see crate::refuse); message
        // byte-identical to the pre-E0 text (K2).
        // 0.6.37: the same HINT the partition trigger raises — one cure,
        // whichever path refused.
        crate::refuse_with_hint(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_LOCK_NOT_AVAILABLE,
            format!(
                "{LOCK_PREFIX} {graph_id} is locked ({reason}): {verb} refused. \
                 Unlock with pgrdf.unlock_graph({graph_id}, '<reason>')."
            ),
            format!("pgrdf.unlock_graph({graph_id}, '<reason>')"),
        );
    }
}

/// Lock a graph against every engine write path, with a mandatory
/// reason. Locking an already-locked graph refuses (explicit state
/// machine — re-locking silently would swallow the standing reason).
#[pg_extern]
fn lock_graph(graph_id: i64, reason: &str) -> bool {
    if reason.trim().is_empty() {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
            "lock_graph: a non-empty reason is required — the reason IS the record".to_string(),
        );
    }
    let existing = Spi::get_two_with_args::<bool, String>(
        "SELECT locked, COALESCE(lock_reason, '') FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
        &[graph_id.into()],
    );
    match existing {
        Ok((Some(true), prior)) => {
            let prior = prior.unwrap_or_default();
            crate::refuse(
                pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_LOCK_NOT_AVAILABLE,
                format!(
                    "lock_graph: graph {graph_id} is already locked ({prior}). \
                     Unlock first — a silent re-lock would swallow the standing reason."
                ),
            );
        }
        Ok((Some(false), _)) => {}
        _ => crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
            format!("lock_graph: no graph with id {graph_id} (see pgrdf._pgrdf_graphs)"),
        ),
    }
    Spi::run_with_args(
        "UPDATE pgrdf._pgrdf_graphs \
         SET locked = true, lock_reason = $2, locked_at = now(), locked_digest = NULL \
         WHERE graph_id = $1",
        &[graph_id.into(), reason.into()],
    )
    .expect("lock_graph: update failed");
    // After the caller's own UPDATE succeeded — so locking still needs
    // UPDATE on _pgrdf_graphs — extend the lock below the engine.
    install_lock_triggers(graph_id, reason);
    true
}

/// Unlock a graph, with a mandatory reason. Unlocking an unlocked
/// graph refuses — the caller's model of the state is wrong and that
/// is worth hearing about.
#[pg_extern]
fn unlock_graph(graph_id: i64, reason: &str) -> bool {
    if reason.trim().is_empty() {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
            "unlock_graph: a non-empty reason is required — the reason IS the record".to_string(),
        );
    }
    let existing = Spi::get_one_with_args::<bool>(
        "SELECT locked FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
        &[graph_id.into()],
    );
    match existing {
        Ok(Some(true)) => {}
        Ok(Some(false)) => crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_OBJECT_NOT_IN_PREREQUISITE_STATE,
            format!("unlock_graph: graph {graph_id} is not locked — nothing to unlock"),
        ),
        _ => crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
            format!("unlock_graph: no graph with id {graph_id} (see pgrdf._pgrdf_graphs)"),
        ),
    }
    Spi::run_with_args(
        "UPDATE pgrdf._pgrdf_graphs \
         SET locked = false, lock_reason = NULL, locked_at = NULL, locked_digest = NULL \
         WHERE graph_id = $1",
        &[graph_id.into(), reason.into()],
    )
    .expect("unlock_graph: update failed");
    drop_lock_triggers(graph_id);
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

    /// E0 negative control (LIB K10): the lock refusal carries
    /// 55P03 `lock_not_available` — asserted by ENUM, never by message
    /// (asserting prose is the L6 defect this line of work retires).
    /// A gate nobody has watched raise its code is unproven.
    ///
    /// No SPI after the catch: the transaction is aborted at that point
    /// (the 0.6.28 vacuity lesson, same as the #114 test).
    #[pg_test]
    fn lock_refusal_carries_lock_not_available() {
        use pgrx::pg_sys::errcodes::PgSqlErrorCode;
        use pgrx::pg_sys::panic::CaughtError;
        setup(980_040);
        let code = pgrx::PgTryBuilder::new(|| {
            Spi::run("SELECT pgrdf.clear_graph(980040)").unwrap();
            None
        })
        .catch_others(|e| match &e {
            CaughtError::PostgresError(r)
            | CaughtError::ErrorReport(r)
            | CaughtError::RustPanic { ereport: r, .. } => Some(r.sql_error_code()),
        })
        .execute();
        assert_eq!(
            code,
            Some(PgSqlErrorCode::ERRCODE_LOCK_NOT_AVAILABLE),
            "lock refusal must reach callers as 55P03 lock_not_available, not XX000"
        );
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

    /// Every direct write is tried inside a plpgsql EXCEPTION block — a
    /// real subtransaction — so one refusal never aborts the test's
    /// transaction. Returns "ok" or "<sqlstate>|<hint>".
    fn f5_try_fn() {
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.f5_try(q text) RETURNS text \
             LANGUAGE plpgsql AS $$ DECLARE st text; h text; BEGIN \
               EXECUTE q; RETURN 'ok'; \
             EXCEPTION WHEN OTHERS THEN \
               GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, h = PG_EXCEPTION_HINT; \
               RETURN st || '|' || coalesce(h, ''); END $$",
        )
        .unwrap();
    }

    fn f5_try(q: &str) -> String {
        Spi::get_one_with_args("SELECT pg_temp.f5_try($1)", &[q.into()])
            .unwrap()
            .unwrap()
    }

    /// A writer granted on the parent before the graph exists, so the
    /// partition inherits the grants (#96) — the realistic shape.
    fn f5_writer_and_locked_graph(role: &str, gid: i64) {
        crate::storage::partition::acquire_partition_ddl_gate();
        Spi::run(&format!("CREATE ROLE {role} NOLOGIN")).unwrap();
        Spi::run(&format!("GRANT USAGE ON SCHEMA pgrdf TO {role}")).unwrap();
        Spi::run(&format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE, TRUNCATE ON pgrdf._pgrdf_quads TO {role}"
        ))
        .unwrap();
        setup(gid); // loads one triple and locks with reason 'test lock'
        f5_try_fn();
    }

    /// F5 (#142): a locked graph refuses EVERY direct SQL write — through
    /// the parent, on the partition, and TRUNCATE — with the lock's
    /// SQLSTATE and the unlock cure as HINT.
    #[pg_test]
    fn locked_graph_refuses_direct_sql_writes() {
        let gid = 980501;
        f5_writer_and_locked_graph("pgrdf_f5_writer", gid);
        let before: i64 = Spi::get_one(&format!(
            "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = {gid}"
        ))
        .unwrap()
        .unwrap();
        Spi::run("SET ROLE pgrdf_f5_writer").unwrap();
        let part = format!("pgrdf._pgrdf_quads_g{gid}");
        for q in [
            format!("INSERT INTO pgrdf._pgrdf_quads VALUES (1, 2, 3, {gid}, false)"),
            format!("INSERT INTO {part} VALUES (1, 2, 3, {gid}, false)"),
            format!("DELETE FROM pgrdf._pgrdf_quads WHERE graph_id = {gid}"),
            format!("UPDATE pgrdf._pgrdf_quads SET is_inferred = true WHERE graph_id = {gid}"),
            format!("TRUNCATE ONLY {part}"),
        ] {
            let got = f5_try(&q);
            assert_eq!(
                got,
                format!("55P03|pgrdf.unlock_graph({gid}, '<reason>')"),
                "{q}"
            );
        }
        Spi::run("RESET ROLE").unwrap();
        // TRUNCATE of the parent reaches the locked partition too.
        assert!(f5_try("TRUNCATE pgrdf._pgrdf_quads").starts_with("55P03|"));
        let after: i64 = Spi::get_one(&format!(
            "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = {gid}"
        ))
        .unwrap()
        .unwrap();
        assert_eq!(before, after, "nothing landed");
    }

    /// F5: unlock removes the triggers and direct writes land again; an
    /// unlocked graph never carries one (the zero-cost path).
    #[pg_test]
    fn unlock_restores_writes_and_leaves_no_trigger() {
        let gid = 980502;
        f5_writer_and_locked_graph("pgrdf_f5_writer2", gid);
        Spi::run(&format!("SELECT pgrdf.unlock_graph({gid}, 'f5 done')")).unwrap();
        let triggers: i64 = Spi::get_one(&format!(
            "SELECT count(*) FROM pg_trigger \
             WHERE tgrelid = 'pgrdf._pgrdf_quads_g{gid}'::regclass AND NOT tgisinternal"
        ))
        .unwrap()
        .unwrap();
        assert_eq!(triggers, 0, "an unlocked graph carries no trigger");
        Spi::run("SET ROLE pgrdf_f5_writer2").unwrap();
        assert_eq!(
            f5_try(&format!(
                "INSERT INTO pgrdf._pgrdf_quads VALUES (7, 8, 9, {gid}, false)"
            )),
            "ok"
        );
        Spi::run("RESET ROLE").unwrap();
    }

    /// F5: the engine fence and the trigger share one cure — the engine
    /// refusal keeps its byte-identical message and gains the same HINT.
    #[pg_test]
    fn engine_fence_carries_the_same_hint() {
        let gid = 980503;
        f5_writer_and_locked_graph("pgrdf_f5_writer3", gid);
        let got = f5_try(&format!("SELECT pgrdf.clear_graph({gid})"));
        assert_eq!(got, format!("55P03|pgrdf.unlock_graph({gid}, '<reason>')"));
    }

    /// F5: lock custody joins graph_integrity — a locked graph whose
    /// triggers were removed by hand is drift, and integrity says so.
    #[pg_test]
    fn integrity_reports_lock_trigger_drift() {
        let gid = 980504;
        f5_writer_and_locked_graph("pgrdf_f5_writer4", gid);
        let clean: bool = Spi::get_one(&format!(
            "SELECT (pgrdf.graph_integrity({gid})->>'clean')::bool"
        ))
        .unwrap()
        .unwrap();
        assert!(clean, "a locked graph with its triggers is clean");
        Spi::run(&format!(
            "DROP TRIGGER pgrdf_lock_row ON pgrdf._pgrdf_quads_g{gid}"
        ))
        .unwrap();
        let (clean, consistent): (Option<bool>, Option<bool>) = Spi::get_two(&format!(
            "SELECT (pgrdf.graph_integrity({gid})->>'clean')::bool, \
                    (pgrdf.graph_integrity({gid})->'lock_custody'->>'consistent')::bool"
        ))
        .unwrap();
        assert_eq!(consistent, Some(false), "a missing trigger is drift");
        assert_eq!(clean, Some(false), "drift makes the graph unclean");
    }

    /// F5 closes the documented v0.6.28 limitation: a graph named INSIDE
    /// a TriG payload was never lock-checked. Its insert now meets the
    /// partition trigger, so a locked graph refuses it like any write.
    #[pg_test]
    fn locked_graph_named_inside_a_trig_payload_refuses() {
        let gid = 980505;
        setup(gid); // locked, reason 'test lock'
        f5_try_fn();
        let iri: String = Spi::get_one(&format!("SELECT pgrdf.graph_iri({gid})"))
            .unwrap()
            .unwrap();
        let target: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:f5:trig-target')")
            .unwrap()
            .unwrap();
        let payload = format!("<{iri}> {{ <urn:l:x> <urn:l:p> <urn:l:y> . }}");
        let got = f5_try(&format!(
            "SELECT pgrdf.parse_trig({}, {target})",
            Spi::get_one_with_args::<String>("SELECT quote_literal($1)", &[payload.into()])
                .unwrap()
                .unwrap()
        ));
        assert!(
            got.starts_with("55P03|"),
            "the payload-named locked graph refuses: {got}"
        );
    }
}
