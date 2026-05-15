//! pgRDF custom GUCs.
//!
//! Phase E group E1 (LLD v0.4 §7.2) introduces the first pgRDF custom
//! GUC: `pgrdf.path_max_depth`. It bounds how far a recursive
//! property-path CTE will walk before the solution set is truncated
//! (truncated, NOT errored — a counter surfaces on `pgrdf.stats()` as
//! `path_depth_truncations`).
//!
//! E1 only **registers** the GUC and makes it readable
//! (`SHOW pgrdf.path_max_depth` / `current_setting(...)`); the depth
//! is not yet *enforced* anywhere because no recursive CTE exists
//! until Phase E group E2. A depth guard without recursion would be
//! dead code, so the read of [`path_max_depth`] only starts mattering
//! when E2 wires `+`.
//!
//! Registration must happen in `_PG_init` (both the postmaster
//! shared-preload path and the lazy backend-load path) — that is the
//! only place `DefineCustomIntVariable` may be called. pgrx's
//! `GucRegistry::define_int_guc` is the safe wrapper.

use pgrx::guc::{GucContext, GucFlags, GucRegistry, GucSetting};

/// Default property-path recursion depth bound (LLD v0.4 §7.2:
/// "`$MAX_DEPTH` defaults to 64").
pub(crate) const DEFAULT_PATH_MAX_DEPTH: i32 = 64;
/// Minimum (LLD v0.4 §7.2 range `1..1024`). A depth of 0 would make
/// every recursive path empty — never useful.
pub(crate) const MIN_PATH_MAX_DEPTH: i32 = 1;
/// Maximum (LLD v0.4 §7.2 range `1..1024`).
pub(crate) const MAX_PATH_MAX_DEPTH: i32 = 1024;

/// `pgrdf.path_max_depth` — recursive property-path depth bound.
/// `Userset` context: any role may `SET` it per-session (it only
/// affects that session's query plans, never another backend).
pub(crate) static PATH_MAX_DEPTH: GucSetting<i32> = GucSetting::<i32>::new(DEFAULT_PATH_MAX_DEPTH);

/// Register every pgRDF custom GUC. Called once from `_PG_init`.
pub fn register() {
    GucRegistry::define_int_guc(
        c"pgrdf.path_max_depth",
        c"Maximum recursion depth for SPARQL property-path CTEs.",
        c"Recursive property-path operators (*, +) walk the graph up to \
          this many hops. Solutions whose path exceeds the bound are \
          truncated (not errored); the count surfaces on pgrdf.stats() \
          as path_depth_truncations. Depth enforcement lands with the \
          recursive operators in Phase E group E2; E1 only registers \
          the knob.",
        &PATH_MAX_DEPTH,
        MIN_PATH_MAX_DEPTH,
        MAX_PATH_MAX_DEPTH,
        GucContext::Userset,
        GucFlags::default(),
    );
}

/// The currently-effective `pgrdf.path_max_depth` for this session.
///
/// E1 has no caller that enforces this yet (no recursive CTE exists);
/// the accessor exists so E2 can guard its `WITH RECURSIVE` walk
/// (`WHERE depth < path_max_depth()`) the moment `+` lands, without
/// re-touching the GUC plumbing. `#[allow(dead_code)]` keeps clippy
/// quiet until E2 wires the first read.
#[allow(dead_code)]
pub(crate) fn path_max_depth() -> i32 {
    PATH_MAX_DEPTH.get()
}
