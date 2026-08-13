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
        storage::staged::jobctl::init_in_postmaster();
    }
}

/// Returns the extension version. Smoke surface used by the install
/// verification: `SELECT pgrdf.version();` should return the version
/// declared in `Cargo.toml`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern(immutable)]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Returns which BUILD of [`version`] this binary is.
///
/// `version()` reports the release line and is identical for every build of
/// that version — two binaries differing by a merged fix both answer
/// `0.6.22`. That makes it useless for the one question asked after loading a
/// new `.so`: *is the module I just dropped the one now running?* This answers
/// it, so a new module is distinguishable from the one it replaced without
/// reading a digest off disk.
///
/// Set at compile time from `git describe --tags --always --dirty`, e.g.
/// `v0.6.22-2-gab92a33-dirty`. `unknown` means the build did not supply one —
/// never assume that is the same as clean.
///
/// **Deliberately narrow.** Any connected role can call this, so it carries
/// only tag, commits-since, short commit and a dirty marker: no filesystem
/// paths, host names, or build users. `build_id_carries_no_paths` enforces
/// that rather than leaving it to review.
#[search_path(pgrdf, pg_temp)]
#[pg_extern(immutable)]
fn build_id() -> &'static str {
    option_env!("PGRDF_BUILD_ID").unwrap_or("unknown")
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

// R2.1 — `CALL` ergonomics for the staged loader. A thin PL/pgSQL wrapper over the coordinator
// FUNCTION `pgrdf.load_turtle_staged_run` (which does the real spawn/wait/gate work; its workers own
// the per-phase commits). Shipped via `extension_sql!` so users can `CALL pgrdf.load_turtle_staged(
// path, graph_id [, n_workers])` instead of `SELECT`ing the function. `requires` the function's
// generated SQL (referenced by its Rust path) so the procedure is created after it exists.
// Design: `_WIP/SPEC.STAGED-LOADER-R2.bgworker-design.md` §3.2.
extension_sql!(
    r#"
CREATE PROCEDURE pgrdf.load_turtle_staged(
    path TEXT,
    graph_id BIGINT,
    n_workers INT DEFAULT 0
)
LANGUAGE plpgsql AS $$
DECLARE
    r JSONB;
BEGIN
    r := pgrdf.load_turtle_staged_run(path, graph_id, n_workers);
    RAISE NOTICE 'pgrdf staged load: %', r;
END;
$$;
"#,
    name = "staged_loader_procedure",
    // pgrx matches a `requires` FullPath by `module_path.ends_with(path-without-last-segment)`; the
    // extern's module_path is `pgrdf::storage::staged::pool` (no `crate::`), so the reference must
    // omit the `crate::` prefix or the suffix match fails (pgrx-sql-entity-graph pgrx_sql.rs:566).
    requires = [storage::staged::pool::load_turtle_staged_run],
);

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn test_version_matches_cargo() {
        assert_eq!(crate::version(), env!("CARGO_PKG_VERSION"));
    }

    /// #115: version()/build_id() are compile-time constants of the
    /// loaded .so — IMMUTABLE is the honest class.
    #[pg_test]
    fn identity_fns_are_immutable() {
        for f in ["version", "build_id"] {
            let v: String = pgrx::Spi::get_one_with_args(
                "SELECT DISTINCT provolatile::text FROM pg_proc p
                   JOIN pg_namespace n ON n.oid = p.pronamespace
                  WHERE n.nspname = 'pgrdf' AND p.proname = $1",
                &[f.into()],
            )
            .unwrap()
            .unwrap();
            assert_eq!(v, "i", "pgrdf.{f} must be IMMUTABLE");
        }
    }

    /// `build_id()` is readable by any connected role, so the disclosure
    /// constraint is a test rather than a comment. A build id that leaked a
    /// build path would publish the operator's filesystem layout to every
    /// user of the database.
    #[test]
    fn build_id_carries_no_paths() {
        let id = crate::build_id();
        assert!(!id.is_empty(), "build_id must never be empty");
        for bad in ['/', '\\'] {
            assert!(
                !id.contains(bad),
                "build_id must not carry filesystem paths, found {bad:?} in {id:?}"
            );
        }
        // `git describe` output and the `unknown` fallback are both single
        // tokens. Whitespace means something else got interpolated.
        assert!(
            !id.contains(char::is_whitespace),
            "build_id must be a single token, got {id:?}"
        );
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

// #88 — invalidate the shmem dictionary cache at CREATE EXTENSION.
//
// The cache keys a term fingerprint to a dictionary id and guards each
// slot with a GENERATION counter, so a stale slot reads as cold. That
// guard was correct and it was only ever advanced by an explicit
// `pgrdf.shmem_reset()` — a manual step `reset()`'s own doc comment
// asked users to remember after `DROP EXTENSION`.
//
// Nothing remembers. On a server with pgrdf preloaded, DROP EXTENSION
// followed by CREATE EXTENSION recreates `_pgrdf_dictionary` empty and
// restarts ids at 1, while every cached fingerprint still carries the
// CURRENT generation. Each one now resolves to whatever term happens to
// hold that id in the new dictionary, and it does so silently:
// `ex:s a ex:T` was measured storing predicate `rdfs:label`, because
// rdf:type's id in the previous extension lifetime is rdfs:label's id
// in this one.
//
// A correctness invariant must not depend on a human running a
// function. Bumping the generation here makes a fresh extension
// unable to inherit a stale cache, by construction.
extension_sql!(
    r#"SELECT pgrdf.shmem_reset();"#,
    name = "dict_cache_generation_bump_on_install",
    // NOTE the missing `crate::` — pgrx matches a `requires` FullPath by
    // `module_path.ends_with(path-without-last-segment)`, and the extern's
    // module_path is `pgrdf::storage::stats`. With `crate::` the suffix match
    // fails SILENTLY: no ordering is enforced, the SELECT is emitted before
    // the function exists, and CREATE EXTENSION dies. The caveat is documented
    // twelve lines above this one and I still wrote it wrong.
    requires = [storage::stats::shmem_reset],
);
