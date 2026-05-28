//! Per-backend cache of `Spi::prepare`-d SPARQL plan SQL.
//!
//! Implements LLD §4.2. Before this slice the executor built a fresh
//! dynamic SQL string per call (`Spi::connect_mut(|c|
//! c.update(sql, None, &[]))`) and Postgres re-parsed + re-planned
//! every invocation. We now:
//!
//! 1. Translate the SPARQL algebra into a **parameterised** SQL
//!    string (every dict id constant becomes a `$N` placeholder).
//! 2. Use the SQL string itself as the canonical cache key — same
//!    algebra shape ⇒ same SQL ⇒ same key — and stash an
//!    [`OwnedPreparedStatement`] under it on first sight.
//! 3. Execute the cached plan with the per-call parameter Datums.
//!
//! Acceptance criterion (LLD §4.2): identical structural queries
//! with varying constants reuse the cached plan; cache-hit ratio is
//! surfaced via `pgrdf.stats()`.
//!
//! Lifetime model. `OwnedPreparedStatement` is a backend-local SPI
//! plan handle (`SPI_keepplan` makes it survive past
//! `SPI_finish`/`SPI_connect` pairs). Backends are single-threaded
//! in Postgres so a `thread_local!` `RefCell<HashMap>` is the right
//! container — no locks needed in the hot path.
//!
//! Stats are cross-backend (counters live in shmem alongside the
//! dict cache; see `crate::storage::shmem_cache::init_in_postmaster`
//! for the registration shape). Per-backend cache size is exposed
//! as a separate field in `pgrdf.stats()`.

use pgrx::prelude::*;
use pgrx::spi::OwnedPreparedStatement;
use pgrx::{pg_shmem_init, PgAtomic};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

// Cumulative counters live in shmem so a multi-backend benchmark
// can read a single fleet-wide view through `pgrdf.stats()`.
pub(crate) static HITS: PgAtomic<AtomicU64> = unsafe { PgAtomic::new(c"pgrdf_plan_cache_hits") };
pub(crate) static MISSES: PgAtomic<AtomicU64> =
    unsafe { PgAtomic::new(c"pgrdf_plan_cache_misses") };
pub(crate) static INSERTS: PgAtomic<AtomicU64> =
    unsafe { PgAtomic::new(c"pgrdf_plan_cache_inserts") };

pub fn init_in_postmaster() {
    pg_shmem_init!(HITS);
    pg_shmem_init!(MISSES);
    pg_shmem_init!(INSERTS);
}

// Per-backend prepared-statement cache. Keyed on the parameterised
// SQL string verbatim — collision-free by construction, no hashing
// needed at this layer. Capacity is unbounded for v1; typical
// backends touch a few dozen distinct shapes per session.
thread_local! {
    static PLANS: RefCell<HashMap<String, OwnedPreparedStatement>> =
        RefCell::new(HashMap::new());
}

/// True iff the local cache currently holds an entry for this SQL.
/// Pure peek — does not advance the counters.
pub fn contains(sql: &str) -> bool {
    PLANS.with(|c| c.borrow().contains_key(sql))
}

/// Insert a freshly-prepared plan and bump the insert counter.
/// Called on the miss path after `client.prepare(...).keep()`.
///
/// The per-backend `PLANS` HashMap insert always runs (it's a local
/// thread_local!). The shmem counter increment is guarded by
/// `shmem_cache::is_ready()` — matches the discipline the dict-cache
/// module already enforces (`shmem_cache::lookup`, `stage_for_commit`,
/// etc.) so a lazy-loaded backend (extension .so loaded outside
/// `shared_preload_libraries`) degrades to a no-op stats path rather
/// than panicking with "PgAtomic was not initialized" on the first
/// plan-cache miss.
pub fn insert(sql: String, plan: OwnedPreparedStatement) {
    PLANS.with(|c| {
        c.borrow_mut().insert(sql, plan);
    });
    if !crate::storage::shmem_cache::is_ready() {
        return;
    }
    INSERTS.get().fetch_add(1, Ordering::Relaxed);
}

/// Number of plans currently cached in THIS backend.
pub fn local_size() -> usize {
    PLANS.with(|c| c.borrow().len())
}

pub fn record_hit() {
    if !crate::storage::shmem_cache::is_ready() {
        return;
    }
    HITS.get().fetch_add(1, Ordering::Relaxed);
}

pub fn record_miss() {
    if !crate::storage::shmem_cache::is_ready() {
        return;
    }
    MISSES.get().fetch_add(1, Ordering::Relaxed);
}

/// Run `f` with a borrow of the cached plan, so the caller can
/// execute it inside the same borrow scope. The closure receives
/// `Some(&OwnedPreparedStatement)` on hit, `None` on miss.
pub fn with_plan<R>(sql: &str, f: impl FnOnce(Option<&OwnedPreparedStatement>) -> R) -> R {
    PLANS.with(|c| {
        let map = c.borrow();
        f(map.get(sql))
    })
}

/// Drop every cached plan in THIS backend. Operators typically don't
/// call this directly; provided for diagnostics and as a tear-down
/// hook for tests.
///
/// SQL: `pgrdf.plan_cache_clear() -> BIGINT` (returns the number of
/// plans dropped).
#[pg_extern]
fn plan_cache_clear() -> i64 {
    let dropped = PLANS.with(|c| {
        let mut m = c.borrow_mut();
        let n = m.len();
        m.clear();
        n
    });
    dropped as i64
}

pub struct Snapshot {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub local_size: usize,
}

pub fn snapshot() -> Snapshot {
    let ready = crate::storage::shmem_cache::is_ready();
    Snapshot {
        hits: if ready {
            HITS.get().load(Ordering::Relaxed)
        } else {
            0
        },
        misses: if ready {
            MISSES.get().load(Ordering::Relaxed)
        } else {
            0
        },
        inserts: if ready {
            INSERTS.get().load(Ordering::Relaxed)
        } else {
            0
        },
        local_size: local_size(),
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// A SPARQL query primes the local plan cache; a repeated query
    /// hits it instead of preparing again. The exact cache size
    /// depends on what else ran beforehand (the loader caches its
    /// own INSERT plan too — see `src/storage/loader.rs`); assert
    /// DELTAS rather than absolutes.
    #[pg_test]
    fn plan_cache_repeats_hit() {
        Spi::run("SELECT pgrdf.plan_cache_clear()").unwrap();
        Spi::run("SELECT pgrdf.add_graph(8100)").unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(
               '@prefix ex: <http://example.com/> . ex:a ex:p ex:b .', 8100)",
        )
        .unwrap();

        // Snapshot AFTER the load. The INSERT plan is now cached;
        // a SPARQL call should add exactly one more slot.
        let size_before: i64 =
            Spi::get_one("SELECT (pgrdf.stats()->>'plan_cache_local_size')::bigint")
                .unwrap()
                .unwrap();
        let inserts_before: i64 =
            Spi::get_one("SELECT (pgrdf.stats()->>'plan_cache_inserts')::bigint")
                .unwrap()
                .unwrap();

        // First SPARQL call — prepare + insert one new plan.
        let _ = Spi::run("SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o }')");
        let size_after_first: i64 =
            Spi::get_one("SELECT (pgrdf.stats()->>'plan_cache_local_size')::bigint")
                .unwrap()
                .unwrap();
        let inserts_after_first: i64 =
            Spi::get_one("SELECT (pgrdf.stats()->>'plan_cache_inserts')::bigint")
                .unwrap()
                .unwrap();
        assert_eq!(
            size_after_first - size_before,
            1,
            "first SPARQL call must populate one new cache slot"
        );
        assert_eq!(
            inserts_after_first - inserts_before,
            1,
            "first call must bump the cumulative insert counter by 1"
        );

        // Second call — hit.
        let _ = Spi::run("SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o }')");
        let size_after_second: i64 =
            Spi::get_one("SELECT (pgrdf.stats()->>'plan_cache_local_size')::bigint")
                .unwrap()
                .unwrap();
        let inserts_after_second: i64 =
            Spi::get_one("SELECT (pgrdf.stats()->>'plan_cache_inserts')::bigint")
                .unwrap()
                .unwrap();
        assert_eq!(
            size_after_second, size_after_first,
            "second identical call must NOT add a slot"
        );
        assert_eq!(
            inserts_after_second, inserts_after_first,
            "second call must not bump the cumulative insert counter"
        );
    }

    /// `plan_cache_clear()` empties the local cache and returns the
    /// number of plans dropped.
    #[pg_test]
    fn plan_cache_clear_returns_count() {
        Spi::run("SELECT pgrdf.plan_cache_clear()").unwrap();
        Spi::run("SELECT pgrdf.add_graph(8101)").unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(
               '@prefix ex: <http://example.com/> . ex:a ex:p ex:b .', 8101)",
        )
        .unwrap();

        let _ = Spi::run("SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o }')");
        let _ = Spi::run(
            "SELECT count(*) FROM pgrdf.sparql(
               'SELECT ?s ?o WHERE { ?s ?p ?o }')",
        );

        let dropped: i64 = Spi::get_one("SELECT pgrdf.plan_cache_clear()")
            .unwrap()
            .unwrap();
        assert!(dropped >= 2, "should have at least two cached plans");
        let after: i64 = Spi::get_one("SELECT (pgrdf.stats()->>'plan_cache_local_size')::bigint")
            .unwrap()
            .unwrap();
        assert_eq!(after, 0);
    }
}
