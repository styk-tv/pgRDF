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
///   "shmem_ready":           true,
///   "shmem_slots":           16384,
///   "shmem_hits":            1234,
///   "shmem_misses":           567,
///   "shmem_inserts":          600,
///   "shmem_evictions":          0,
///   "plan_cache_hits":         42,
///   "plan_cache_misses":        7,
///   "plan_cache_inserts":       7,
///   "plan_cache_local_size":    7,
///   "path_depth_truncations":   0
/// }
/// ```
///
/// `path_depth_truncations` (Phase E group E1, LLD v0.4 §7.2) counts
/// SPARQL property-path solutions truncated at `pgrdf.path_max_depth`.
/// Always 0 in v0.4.5 (the recursive CTE that would truncate lands
/// with the `+` operator in Phase E group E2); `pgrdf.shmem_reset()`
/// zeroes it.
/// `shmem_ready: false` means the .so was not loaded via
/// `shared_preload_libraries`; the shmem counters are all zero in
/// that case and `put_term_full` runs without the cross-backend
/// cache. The `plan_cache_*` fields come from the per-backend
/// prepared-statement cache (LLD §4.2); `plan_cache_local_size`
/// is THIS backend's cache size — the other plan_cache counters
/// are cumulative across all backends in shmem.
///
/// SQL: `pgrdf.stats() -> JSONB`.
#[search_path(pgrdf, pg_temp)]
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
        // Phase E group E1 (LLD v0.4 §7.2): property-path depth-guard
        // scaffold. Always 0 in E1 (the recursive CTE that would
        // truncate lands in group E2); the field is present so
        // tooling can rely on its shape from v0.4.5 onward.
        "path_depth_truncations": s.path_depth_truncations,
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
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn shmem_reset() {
    shmem_cache::reset();
}

/// TA-D2 spike — pre-warm the shmem dict cache from
/// `_pgrdf_dictionary` so a fresh backend's first ingest can hit
/// the cache for any term already known to the dictionary.
///
/// Walks `_pgrdf_dictionary` ordered by `id` (oldest first — most-
/// likely-shared, e.g. core RDF/RDFS/OWL predicates) and calls
/// `shmem_cache::insert_committed` for each row, up to `limit` rows.
///
/// Returns the number of rows pre-warmed (clipped to actual rows in
/// the dictionary if smaller than `limit`).
///
/// **Use cases:**
///
/// - Boot a fresh backend connecting to a database that already has
///   `_pgrdf_dictionary` populated by prior sessions. Without
///   pre-warm, the new backend's per-ingest path hits SPI for every
///   term despite the dict already knowing them.
/// - After `pgrdf.shmem_reset()` (e.g. post `DROP/CREATE EXTENSION`),
///   re-establish cache contents in one call instead of bleeding
///   through ingest-by-ingest.
///
/// **Measurement scope (TA-D2 spike):**
///
/// 1. Cold ingest LUBM-1 (records baseline shmem_cache_hits).
/// 2. `shmem_reset()` to drop cache contents (dict survives).
/// 3. `shmem_cache_prewarm(100000)` to refill from
///    `_pgrdf_dictionary` (now populated).
/// 4. Ingest LUBM-1 into a different graph — expect
///    shmem_cache_hits dominant, dict_db_calls → near-zero.
///
/// The spike measurement informs the TA-D1 decision (combine with
/// TA-D3 batch path; pre-warm complements but does not replace).
///
/// SQL: `pgrdf.shmem_cache_prewarm(limit BIGINT DEFAULT 100000) -> BIGINT`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn shmem_cache_prewarm(limit: default!(i64, 100000)) -> i64 {
    use pgrx::Spi;
    let mut count: i64 = 0;
    Spi::connect(|client| {
        let table = client
            .select(
                "SELECT id, term_type, lexical_value, datatype_iri_id, language_tag
                 FROM pgrdf._pgrdf_dictionary
                 ORDER BY id
                 LIMIT $1",
                None,
                &[limit.into()],
            )
            .expect("shmem_cache_prewarm: select failed");
        for row in table {
            let id: i64 = row.get(1).expect("id").expect("id NULL");
            let term_type: i16 = row.get(2).expect("term_type").expect("term_type NULL");
            let value: String = row.get(3).expect("value").expect("value NULL");
            let datatype_id: Option<i64> = row.get(4).expect("datatype");
            let language: Option<String> = row.get(5).expect("language");
            shmem_cache::insert_committed(term_type, &value, datatype_id, language.as_deref(), id);
            count += 1;
        }
    });
    count
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
