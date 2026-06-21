//! R2.0 — the dynamic background-worker POOL: spawn discipline, the worker entry point, and a
//! minimal "ping" coordinator that proves the machinery end-to-end.
//!
//! This is the pool skeleton from `_WIP/SPEC.STAGED-LOADER-R2.bgworker-design.md` §6/§7. It does
//! NOT yet run the real STAGE/DICT/RESOLVE/INDEX SQL — that lands in R2.1. What it proves here is
//! the load-bearing, easy-to-get-wrong part: that a SQL-callable coordinator can spawn N dynamic
//! background workers, each its OWN backend running its OWN committed transaction, detect that all N
//! ran, and survive pool exhaustion without crashing the postmaster.
//!
//! ## Two decisions worth reading before the code
//!
//! **How a worker reaches the right database.** A dynamic bgworker does NOT inherit the spawner's
//! database (`bgworkers.rs::load_dynamic` just registers a worker; it has no DB context). The
//! coordinator records its own `pg_sys::MyDatabaseId` into the [`jobctl::JobSlot`]; the worker reads
//! it back from shmem and calls `connect_worker_to_spi_by_oid(Some(db_oid), None)`. We use the OID
//! form (not the name form) deliberately — an OID is a fixed-width `u32` that already lives in the
//! shmem slot, so there is no path-style "name longer than the inline byte array" truncation risk
//! (§3 risk #3b / §6 note).
//!
//! **How the ping table avoids a self-deadlock.** The coordinator's `#[pg_extern]` body runs inside
//! the CALLER's single, still-open transaction (pgrx 0.16 functions can't COMMIT — §1.2). The
//! workers run in SEPARATE backends with their OWN snapshots, so they can't see anything the
//! coordinator's uncommitted transaction wrote, and — critically — if the coordinator held
//! `TRUNCATE`'s `ACCESS EXCLUSIVE` lock on the ping table while blocked in `wait_for_shutdown()`,
//! the workers' `INSERT`s would block on that lock forever: a deadlock (parent waits on workers,
//! workers wait on the parent's lock). So the ping table is shipped in the install SQL (committed at
//! `CREATE EXTENSION`, visible to every backend) and the coordinator takes NO conflicting lock on
//! it. Each worker `INSERT`s a row tagged with this run's `job_id`; the coordinator counts only
//! `WHERE job_id = <this job>`, so prior runs don't pollute the count and no `TRUNCATE` is needed.
//! `ping_rows` (an SPI `COUNT` the coordinator reads after all workers committed, under the caller's
//! default `READ COMMITTED` snapshot) is therefore an INDEPENDENT, in-table proof that each worker
//! committed — distinct from `succeeded`, which is the shmem `WorkerSlot.status` flag.

use crate::storage::staged::jobctl;
use pgrx::bgworkers::{
    BackgroundWorker, BackgroundWorkerBuilder, BackgroundWorkerStatus, DynamicBackgroundWorker,
    SignalWakeFlags,
};
use pgrx::prelude::*;
use serde_json::json;

/// The exported symbol Postgres dlsym's as each worker's `main`. MUST match
/// `set_function(...)` below and be `#[pg_guard] extern "C-unwind" fn(Datum)` returning void.
const WORKER_FN: &str = "pgrdf_staged_worker_main";
/// This extension's library name — the `.so`/`.dylib` Postgres loads the worker symbol from.
const LIBRARY: &str = "pgrdf";

/// Build + register ONE dynamic background worker bound to [`WORKER_FN`], passing it `worker_slot`
/// (its [`jobctl::WorkerSlot`] index) as the single `Datum` argument. Returns the live handle or a
/// clear `String` on failure.
///
/// This is the single `spawn_checked` chokepoint mandated by §7/§8 risk #1 so the two
/// correctness-critical invariants can never be forgotten at a call site:
///
/// * **`set_notify_pid(MyProcPid)` is ALWAYS set** — without it `wait_for_startup`/`wait_for_shutdown`
///   return `Err(Untracked)` and the coordinator can't track the worker at all.
/// * **`load_dynamic()`'s `Result` is ALWAYS matched, never `.unwrap()`ed** — an ignored `Err`
///   (pool exhausted: `max_worker_processes` reached) was the historical #1417 null-handle segfault.
///   In 0.16.1 it's a clean `Err`, but only if we don't `.unwrap()`. On `Err` we return a `String`;
///   the coordinator counts it as a spawn failure and keeps going (graceful pool-exhaustion path).
///
/// `set_restart_time(None)` ⇒ a crashed worker is NOT auto-respawned (deterministic fail-fast, not a
/// thrashing restart loop). `enable_spi_access()` ⇒ the worker may use SPI and starts at
/// `RecoveryFinished`.
pub fn spawn_checked(name: &str, worker_slot: usize) -> Result<DynamicBackgroundWorker, String> {
    let arg = (worker_slot as i32)
        .into_datum()
        .ok_or_else(|| "spawn_checked: failed to encode worker slot index as Datum".to_string())?;
    let notify_pid = unsafe { pg_sys::MyProcPid };
    BackgroundWorkerBuilder::new(name)
        .set_library(LIBRARY)
        .set_function(WORKER_FN)
        .set_argument(Some(arg))
        .set_notify_pid(notify_pid)
        .enable_spi_access()
        .set_restart_time(None)
        .load_dynamic()
        .map_err(|_| {
            format!(
                "spawn_checked: load_dynamic failed for slot {worker_slot} \
                 (max_worker_processes likely exhausted)"
            )
        })
}

/// Worker entry point. One per spawned backend; the `arg` Datum is its [`jobctl::WorkerSlot`] index.
///
/// Lifecycle (§6 template, minimal phase): read slot → attach signal handlers → connect to the
/// coordinator's DB (by the OID recorded in the job) → run ONE committed transaction that INSERTs a
/// marker row → report success/failure into shmem → return. Returning makes the backend exit, which
/// unblocks the coordinator's `wait_for_shutdown()`.
///
/// The whole DB-touching body is wrapped in `std::panic::catch_unwind` so a Rust panic becomes a
/// recorded failure (`WorkerSlot.status = error` + message) instead of an uncaught unwind across the
/// FFI boundary. The outcome is reported via shmem, NOT the exit code — a worker that `ereport`s
/// ERROR still "stops", so the parent's `wait_for_shutdown` returns `Ok(())` regardless (§7).
#[pg_guard]
pub extern "C-unwind" fn pgrdf_staged_worker_main(arg: pg_sys::Datum) {
    let slot =
        unsafe { i32::from_datum(arg, false) }.expect("staged worker: missing slot arg") as usize;

    BackgroundWorker::attach_signal_handlers(SignalWakeFlags::SIGTERM | SignalWakeFlags::SIGHUP);

    // A dynamic worker has no inherited DB; reconnect to the one the coordinator ran in.
    let job = jobctl::read_worker(slot);
    let dbjob = jobctl::read_job(job.job_idx as usize);
    let db_oid = pg_sys::Oid::from_u32(dbjob.db_oid);
    BackgroundWorker::connect_worker_to_spi_by_oid(Some(db_oid), None);

    let job_id = dbjob.job_id;
    let pid = unsafe { pg_sys::MyProcPid };

    // ONE committed transaction = this worker's recovery point. `BackgroundWorker::transaction`
    // begins/commits the xact and runs the body under `PgTryBuilder`, so a SQL ERROR inside is
    // caught and surfaced as a panic to `catch_unwind` rather than longjmp'ing past us.
    let result = std::panic::catch_unwind(|| {
        BackgroundWorker::transaction(|| {
            Spi::run_with_args(
                "INSERT INTO pgrdf._pgrdf_staged_ping (job_id, worker_slot, pid) VALUES ($1, $2, $3)",
                &[job_id.into(), (slot as i64).into(), (pid as i64).into()],
            )
            .expect("staged worker: ping INSERT failed");
        })
    });

    let (ok, err) = match result {
        Ok(()) => (true, String::new()),
        Err(panic) => {
            let msg = panic
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| panic.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "staged worker: unknown panic".to_string());
            (false, msg)
        }
    };
    jobctl::report_worker(slot, ok, &err);
    // Returning exits the backend → coordinator's wait_for_shutdown() unblocks.
}

/// TEST-ONLY coordinator for R2.0 — proves the bgworker pool runs end-to-end. The real
/// `load_turtle_staged_run` (which drives the STAGE/DICT/RESOLVE/INDEX pipeline) lands in R2.1.
///
/// Spawns `n_workers` dynamic background workers, each of which commits one marker row into
/// `pgrdf._pgrdf_staged_ping`, then waits for all of them and reports what happened as JSONB:
/// `{job_id, spawned, succeeded, failed, ping_rows}`.
///
/// * `spawned`   — workers that `load_dynamic()` accepted (≤ `n_workers`; fewer ⇒ pool exhausted).
/// * `succeeded` / `failed` — from each `WorkerSlot.status` in shmem (the authoritative outcome).
/// * `ping_rows` — rows this job actually committed to the table (SPI `COUNT … WHERE job_id = …`),
///   the independent in-table proof of commit. On the happy path `succeeded == ping_rows == spawned`.
///
/// Refuses with a clear, user-actionable error (NOT a panic) when pgRDF isn't in
/// `shared_preload_libraries`, since the worker pool needs the shmem job segment.
///
/// SQL: `pgrdf.load_turtle_staged_ping(n_workers INT) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn load_turtle_staged_ping(n_workers: i32) -> pgrx::JsonB {
    if !jobctl::is_ready() {
        error!(
            "pgrdf staged loader requires pgrdf in shared_preload_libraries \
             (the worker pool needs the shmem job-control segment)"
        );
    }
    let want = n_workers.max(0) as usize;

    // The ping table is shipped in the install SQL (committed at CREATE EXTENSION). The coordinator
    // takes NO conflicting lock on it (see module docs: TRUNCATE here would deadlock against
    // wait_for_shutdown). Each worker's row carries this run's job_id so the count is isolated.
    let db_oid: u32 = unsafe { pg_sys::MyDatabaseId }.to_u32();
    let job_idx = jobctl::create_job("<ping>", 0, db_oid, want as u16, 0)
        .unwrap_or_else(|| error!("pgrdf staged loader: no free job slot (MAX_JOBS reached)"));
    let job_id = jobctl::job_id_of(job_idx);

    // Spawn every worker, honouring each load_dynamic Result (§7). A spawn failure (pool exhausted)
    // is recorded and we keep going — partial result, never a crash.
    let mut handles: Vec<DynamicBackgroundWorker> = Vec::with_capacity(want);
    let mut spawn_failures = 0usize;
    for i in 0..want {
        let wslot = match jobctl::claim_worker_slot(job_idx, jobctl::phase::STAGE, i as u16) {
            Some(s) => s,
            None => {
                spawn_failures += 1;
                continue;
            }
        };
        let name = format!("pgrdf:ping:job={job_id}:w={i}");
        match spawn_checked(&name, wslot) {
            Ok(h) => handles.push(h),
            Err(_) => {
                // Spawn rejected (max_worker_processes). Mark the claimed slot failed so the
                // succeeded/failed tally stays consistent, and account it as a spawn failure.
                jobctl::report_worker(wslot, false, "load_dynamic rejected (pool exhausted)");
                spawn_failures += 1;
            }
        }
    }
    let spawned = handles.len();

    // Wait for startup then shutdown on every spawned handle, honouring the Results (§7). A worker
    // that errored still STOPS, so wait_for_shutdown returns Ok — the real outcome is read from
    // shmem afterwards. wait_for_startup Err (never started) is itself counted as a failed worker.
    let mut startup_failures = 0usize;
    let mut postmaster_died = false;
    let mut started: Vec<DynamicBackgroundWorker> = Vec::with_capacity(spawned);
    for h in handles {
        // wait_for_startup borrows &self, so h is still owned afterwards and can move into `started`.
        match h.wait_for_startup() {
            Ok(_pid) => started.push(h),
            Err(BackgroundWorkerStatus::PostmasterDied) => {
                postmaster_died = true;
            }
            Err(_status) => {
                startup_failures += 1;
            }
        }
    }
    for h in started {
        match h.wait_for_shutdown() {
            Ok(()) => {}
            Err(BackgroundWorkerStatus::PostmasterDied) => {
                postmaster_died = true;
            }
            Err(_status) => {
                // Untracked/other: the worker still reports via shmem; nothing to add here.
            }
        }
    }

    // Tally outcomes from shmem (authoritative): status 1 = ok, 2 = error. A worker that started
    // then vanished without reporting stays 0 and counts as neither — startup_failures covers the
    // "never started" case separately.
    let (succeeded, mut failed) = jobctl::tally_job(job_idx);
    failed += startup_failures;

    // ping_rows: rows this job committed. Read AFTER all workers exited; under the caller's default
    // READ COMMITTED snapshot this SPI count sees their committed inserts — the end-to-end proof.
    let ping_rows: i64 = Spi::get_one_with_args::<i64>(
        "SELECT count(*) FROM pgrdf._pgrdf_staged_ping WHERE job_id = $1",
        &[job_id.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0);

    let first_err = jobctl::job_err(job_idx);
    if failed == 0 && spawn_failures == 0 && !postmaster_died {
        jobctl::mark_job_done(job_idx);
    }
    jobctl::release_job(job_idx);

    pgrx::JsonB(json!({
        "job_id": job_id,
        "requested": want,
        "spawned": spawned,
        "succeeded": succeeded,
        "failed": failed,
        "spawn_failures": spawn_failures,
        "ping_rows": ping_rows,
        "postmaster_died": postmaster_died,
        "error": if first_err.is_empty() { serde_json::Value::Null } else { json!(first_err) },
    }))
}
