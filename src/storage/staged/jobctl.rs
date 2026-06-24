//! Shared-memory job-control segment for the staged loader's background-worker pool.
//!
//! `bgw_main_arg` is a single `Datum` and `bgw_extra` is only 127 usable bytes — far too small for a
//! server-side path + GUC blob. So a spawned worker receives only its **integer slot index** (via
//! `set_argument`); the real payload (path, byte range, GUCs, graph_id, db oid) lives here in shmem
//! as **fixed inline byte arrays** (no Rust `String`/pointers — a pointer is invalid in another
//! backend's address space). This mirrors `storage::shmem_cache` exactly: `#[repr(C)] + Copy` slots,
//! a `const fn default_const()` initialiser (std `Default` only derives for arrays ≤ 32), a
//! `PgLwLock<[T; N]>` table, `PgAtomic` counters, and registration from the `_PG_init` postmaster
//! path. See `_WIP/SPEC.STAGED-LOADER-R2.bgworker-design.md` §4.

// R2.1 wires the real phases, so `range_lo`/`range_hi` (STAGE byte ranges) and every phase ordinal
// are now used. The `tspace`/`guc` inline byte arrays remain RESERVED: R2.1 applies the per-session
// parallel levers via `phases::apply_session_gucs` (computed in-worker from `num_cpus`) rather than
// shipping a serialised GUC blob through shmem, and the staging tablespace is not yet a coordinator
// argument. Keep a narrow allow for those reserved fields + the still-unused `resume`-path helpers
// rather than deleting struct fields the design (§4/§5) calls for.
#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use pgrx::prelude::*; // pg_shmem_init! expands to code referencing pg_guard / pg_sys (as in shmem_cache.rs)
use pgrx::{pg_shmem_init, PGRXSharedMemory, PgAtomic, PgLwLock};

/// Concurrent staged loads. Tiny — each is a heavyweight whole-file ingest.
pub const MAX_JOBS: usize = 8;
/// Worker slots across all jobs (headroom above any real core count).
pub const MAX_SLOTS: usize = 256;
/// Server-side `.nt` path bytes (fixed, inline).
pub const PATH_CAP: usize = 1024;
/// Staging tablespace name bytes.
pub const TSPACE_CAP: usize = 64;
/// Serialised `k=v;k=v` session GUCs the worker re-applies via `SET LOCAL`.
pub const GUC_CAP: usize = 512;
/// First worker error string, surfaced to the coordinator.
pub const ERR_CAP: usize = 512;

/// Phase ordinals (also the resumable high-water mark in [`JobSlot::phase`]).
pub mod phase {
    pub const NONE: u8 = 0;
    pub const STAGE: u8 = 1; // A — parallel parse → UNLOGGED staging
    pub const DICT: u8 = 2; // B — set-based dedup → dictionary
    pub const RESOLVE: u8 = 3; // C — parallel hash-join → quads
    pub const INDEX: u8 = 4; // D — hexastore indexes (concurrent workers)
    pub const DONE: u8 = 5;
    /// T3 — the one-time STAGE prep sub-step (defer indexes + `CREATE UNLOGGED` staging table), run by
    /// a SINGLE worker BEFORE the N parallel STAGE workers so the prep DDL can never race across the
    /// pool. This is a worker DISPATCH label only, NOT a high-water mark: the coordinator records the
    /// STAGE high-water mark (`phase::STAGE`) once prep + all STAGE workers succeed, so the resumable
    /// ordering (STAGE=1 < DICT=2 < …) is unchanged. Its ordinal is intentionally OUTSIDE the 1..5
    /// high-water range so it can never be mistaken for one.
    pub const STAGE_PREP: u8 = 6;
    /// R2.0 pool-proof sentinel — the `load_turtle_staged_ping` worker body (a marker INSERT), kept
    /// as a standalone regression test of the spawn/wait/report machinery distinct from the real
    /// phases. Never a real high-water mark; only set on the ping coordinator's worker slots.
    pub const PING: u8 = 250;
}

/// Job lifecycle state.
pub mod state {
    pub const IDLE: u8 = 0;
    pub const RUNNING: u8 = 1;
    pub const FAILED: u8 = 2;
    pub const DONE: u8 = 3;
}

/// One staged-load job. Addressed by job index `0..MAX_JOBS`; `job_id` is the monotonic public id
/// (and names the staging table `_pgrdf_stg_<job_id>` so a resumed run re-finds it).
#[derive(Copy, Clone)]
#[repr(C)]
pub struct JobSlot {
    pub in_use: u8,
    pub phase: u8, // high-water mark (resumable)
    pub state: u8, // see [`state`]
    pub _pad0: u8,
    pub n_workers: u16,
    pub n_shards: u16,
    pub db_oid: u32, // worker reconnects to THIS db (it doesn't inherit the spawner's)
    pub graph_id: i64,
    pub job_id: i64,
    pub path_len: u16,
    pub tspace_len: u16,
    pub guc_len: u16,
    pub err_len: u16,
    pub path: [u8; PATH_CAP],
    pub tspace: [u8; TSPACE_CAP],
    pub guc: [u8; GUC_CAP],
    pub err: [u8; ERR_CAP],
}
unsafe impl PGRXSharedMemory for JobSlot {}

impl JobSlot {
    /// The server-side `.nt` path, decoded from the inline byte array (`path_len` bytes). The
    /// coordinator validated `path.len() <= PATH_CAP` at [`create_job`], so this never truncates.
    pub fn path(&self) -> String {
        let n = (self.path_len as usize).min(PATH_CAP);
        String::from_utf8_lossy(&self.path[..n]).into_owned()
    }

    const fn default_const() -> Self {
        Self {
            in_use: 0,
            phase: phase::NONE,
            state: state::IDLE,
            _pad0: 0,
            n_workers: 0,
            n_shards: 0,
            db_oid: 0,
            graph_id: 0,
            job_id: 0,
            path_len: 0,
            tspace_len: 0,
            guc_len: 0,
            err_len: 0,
            path: [0; PATH_CAP],
            tspace: [0; TSPACE_CAP],
            guc: [0; GUC_CAP],
            err: [0; ERR_CAP],
        }
    }
}

/// One spawned worker. Addressed by the `set_argument` slot index `0..MAX_SLOTS`.
#[derive(Copy, Clone)]
#[repr(C)]
pub struct WorkerSlot {
    pub in_use: u8,
    pub phase: u8,  // which phase this worker runs
    pub status: u8, // 0 spawned, 1 ok, 2 error
    pub _pad0: u8,
    pub job_idx: u16, // → which JobSlot
    pub shard: u16,   // 0..n_shards (dict shard / COPY / resolve range index)
    pub range_lo: u64,
    pub range_hi: u64,
}
unsafe impl PGRXSharedMemory for WorkerSlot {}

impl WorkerSlot {
    const fn default_const() -> Self {
        Self {
            in_use: 0,
            phase: phase::NONE,
            status: 0,
            _pad0: 0,
            job_idx: 0,
            shard: 0,
            range_lo: 0,
            range_hi: 0,
        }
    }
}

static JOBS: PgLwLock<[JobSlot; MAX_JOBS]> = unsafe { PgLwLock::new(c"pgrdf_staged_jobs") };
static WSLOTS: PgLwLock<[WorkerSlot; MAX_SLOTS]> = unsafe { PgLwLock::new(c"pgrdf_staged_wslots") };
/// Monotonic public job id; starts at 1 so a 0 reads as "unset".
static NEXT_JOB_ID: PgAtomic<AtomicU64> = unsafe { PgAtomic::new(c"pgrdf_staged_next_job") };

/// Registered from `_PG_init` ONLY in the postmaster path (`process_shared_preload_libraries_in_progress`),
/// next to `storage::shmem_cache::init_in_postmaster()` — same gate, same place. Running `pg_shmem_init!`
/// outside the postmaster scan installs hooks that never fire and leaks the pointer slots.
pub fn init_in_postmaster() {
    pg_shmem_init!(JOBS = [JobSlot::default_const(); MAX_JOBS]);
    pg_shmem_init!(WSLOTS = [WorkerSlot::default_const(); MAX_SLOTS]);
    pg_shmem_init!(NEXT_JOB_ID = AtomicU64::new(1));
    SHMEM_READY.store(true, Ordering::Relaxed);
}

/// True once [`init_in_postmaster`] has run — i.e. pgRDF is in `shared_preload_libraries`. The
/// staged coordinator must refuse (clear error, not panic) when this is false: the worker pool needs
/// the shmem job segment. A hard prerequisite for R2, like the dict cache already documents.
static SHMEM_READY: AtomicBool = AtomicBool::new(false);

pub fn is_ready() -> bool {
    SHMEM_READY.load(Ordering::Relaxed)
}

// ── R2.0 accessors ────────────────────────────────────────────────────────────────────────────
//
// The parent (coordinator) and the spawned workers talk ONLY through these. Locking mirrors
// `storage::shmem_cache` exactly: `JOBS.exclusive()` / `WSLOTS.exclusive()` for mutation,
// `.share()` for a read snapshot. Every accessor short-circuits when `!is_ready()` (pgRDF not in
// `shared_preload_libraries`) so a mis-deployed extension degrades to a clear coordinator-side error
// rather than touching an uninitialised pointer slot.

/// Claim a free [`JobSlot`], assign the next monotonic `job_id`, and copy the server-side `path`
/// in as inline bytes. Returns the JOB INDEX (`0..MAX_JOBS`) for the coordinator to address the row
/// (NOT the public `job_id`; read that back via [`read_job`]). `None` ⇒ all job slots in use OR
/// `path` longer than [`PATH_CAP`] (validated, never truncated — §3 risk #3b) OR shmem not ready.
///
/// `db_oid` is the coordinator's `pg_sys::MyDatabaseId`: a dynamic worker does NOT inherit the
/// spawner's database, so each worker reconnects with `connect_worker_to_spi_by_oid(Some(db_oid))`.
pub fn create_job(
    path: &str,
    graph_id: i64,
    db_oid: u32,
    n_workers: u16,
    n_shards: u16,
) -> Option<usize> {
    if !is_ready() {
        return None;
    }
    let bytes = path.as_bytes();
    if bytes.len() > PATH_CAP {
        return None;
    }
    let job_id = NEXT_JOB_ID.get().fetch_add(1, Ordering::Relaxed) as i64;
    let mut jobs = JOBS.exclusive();
    for idx in 0..MAX_JOBS {
        if jobs[idx].in_use == 0 {
            let mut slot = JobSlot::default_const();
            slot.in_use = 1;
            slot.phase = phase::NONE;
            slot.state = state::RUNNING;
            slot.n_workers = n_workers;
            slot.n_shards = n_shards;
            slot.db_oid = db_oid;
            slot.graph_id = graph_id;
            slot.job_id = job_id;
            slot.path_len = bytes.len() as u16;
            slot.path[..bytes.len()].copy_from_slice(bytes);
            jobs[idx] = slot;
            return Some(idx);
        }
    }
    None
}

/// Claim a free [`WorkerSlot`] for `job_idx` running `phase`/`shard`. Returns the SLOT INDEX
/// (`0..MAX_SLOTS`) — this integer is what gets passed to the worker via `set_argument`, and the
/// worker reads its slot (then follows `job_idx` to the job) on entry. `None` ⇒ no free slot / not
/// ready. Status starts at 0 (`spawned`); the worker overwrites it via [`report_worker`].
///
/// `range_lo`/`range_hi` are the STAGE worker's file byte offsets (snapped to newline boundaries by
/// the coordinator, §6.A); pass `0, 0` for phases that don't byte-range (DICT/RESOLVE/INDEX and the
/// ping). `shard` selects the INDEX DDL (0..n) for the INDEX phase.
pub fn claim_worker_slot(
    job_idx: usize,
    phase: u8,
    shard: u16,
    range_lo: u64,
    range_hi: u64,
) -> Option<usize> {
    if !is_ready() {
        return None;
    }
    let mut wslots = WSLOTS.exclusive();
    for idx in 0..MAX_SLOTS {
        if wslots[idx].in_use == 0 {
            wslots[idx] = WorkerSlot {
                in_use: 1,
                phase,
                status: 0,
                _pad0: 0,
                job_idx: job_idx as u16,
                shard,
                range_lo,
                range_hi,
            };
            return Some(idx);
        }
    }
    None
}

/// Read a copy of [`JobSlot`] `idx` out from under the share lock. Returns by value (`Copy`) so the
/// caller holds no lock while it works — the same "snapshot then release" shape the dict cache uses.
pub fn read_job(idx: usize) -> JobSlot {
    let jobs = JOBS.share();
    jobs[idx]
}

/// Read a copy of [`WorkerSlot`] `idx`. By value; lock released on return (see [`read_job`]).
pub fn read_worker(idx: usize) -> WorkerSlot {
    let wslots = WSLOTS.share();
    wslots[idx]
}

/// Worker outcome channel — §7. A bgworker that `ereport`s ERROR still *stops*, so the parent's
/// `wait_for_shutdown()` returns `Ok(())`; the worker therefore signals success/failure HERE, in
/// shmem, before it returns. `ok=false` records `err` (truncated to [`ERR_CAP`]) into the owning
/// [`JobSlot`] as the first-failure string the coordinator surfaces, and flips the job to
/// `state::FAILED`. No-op when shmem isn't ready.
pub fn report_worker(idx: usize, ok: bool, err: &str) {
    if !is_ready() {
        return;
    }
    let job_idx = {
        let mut wslots = WSLOTS.exclusive();
        wslots[idx].status = if ok { 1 } else { 2 };
        wslots[idx].job_idx as usize
    };
    if !ok {
        let mut jobs = JOBS.exclusive();
        let j = &mut jobs[job_idx];
        // Only record the FIRST failure (don't clobber an earlier worker's error string).
        if j.err_len == 0 {
            let bytes = err.as_bytes();
            let n = bytes.len().min(ERR_CAP);
            j.err[..n].copy_from_slice(&bytes[..n]);
            j.err_len = n as u16;
        }
        j.state = state::FAILED;
    }
}

/// Mark job index `idx` done (state + `phase` high-water = DONE). Coordinator calls this after every
/// worker has been joined and no failure was recorded. No-op when not ready.
pub fn mark_job_done(idx: usize) {
    if !is_ready() {
        return;
    }
    let mut jobs = JOBS.exclusive();
    jobs[idx].state = state::DONE;
    jobs[idx].phase = phase::DONE;
}

/// Advance the job's `phase` high-water mark after a phase's workers have all succeeded. This is the
/// committed-by-the-coordinator record of "this phase is complete"; on a resume the coordinator reads
/// it back and skips finished phases (§7). Coordinator-only; no-op when not ready.
pub fn advance_phase(idx: usize, phase: u8) {
    if !is_ready() {
        return;
    }
    let mut jobs = JOBS.exclusive();
    jobs[idx].phase = phase;
}

/// The dictionary/quad index-rebuild DDLs for the staged loader's **INDEX** phase — the SAME 5
/// statements `loader.rs::bulk_rebuild_indexes` runs after a single-backend bulk load (3 hexastore
/// covering indexes on `_pgrdf_quads`, the dict `lexical_value` hash index, and the re-add of the
/// `unique_term` constraint that validates dictionary uniqueness over the loaded data). The staged
/// INDEX phase spawns one worker per entry so all 5 build/validate steps run SIMULTANEOUSLY across
/// backends (§6.D); each worker's `shard` field is the index into this slice. Kept here (next to the
/// worker plumbing) so the coordinator can size the INDEX phase by `index_ddls().len()`.
pub fn index_ddls() -> &'static [&'static str] {
    &[
        "CREATE INDEX _pgrdf_idx_spo ON pgrdf._pgrdf_quads (subject_id, predicate_id, object_id) INCLUDE (is_inferred)",
        "CREATE INDEX _pgrdf_idx_pos ON pgrdf._pgrdf_quads (predicate_id, object_id, subject_id) INCLUDE (is_inferred)",
        "CREATE INDEX _pgrdf_idx_osp ON pgrdf._pgrdf_quads (object_id, subject_id, predicate_id) INCLUDE (is_inferred)",
        "CREATE INDEX _pgrdf_dict_val_idx ON pgrdf._pgrdf_dictionary USING HASH (lexical_value)",
        "ALTER TABLE pgrdf._pgrdf_dictionary ADD CONSTRAINT unique_term \
         UNIQUE (term_type, lexical_md5, datatype_iri_id, language_tag)",
    ]
}

/// Release job `idx` and every [`WorkerSlot`] that pointed at it (frees the `in_use` flags so the
/// fixed-capacity tables can be reused). Coordinator's final cleanup, success OR failure — the
/// workers have already exited, so their slots are inert and safe to reclaim. No-op when not ready.
pub fn release_job(idx: usize) {
    if !is_ready() {
        return;
    }
    {
        let mut wslots = WSLOTS.exclusive();
        for w in wslots.iter_mut() {
            if w.in_use != 0 && w.job_idx as usize == idx {
                *w = WorkerSlot::default_const();
            }
        }
    }
    let mut jobs = JOBS.exclusive();
    jobs[idx] = JobSlot::default_const();
}

/// The first-failure error string recorded in [`JobSlot`] `idx`, if any. Empty ⇒ no worker failed.
pub fn job_err(idx: usize) -> String {
    let j = read_job(idx);
    let n = (j.err_len as usize).min(ERR_CAP);
    String::from_utf8_lossy(&j.err[..n]).into_owned()
}

/// The public monotonic `job_id` of job index `idx`.
pub fn job_id_of(idx: usize) -> i64 {
    read_job(idx).job_id
}

/// Count `(succeeded, failed)` across every in-use [`WorkerSlot`] pointing at job `idx`, from the
/// shmem `status` field (1 = ok, 2 = error) — the authoritative outcome channel (§7). A worker that
/// started but never reported (status still 0) counts as neither. `(0, 0)` when not ready.
pub fn tally_job(idx: usize) -> (usize, usize) {
    if !is_ready() {
        return (0, 0);
    }
    let wslots = WSLOTS.share();
    let mut ok = 0usize;
    let mut err = 0usize;
    for w in wslots.iter() {
        if w.in_use != 0 && w.job_idx as usize == idx {
            match w.status {
                1 => ok += 1,
                2 => err += 1,
                _ => {}
            }
        }
    }
    (ok, err)
}
