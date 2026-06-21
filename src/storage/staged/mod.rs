//! R2 — native STAGED bulk loader on a pgrx background-worker pool (v0.7).
//!
//! The E32-proven method (parse → UNLOGGED staging → PG parallel set-based dedup → parallel
//! hash-join resolve → concurrent index, all committed per phase) ported into pgRDF core. A single
//! `#[pg_extern]` function can't COMMIT mid-phase, run 3 `CREATE INDEX` concurrently, or own N COPY
//! streams — those need multiple backends, so this is a **dynamic background-worker pool**: a thin
//! coordinator (callable from SQL) spawns N workers at runtime; each worker is its own backend with
//! its own transaction(s) and owns one phase/shard. Commit-per-phase lives in the *workers*
//! (`BackgroundWorker::transaction(|| …)`), not the coordinator — that's the load-bearing design
//! decision (pgrx 0.16 cannot emit a transaction-controlling PROCEDURE).
//!
//! Design: `_WIP/SPEC.STAGED-LOADER-R2.bgworker-design.md`. Phasing: R2.0 (this) = the pool
//! skeleton + shmem job segment; later R2.1 = multi-stream COPY + sharded dict; R2.2 = tuning.

pub mod jobctl;
pub mod pool;
