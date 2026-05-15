//! pgRDF — Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning.
//!
//! Module map (mirrors SPEC.pgRDF.LLD.v0.2 §4):
//!   storage    — shmem dictionary + partitioned hexastore + COPY BINARY loader
//!   query      — SPARQL parser + BGP-to-prepared-SQL translator + plan cache
//!   inference  — reasonable (OWL 2 RL) materialization
//!   validation — SHACL validation reports

// `oxrdf::Term` and `spargebra` enums are `#[non_exhaustive]` upstream so
// our catch-all `other => panic!(...)` defensive arms are flagged by
// rustc 1.83+ as unreachable for the variants we already match. Keep
// the arms (they future-proof the translator against upstream variant
// additions) and silence the lint at crate scope.
#![allow(unreachable_patterns)]
// The translator's module + function docs use vertically-aligned ASCII
// continuation lines that clippy reads as malformed Markdown list
// items. The rendered rustdoc output looks correct (continuation
// paragraphs); reformatting under the lint would damage readability.
#![allow(clippy::doc_lazy_continuation)]
// `SetOfIterator::new(rows.into_iter())` is a deliberate readability
// choice — the explicit `.into_iter()` makes the intent obvious at
// the call-site even though `Vec<T>` already implements
// `IntoIterator`. Allow the lint at crate scope so we don't have to
// litter call sites with annotations.
#![allow(clippy::useless_conversion)]

use pgrx::prelude::*;

::pgrx::pg_module_magic!();

pub mod inference;
pub mod query;
pub mod storage;
pub mod validation;

/// Postgres entrypoint. Runs once per process: in the postmaster
/// when `pgrdf` is in `shared_preload_libraries` (the supported
/// production deployment), or lazily in a backend on first extension
/// use. Only the postmaster path can register shmem hooks — see
/// `storage::shmem_cache`.
#[pg_guard]
pub extern "C-unwind" fn _PG_init() {
    // Custom GUCs MUST be registered in `_PG_init` in BOTH the
    // postmaster shared-preload path AND the lazy backend-load path
    // (Postgres calls `DefineCustomIntVariable` from either). Register
    // before the postmaster-only shmem hooks so the knob is always
    // visible via `SHOW` regardless of how the .so was loaded —
    // Phase E group E1, LLD v0.4 §7.2.
    query::guc::register();
    let in_postmaster = unsafe { pgrx::pg_sys::process_shared_preload_libraries_in_progress };
    if in_postmaster {
        storage::shmem_cache::init_in_postmaster();
        query::plan_cache::init_in_postmaster();
    }
}

/// Returns the extension version. Smoke surface used by the install
/// verification: `SELECT pgrdf.version();` should return the version
/// declared in `Cargo.toml`.
#[pg_extern]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

extension_sql_file!("../sql/schema_v0_2_0.sql", name = "schema_v0_2_0");
// v0.4 — adds `_pgrdf_graphs` IRI ↔ graph_id mapping (LLD v0.4 §3.1).
// `requires` enforces ordering: the v0.2 baseline lands first; the
// graphs table appends after.
extension_sql_file!(
    "../sql/schema_v0_4_0_graphs.sql",
    name = "schema_v0_4_0_graphs",
    requires = ["schema_v0_2_0"],
);

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn test_version_matches_cargo() {
        assert_eq!(crate::version(), env!("CARGO_PKG_VERSION"));
    }
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}
    /// Force the test instance to load `pgrdf` via shared_preload_libraries
    /// so `_PG_init` runs in postmaster context — required for the shmem
    /// dict cache (LLD §4.1) to register its hooks.
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec!["shared_preload_libraries='pgrdf'"]
    }
}
