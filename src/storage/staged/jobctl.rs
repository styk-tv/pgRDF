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

// R2.0 foundation: the shmem segment + its registration. The slot fields, the JOBS/WSLOTS tables,
// and the accessors are read by the coordinator + worker bodies landing in the next R2.0 step; until
// then they are intentionally not-yet-read. Remove this allow when the coordinator wires them up.
#![allow(dead_code)]

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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
    pub const STAGE: u8 = 1; // A — parallel COPY → UNLOGGED staging
    pub const DICT: u8 = 2; // B — set-based dedup → dictionary
    pub const RESOLVE: u8 = 3; // C — parallel hash-join → quads
    pub const INDEX: u8 = 4; // D — hexastore indexes (concurrent workers)
    pub const DONE: u8 = 5;
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
