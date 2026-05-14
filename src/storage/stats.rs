//! `pgrdf.stats()` — cross-backend observability surface.
//!
//! Phase 3 step 1 (LLD §4.1 acceptance): exposes cumulative shmem
//! dict cache counters so the perf regression on `synth-100` can
//! assert that a second load lands the bulk of its term references
//! in shmem rather than the dictionary table.
//!
//! Counters are cumulative since postmaster start. Tests compare
//! deltas rather than absolutes.

use crate::query::plan_cache;
use crate::storage::shmem_cache;
use pgrx::prelude::*;
use serde_json::json;

/// Cumulative cache snapshot as JSONB:
/// ```json
/// {
///   "shmem_ready":   true,
///   "shmem_slots":   16384,
///   "shmem_hits":    1234,
///   "shmem_misses":   567,
///   "shmem_inserts":  600,
///   "shmem_evictions": 0
/// }
/// ```
/// `shmem_ready: false` means the .so was not loaded via
/// `shared_preload_libraries`; counters are all zero in that case
/// and `put_term_full` runs without the cross-backend cache.
///
/// SQL: `pgrdf.stats() -> JSONB`.
#[pg_extern]
fn stats() -> pgrx::JsonB {
    let s = shmem_cache::snapshot();
    let p = plan_cache::snapshot();
    pgrx::JsonB(json!({
        "shmem_ready":          s.ready,
        "shmem_slots":          s.slots,
        "shmem_hits":           s.hits,
        "shmem_misses":         s.misses,
        "shmem_inserts":        s.inserts,
        "shmem_evictions":      s.evictions,
        "plan_cache_hits":      p.hits,
        "plan_cache_misses":    p.misses,
        "plan_cache_inserts":   p.inserts,
        "plan_cache_local_size": p.local_size,
    }))
}

/// Bump the shmem dict cache generation so every previously-cached
/// term reads as cold on next lookup. Idempotent. Cheap (atomic
/// increment).
///
/// Call after `DROP EXTENSION pgrdf; CREATE EXTENSION pgrdf;` if you
/// run that during a session — the new extension's dict id space is
/// fresh and unrelated to the cached one. Production workloads that
/// never drop the extension never need to call this.
///
/// SQL: `pgrdf.shmem_reset() -> void`.
#[pg_extern]
fn shmem_reset() {
    shmem_cache::reset();
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn stats_returns_object() {
        let j: pgrx::JsonB = Spi::get_one("SELECT pgrdf.stats()").unwrap().unwrap();
        let v = &j.0;
        assert!(v.is_object(), "stats() must return a JSON object");
        assert!(v.get("shmem_ready").is_some());
        assert!(v.get("shmem_slots").is_some());
        assert!(v.get("shmem_hits").is_some());
    }

    /// `pgrdf.shmem_reset()` invalidates pre-reset cache entries.
    #[pg_test]
    fn shmem_reset_invalidates_slots() {
        use crate::storage::dict::term_type;
        use crate::storage::shmem_cache::{insert_committed, lookup};

        let key = "http://example.com/reset-victim";
        insert_committed(term_type::URI, key, None, None, 12345);
        assert_eq!(lookup(term_type::URI, key, None, None), Some(12345));

        Spi::run("SELECT pgrdf.shmem_reset()").unwrap();
        assert_eq!(
            lookup(term_type::URI, key, None, None),
            None,
            "reset must drop pre-reset entries"
        );
    }
}
