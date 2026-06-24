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

use crate::storage::staged::{jobctl, phases};
use pgrx::bgworkers::{
    BackgroundWorker, BackgroundWorkerBuilder, BackgroundWorkerStatus, DynamicBackgroundWorker,
    SignalWakeFlags,
};
use pgrx::prelude::*;
use serde_json::json;
use std::time::Instant;

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
///
/// `#[no_mangle]` is MANDATORY here: the postmaster resolves this worker by the *string* name
/// passed to `set_function("pgrdf_staged_worker_main")` via `dlsym`, so the symbol must be exported
/// UNMANGLED and under exactly that name. Without it Rust both name-mangles the symbol AND
/// dead-code-eliminates it (nothing references the function by Rust path — only by string), so the
/// postmaster's launcher fails with `could not find function "pgrdf_staged_worker_main" in
/// pgrdf.so` and every worker exits code 1 before its body runs (observed on E160).
#[no_mangle]
#[pg_guard]
pub extern "C-unwind" fn pgrdf_staged_worker_main(arg: pg_sys::Datum) {
    let slot =
        unsafe { i32::from_datum(arg, false) }.expect("staged worker: missing slot arg") as usize;

    BackgroundWorker::attach_signal_handlers(SignalWakeFlags::SIGTERM | SignalWakeFlags::SIGHUP);

    // A dynamic worker has no inherited DB; reconnect to the one the coordinator ran in.
    let w = jobctl::read_worker(slot);
    let job = jobctl::read_job(w.job_idx as usize);
    let db_oid = pg_sys::Oid::from_u32(job.db_oid);
    BackgroundWorker::connect_worker_to_spi_by_oid(Some(db_oid), None);

    let job_id = job.job_id;
    let pid = unsafe { pg_sys::MyProcPid };

    // ONE committed transaction = this worker's recovery point. `BackgroundWorker::transaction`
    // begins/commits the xact and runs the body under `PgTryBuilder`, so a SQL ERROR inside is
    // caught and surfaced as a panic to `catch_unwind` rather than longjmp'ing past us. The body
    // dispatches on the worker's phase — the R2.1 STAGE/DICT/RESOLVE/INDEX bodies, or the R2.0 PING
    // marker INSERT kept as a standalone pool-machinery regression.
    let result = std::panic::catch_unwind(|| {
        BackgroundWorker::transaction(|| match w.phase {
            jobctl::phase::PING => {
                Spi::run_with_args(
                    "INSERT INTO pgrdf._pgrdf_staged_ping (job_id, worker_slot, pid) VALUES ($1, $2, $3)",
                    &[job_id.into(), (slot as i64).into(), (pid as i64).into()],
                )
                .expect("staged worker: ping INSERT failed");
            }
            jobctl::phase::STAGE_PREP => {
                phases::apply_session_gucs();
                // T3: Phase-A prep (defer indexes, create the UNLOGGED staging table) is its OWN
                // single worker, run BEFORE the N parallel STAGE workers. It runs HERE in a committed
                // worker transaction — never the coordinator's — so its ACCESS EXCLUSIVE locks release
                // before STAGE/DICT/RESOLVE/INDEX run (see `phases::prepare_for_load`). Splitting prep
                // out is what lets STAGE go N-way without the `bulk_drop_indexes` / `CREATE TABLE IF
                // NOT EXISTS` DDL racing across the pool.
                phases::prepare_for_load(&job);
            }
            jobctl::phase::STAGE => {
                phases::apply_session_gucs();
                // T3: a pure STAGE worker — prep already ran in the STAGE_PREP worker, so this one only
                // parses its newline-snapped byte range and COPYs into the SHARED staging table. N of
                // these run concurrently, each its own COPY-issuing backend (the multi-backend win).
                let _ = phases::stage(&job, &w);
            }
            jobctl::phase::DICT => {
                phases::apply_session_gucs();
                let _ = phases::dict(&job, &w);
            }
            jobctl::phase::RESOLVE => {
                phases::apply_session_gucs();
                let _ = phases::resolve(&job, &w);
            }
            jobctl::phase::INDEX => {
                phases::apply_session_gucs();
                phases::build_index(&job, &w);
            }
            other => panic!("staged worker: unknown phase ordinal {other}"),
        })
    });

    let (ok, err) = match result {
        Ok(()) => (true, String::new()),
        // Recover the REAL failure message from the panic payload (see `panic_message`). The most
        // diagnostically important case is a SQL ERROR (e.g. the 8.2 B-row RESOLVE failure): inside
        // `BackgroundWorker::transaction`'s `PgTryBuilder` a Postgres ERROR is re-raised as a Rust
        // panic carrying a pgrx `ErrorReportWithLevel`, NOT a `&str`/`String`, so the old
        // `&str`/`String`-only downcast always fell through to "unknown panic" — losing the message.
        Err(panic) => (false, panic_message(panic, w.phase, w.shard, pid)),
    };
    jobctl::report_worker(slot, ok, &err);
    // Returning exits the backend → coordinator's wait_for_shutdown() unblocks.
}

/// Recover the most specific human-readable message from a worker's caught panic payload, so the
/// coordinator surfaces the REAL cause (into the `JobSlot` error string → `ingest.json`'s `error`)
/// instead of the opaque `"staged worker: unknown panic"` an 8.2 B-row RESOLVE failure produced.
///
/// The payload type depends on HOW the worker died, and the order here matters (try the richest
/// pgrx error types first, since they carry the actual `ereport`/SPI message):
///
/// * **A Postgres/SQL ERROR** — a SPI failure, `error!()`, an OOM the planner surfaces, etc. Inside
///   `BackgroundWorker::transaction` (→ pgrx `PgTryBuilder`) such an ERROR is re-raised as a Rust
///   panic via `panic_any(ErrorReportWithLevel)` (pgrx `submodules/panic.rs`). So the payload is a
///   [`pgrx::pg_sys::panic::ErrorReportWithLevel`] (or a [`CaughtError`] wrapping one when a guard
///   already classified it, or a bare [`ErrorReport`]) — NOT a `&str`/`String`. We pull
///   `ErrorReportWithLevel::message()` (the `ereport` primary message, e.g. the SQL error text),
///   which is exactly what was missing at scale.
/// * **A plain Rust `panic!()`** — e.g. our own `panic!("staged STAGE: …")` guards in `phases.rs`.
///   The payload is a `&str` or a formatted `String`; recover it directly.
/// * **Genuinely unrecognised payload** — fall back to a message that is still actionable: it names
///   the failed phase, the worker's shard, and its pid so the failure can be located in the server
///   log even when the message itself can't be recovered in-process.
fn panic_message(panic: Box<dyn std::any::Any + Send>, phase: u8, shard: u16, pid: i32) -> String {
    use pgrx::pg_sys::panic::{CaughtError, ErrorReport, ErrorReportWithLevel};

    // (1) A pgrx-classified caught error (the variant a guard may rethrow). Extract the inner
    // ereport message for the Postgres/Rust-ereport arms; the RustPanic arm also carries an ereport
    // built from the original &str/String, so its `.message()` is the panic text.
    if let Some(caught) = panic.downcast_ref::<CaughtError>() {
        let m = match caught {
            CaughtError::PostgresError(e)
            | CaughtError::ErrorReport(e)
            | CaughtError::RustPanic { ereport: e, .. } => e.message().to_string(),
        };
        if !m.is_empty() {
            return m;
        }
    }
    // (2) The common SQL-ERROR shape: `panic_any(ErrorReportWithLevel)` from PgTryBuilder. This is
    // the case that fixes the at-scale RESOLVE "unknown panic" — `.message()` is the ereport text.
    if let Some(e) = panic.downcast_ref::<ErrorReportWithLevel>() {
        let m = e.message();
        if !m.is_empty() {
            return m.to_string();
        }
    }
    // (3) A bare ErrorReport, if one is ever panicked directly.
    if let Some(e) = panic.downcast_ref::<ErrorReport>() {
        let m = e.message();
        if !m.is_empty() {
            return m.to_string();
        }
    }
    // (4) A plain Rust panic!(): &str (string literals) or a formatted String.
    if let Some(s) = panic.downcast_ref::<&str>() {
        return (*s).to_string();
    }
    if let Some(s) = panic.downcast_ref::<String>() {
        return s.clone();
    }
    // (5) Unrecognised payload — keep the fallback as SPECIFIC as we can: phase + shard + pid let the
    // operator find the worker's own ERROR line in the server log even without the message in-hand.
    format!(
        "staged worker: unrecoverable panic (phase={}, shard={shard}, pid={pid}) \
         — see the worker's ERROR line in the PostgreSQL server log",
        phase_label(phase)
    )
}

/// Human label for a phase ordinal, for the panic fallback message (so it reads `phase=resolve`
/// rather than `phase=3`). Mirrors [`jobctl::phase`].
fn phase_label(phase: u8) -> &'static str {
    match phase {
        jobctl::phase::PING => "ping",
        jobctl::phase::STAGE_PREP => "stage-prep",
        jobctl::phase::STAGE => "stage",
        jobctl::phase::DICT => "dict",
        jobctl::phase::RESOLVE => "resolve",
        jobctl::phase::INDEX => "index",
        jobctl::phase::DONE => "done",
        _ => "unknown",
    }
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
        let wslot = match jobctl::claim_worker_slot(job_idx, jobctl::phase::PING, i as u16, 0, 0) {
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

/// One spawned worker's spec for a phase: its `shard` (selects the INDEX DDL; 0 elsewhere) and its
/// `[range_lo, range_hi)` file byte offsets (STAGE only; `0,0` for DICT/RESOLVE/INDEX).
type WorkerSpec = (u16, u64, u64);

/// The outcome of one phase, surfaced by [`run_phase`] so the coordinator can gate A→B→C→D.
enum PhaseOutcome {
    /// Every worker of the phase reported success in shmem.
    Ok,
    /// The phase failed; `String` is the human-readable reason (the first worker error string when
    /// one was recorded, else a synthesised spawn/startup/postmaster cause). The coordinator ABORTS
    /// on this — it leaves the staging table in place as the resume point and returns the error.
    Failed(String),
}

/// Run ONE phase of the staged pipeline: for each spec claim a [`jobctl::WorkerSlot`], spawn a
/// dynamic worker bound to it, then `wait_for_startup` + `wait_for_shutdown` on every handle —
/// honouring each `Result` per §7 — and finally read each worker's shmem `status` to decide the
/// outcome. This is the proven `load_turtle_staged_ping` spawn/wait/report pattern, generalised so
/// the coordinator can drive it once per phase and gate on the result.
///
/// The worker outcome channel is **shmem, not the exit code**: a worker that `ereport`s ERROR still
/// *stops*, so `wait_for_shutdown` returns `Ok(())`; the real success/failure is the `WorkerSlot
/// .status` it wrote via [`jobctl::report_worker`] before returning. So after every handle has been
/// joined we scan exactly THIS phase's claimed slots (not the whole job) — any `status == 2`
/// (error), any spawn rejection, any failed startup, or a dead postmaster ⇒ [`PhaseOutcome::Failed`].
///
/// The coordinator holds NO lock on any shared table while this blocks in `wait_for_shutdown`, so a
/// worker that needs `_pgrdf_dictionary` / `_pgrdf_quads` (DICT/RESOLVE/INDEX) can take its locks
/// freely — the deadlock the ping module documents is avoided by keeping all table mutation in the
/// workers (see `phases::prepare_for_load`).
fn run_phase(job_idx: usize, phase: u8, specs: &[WorkerSpec]) -> PhaseOutcome {
    // Spawn each worker, honouring every load_dynamic Result. A spawn failure (pool exhausted) is a
    // phase failure for the real pipeline — unlike the best-effort ping, the staged load is only
    // correct if every phase worker actually ran.
    let mut handles: Vec<DynamicBackgroundWorker> = Vec::with_capacity(specs.len());
    let mut claimed: Vec<usize> = Vec::with_capacity(specs.len());
    let mut spawn_err: Option<String> = None;
    for &(shard, lo, hi) in specs {
        let wslot = match jobctl::claim_worker_slot(job_idx, phase, shard, lo, hi) {
            Some(s) => s,
            None => {
                spawn_err.get_or_insert_with(|| {
                    "no free worker slot (jobctl MAX_SLOTS reached)".to_string()
                });
                continue;
            }
        };
        claimed.push(wslot);
        let name = format!(
            "pgrdf:job={}:phase={}:shard={}",
            jobctl::job_id_of(job_idx),
            phase,
            shard
        );
        match spawn_checked(&name, wslot) {
            Ok(h) => handles.push(h),
            Err(e) => {
                // Record the claimed slot as failed so the per-phase scan is consistent.
                jobctl::report_worker(wslot, false, &e);
                spawn_err.get_or_insert(e);
            }
        }
    }

    // wait_for_startup borrows &self (handle survives to move into `started`); wait_for_shutdown
    // consumes it. A startup Err means the worker never ran ⇒ phase failure; PostmasterDied aborts.
    let mut started: Vec<DynamicBackgroundWorker> = Vec::with_capacity(handles.len());
    let mut startup_failures = 0usize;
    let mut postmaster_died = false;
    for h in handles {
        match h.wait_for_startup() {
            Ok(_pid) => started.push(h),
            Err(BackgroundWorkerStatus::PostmasterDied) => postmaster_died = true,
            Err(_status) => startup_failures += 1,
        }
    }
    for h in started {
        match h.wait_for_shutdown() {
            Ok(()) => {}
            Err(BackgroundWorkerStatus::PostmasterDied) => postmaster_died = true,
            Err(_status) => {} // worker still reported via shmem; nothing to add
        }
    }

    // Scan THIS phase's claimed slots only (status: 1 ok, 2 error, 0 never reported).
    let mut errored = 0usize;
    let mut never_reported = 0usize;
    for &idx in &claimed {
        match jobctl::read_worker(idx).status {
            1 => {}
            2 => errored += 1,
            _ => never_reported += 1,
        }
    }

    if postmaster_died {
        return PhaseOutcome::Failed("postmaster died during phase".to_string());
    }
    if errored > 0 || spawn_err.is_some() || startup_failures > 0 || never_reported > 0 {
        // Prefer the actual worker error string the failing worker recorded in the JobSlot.
        let job_err = jobctl::job_err(job_idx);
        let reason = if !job_err.is_empty() {
            job_err
        } else if let Some(e) = spawn_err {
            e
        } else {
            format!(
                "phase failed (errored={errored}, startup_failures={startup_failures}, \
                 never_reported={never_reported})"
            )
        };
        return PhaseOutcome::Failed(reason);
    }
    PhaseOutcome::Ok
}

/// **R2.1 coordinator** — the real native staged bulk loader. Spawns the STAGE → DICT → RESOLVE →
/// INDEX worker pipeline (each phase its own background worker(s), each its own committed
/// transaction), gating on every phase, and returns load stats as JSONB.
///
/// Pipeline (`_WIP/SPEC.STAGED-LOADER-R2.bgworker-design.md` §3.3/§6):
/// * **A STAGE** (T3 — multi-backend) — split into two sub-steps: ONE `STAGE_PREP` worker defers
///   indexes + creates the UNLOGGED staging table (`phases::prepare_for_load`), then N `STAGE` workers
///   each parse a newline-snapped byte range of the `.nt` leniently and COPY it into the SAME shared
///   staging table (`phases::stage`) — N concurrent COPY-issuing backends. Splitting prep into its own
///   single worker is what keeps the prep DDL from racing across the pool; the staged column layout,
///   lenient skip, windowing, and per-line output are unchanged, so DICT/RESOLVE see byte-identical
///   staged data regardless of N. The destination partition is NOT created here — RESOLVE owns it
///   (standalone CTAS → ATTACH).
/// * **B DICT** — 1 worker: parallel `CREATE TABLE … AS SELECT … row_number()` dedup, then ONE
///   `INSERT … OVERRIDING SYSTEM VALUE` into `_pgrdf_dictionary` (pre-assigned ids, no per-row
///   `nextval`); also builds the `(term_type, lexical_md5)` join index + widens the dict's
///   `parallel_workers` so RESOLVE runs N-wide (`phases::dict`).
/// * **C RESOLVE** — 1 worker: a **parallel hash-join** `CREATE TABLE _pgrdf_quads_g<g> AS SELECT …`
///   (3× join to the dict), then `ATTACH` it as the graph's partition — a CTAS, not a serial INSERT
///   into the routed parent (`phases::resolve`).
/// * **D INDEX** — one worker per [`jobctl::index_ddls`] entry (5): the 3 hexastore indexes (built
///   parent-wide over the now-attached partition) + the dict hash index + the `unique_term`
///   constraint re-add, built SIMULTANEOUSLY across backends. The coordinator then drops DICT's
///   transient RESOLVE join index in its final cleanup.
///
/// **No coordinator-held table locks.** The coordinator never touches `_pgrdf_dictionary` /
/// `_pgrdf_quads` / the staging table while workers run (all mutation is in workers, which COMMIT and
/// release locks) — otherwise the locks it holds across `wait_for_shutdown` would deadlock the very
/// workers it waits on. It reads the final counts only AFTER the INDEX phase, when no worker remains.
///
/// **Gating + abort/resume.** After each phase the coordinator records the `phase` high-water mark
/// ([`jobctl::advance_phase`]); on ANY phase failure it ABORTS — returns the error, leaves the
/// staging table as the resume point, and does NOT mark the job done. On success it drops staging and
/// returns `{job_id, triples, dict_terms, quads, phase_ms:{stage,dict,resolve,index}, n_workers}`.
///
/// Refuses (clear error, not a panic) when pgRDF isn't in `shared_preload_libraries` — the worker
/// pool needs the shmem job-control segment.
///
/// SQL: `pgrdf.load_turtle_staged_run(path TEXT, graph_id BIGINT, n_workers INT DEFAULT 0) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn load_turtle_staged_run(path: &str, graph_id: i64, n_workers: default!(i32, 0)) -> pgrx::JsonB {
    if !jobctl::is_ready() {
        error!(
            "pgrdf staged loader requires pgrdf in shared_preload_libraries \
             (the worker pool needs the shmem job-control segment)"
        );
    }

    // T3: STAGE now runs N concurrent COPY backends (`requested` width); DICT/RESOLVE stay a single
    // worker each (their parallelism is PG's intra-query parallel hash-agg / hash-join); INDEX is one
    // worker per DDL. `n_workers > 0` ⇒ that explicit STAGE width; `0` ⇒ AUTO = the host core count,
    // bounded to a sane ceiling so a very-high-core box can't exhaust `max_worker_processes` /
    // jobctl `MAX_SLOTS` (the phases are sequential, so STAGE and INDEX never hold slots at once).
    // INDEX width is fixed by index_ddls().len().
    const STAGE_WORKERS_AUTO_CAP: usize = 32;
    let requested = if n_workers > 0 {
        (n_workers as usize).min(jobctl::MAX_SLOTS)
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8)
            .clamp(1, STAGE_WORKERS_AUTO_CAP)
    };

    // The file length bounds the (single) STAGE worker's byte range [0, len). Read BEFORE creating
    // the job so a bad path errors cleanly with no orphaned job slot.
    let file_len = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(e) => error!("pgrdf staged loader: cannot stat {path:?}: {e}"),
    };

    let db_oid: u32 = unsafe { pg_sys::MyDatabaseId }.to_u32();
    let job_idx =
        jobctl::create_job(path, graph_id, db_oid, requested as u16, 0).unwrap_or_else(|| {
            error!(
                "pgrdf staged loader: no free job slot (MAX_JOBS reached) or path exceeds PATH_CAP"
            )
        });
    let job_id = jobctl::job_id_of(job_idx);

    // Helper: on a phase failure, leave staging intact (resume point), free the shmem slots, and
    // return the abort JSONB. The job row stays FAILED (report_worker set it) — not marked done.
    let abort = |phase_label: &str, reason: String| -> pgrx::JsonB {
        let out = json!({
            "job_id": job_id,
            "ok": false,
            "failed_phase": phase_label,
            "error": reason,
            "n_workers": requested,
            "note": "staging table left in place as the resume point",
        });
        jobctl::release_job(job_idx);
        pgrx::JsonB(out)
    };

    // ── PHASE A — STAGE ──────────────────────────────────────────────────────────────────────────
    // T3: prep + parse/COPY are split into TWO sub-steps so STAGE can go N-way without the prep DDL
    // racing across the pool:
    //   A0  STAGE_PREP — ONE worker: defer indexes + create the UNLOGGED staging table.
    //   A1  STAGE      — N workers : each parses its own newline-snapped byte range of the .nt and
    //                    COPYs into the SAME shared staging table (N concurrent COPY backends).
    // Both phases write only their own committed transactions, so the coordinator holds no lock.
    let t = Instant::now();
    if let PhaseOutcome::Failed(reason) =
        run_phase(job_idx, jobctl::phase::STAGE_PREP, &[(0u16, 0u64, 0u64)])
    {
        return abort("stage", reason);
    }
    // N newline-snapped byte ranges, one per STAGE worker. `requested` is the pool width; each range
    // is a WHOLE number of lines and their union is exactly [0, file_len) (see `stage_byte_ranges`),
    // so every line is staged by exactly one worker — no split triple, no gap, no overlap. A single
    // range (tiny file / requested == 1) is byte-identical to the prior single-worker behaviour.
    let ranges = phases::stage_byte_ranges(path, file_len, requested);
    let stage_specs: Vec<WorkerSpec> = ranges
        .iter()
        .enumerate()
        .map(|(i, &(lo, hi))| (i as u16, lo, hi))
        .collect();
    if let PhaseOutcome::Failed(reason) = run_phase(job_idx, jobctl::phase::STAGE, &stage_specs) {
        return abort("stage", reason);
    }
    let stage_ms = t.elapsed().as_secs_f64() * 1000.0;
    jobctl::advance_phase(job_idx, jobctl::phase::STAGE);

    // ── PHASE B — DICT (parallel row_number() dedup + OVERRIDING SYSTEM VALUE insert), 1 worker ──
    let t = Instant::now();
    if let PhaseOutcome::Failed(reason) =
        run_phase(job_idx, jobctl::phase::DICT, &[(0u16, 0u64, 0u64)])
    {
        return abort("dict", reason);
    }
    let dict_ms = t.elapsed().as_secs_f64() * 1000.0;
    jobctl::advance_phase(job_idx, jobctl::phase::DICT);

    // ── PHASE C — RESOLVE (parallel hash-join CTAS → ATTACH partition), single worker ────────────
    let t = Instant::now();
    if let PhaseOutcome::Failed(reason) =
        run_phase(job_idx, jobctl::phase::RESOLVE, &[(0u16, 0u64, 0u64)])
    {
        return abort("resolve", reason);
    }
    let resolve_ms = t.elapsed().as_secs_f64() * 1000.0;
    jobctl::advance_phase(job_idx, jobctl::phase::RESOLVE);

    // ── PHASE D — INDEX (the 5 index_ddls, one worker each, built simultaneously) ────────────────
    let n_index = jobctl::index_ddls().len();
    let index_specs: Vec<WorkerSpec> = (0..n_index).map(|i| (i as u16, 0u64, 0u64)).collect();
    let t = Instant::now();
    if let PhaseOutcome::Failed(reason) = run_phase(job_idx, jobctl::phase::INDEX, &index_specs) {
        return abort("index", reason);
    }
    let index_ms = t.elapsed().as_secs_f64() * 1000.0;
    jobctl::advance_phase(job_idx, jobctl::phase::INDEX);

    // ── Done — read the final counts (no worker runs now, so the coordinator's ACCESS SHARE on
    // these tables conflicts with nothing), then drop the staging table and release the job. ──────
    let stg = phases::staging_table(job_id);
    let triples = Spi::get_one::<i64>(&format!("SELECT count(*)::bigint FROM {stg}"))
        .ok()
        .flatten()
        .unwrap_or(0);
    let dict_terms = Spi::get_one::<i64>("SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary")
        .ok()
        .flatten()
        .unwrap_or(0);
    let quads = Spi::get_one_with_args::<i64>(
        "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = $1",
        &[graph_id.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0);

    Spi::run(&format!("DROP TABLE IF EXISTS {stg}")).expect("staged loader: drop staging table");

    // Drop the transient `(term_type, lexical_md5)` index DICT built for RESOLVE's hash joins — it is
    // redundant now Phase D's `unique_term` UNIQUE (a `(term_type, lexical_md5, …)` prefix) exists, so
    // it is not part of the canonical schema. Safe here: no worker runs, nothing holds the dict lock.
    Spi::run(&format!(
        "DROP INDEX IF EXISTS pgrdf.{}",
        phases::dict_resolve_index()
    ))
    .expect("staged loader: drop RESOLVE join index");

    jobctl::mark_job_done(job_idx);
    jobctl::release_job(job_idx);

    pgrx::JsonB(json!({
        "job_id": job_id,
        "ok": true,
        "triples": triples,
        "dict_terms": dict_terms,
        "quads": quads,
        "phase_ms": {
            "stage": stage_ms,
            "dict": dict_ms,
            "resolve": resolve_ms,
            "index": index_ms,
        },
        "n_workers": requested,
    }))
}
