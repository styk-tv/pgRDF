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
use std::ffi::CString;

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

/// `pgrdf.on_path_truncation` — what a query does when a recursive
/// property-path walk (`+` / `*`) actually hits `pgrdf.path_max_depth`
/// (issue #14, fail-closed truncation; detected by the per-`+`
/// truncation probe, so a cycle that terminates naturally never
/// triggers it):
///
/// * `count` — the pre-#14 behaviour: bump the cumulative
///   `pgrdf.stats().path_depth_truncations` counter and return the
///   (depth-limited) rows silently.
/// * `warn` (default) — `count` plus a client-visible WARNING per
///   truncated walk, so a partial result is never silent.
/// * `error` — fail the query with a stable-prefix error instead of
///   returning a partial result. The fail-closed mode for closure
///   queries (e.g. an un-materialised `rdfs:subClassOf*` type-closure
///   walk feeding a carve) where under-collection would silently
///   propagate into a curated slice.
///
/// An unrecognised value logs a warning and behaves as `warn` (the
/// default — honest but non-fatal).
pub(crate) static ON_PATH_TRUNCATION: GucSetting<Option<CString>> =
    GucSetting::<Option<CString>>::new(Some(c"warn"));

// ─────────────────────────────────────────────────────────────────────
// TA-7 — dict-path GUCs
// ─────────────────────────────────────────────────────────────────────
//
// TA-D1's USER APPROVED decision: combine TA-D3 (batched dict
// resolution) + TA-D2 (shmem cache prewarm) into the default
// `parse_turtle` ingest path. These three GUCs gate the rollout so
// the legacy single-term SPI path stays callable for parity testing.

/// Default dict batch size — terms per `put_terms_batch` chunk in
/// the `combined` / `batched` paths. 500 balances per-SPI roundtrip
/// amortisation against the deferred latency between parsing a term
/// and seeing it in the quad buffer.
pub(crate) const DEFAULT_DICT_BATCH_SIZE: i32 = 500;
/// Minimum `dict_batch_size`. `0` is reserved as an alias for the
/// legacy single-term path; the int-range minimum stays at 1.
pub(crate) const MIN_DICT_BATCH_SIZE: i32 = 0;
/// Maximum `dict_batch_size`. Beyond ~10k the memory footprint of
/// the defer queue starts to matter on small-RAM hosts.
pub(crate) const MAX_DICT_BATCH_SIZE: i32 = 10_000;

/// Default `pgrdf.bulk_defer_index_min` — the parallel bulk loader
/// (`load_turtle(..., bulk_load => true)`) only defers + rebuilds the
/// hexastore + dict-hash indexes when the parsed triple count reaches
/// this threshold. 100k sits well above any unit-test fixture and below
/// the smallest benchmark scale (LUBM-10 = 1.32M), so tiny loads —
/// including the parallel pgrx test suite — never trigger the global
/// ACCESS-EXCLUSIVE index DDL.
pub(crate) const DEFAULT_BULK_DEFER_INDEX_MIN: i32 = 100_000;

/// `pgrdf.ingest_dict_path` — selects which dict-resolution path
/// `parse_turtle` + `load_turtle` (and verbose variants) dispatch
/// through. Valid values:
///
/// * `baseline` — legacy single-term `put_term_full` SPI per term.
///   What v0.5.36 and earlier defaulted to. Kept for parity tests.
/// * `batched`  — TA-D3 path: 2-pass (materialise all triples, then
///   bulk resolve via `put_terms_batch`). Validated as the v0.5.27
///   spike (-17% e2e at LUBM-1).
/// * `shmem_warm` — TA-D2 path: hot-cache check first, fallback to
///   per-term SPI on miss. No batching. Validated as the v0.5.28
///   spike (-54% e2e at LUBM-1 with prewarm).
/// * `combined` — production target: single-pass streaming with
///   hot-cache check first, defer queue for misses, bulk-resolve at
///   `dict_batch_size` or quad-flush boundary. The TA-7 production
///   landing. Default since v0.5.37.
///
/// `Userset` context (per-session override is safe — the path is
/// idempotent w.r.t. the resulting `_pgrdf_quads` rows; only the
/// per-term SPI shape differs).
pub(crate) static INGEST_DICT_PATH: GucSetting<Option<CString>> =
    GucSetting::<Option<CString>>::new(Some(c"combined"));

/// `pgrdf.dict_batch_size` — terms per `put_terms_batch` chunk in
/// the `combined` / `batched` paths. `0` is interpreted as "fall
/// back to single-term SPI" (equivalent to selecting `baseline`).
pub(crate) static DICT_BATCH_SIZE: GucSetting<i32> =
    GucSetting::<i32>::new(DEFAULT_DICT_BATCH_SIZE);

/// `pgrdf.staged_temp_tablespaces` — when non-empty, the staged bulk
/// loader emits `SET LOCAL temp_tablespaces = '<value>'` in every
/// staged phase's session GUCs (STAGE/DICT/RESOLVE/INDEX), routing
/// that phase's temp-file spill — dominated by RESOLVE's forced
/// parallel 3-way hash join, which spilled ~3 TB at 8.2 B rows — off
/// the PGDATA disk and onto the named tablespace(s). Empty (the
/// default) emits nothing, so the server's own `temp_tablespaces`
/// default is inherited (the prior behaviour, unchanged).
///
/// The value is a tablespace-name list — a comma-separated list of
/// SQL identifiers — exactly like the core `temp_tablespaces` GUC. It
/// is validated as such before interpolation (see
/// [`crate::storage::staged::phases::temp_tablespaces_set_fragment`]);
/// anything carrying a quote, semicolon, or other non-identifier
/// character is rejected so the value can never break out of the
/// emitted `SET LOCAL` statement.
///
/// `Userset` context: an operator can `SET` it per session before a
/// staged load to steer that load's spill; it never affects another
/// backend.
pub(crate) static STAGED_TEMP_TABLESPACES: GucSetting<Option<CString>> =
    GucSetting::<Option<CString>>::new(None);

/// `pgrdf.staged_resolve_strategy` — selects the planner join strategy
/// the staged loader's RESOLVE phase forces. RESOLVE joins the staged
/// rows against the dictionary; the join OUTPUT is identical for any
/// join method, so this is purely a PERFORMANCE knob (it does not change
/// the rows written).
///
/// One of `auto` | `hash` | `index`:
/// * `auto` — emit no `enable_*` forcing; let the planner choose using
///   the adaptive `work_mem` + the dict resolve index + the parallel
///   reloption already in place. Still bumps `hash_mem_multiplier`.
/// * `hash` — force the all-hash-join (the historical behaviour:
///   `enable_hashjoin = on`, everything else off). Identical output, but
///   at 8.2 B rows the hash build spills multi-TB to temp.
/// * `index` (default) — force the low-spill index-nested-loop path
///   (`enable_nestloop = on` + index scans on, hash/merge off). The
///   at-scale-validated default: an out-of-the-box 8.2 B-triple
///   Wikidata-truthy load on E64ads_v7 completes with no multi-TB hash
///   spill / no ENOSPC.
///
/// An unrecognised value logs a warning and falls back to `hash`, the
/// known-safe historical behaviour.
///
/// `Userset` context: an operator can `SET` it per session before a
/// staged load to steer that load's RESOLVE plan.
pub(crate) static STAGED_RESOLVE_STRATEGY: GucSetting<Option<CString>> =
    GucSetting::<Option<CString>>::new(Some(c"index"));

/// `pgrdf.shmem_prewarm_on_init` — when on, the shmem dict cache
/// is auto-prewarmed from `_pgrdf_dictionary` once per backend
/// before the first ingest call observes it. Default off because
/// the prewarm cost (~one SPI scan of the full dict) is paid
/// up-front and rarely amortises in short-lived workloads.
pub(crate) static SHMEM_PREWARM_ON_INIT: GucSetting<bool> = GucSetting::<bool>::new(false);

/// `pgrdf.auto_analyze` — when on (default), `pgrdf.materialize` runs
/// `ANALYZE pgrdf._pgrdf_quads` after writing inferred triples so the
/// planner has fresh statistics for the inference-inflated table. The
/// closure of an `owl:TransitiveProperty` (e.g. LUBM `subOrganizationOf`)
/// inflates join cardinalities; without stats the planner mis-plans
/// complex multi-pattern queries on a freshly materialized graph (LUBM
/// Q2: 180 s → 1 s). `ANALYZE` is sample-based (fixed sub-second cost),
/// so this is on by default; set off to manage `ANALYZE` externally.
pub(crate) static AUTO_ANALYZE: GucSetting<bool> = GucSetting::<bool>::new(true);

/// `pgrdf.bulk_defer_index_min` — triple-count threshold at/above which
/// `load_turtle(..., bulk_load => true)` drops the hexastore (SPO/POS/
/// OSP) + dict `_pgrdf_dict_val_idx` indexes before loading and rebuilds
/// them after, skipping per-row index maintenance during a fresh bulk
/// load. The drop/rebuild takes ACCESS EXCLUSIVE on the global tables,
/// so it is gated above this threshold (default 100k). Only consulted on
/// the empty-dict fast path.
pub(crate) static BULK_DEFER_INDEX_MIN: GucSetting<i32> =
    GucSetting::<i32>::new(DEFAULT_BULK_DEFER_INDEX_MIN);

/// `pgrdf.ingest_dict_path` parsed into a Rust enum so callers
/// don't keep matching the raw string. `parse_turtle` / `load_turtle`
/// dispatch on this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IngestDictPath {
    Baseline,
    Batched,
    ShmemWarm,
    Combined,
}

impl IngestDictPath {
    fn from_guc_string(raw: Option<&str>) -> Self {
        match raw.map(str::trim).unwrap_or("combined") {
            "baseline" => Self::Baseline,
            "batched" => Self::Batched,
            "shmem_warm" => Self::ShmemWarm,
            _ => Self::Combined,
        }
    }

    /// Canonical lowercase name of the path, matching the GUC enum
    /// values. Surfaced as the `path` field in the verbose-ingest
    /// JSONB (TA-5) so callers can confirm which route the dispatch
    /// actually selected for a given call.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Batched => "batched",
            Self::ShmemWarm => "shmem_warm",
            Self::Combined => "combined",
        }
    }
}

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
    GucRegistry::define_string_guc(
        c"pgrdf.on_path_truncation",
        c"Behaviour when a property-path walk hits pgrdf.path_max_depth.",
        c"One of 'count' | 'warn' | 'error'. 'count' silently bumps \
          pgrdf.stats().path_depth_truncations (the pre-#14 \
          behaviour); 'warn' (default) additionally raises a \
          client-visible WARNING per truncated walk so a partial \
          result is never silent; 'error' fails the query instead of \
          returning a depth-truncated result — the fail-closed mode \
          for closure queries feeding a carve. An unrecognised value \
          logs a warning and behaves as 'warn'.",
        &ON_PATH_TRUNCATION,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_string_guc(
        c"pgrdf.ingest_dict_path",
        c"Dict-resolution path used by parse_turtle / load_turtle.",
        c"One of 'baseline' | 'batched' | 'shmem_warm' | 'combined'. \
          'baseline' is the pre-v0.5.37 single-term SPI path; \
          'batched' is the TA-D3 2-pass path; 'shmem_warm' is the \
          TA-D2 hot-cache-then-SPI path; 'combined' is the v0.5.37+ \
          production default that streams the parser with a defer \
          queue, falling back to put_terms_batch at \
          pgrdf.dict_batch_size or at quad-flush boundaries. An \
          unrecognised value silently falls back to 'combined'.",
        &INGEST_DICT_PATH,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_int_guc(
        c"pgrdf.dict_batch_size",
        c"Terms per put_terms_batch flush in the batched / combined paths.",
        c"500 by default. Higher values amortise the per-SPI cost \
          over more terms at the price of per-batch memory + latency \
          between parsing a term and writing the quad it appears in. \
          0 = fall back to single-term SPI (equivalent to selecting \
          ingest_dict_path = 'baseline').",
        &DICT_BATCH_SIZE,
        MIN_DICT_BATCH_SIZE,
        MAX_DICT_BATCH_SIZE,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_bool_guc(
        c"pgrdf.shmem_prewarm_on_init",
        c"Auto-prewarm the shmem dict cache before the first ingest.",
        c"When on, the first parse_turtle / load_turtle call in a \
          backend will run pgrdf.shmem_cache_prewarm() once before \
          its main work. Default off: the prewarm is one full SPI \
          scan of _pgrdf_dictionary and rarely amortises in \
          short-lived workloads.",
        &SHMEM_PREWARM_ON_INIT,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_bool_guc(
        c"pgrdf.auto_analyze",
        c"Run ANALYZE after materialize so the planner has fresh stats.",
        c"When on (default), pgrdf.materialize runs ANALYZE on \
          _pgrdf_quads after writing inferred triples. The inference \
          closure (e.g. owl:TransitiveProperty) inflates join \
          cardinalities; without fresh stats the planner mis-plans \
          complex multi-pattern queries on the materialized graph \
          (LUBM Q2: 180 s -> 1 s). ANALYZE is sample-based (sub-second), \
          so default on; set off to manage ANALYZE externally.",
        &AUTO_ANALYZE,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_string_guc(
        c"pgrdf.staged_temp_tablespaces",
        c"Tablespace(s) the staged bulk loader routes its temp spill to.",
        c"Empty by default (inherit the server's temp_tablespaces). When set \
          to a tablespace-name list (the same comma-separated identifier list \
          temp_tablespaces takes), the staged loader runs every phase \
          (STAGE/DICT/RESOLVE/INDEX) with SET LOCAL temp_tablespaces = '<value>', \
          routing temp files — dominated by RESOLVE's forced parallel 3-way hash \
          join, which spilled ~3 TB at 8.2 B rows — off the PGDATA disk onto the \
          named tablespace(s). The value must be a plain identifier list (no \
          quotes/semicolons); an unsafe value is rejected and the spill stays on \
          the server default.",
        &STAGED_TEMP_TABLESPACES,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_string_guc(
        c"pgrdf.staged_resolve_strategy",
        c"Join strategy the staged loader's RESOLVE phase forces.",
        c"One of 'auto' | 'hash' | 'index'. 'auto' forces no \
          join method, letting the planner choose with the adaptive \
          work_mem + dict resolve index already in place (bumps \
          hash_mem_multiplier only). 'hash' forces the all-hash-join \
          (the historical behaviour; identical output but spills multi-TB \
          to temp at 8.2 B rows). 'index' (default) forces the low-spill \
          index-nested-loop path — the at-scale-validated default \
          (out-of-the-box 8.2 B-triple load, no ENOSPC). The join output \
          is identical for any method — this is a performance knob, not a \
          correctness one. An unrecognised value warns and falls back to \
          'hash'.",
        &STAGED_RESOLVE_STRATEGY,
        GucContext::Userset,
        GucFlags::default(),
    );
    GucRegistry::define_int_guc(
        c"pgrdf.bulk_defer_index_min",
        c"Triple threshold above which bulk_load defers + rebuilds indexes.",
        c"load_turtle(..., bulk_load => true) on a fresh database drops the \
          hexastore (SPO/POS/OSP) and the dict _pgrdf_dict_val_idx index \
          before loading and rebuilds them afterwards, when the parsed \
          triple count is at least this value (default 100000). The \
          drop/rebuild takes ACCESS EXCLUSIVE on the global tables, so \
          small loads stay below it. Set very high to disable; 0 = always \
          defer (only safe with no concurrent writers on the tables).",
        &BULK_DEFER_INDEX_MIN,
        0,
        i32::MAX,
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

/// Resolved `pgrdf.ingest_dict_path` for this call. Reads the GUC,
/// parses it into the enum, applies the `dict_batch_size = 0` →
/// baseline override so the two GUCs combine in the obvious way.
pub(crate) fn ingest_dict_path() -> IngestDictPath {
    if DICT_BATCH_SIZE.get() == 0 {
        return IngestDictPath::Baseline;
    }
    let raw = INGEST_DICT_PATH.get();
    let s = raw.as_ref().and_then(|c| c.to_str().ok());
    IngestDictPath::from_guc_string(s)
}

/// Resolved `pgrdf.auto_analyze` — whether `materialize` should run
/// `ANALYZE` after writing inferred triples (M1; default on).
pub(crate) fn auto_analyze() -> bool {
    AUTO_ANALYZE.get()
}

/// Parsed `pgrdf.on_path_truncation` policy (issue #14).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OnPathTruncation {
    /// Bump `path_depth_truncations` only (silent; pre-#14 behaviour).
    Count,
    /// Counter + a client-visible WARNING per truncated walk.
    Warn,
    /// Fail the query rather than return a depth-truncated result.
    Error,
}

/// Resolved `pgrdf.on_path_truncation` for this session. An
/// unrecognised value warns (once per read) and behaves as `Warn` —
/// the default is already the honest mode, so a typo never *loosens*
/// the policy to silent.
pub(crate) fn on_path_truncation() -> OnPathTruncation {
    let raw = ON_PATH_TRUNCATION.get();
    match raw.as_ref().and_then(|c| c.to_str().ok()) {
        Some("count") => OnPathTruncation::Count,
        Some("warn") | None => OnPathTruncation::Warn,
        Some("error") => OnPathTruncation::Error,
        Some(other) => {
            pgrx::warning!(
                "pgrdf.on_path_truncation: unrecognised value '{other}' — behaving as 'warn'"
            );
            OnPathTruncation::Warn
        }
    }
}

/// Resolved `pgrdf.dict_batch_size` for this call. Returns at least
/// 1 (a 0 GUC routes to the baseline path before this is consulted).
pub(crate) fn dict_batch_size() -> usize {
    let n = DICT_BATCH_SIZE.get();
    if n <= 0 {
        DEFAULT_DICT_BATCH_SIZE as usize
    } else {
        n as usize
    }
}

/// Resolved `pgrdf.shmem_prewarm_on_init`. The lazy-prewarm latch in
/// `loader::ingest_dispatch` consults this on every ingest call.
pub(crate) fn shmem_prewarm_on_init() -> bool {
    SHMEM_PREWARM_ON_INIT.get()
}

/// Resolved `pgrdf.bulk_defer_index_min` — the triple threshold at/above
/// which the parallel bulk loader defers + rebuilds its indexes.
pub(crate) fn bulk_defer_index_min() -> i32 {
    BULK_DEFER_INDEX_MIN.get()
}

/// Resolved `pgrdf.staged_temp_tablespaces` for this session, trimmed.
/// `None`/blank → an empty string (the staged loader then emits no
/// `temp_tablespaces` override and inherits the server default). The
/// returned string is the operator's raw list; validation + SQL
/// fragment construction live in
/// [`crate::storage::staged::phases::temp_tablespaces_set_fragment`],
/// which the staged loader consults. Reads the GUC the same way as
/// [`ingest_dict_path`] (`.get()` → `Option<CString>` → `&str`).
pub(crate) fn staged_temp_tablespaces() -> String {
    let raw = STAGED_TEMP_TABLESPACES.get();
    raw.as_ref()
        .and_then(|c| c.to_str().ok())
        .map(str::trim)
        .unwrap_or("")
        .to_string()
}

/// Resolved `pgrdf.staged_resolve_strategy` for this session, trimmed
/// and lowercased. The GucSetting default is `"index"` (the
/// at-scale-validated low-spill path); a `None`/blank value — only
/// reachable if an operator explicitly `RESET`s/clears it — falls back
/// to `"auto"` here, which the mapping fn then renders as no forcing.
/// The returned
/// string is the operator's raw selection; the strategy → SQL-fragment
/// mapping (and the fallback for an unrecognised value) lives in
/// [`crate::storage::staged::phases::resolve_join_strategy_sql`], which
/// the staged loader's RESOLVE phase consults. Reads the GUC the same
/// way as [`staged_temp_tablespaces`].
pub(crate) fn staged_resolve_strategy() -> String {
    let raw = STAGED_RESOLVE_STRATEGY.get();
    raw.as_ref()
        .and_then(|c| c.to_str().ok())
        .map(str::trim)
        .unwrap_or("auto")
        .to_ascii_lowercase()
}
