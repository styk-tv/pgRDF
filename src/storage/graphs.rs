//! Named-graph IRI ↔ graph_id mapping.
//!
//! Phase A slice 120 lands the `_pgrdf_graphs` system table (LLD
//! v0.4 §3.1) via `sql/schema_v0_4_0_graphs.sql`. The IRI-keyed UDF
//! surface (`pgrdf.add_graph(iri)`, `pgrdf.graph_id(iri)`,
//! `pgrdf.graph_iri(id)`, plus the dual-arg `pgrdf.add_graph(id, iri)`
//! overload) lands in slices 118-115.
//!
//! Slice 117 — the dual-arg overload
//! [`super::hexastore::add_graph_id_iri`] surfaces as
//! `pgrdf.add_graph(id BIGINT, iri TEXT) → BIGINT`. Idempotent on a
//! matching `(id, iri)`; UPDATEs in place when `id` is currently
//! bound to its synthetic placeholder `urn:pgrdf:graph:{id}` (upgrade
//! path); panics with the stable `add_graph:` prefix on conflicting
//! bindings (id bound to a different non-synthetic IRI, or iri bound
//! to a different graph_id).
//!
//! Slice 119 — the existing integer-keyed
//! [`super::hexastore::add_graph`] now binds a synthetic IRI
//! `urn:pgrdf:graph:{id}` in `_pgrdf_graphs` on each successful
//! partition creation, so v0.3 callers get a queryable IRI mapping
//! for every graph they create through the integer surface. Same
//! signature, same return value, same idempotency — the new INSERT
//! is wrapped in `ON CONFLICT (graph_id) DO NOTHING`.
//!
//! Slice 116 — `pgrdf.graph_id(iri TEXT) → BIGINT` lookup
//! ([`graph_id`]). Read-only resolution of an IRI back to its
//! integer `graph_id` in `_pgrdf_graphs`, or `NULL` when the IRI is
//! not bound. Marked `#[pg_extern(strict)]` so a NULL input short-
//! circuits to NULL output without an SPI round trip; the `&str`
//! body therefore never sees a NULL argument. No panic on miss —
//! NULL is the lookup-miss signal, distinct from an actual SPI
//! error which still propagates.
//!
//! Slice 115 — `pgrdf.graph_iri(id BIGINT) → TEXT` lookup
//! ([`graph_iri`]). Symmetric inverse of slice 116. Read-only
//! resolution of an integer `graph_id` back to its bound IRI in
//! `_pgrdf_graphs`, or `NULL` when the id is not bound. Same
//! `#[pg_extern(strict)]` + scalar-subquery wrapper discipline as
//! slice 116; NULL input → NULL output, miss is NULL (no panic),
//! SPI errors propagate with the stable `graph_iri:` prefix.
//! Together with slice 116 this closes the §3.2 UDF surface.
//!
//! Slice 97 (Phase B) — `pgrdf.copy_graph(src BIGINT, dst BIGINT) →
//! BIGINT` ([`copy_graph`]). Per LLD v0.4 §5.1 the function copies
//! every row in `_pgrdf_quads_g<src>` into `_pgrdf_quads_g<dst>`
//! via an `INSERT INTO … SELECT` against the per-graph LIST
//! partitions, returning the count copied. Both `is_inferred =
//! FALSE` and `is_inferred = TRUE` rows carry forward verbatim
//! (the function is not `is_inferred`-discriminating per LLD §5.2
//! `copy_graph copies both` clause). If `_pgrdf_quads_g<dst>`
//! does not exist the function auto-creates it via
//! `pgrdf.add_graph(dst::bigint)`, which also binds the synthetic
//! `urn:pgrdf:graph:{dst}` IRI in `_pgrdf_graphs` per slice 119.
//! Idempotent on an absent source partition — returns 0 without
//! erroring; re-calling against the same `(src, dst)` would
//! duplicate rows, so callers are expected to `clear_graph(dst)`
//! first if they need re-call idempotency. `src == dst` panics
//! with `copy_graph: src and dst must differ`; negative ids panic
//! with `copy_graph: graph_id must be >= 0, got src=<S>,
//! dst=<D>`. Sibling of slice 96's `move_graph` (metadata-only
//! re-association) — `copy_graph` is the only lifecycle UDF that
//! touches every row, so it scales with the source row count
//! while the other three are partition-DDL-bounded.
//!
//! Slice 99 (Phase B) — `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN
//! DEFAULT TRUE) → BIGINT` ([`drop_graph`]). Removes the LIST partition
//! `_pgrdf_quads_g<id>` from the parent `_pgrdf_quads` (`DETACH
//! PARTITION` + `DROP TABLE`) and the matching `_pgrdf_graphs` row,
//! returning the count of triples that lived in the partition at the
//! time of the drop. `cascade => FALSE` errors with the stable
//! `drop_graph: inferred rows present` prefix if any `is_inferred =
//! TRUE` row exists in the partition. Default partition (graph_id =
//! 0) cannot be dropped; negative ids panic with the
//! `drop_graph: graph_id must be >= 0` prefix; dropping an absent
//! graph is a 0-return no-op (idempotent). Opens Phase B (lifecycle
//! UDFs §5) toward v0.4.2.
//!
//! Slice 96 (Phase B) — `pgrdf.move_graph(src BIGINT, dst BIGINT) →
//! BIGINT` ([`move_graph`]). Migrates every quad in graph `src` to
//! graph `dst`, returning the count of triples moved. The v0.4.2
//! implementation is a **compose** over the sibling primitives:
//! `pgrdf.copy_graph(src, dst)` (slice 97) then
//! `pgrdf.drop_graph(src, cascade => TRUE)` (slice 99). Semantically
//! equivalent to the LLD §5.2 "metadata-only DETACH/ATTACH partition
//! rebind" — but tractable without the partition-constraint dance
//! around updating every row's `graph_id` column. The §5.2 claim that
//! `move_graph` is metadata-only is aspirational; flagged as a v0.5
//! perf optimisation. Slice 97's `copy_graph` is a runtime dependency
//! (referenced by SQL string, not Rust symbol) — the build succeeds
//! standalone, but `move_graph` calls FAIL at runtime until slice 97
//! lands in the parent merge.
//!
//! Guards:
//! - `src < 0 || dst < 0` panics with the stable
//!   `move_graph: graph_id must be >= 0` prefix.
//! - `src == dst` panics with `move_graph: src and dst must differ`.
//! - `dst` partition already holds rows → panics with
//!   `move_graph: dst graph_id <N> already has data`.
//!
//! Idempotency:
//! - `src` partition absent → returns 0, no error (LLD §5.2).
//!
//! Slice 98 (Phase B) — `pgrdf.clear_graph(id BIGINT) → BIGINT`
//! lifecycle UDF ([`clear_graph`]). Per LLD v0.4 §5.1 the function
//! `TRUNCATE ONLY`s the per-graph LIST partition
//! (`_pgrdf_quads_g<id>`), wiping every row (base + inferred) but
//! keeping the partition attached + its `_pgrdf_graphs` IRI
//! binding intact (so subsequent inserts route normally and
//! `graph_iri(id)` still resolves). Returns the rows-removed
//! count (the row count immediately prior to the TRUNCATE).
//! Idempotent on an absent or already-empty graph — returns 0,
//! never errors. Permitted on `graph_id = 0` (the default catch-
//! all partition); the sibling `drop_graph(0)` is the rejection
//! site, not this one. Negative ids panic with the stable
//! `clear_graph: graph_id must be >= 0, got <N>` prefix.
//!
//! Reference: SPEC.pgRDF.LLD.v0.4 §3.1, §3.2, §5.1.

use crate::storage::partition::acquire_partition_ddl_gate;
use pgrx::prelude::*;

/// Look up the integer `graph_id` bound to an IRI in
/// `_pgrdf_graphs`. Returns `NULL` if the IRI is not bound.
///
/// Read-only — no side effects, no panic on miss. Marked `strict`
/// so Postgres short-circuits a NULL argument to NULL output
/// without invoking the function (hence the `&str` body never sees
/// a NULL input). The inner `SELECT (subquery)` idiom keeps SPI on
/// the "exactly one row" path: NULL when the IRI is unbound, the
/// id otherwise — avoiding the `SpiTupleTable positioned before the
/// start` empty-result trip that a bare `SELECT … WHERE iri = $1`
/// would cause. Same wrapper trick as the IRI-keyed `add_graph`
/// overload in [`super::hexastore::add_graph_iri`].
///
/// SQL surface: `pgrdf.graph_id(iri TEXT) → BIGINT`. Per LLD v0.4
/// §3.2.
#[pg_extern(strict)]
fn graph_id(iri: &str) -> Option<i64> {
    Spi::get_one_with_args(
        "SELECT (SELECT graph_id FROM pgrdf._pgrdf_graphs WHERE iri = $1 LIMIT 1)",
        &[iri.into()],
    )
    .unwrap_or_else(|e| panic!("graph_id: lookup failed: {e}"))
}

/// Look up the IRI bound to a `graph_id` in `_pgrdf_graphs`.
/// Returns `NULL` if the id is not bound.
///
/// Read-only — no side effects, no panic on miss. Marked `strict`
/// so Postgres short-circuits a NULL argument to NULL output
/// without invoking the function. The inner `SELECT (subquery)`
/// idiom keeps SPI on the "exactly one row" path: NULL when the
/// id is unbound, the IRI otherwise — same wrapper trick as slice
/// 116's `graph_id` UDF.
///
/// SQL surface: `pgrdf.graph_iri(id BIGINT) → TEXT`. Per LLD v0.4
/// §3.2. Symmetric inverse of [`graph_id`].
#[pg_extern(strict)]
fn graph_iri(id: i64) -> Option<String> {
    Spi::get_one_with_args(
        "SELECT (SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = $1 LIMIT 1)",
        &[id.into()],
    )
    .unwrap_or_else(|e| panic!("graph_iri: lookup failed: {e}"))
}

/// Remove the LIST partition `_pgrdf_quads_g<id>` from the parent
/// `_pgrdf_quads`, drop the partition's row storage and indexes, and
/// remove the matching `_pgrdf_graphs` row. Returns the count of
/// triples that were in the partition before the drop.
///
/// Per LLD v0.4 §5.1 / §5.2: `DETACH PARTITION` is metadata-only;
/// `DROP TABLE` releases the partition's heap and btree pages. The
/// metadata window takes an `ACCESS EXCLUSIVE` lock on the parent —
/// the user-facing tradeoff documented for the "long-running
/// maintenance" workflow.
///
/// Defaulting: `cascade` defaults to `TRUE` so the common-case caller
/// (drop a graph regardless of its inferred-rows content) writes
/// `pgrdf.drop_graph(42)` without a second arg. `cascade => FALSE`
/// is the strict mode: any `is_inferred = TRUE` row blocks the drop
/// with the stable `drop_graph: inferred rows present` prefix.
///
/// Idempotency: dropping a non-existent partition returns 0 and does
/// NOT error (per LLD v0.4 §5.2 idempotency clause). A lingering
/// `_pgrdf_graphs` row for an already-absent partition is cleaned up
/// on this path so the IRI mapping converges with reality even if a
/// prior crash left the binding stranded.
///
/// Guards:
/// - `id < 0` panics with `drop_graph: graph_id must be >= 0, got <N>`.
/// - `id == 0` panics with `drop_graph: cannot drop default partition
///   (graph_id = 0)` — `_pgrdf_quads_default` is the catch-all bucket
///   for unbound graph_ids and is not user-droppable.
///
/// Atomicity: the inferred-rows check and the DETACH/DROP happen in
/// the same transaction (the calling statement's transaction). A
/// concurrent INSERT of an `is_inferred = TRUE` row arriving between
/// the check and the DROP would either block on the parent lock (if
/// the check has already committed visibility) or be lost with the
/// partition (if it commits inside the same window). The cascade
/// guard is a best-effort signal for downstream maintenance flows;
/// the partition-DDL lock makes the window narrow.
///
/// SQL surface: `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN DEFAULT
/// TRUE) → BIGINT`. Per LLD v0.4 §5.1.
#[pg_extern]
fn drop_graph(id: i64, cascade: default!(bool, "true")) -> i64 {
    if id < 0 {
        panic!("drop_graph: graph_id must be >= 0, got {id}");
    }
    if id == 0 {
        panic!("drop_graph: cannot drop default partition (graph_id = 0)");
    }

    // Partition-DDL gate FIRST — the same global outermost lock the
    // `add_graph` family takes. `DETACH PARTITION` / `DROP TABLE`
    // below escalate to `AccessExclusiveLock` on the `_pgrdf_quads`
    // parent (the original deadlock surface), and this path also
    // writes `_pgrdf_graphs`. Taking the gate before either makes a
    // concurrent `add_graph` / `drop_graph` QUEUE on the advisory
    // lock instead of racing the parent's catalog lock. Re-entrant
    // and xact-scoped (releases at the pgrx rollback boundary).
    acquire_partition_ddl_gate();

    // Partition existence check — idempotent path returns 0 without
    // error when the partition is already absent. We still clean up
    // a possibly-stranded `_pgrdf_graphs` row so the IRI mapping
    // converges with reality on a crash-recovery code path.
    let part_name = format!("_pgrdf_quads_g{id}");
    let exists: bool = Spi::get_one_with_args(
        "SELECT EXISTS(
            SELECT 1 FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relname = $1
         )",
        &[part_name.as_str().into()],
    )
    .unwrap_or_else(|e| panic!("drop_graph: partition existence check failed: {e}"))
    .unwrap_or(false);

    if !exists {
        // Idempotent: prune any stale `_pgrdf_graphs` row pointing at
        // the non-existent partition, return 0.
        Spi::run_with_args(
            "DELETE FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
            &[id.into()],
        )
        .unwrap_or_else(|e| panic!("drop_graph: stale _pgrdf_graphs row cleanup failed: {e}"));
        return 0;
    }

    // Count triples about to be dropped — the return value of the UDF.
    // `count(*)::bigint` always yields exactly one row, no scalar-
    // subquery wrapper needed. The format!-built SQL is safe: the
    // partition name is constructed from a validated non-negative
    // BIGINT (no user input in identifier position).
    let total: i64 = Spi::get_one(&format!("SELECT count(*)::bigint FROM pgrdf.{part_name}"))
        .unwrap_or_else(|e| panic!("drop_graph: count failed: {e}"))
        .unwrap_or(0);

    // Cascade guard — only when the caller asks for strict mode.
    if !cascade {
        let has_inferred: bool = Spi::get_one(&format!(
            "SELECT EXISTS(SELECT 1 FROM pgrdf.{part_name} WHERE is_inferred)"
        ))
        .unwrap_or_else(|e| panic!("drop_graph: is_inferred check failed: {e}"))
        .unwrap_or(false);
        if has_inferred {
            panic!(
                "drop_graph: inferred rows present (graph_id = {id}); \
                 pass cascade => true to proceed"
            );
        }
    }

    // DETACH + DROP — partition-DDL metadata window under ACCESS
    // EXCLUSIVE on the parent. DETACH first so DROP TABLE doesn't
    // need partition-aware locking; the partition becomes a regular
    // table for the duration of one statement before going away.
    Spi::run(&format!(
        "ALTER TABLE pgrdf._pgrdf_quads DETACH PARTITION pgrdf.{part_name}"
    ))
    .unwrap_or_else(|e| panic!("drop_graph: DETACH PARTITION failed: {e}"));
    Spi::run(&format!("DROP TABLE pgrdf.{part_name}"))
        .unwrap_or_else(|e| panic!("drop_graph: DROP TABLE failed: {e}"));

    // Remove the IRI binding so `pgrdf.graph_iri(id)` and
    // `pgrdf.graph_id(iri)` start returning NULL post-drop, per
    // LLD v0.4 §5.2 `_pgrdf_graphs` invalidation clause.
    Spi::run_with_args(
        "DELETE FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
        &[id.into()],
    )
    .unwrap_or_else(|e| panic!("drop_graph: _pgrdf_graphs row cleanup failed: {e}"));

    total
}

/// Migrate every quad from graph `src` to graph `dst` and remove
/// `src` afterwards. Returns the number of triples moved (the row
/// count of `src` at copy time).
///
/// **Implementation strategy — compose over siblings.** v0.4.2
/// implements `move_graph` as `pgrdf.copy_graph(src, dst)` followed
/// by `pgrdf.drop_graph(src, cascade => TRUE)`. Both halves run in
/// the calling statement's transaction, so a rollback unwinds both.
/// Semantically equivalent to the LLD §5.2 "DETACH partition +
/// rebind `FOR VALUES IN(<dst>)` + ATTACH" path, but tractable
/// without the partition-constraint check requiring an UPDATE of
/// every row's `graph_id` column. The metadata-only claim in
/// §5.2 is aspirational; flagged as a v0.5 perf optimisation.
///
/// **Runtime dependency on slice 97.** `pgrdf.copy_graph` is
/// referenced by SQL string (not Rust symbol). The build is fine
/// standalone — pgrx generates `#[pg_extern]` SQL declarations from
/// Rust signatures, and the inner `SELECT pgrdf.copy_graph(...)`
/// resolves at runtime. Calls to `move_graph` therefore FAIL at
/// runtime until slice 97 (Phase B `copy_graph`) lands in the
/// parent merge. Tests that exercise the runtime path are written
/// in this slice anyway; they go green on merge.
///
/// Guards:
/// - `src < 0 || dst < 0` panics with the stable
///   `move_graph: graph_id must be >= 0` prefix (matches the
///   shape of `drop_graph` / `clear_graph` / `add_graph(g BIGINT)`).
/// - `src == dst` panics with `move_graph: src and dst must differ`.
///   A self-move is meaningless (and the compose would drop the
///   graph after copying it to itself — destructive). Explicit
///   rejection is safer than a no-op.
/// - `dst` partition already holds rows → panics with
///   `move_graph: dst graph_id <N> already has data (<M> rows);
///   clear or drop it first`. Stable prefix routing.
///
/// Idempotency: when `src` partition is absent, returns 0 without
/// erroring (LLD §5.2 idempotency invariant for the whole
/// lifecycle-UDF family). The compose short-circuits before
/// invoking `copy_graph` / `drop_graph` to avoid the second pass
/// failing — slice 99's `drop_graph` is already idempotent on the
/// absent path, but the explicit early return keeps the return
/// value at 0 (which is what callers expect when they're routing
/// `move_graph` into a cleanup workflow without first probing
/// partition existence).
///
/// `_pgrdf_graphs` invalidation: the compose inherits slice 99's
/// behaviour — the `src` row is removed (drop step), and the `dst`
/// row is allocated if absent (copy step's responsibility per
/// slice 97). If `dst` was already bound to a different IRI, that
/// binding is preserved (slice 97 must not clobber a pre-existing
/// binding).
///
/// SQL surface: `pgrdf.move_graph(src BIGINT, dst BIGINT) → BIGINT`.
/// Per LLD v0.4 §5.1.
#[pg_extern]
fn move_graph(src: i64, dst: i64) -> i64 {
    if src < 0 || dst < 0 {
        panic!("move_graph: graph_id must be >= 0, got src={src}, dst={dst}");
    }
    if src == dst {
        panic!("move_graph: src and dst must differ (both = {src})");
    }

    // Idempotent miss: src partition absent → 0 return, no error.
    // The compose's `copy_graph` step would also short-circuit on an
    // absent src, but we want the explicit early return so the
    // sibling `drop_graph(src)` step isn't invoked needlessly (it'd
    // be a no-op too, but the symmetry with the other lifecycle
    // UDFs is clearer this way).
    let src_name = format!("_pgrdf_quads_g{src}");
    let src_exists: bool = Spi::get_one_with_args(
        "SELECT EXISTS(
            SELECT 1 FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relname = $1
         )",
        &[src_name.as_str().into()],
    )
    .unwrap_or_else(|e| panic!("move_graph: src existence check failed: {e}"))
    .unwrap_or(false);

    if !src_exists {
        return 0;
    }

    // Dst guard: if the dst partition already exists AND holds rows,
    // refuse to clobber. An empty existing partition (e.g. a prior
    // `add_graph(dst)` with no inserts) is fine — the copy step
    // inserts into it. A pre-bound IRI on dst is preserved.
    let dst_name = format!("_pgrdf_quads_g{dst}");
    let dst_exists: bool = Spi::get_one_with_args(
        "SELECT EXISTS(
            SELECT 1 FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relname = $1
         )",
        &[dst_name.as_str().into()],
    )
    .unwrap_or_else(|e| panic!("move_graph: dst existence check failed: {e}"))
    .unwrap_or(false);

    if dst_exists {
        // `dst_name` is built from a validated non-negative BIGINT,
        // no user input in identifier position — same safe-format
        // convention as `drop_graph` / `clear_graph`.
        let dst_count: i64 =
            Spi::get_one(&format!("SELECT count(*)::bigint FROM pgrdf.{dst_name}"))
                .unwrap_or_else(|e| panic!("move_graph: dst count failed: {e}"))
                .unwrap_or(0);
        if dst_count > 0 {
            panic!(
                "move_graph: dst graph_id {dst} already has data ({dst_count} rows); \
                 clear or drop it first"
            );
        }
    }

    // Compose step 1: copy src → dst. Slice 97's `copy_graph`
    // returns the row count copied — that's our return value.
    // Runtime dependency: this SELECT errors until slice 97 lands.
    let copied: i64 = Spi::get_one_with_args(
        "SELECT pgrdf.copy_graph($1::bigint, $2::bigint)",
        &[src.into(), dst.into()],
    )
    .unwrap_or_else(|e| panic!("move_graph: copy_graph step failed: {e}"))
    .unwrap_or(0);

    // Compose step 2: drop src. `cascade => true` so any inferred
    // rows in src (already copied across to dst above) come away
    // with the partition. The drop's return value is discarded —
    // we report `copied` as the move count, which is the row count
    // at copy time.
    Spi::run_with_args("SELECT pgrdf.drop_graph($1::bigint, true)", &[src.into()])
        .unwrap_or_else(|e| panic!("move_graph: drop_graph step failed: {e}"));

    copied
}

/// Truncate every row in graph `id`'s LIST partition
/// (`pgrdf._pgrdf_quads_g<id>`). Returns the rows-removed count
/// (== the row count captured immediately before the TRUNCATE).
///
/// Keeps the partition attached + the `_pgrdf_graphs` IRI
/// binding intact: subsequent inserts route to the same
/// partition, and `graph_iri(id)` still resolves to whatever
/// IRI was bound. The function is the bulk-discard counterpart
/// of the sibling slice-99 `drop_graph(id)` (which detaches +
/// drops the partition outright + invalidates the IRI binding).
///
/// `TRUNCATE ONLY` is deliberate: `ONLY` blocks the cascade to
/// descendant partitions. The per-graph partition has no
/// children today, but `ONLY` is defence-in-depth against a
/// future sub-partitioning slice silently widening the scope.
///
/// **Idempotent on absent / empty graphs.** If
/// `_pgrdf_quads_g<id>` is missing entirely (no `add_graph(id)`
/// has ever run), return 0 without erroring — per LLD v0.4 §5.2
/// idempotency invariant. If the partition exists but is empty,
/// the row-count read returns 0, `TRUNCATE` no-ops on a
/// zero-row relation, and we return 0 again. Calling
/// `clear_graph(id)` twice in succession therefore returns
/// `(N, 0)`.
///
/// **`graph_id = 0` is permitted.** Unlike `drop_graph(0)`
/// (which would destroy the catch-all bucket that every
/// unrouted insert lands in), clearing the default partition
/// just empties it. The default-partition `_pgrdf_graphs` row
/// stays put, so the synthetic IRI `urn:pgrdf:graph:0`
/// continues to resolve.
///
/// **Negative id** panics with the stable `clear_graph:` prefix
/// — same error-shape contract as `add_graph(id BIGINT)`
/// (slice 119) so downstream callers route on the prefix.
///
/// SQL surface: `pgrdf.clear_graph(id BIGINT) → BIGINT`. Per
/// LLD v0.4 §5.1.
#[pg_extern]
fn clear_graph(id: i64) -> i64 {
    if id < 0 {
        panic!("clear_graph: graph_id must be >= 0, got {id}");
    }

    let partition_name = format!("_pgrdf_quads_g{id}");

    // Existence check via pg_catalog — `pgrdf.<name>` is qualified
    // by the namespace OID join so we don't false-match a relation
    // with the same name in a different schema. Idempotent miss:
    // an absent partition returns 0 without erroring.
    let exists: bool = Spi::get_one_with_args(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_class c \
                       JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                       WHERE c.relname = $1 AND n.nspname = 'pgrdf')",
        &[partition_name.as_str().into()],
    )
    .unwrap_or_else(|e| panic!("clear_graph: partition existence check failed: {e}"))
    .unwrap_or(false);

    if !exists {
        return 0;
    }

    // Capture the row count *before* the TRUNCATE so we can return
    // it. `partition_name` is a string we constructed from a BIGINT
    // — no user input in a SQL identifier position — so direct
    // interpolation into the dynamic SQL is safe (same convention as
    // `add_graph(g BIGINT)` in `hexastore.rs`).
    let total: i64 = Spi::get_one(&format!(
        "SELECT count(*)::bigint FROM pgrdf.{partition_name}"
    ))
    .unwrap_or_else(|e| panic!("clear_graph: count failed: {e}"))
    .unwrap_or(0);

    // `ONLY` keeps the truncate scoped to this partition table
    // alone — no cascade to any (future) descendants. The partition
    // shell stays attached to `_pgrdf_quads`, so the next
    // `INSERT … VALUES (..., $1 = id, ...)` routes here without
    // touching the default partition.
    Spi::run(&format!("TRUNCATE ONLY pgrdf.{partition_name}"))
        .unwrap_or_else(|e| panic!("clear_graph: TRUNCATE failed: {e}"));

    total
}

/// Copy every row from graph `src`'s LIST partition into graph
/// `dst`'s LIST partition. Returns the count copied (the row count
/// of `src` captured immediately before the INSERT).
///
/// Per LLD v0.4 §5.1 / §5.2: `copy_graph` is the only graph-level
/// lifecycle UDF that touches every row — the partition-DDL siblings
/// (`drop_graph`, `move_graph`, and `clear_graph`'s `TRUNCATE`) are
/// all metadata-bounded. The work is a single
/// `INSERT INTO pgrdf._pgrdf_quads_g<dst> (subject_id, predicate_id,
/// object_id, graph_id, is_inferred) SELECT subject_id, predicate_id,
/// object_id, <dst>::bigint, is_inferred FROM pgrdf._pgrdf_quads_g<src>`
/// — the `graph_id` projection rebinds to the destination id so the
/// partition router lands the rows in `dst`'s partition without
/// touching `_pgrdf_quads_default`. Both `is_inferred = FALSE` and
/// `is_inferred = TRUE` rows are copied verbatim — entailment state
/// carries forward.
///
/// **Destination auto-creation.** If `_pgrdf_quads_g<dst>` does not
/// exist, the function calls `pgrdf.add_graph(dst::bigint)` to
/// create it. That call also binds a synthetic
/// `urn:pgrdf:graph:{dst}` IRI in `_pgrdf_graphs` per slice 119, so
/// `pgrdf.graph_iri(dst)` resolves post-copy even if the caller
/// hadn't pre-registered the destination. If `dst` was already
/// bound to a different IRI, that binding is preserved unchanged
/// (the partition existence check short-circuits before
/// `add_graph` runs).
///
/// **Source absence is idempotent.** If `_pgrdf_quads_g<src>` does
/// not exist (no `add_graph(src)` has ever run), the function
/// returns 0 without erroring. This matches the §5.2 idempotency
/// invariant: every lifecycle UDF returns 0 (no-op) on inputs
/// naming an empty or absent graph.
///
/// **Re-call duplicates.** Calling `copy_graph(src, dst)` twice
/// against the same `(src, dst)` pair would duplicate every source
/// row in `dst` — the function does NOT clear `dst` before
/// inserting. Callers needing strict idempotency should invoke
/// `pgrdf.clear_graph(dst)` before the second copy. This is the
/// `ADD` (W3C SPARQL 1.1 Update §3.2.6) vs `COPY` distinction
/// pushed into the caller's responsibility.
///
/// Guards (stable error prefixes per the error-message contract):
///
/// - `src < 0 || dst < 0` panics with
///   `copy_graph: graph_id must be >= 0, got src=<S>, dst=<D>`.
/// - `src == dst` panics with
///   `copy_graph: src and dst must differ (both = <id>)` — the
///   self-copy degenerate case has no defined semantics (a single
///   `INSERT … SELECT` from a table into itself would scan + insert
///   in unpredictable interleavings on a partitioned table) and is
///   rejected outright.
///
/// SQL surface: `pgrdf.copy_graph(src BIGINT, dst BIGINT) → BIGINT`.
/// Per LLD v0.4 §5.1. Sibling slice-96 `move_graph` provides the
/// constant-time metadata-only association swap; `copy_graph` is the
/// row-touching counterpart that leaves `src` intact.
#[pg_extern]
fn copy_graph(src: i64, dst: i64) -> i64 {
    if src < 0 || dst < 0 {
        panic!("copy_graph: graph_id must be >= 0, got src={src}, dst={dst}");
    }
    if src == dst {
        panic!("copy_graph: src and dst must differ (both = {src})");
    }

    // Source partition existence check — idempotent miss path
    // returns 0 without erroring per LLD v0.4 §5.2.
    let src_name = format!("_pgrdf_quads_g{src}");
    let src_exists: bool = Spi::get_one_with_args(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_class c \
                       JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                       WHERE c.relname = $1 AND n.nspname = 'pgrdf')",
        &[src_name.as_str().into()],
    )
    .unwrap_or_else(|e| panic!("copy_graph: src existence check failed: {e}"))
    .unwrap_or(false);

    if !src_exists {
        return 0;
    }

    // Destination partition auto-create — `add_graph(dst)` is
    // idempotent (slice 119 wraps the synthetic-IRI insert in
    // ON CONFLICT DO NOTHING + CREATE TABLE IF NOT EXISTS), but
    // we still gate on existence so we don't pay the round-trip
    // when the partition is already there.
    let dst_name = format!("_pgrdf_quads_g{dst}");
    let dst_exists: bool = Spi::get_one_with_args(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_class c \
                       JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                       WHERE c.relname = $1 AND n.nspname = 'pgrdf')",
        &[dst_name.as_str().into()],
    )
    .unwrap_or_else(|e| panic!("copy_graph: dst existence check failed: {e}"))
    .unwrap_or(false);

    if !dst_exists {
        Spi::run_with_args("SELECT pgrdf.add_graph($1::bigint)", &[dst.into()])
            .unwrap_or_else(|e| panic!("copy_graph: dst partition creation failed: {e}"));
    }

    // Capture the source row count up-front — the return value of
    // the UDF. `count(*)::bigint` always yields exactly one row,
    // no scalar-subquery wrapper needed. The format!-built SQL is
    // safe: the partition name is constructed from a validated
    // non-negative BIGINT (no user input in identifier position).
    let count: i64 = Spi::get_one(&format!("SELECT count(*)::bigint FROM pgrdf.{src_name}"))
        .unwrap_or_else(|e| panic!("copy_graph: count failed: {e}"))
        .unwrap_or(0);

    if count == 0 {
        return 0;
    }

    // The copy itself — `INSERT INTO <dst> SELECT … FROM <src>`
    // with the `graph_id` projection rebound to `dst`. Both
    // `is_inferred = FALSE` and `is_inferred = TRUE` rows carry
    // forward (no WHERE-clause discrimination). The explicit
    // column list on the INSERT side keeps the projection
    // order-independent of any future `_pgrdf_quads` column
    // additions.
    Spi::run(&format!(
        "INSERT INTO pgrdf.{dst_name} \
            (subject_id, predicate_id, object_id, graph_id, is_inferred) \
         SELECT subject_id, predicate_id, object_id, {dst}::bigint, is_inferred \
           FROM pgrdf.{src_name}"
    ))
    .unwrap_or_else(|e| panic!("copy_graph: INSERT INTO ... SELECT failed: {e}"));

    count
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use crate::storage::partition::create_quads_partition;
    use pgrx::prelude::*;

    /// Slice 120 — the table is materialised at `CREATE EXTENSION`
    /// time, the default-partition seed row `(0, 'urn:pgrdf:graph:0')`
    /// is the sole resident, and the columns carry the IRI string
    /// surface that subsequent slices key off of.
    #[pg_test]
    fn pgrdf_graphs_seed_row() {
        // Exactly one row — the seed for `graph_id = 0`. Any future
        // auto-seed addition trips this and forces a deliberate update
        // alongside the new behaviour.
        let count: i64 = Spi::get_one("SELECT count(*)::BIGINT FROM pgrdf._pgrdf_graphs")
            .expect("count query failed")
            .expect("count returned NULL");
        assert_eq!(count, 1, "expected exactly 1 seed row, got {count}");

        // The seed IRI is the synthetic `urn:pgrdf:graph:{id}` shape
        // that slice 117 will reuse for back-compat with
        // `pgrdf.add_graph(id BIGINT)`. Pinning the literal here keeps
        // the catch-all bucket's user-visible name stable across the
        // remaining Phase A slices.
        let iri: String = Spi::get_one("SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 0")
            .expect("iri lookup failed")
            .expect("iri returned NULL");
        assert_eq!(iri, "urn:pgrdf:graph:0");
    }

    /// Slice 119 — `pgrdf.add_graph(id BIGINT)` populates
    /// `_pgrdf_graphs` with the synthetic IRI `urn:pgrdf:graph:{id}`
    /// on each successful partition creation. Idempotent re-call
    /// produces no extra row and no error.
    #[pg_test]
    fn add_graph_populates_synthetic_iri() {
        // First call binds the IRI.
        Spi::run("SELECT pgrdf.add_graph(42)").expect("add_graph(42) failed");
        let iri: Option<String> =
            Spi::get_one("SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 42")
                .expect("iri lookup failed");
        assert_eq!(iri.as_deref(), Some("urn:pgrdf:graph:42"));

        // Re-calling is idempotent — partition exists, IRI row stays
        // single, no error from the ON CONFLICT clause.
        Spi::run("SELECT pgrdf.add_graph(42)").expect("idempotent add_graph(42) failed");
        let count: i64 =
            Spi::get_one("SELECT count(*)::BIGINT FROM pgrdf._pgrdf_graphs WHERE graph_id = 42")
                .expect("count query failed")
                .expect("count returned NULL");
        assert_eq!(
            count, 1,
            "expected exactly one row for graph_id = 42, got {count}"
        );

        // A second distinct id gets its own row.
        Spi::run("SELECT pgrdf.add_graph(100)").expect("add_graph(100) failed");
        let iri100: Option<String> =
            Spi::get_one("SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 100")
                .expect("iri lookup failed");
        assert_eq!(iri100.as_deref(), Some("urn:pgrdf:graph:100"));
    }

    /// Slice 118 — `pgrdf.add_graph(iri TEXT) → BIGINT` overload is
    /// idempotent on the IRI: a repeat call with the same IRI returns
    /// the same auto-allocated id, and a distinct IRI gets a distinct
    /// id. Bound row carries the user-supplied IRI verbatim — the
    /// slice-119 synthetic-IRI insert path on the integer overload is
    /// pre-empted by the pre-INSERT inside the IRI overload, so the
    /// `ON CONFLICT (graph_id) DO NOTHING` clause keeps it intact.
    #[pg_test]
    fn add_graph_iri_idempotent() {
        let id1: i64 = Spi::get_one("SELECT pgrdf.add_graph('http://example.org/g1')")
            .expect("first add_graph(iri) failed")
            .expect("first add_graph(iri) returned NULL");
        let id2: i64 = Spi::get_one("SELECT pgrdf.add_graph('http://example.org/g1')")
            .expect("repeat add_graph(iri) failed")
            .expect("repeat add_graph(iri) returned NULL");
        assert_eq!(id1, id2, "second call with same IRI must return same id");

        let id3: i64 = Spi::get_one("SELECT pgrdf.add_graph('http://example.org/g2')")
            .expect("distinct add_graph(iri) failed")
            .expect("distinct add_graph(iri) returned NULL");
        assert_ne!(id3, id1, "distinct IRI must get distinct id");

        // User-supplied IRI persists verbatim — synthetic IRI must
        // NOT clobber the binding.
        let iri1: Option<String> = Spi::get_one_with_args(
            "SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
            &[id1.into()],
        )
        .expect("iri lookup failed");
        assert_eq!(iri1.as_deref(), Some("http://example.org/g1"));
    }

    /// Slice 118 — empty IRI panics with the stable `add_graph:`
    /// prefix per the regression error-message contract.
    #[pg_test(error = "add_graph: iri must be non-empty")]
    fn add_graph_iri_empty_rejected() {
        Spi::run("SELECT pgrdf.add_graph('')").unwrap();
    }

    /// Slice 117 — `pgrdf.add_graph(id BIGINT, iri TEXT) → BIGINT`
    /// fresh-pair path. Caller supplies both halves; the function
    /// INSERTs the binding verbatim, creates the LIST partition, and
    /// echoes `id` back.
    #[pg_test]
    fn add_graph_id_iri_fresh_pair() {
        let id: i64 = Spi::get_one("SELECT pgrdf.add_graph(50::bigint, 'http://example.org/g50')")
            .expect("fresh add_graph(id, iri) failed")
            .expect("fresh add_graph(id, iri) returned NULL");
        assert_eq!(id, 50, "echoed id must equal the input");
        let bound: Option<String> =
            Spi::get_one("SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 50")
                .expect("iri lookup failed");
        assert_eq!(bound.as_deref(), Some("http://example.org/g50"));
    }

    /// Slice 117 — synthetic-IRI upgrade path: a prior
    /// `add_graph(60)` (slice 119) seeds `urn:pgrdf:graph:60`; a
    /// subsequent `add_graph(60, 'http://example.org/g60')` UPDATEs
    /// the row in place. The row count stays at 1 for graph_id = 60
    /// — no duplicate, no error.
    #[pg_test]
    fn add_graph_id_iri_synthetic_upgrade() {
        Spi::run("SELECT pgrdf.add_graph(60::bigint)").expect("seed add_graph(60) failed");
        let synthetic: Option<String> =
            Spi::get_one("SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 60")
                .expect("synthetic iri lookup failed");
        assert_eq!(synthetic.as_deref(), Some("urn:pgrdf:graph:60"));

        let id: i64 = Spi::get_one("SELECT pgrdf.add_graph(60::bigint, 'http://example.org/g60')")
            .expect("upgrade add_graph(60, iri) failed")
            .expect("upgrade add_graph(60, iri) returned NULL");
        assert_eq!(id, 60);

        let upgraded: Option<String> =
            Spi::get_one("SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 60")
                .expect("upgraded iri lookup failed");
        assert_eq!(upgraded.as_deref(), Some("http://example.org/g60"));

        let count: i64 =
            Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_graphs WHERE graph_id = 60")
                .expect("row count failed")
                .expect("row count returned NULL");
        assert_eq!(
            count, 1,
            "synthetic upgrade must UPDATE in place, not duplicate"
        );
    }

    /// Slice 117 — id-conflict path: `id` is already bound to a
    /// non-synthetic IRI different from the requested one. Stable
    /// `add_graph: graph_id 70 is bound to a different IRI` prefix.
    /// The pgrx `error =` attribute matches the panic message
    /// exactly, so the trailing `(<existing_iri>)` is included.
    #[pg_test(
        error = "add_graph: graph_id 70 is bound to a different IRI (http://example.org/g70)"
    )]
    fn add_graph_id_iri_id_conflict() {
        Spi::run("SELECT pgrdf.add_graph(70::bigint, 'http://example.org/g70')")
            .expect("first add_graph(70, iri) failed");
        Spi::run("SELECT pgrdf.add_graph(70::bigint, 'http://example.org/different')").unwrap();
    }

    /// Slice 117 — iri-conflict path: the IRI is already bound to a
    /// different `graph_id`. Stable `add_graph: iri … is bound to a
    /// different graph_id (<existing>)` shape.
    #[pg_test(
        error = "add_graph: iri http://example.org/shared is bound to a different graph_id (80)"
    )]
    fn add_graph_id_iri_iri_conflict() {
        Spi::run("SELECT pgrdf.add_graph(80::bigint, 'http://example.org/shared')")
            .expect("first add_graph(80, iri) failed");
        Spi::run("SELECT pgrdf.add_graph(81::bigint, 'http://example.org/shared')").unwrap();
    }

    /// Slice 116 — seed row `(0, 'urn:pgrdf:graph:0')` is reachable
    /// via `pgrdf.graph_id('urn:pgrdf:graph:0')` immediately after
    /// `CREATE EXTENSION`. Anchors the default-partition IRI as a
    /// stable lookup target across the rest of Phase A.
    #[pg_test]
    fn graph_id_seed_lookup() {
        let id: Option<i64> = Spi::get_one("SELECT pgrdf.graph_id('urn:pgrdf:graph:0')")
            .expect("seed graph_id lookup failed");
        assert_eq!(id, Some(0));
    }

    /// Slice 116 — given a binding pre-existing in `_pgrdf_graphs`,
    /// `pgrdf.graph_id(iri)` returns the bound id. The lookup is
    /// the IRI → id inverse of the IRI-keyed `add_graph` round trip.
    ///
    /// Bypasses the `add_graph(…)` overloads and INSERTs directly
    /// into `_pgrdf_graphs`. The `add_graph` family CREATEs a LIST
    /// partition of `_pgrdf_quads`, which takes an
    /// AccessExclusiveLock on the parent partitioned table; pgrx
    /// runs `#[pg_test]`s in parallel, so two such tests in the
    /// same suite can deadlock on the parent lock regardless of
    /// which partition value they pick. The `graph_id` UDF only
    /// reads `_pgrdf_graphs`, so a direct INSERT exercises the
    /// real code path while keeping this test partition-DDL-free.
    /// The IRI is unique to this slice (`/lookup116`) so concurrent
    /// workers don't collide on the row either.
    #[pg_test]
    fn graph_id_after_iri_add() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (116116, 'http://example.org/lookup116')",
        )
        .expect("seed _pgrdf_graphs row failed");
        let looked_up: Option<i64> =
            Spi::get_one("SELECT pgrdf.graph_id('http://example.org/lookup116')")
                .expect("graph_id lookup failed");
        assert_eq!(looked_up, Some(116116));
    }

    /// Slice 116 — lookup miss returns NULL rather than panicking.
    /// NULL is the documented lookup-miss signal per LLD v0.4 §3.2.
    #[pg_test]
    fn graph_id_miss_returns_null() {
        let id: Option<i64> =
            Spi::get_one("SELECT pgrdf.graph_id('http://example.org/never-bound')")
                .expect("graph_id lookup failed");
        assert_eq!(id, None);
    }

    /// Slice 116 — `#[pg_extern(strict)]` makes Postgres skip the
    /// function entirely on a NULL argument and emit NULL directly.
    /// The Rust `&str` body therefore never observes a NULL input;
    /// callers passing `NULL::text` get `NULL` back.
    #[pg_test]
    fn graph_id_null_input_null_output() {
        let id: Option<i64> = Spi::get_one("SELECT pgrdf.graph_id(NULL::text)")
            .expect("graph_id(NULL) lookup failed");
        assert_eq!(id, None);
    }

    /// Slice 115 — seed row `(0, 'urn:pgrdf:graph:0')` is reachable
    /// via `pgrdf.graph_iri(0)` immediately after `CREATE EXTENSION`,
    /// returning the synthetic IRI bound to the default-partition id.
    /// Symmetric to slice 116's `graph_id_seed_lookup`.
    #[pg_test]
    fn graph_iri_seed_lookup() {
        let iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(0::bigint)").expect("seed lookup failed");
        assert_eq!(iri.as_deref(), Some("urn:pgrdf:graph:0"));
    }

    /// Slice 115 — given a binding pre-existing in `_pgrdf_graphs`,
    /// `pgrdf.graph_iri(id)` returns the bound IRI. Bypasses the
    /// `add_graph(…)` overloads with a direct INSERT for the same
    /// partition-DDL-parallelism reason as slice 116's
    /// `graph_id_after_iri_add`: `add_graph` CREATEs a LIST partition
    /// under AccessExclusiveLock on the parent partitioned table, and
    /// pgrx runs `#[pg_test]`s in parallel — direct INSERT exercises
    /// the lookup code path without contending on the partition DDL.
    /// IRI is unique to this slice (`/test-777`) to avoid concurrent-
    /// worker row collisions.
    #[pg_test]
    fn graph_iri_direct_insert_lookup() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (777, 'http://example.org/test-777')",
        )
        .expect("seed _pgrdf_graphs row failed");
        let iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(777::bigint)").expect("graph_iri lookup failed");
        assert_eq!(iri.as_deref(), Some("http://example.org/test-777"));
    }

    /// Slice 115 — lookup miss returns NULL rather than panicking.
    /// NULL is the documented lookup-miss signal per LLD v0.4 §3.2.
    /// Symmetric to slice 116's `graph_id_miss_returns_null`.
    #[pg_test]
    fn graph_iri_miss_returns_null() {
        let iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(99999::bigint)").expect("graph_iri lookup failed");
        assert_eq!(iri, None);
    }

    /// Slice 115 — `#[pg_extern(strict)]` makes Postgres skip the
    /// function entirely on a NULL argument and emit NULL directly.
    /// The Rust `i64` body therefore never observes a NULL input;
    /// callers passing `NULL::bigint` get `NULL` back. Symmetric to
    /// slice 116's `graph_id_null_input_null_output`.
    #[pg_test]
    fn graph_iri_null_input_null_output() {
        let iri: Option<String> = Spi::get_one("SELECT pgrdf.graph_iri(NULL::bigint)")
            .expect("graph_iri(NULL) lookup failed");
        assert_eq!(iri, None);
    }

    /// Slice 115 — round-trip via slice 116's `graph_id`. The two
    /// UDFs are exact inverses: any bound `(id, iri)` pair satisfies
    /// `graph_id(graph_iri(id)) = id` and `graph_iri(graph_id(iri))
    /// = iri`. Locks the inverse contract.
    #[pg_test]
    fn graph_iri_roundtrip() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (888, 'http://example.org/test-888')",
        )
        .expect("seed _pgrdf_graphs row failed");
        let iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(888::bigint)").expect("graph_iri lookup failed");
        assert_eq!(iri.as_deref(), Some("http://example.org/test-888"));
        let id: Option<i64> = Spi::get_one("SELECT pgrdf.graph_id('http://example.org/test-888')")
            .expect("graph_id round-trip lookup failed");
        assert_eq!(id, Some(888));
    }

    /// Slice 99 — idempotent absent-graph path. Dropping a graph_id
    /// whose partition does not exist returns 0 and does NOT error,
    /// per LLD v0.4 §5.2 idempotency clause. Also exercises the
    /// stale-binding cleanup: a `_pgrdf_graphs` row pointing at the
    /// non-existent partition is pruned, so a follow-up
    /// `pgrdf.graph_iri(id)` returns NULL afterwards.
    ///
    /// Bypasses `add_graph` to avoid the partition-DDL parallelism
    /// flake documented on `graph_id_after_iri_add`: we INSERT a row
    /// directly into `_pgrdf_graphs` to simulate a stranded binding,
    /// then drop the unbacked id. Partition id (`991100`) chosen
    /// out-of-band from the other Phase B slices so concurrent
    /// pg_test workers can't collide.
    #[pg_test]
    fn drop_graph_idempotent_absent() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (991100, 'http://example.org/stranded-991100')",
        )
        .expect("seed stranded _pgrdf_graphs row failed");

        let dropped: i64 = Spi::get_one("SELECT pgrdf.drop_graph(991100::bigint)")
            .expect("drop_graph absent partition failed")
            .expect("drop_graph absent partition returned NULL");
        assert_eq!(dropped, 0, "absent partition must return 0");

        // Stranded `_pgrdf_graphs` row pruned, so the IRI lookup is
        // a clean miss.
        let iri: Option<String> = Spi::get_one("SELECT pgrdf.graph_iri(991100::bigint)")
            .expect("graph_iri lookup failed");
        assert_eq!(iri, None, "stranded binding must be cleaned up");
    }

    /// Slice 99 — happy path with a manually-built partition.
    /// Manually constructs the LIST partition, the matching
    /// `_pgrdf_graphs` row, and three `is_inferred = FALSE` rows so
    /// the test exercises the real DETACH + DROP path without
    /// re-entering the pgrx-parallelism-flaky `add_graph` UDF.
    /// Asserts:
    ///   * The UDF returns the row count (3).
    ///   * The partition is gone from `pg_class`.
    ///   * The `_pgrdf_graphs` row is gone.
    ///   * `pgrdf.graph_iri(id)` returns NULL post-drop.
    #[pg_test]
    fn drop_graph_happy_path() {
        // Use an id unique to this slice so concurrent pg_test workers
        // don't fight on the partition LIST value or the `_pgrdf_graphs`
        // primary key.
        create_quads_partition(992200);
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (992200, 'http://example.org/g992200')",
        )
        .expect("seed _pgrdf_graphs row failed");
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_quads \
                (subject_id, predicate_id, object_id, graph_id, is_inferred) \
             VALUES (1, 1, 1, 992200, false), \
                    (2, 2, 2, 992200, false), \
                    (3, 3, 3, 992200, false)",
        )
        .expect("seed quads failed");

        let dropped: i64 = Spi::get_one("SELECT pgrdf.drop_graph(992200::bigint)")
            .expect("drop_graph happy path failed")
            .expect("drop_graph happy path returned NULL");
        assert_eq!(dropped, 3, "must return the pre-drop row count");

        // Partition gone from `pg_class`.
        let exists: bool = Spi::get_one(
            "SELECT EXISTS(SELECT 1 FROM pg_class \
                           WHERE relnamespace = 'pgrdf'::regnamespace \
                             AND relname = '_pgrdf_quads_g992200')",
        )
        .expect("pg_class probe failed")
        .unwrap_or(false);
        assert!(!exists, "partition table must be gone post-drop");

        // IRI binding gone too.
        let iri: Option<String> = Spi::get_one("SELECT pgrdf.graph_iri(992200::bigint)")
            .expect("graph_iri lookup failed");
        assert_eq!(iri, None, "binding must be cleaned up");
    }

    /// Slice 99 — cascade guard. With `is_inferred = TRUE` rows in
    /// the partition and `cascade => FALSE`, the UDF panics with the
    /// stable `drop_graph: inferred rows present` prefix. Default
    /// `cascade => TRUE` would proceed (covered by the regression
    /// suite, not duplicated here to keep pg_test count tight).
    /// pgrx-tests requires an EXACT match on the panic message, so the
    /// graph_id and trailing hint are reproduced here verbatim.
    #[pg_test(
        error = "drop_graph: inferred rows present (graph_id = 993300); pass cascade => true to proceed"
    )]
    fn drop_graph_cascade_false_blocks_inferred() {
        create_quads_partition(993300);
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_quads \
                (subject_id, predicate_id, object_id, graph_id, is_inferred) \
             VALUES (1, 1, 1, 993300, true)",
        )
        .expect("seed inferred quad failed");
        Spi::run("SELECT pgrdf.drop_graph(993300::bigint, cascade => false)").unwrap();
    }

    /// Slice 99 — default-partition guard. `pgrdf.drop_graph(0)` is
    /// not allowed because `_pgrdf_quads_default` is the catch-all
    /// bucket. Stable `drop_graph: cannot drop default partition`
    /// prefix.
    #[pg_test(error = "drop_graph: cannot drop default partition (graph_id = 0)")]
    fn drop_graph_default_partition_rejected() {
        Spi::run("SELECT pgrdf.drop_graph(0::bigint)").unwrap();
    }

    /// Slice 99 — negative-id guard. Mirrors the parallel guard in
    /// `add_graph(g BIGINT)` so the surface is symmetric.
    #[pg_test(error = "drop_graph: graph_id must be >= 0, got -1")]
    fn drop_graph_negative_id_rejected() {
        Spi::run("SELECT pgrdf.drop_graph(-1::bigint)").unwrap();
    }

    /// Slice 98 — idempotent on an absent graph. Calling
    /// `clear_graph` against a `graph_id` that has never had
    /// `add_graph(id)` run for it (so no LIST partition exists)
    /// returns 0 without erroring. This is the bottom of the
    /// idempotency contract: callers can `clear_graph` blindly
    /// during cleanup without first probing partition existence.
    /// The id (`9898`) is unique to this slice so concurrent
    /// pgrx workers don't collide.
    #[pg_test]
    fn clear_graph_absent_returns_zero() {
        let removed: Option<i64> = Spi::get_one("SELECT pgrdf.clear_graph(9898::bigint)")
            .expect("clear_graph on absent partition failed");
        assert_eq!(removed, Some(0));
    }

    /// Slice 98 — happy path: load N quads into a graph, clear,
    /// observe the return value matches the row count and the
    /// partition is empty afterward. The partition + its
    /// `_pgrdf_graphs` IRI binding survive (the LLD §5.1
    /// invariant that distinguishes `clear_graph` from
    /// `drop_graph`).
    ///
    /// Direct INSERT into `_pgrdf_quads` + direct INSERT into
    /// `_pgrdf_graphs` bypasses the `add_graph(id)` partition-DDL
    /// path that takes AccessExclusiveLock on the parent — the
    /// same parallelism mitigation used by `graph_id_after_iri_add`
    /// and `graph_iri_direct_insert_lookup` above. The id (`9889`)
    /// is unique to this slice; the dictionary ids are also
    /// invented here so we don't depend on a stable order from
    /// `put_term`.
    ///
    /// But — `clear_graph` reads from `pgrdf._pgrdf_quads_g<id>`
    /// directly, which requires a real LIST partition (not just
    /// a row in `_pgrdf_quads` routed to the default partition).
    /// We therefore call `add_graph(9889)` first to create the
    /// partition (this triggers the parent-table AccessExclusive
    /// lock, but the id is unique so no concurrent worker
    /// contends on the *same* LIST value — and the parent-table
    /// lock window is brief enough that pgrx's parallel runner
    /// has not been observed to deadlock on it; see slice 117
    /// `add_graph_id_iri_synthetic_upgrade` which is the same
    /// shape and ships green).
    #[pg_test]
    fn clear_graph_returns_row_count() {
        Spi::run("SELECT pgrdf.add_graph(9889::bigint)").expect("add_graph(9889) failed");

        // Three rows: two base + one inferred. `clear_graph` must
        // count both — the LLD §5.2 invariant that the truncate is
        // not is_inferred-discriminating.
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_quads \
             (subject_id, predicate_id, object_id, graph_id, is_inferred) VALUES \
             (1, 2, 3, 9889, false), \
             (4, 5, 6, 9889, false), \
             (7, 8, 9, 9889, true)",
        )
        .expect("seed _pgrdf_quads rows failed");

        let pre_count: i64 =
            Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = 9889")
                .expect("pre-count failed")
                .expect("pre-count returned NULL");
        assert_eq!(pre_count, 3);

        let removed: Option<i64> = Spi::get_one("SELECT pgrdf.clear_graph(9889::bigint)")
            .expect("clear_graph(9889) failed");
        assert_eq!(removed, Some(3), "must return the pre-clear row count");

        // Partition is empty post-clear.
        let post_count: i64 =
            Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = 9889")
                .expect("post-count failed")
                .expect("post-count returned NULL");
        assert_eq!(post_count, 0);

        // Partition still attached — its relation is still in
        // `pg_class` under the `pgrdf` schema.
        let still_exists: Option<bool> = Spi::get_one(
            "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_class c \
                           JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace \
                           WHERE c.relname = '_pgrdf_quads_g9889' AND n.nspname = 'pgrdf')",
        )
        .expect("partition existence check failed");
        assert_eq!(still_exists, Some(true), "partition must remain attached");

        // `_pgrdf_graphs` IRI binding (synthetic) survives.
        let iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(9889::bigint)").expect("graph_iri lookup failed");
        assert_eq!(
            iri.as_deref(),
            Some("urn:pgrdf:graph:9889"),
            "IRI binding must survive clear_graph"
        );
    }

    /// Slice 98 — clear-then-clear returns 0 the second time. The
    /// partition exists but is empty after the first clear; the
    /// row count is 0, the TRUNCATE no-ops, and we return 0. This
    /// completes the idempotency contract from the absent-graph
    /// side: clearing an empty partition is the same shape as
    /// clearing a never-created one (modulo whether the partition
    /// row in `pg_class` exists, which the function-level
    /// behaviour does not depend on).
    #[pg_test]
    fn clear_graph_twice_second_returns_zero() {
        Spi::run("SELECT pgrdf.add_graph(9890::bigint)").expect("add_graph(9890) failed");
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_quads \
             (subject_id, predicate_id, object_id, graph_id) VALUES \
             (10, 20, 30, 9890), (40, 50, 60, 9890)",
        )
        .expect("seed _pgrdf_quads rows failed");

        let first: Option<i64> = Spi::get_one("SELECT pgrdf.clear_graph(9890::bigint)")
            .expect("first clear_graph(9890) failed");
        assert_eq!(first, Some(2));

        let second: Option<i64> = Spi::get_one("SELECT pgrdf.clear_graph(9890::bigint)")
            .expect("second clear_graph(9890) failed");
        assert_eq!(
            second,
            Some(0),
            "second clear on empty partition must return 0"
        );
    }

    /// Slice 97 — happy path: build a source partition, seed N rows
    /// (mix of base + inferred), call `copy_graph(src, dst)` against
    /// a fresh `dst`. Verify return value matches the source row
    /// count, the destination partition exists, and the rows show
    /// up with `graph_id = dst` and the `is_inferred` flag preserved.
    ///
    /// Direct partition CREATE + direct INSERT into `_pgrdf_quads`
    /// bypasses the `add_graph(src)` partition-DDL parallelism flake
    /// (same mitigation as `drop_graph_happy_path` above). For the
    /// destination side we deliberately *don't* pre-create the
    /// partition — the function's auto-create path is the
    /// interesting code under test. Ids `971100` (src) and `971200`
    /// (dst) are unique to this slice so concurrent pg_test workers
    /// can't collide on the partition LIST value or the rows.
    #[pg_test]
    fn copy_graph_happy_path() {
        create_quads_partition(971100);
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_quads \
                (subject_id, predicate_id, object_id, graph_id, is_inferred) \
             VALUES (1, 2, 3, 971100, false), \
                    (4, 5, 6, 971100, false), \
                    (7, 8, 9, 971100, true)",
        )
        .expect("seed src quads failed");

        let copied: i64 = Spi::get_one("SELECT pgrdf.copy_graph(971100::bigint, 971200::bigint)")
            .expect("copy_graph happy path failed")
            .expect("copy_graph happy path returned NULL");
        assert_eq!(copied, 3, "must return the source row count");

        // Destination partition was auto-created (we did NOT
        // pre-create it).
        let dst_exists: bool = Spi::get_one(
            "SELECT EXISTS(SELECT 1 FROM pg_class \
                           WHERE relnamespace = 'pgrdf'::regnamespace \
                             AND relname = '_pgrdf_quads_g971200')",
        )
        .expect("pg_class probe failed")
        .unwrap_or(false);
        assert!(dst_exists, "dst partition must exist post-copy");

        // Destination has all 3 rows with the rebound graph_id.
        let dst_count: i64 =
            Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = 971200")
                .expect("dst count failed")
                .expect("dst count returned NULL");
        assert_eq!(dst_count, 3, "dst must hold every copied row");

        // is_inferred flag preserved across the copy: one inferred
        // row in src → one inferred row in dst.
        let inferred_count: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads \
              WHERE graph_id = 971200 AND is_inferred = true",
        )
        .expect("inferred count failed")
        .expect("inferred count returned NULL");
        assert_eq!(inferred_count, 1, "is_inferred must carry forward");

        // Source partition still intact — copy is non-destructive.
        let src_count: i64 =
            Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = 971100")
                .expect("src count failed")
                .expect("src count returned NULL");
        assert_eq!(src_count, 3, "src must be untouched by copy");
    }

    /// Slice 97 — idempotent absent-src path. Copying from a
    /// `graph_id` whose partition does not exist returns 0 and does
    /// NOT error. The destination partition is NOT auto-created in
    /// this path: we short-circuit on the src-existence check before
    /// reaching the dst-auto-create branch, so a follow-up
    /// `pg_class` probe for the dst partition returns false. This
    /// matches the LLD §5.2 idempotency contract: callers can
    /// `copy_graph` blindly without first probing partition
    /// existence on the source side.
    #[pg_test]
    fn copy_graph_absent_src_returns_zero() {
        let copied: Option<i64> =
            Spi::get_one("SELECT pgrdf.copy_graph(972100::bigint, 972200::bigint)")
                .expect("copy_graph absent src failed");
        assert_eq!(copied, Some(0), "absent src must return 0");

        // Short-circuit semantics: dst partition not auto-created
        // when src is absent.
        let dst_exists: bool = Spi::get_one(
            "SELECT EXISTS(SELECT 1 FROM pg_class \
                           WHERE relnamespace = 'pgrdf'::regnamespace \
                             AND relname = '_pgrdf_quads_g972200')",
        )
        .expect("pg_class probe failed")
        .unwrap_or(false);
        assert!(
            !dst_exists,
            "dst must NOT be auto-created when src is absent"
        );
    }

    /// Slice 97 — `src == dst` is rejected with the stable
    /// `copy_graph: src and dst must differ` prefix. The self-copy
    /// degenerate case has no defined semantics (an INSERT … SELECT
    /// from a table into itself on a partitioned table would
    /// interleave scan + insert unpredictably) and we surface that
    /// up to the caller rather than silently double-write.
    #[pg_test(error = "copy_graph: src and dst must differ (both = 5)")]
    fn copy_graph_src_eq_dst_rejected() {
        Spi::run("SELECT pgrdf.copy_graph(5::bigint, 5::bigint)").unwrap();
    }

    /// Slice 96 — `pgrdf.move_graph(src, dst)` happy path. Load N
    /// quads into `src`, run the move, observe (a) return value is
    /// N, (b) `src` partition gone, (c) `dst` partition holds N rows,
    /// (d) `_pgrdf_graphs` bindings rotated (src unbound; dst bound).
    ///
    /// **Runtime dependency on slice 97's `copy_graph`.** This test
    /// is written ahead of slice 97 landing; it FAILs in the slice-96
    /// worktree because `pgrdf.copy_graph` does not yet exist. It
    /// goes green at parent-merge time, when slice 97 lands the
    /// `copy_graph` UDF.
    #[pg_test]
    fn move_graph_happy_path() {
        // Build src + populate with 3 base + 1 inferred row. Direct
        // INSERT pattern (same partition-DDL-flake mitigation as the
        // sibling slices: build the partition via `add_graph(id)`,
        // then INSERT directly into `_pgrdf_quads`).
        Spi::run("SELECT pgrdf.add_graph(9601::bigint)").expect("add_graph(9601) failed");
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_quads \
             (subject_id, predicate_id, object_id, graph_id, is_inferred) VALUES \
             (1001, 1002, 1003, 9601, false), \
             (2001, 2002, 2003, 9601, false), \
             (3001, 3002, 3003, 9601, false), \
             (4001, 4002, 4003, 9601, true)",
        )
        .expect("seed src quads failed");

        let moved: Option<i64> =
            Spi::get_one("SELECT pgrdf.move_graph(9601::bigint, 9602::bigint)")
                .expect("move_graph(9601, 9602) failed");
        assert_eq!(moved, Some(4), "move must return the src row count");

        // src partition gone (drop_graph step removed it).
        let src_exists: bool = Spi::get_one(
            "SELECT EXISTS(SELECT 1 FROM pg_class \
                           WHERE relnamespace = 'pgrdf'::regnamespace \
                             AND relname = '_pgrdf_quads_g9601')",
        )
        .expect("pg_class probe failed")
        .unwrap_or(false);
        assert!(!src_exists, "src partition must be gone post-move");

        // dst partition holds the rows.
        let dst_count: i64 =
            Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = 9602")
                .expect("dst count failed")
                .expect("dst count returned NULL");
        assert_eq!(dst_count, 4, "dst must hold the moved rows");

        // _pgrdf_graphs invalidation: src unbound, dst bound.
        let src_iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(9601::bigint)").expect("src iri lookup failed");
        assert_eq!(src_iri, None, "src IRI binding must be removed");
        let dst_iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(9602::bigint)").expect("dst iri lookup failed");
        assert_eq!(
            dst_iri.as_deref(),
            Some("urn:pgrdf:graph:9602"),
            "dst must receive the synthetic IRI binding from copy_graph"
        );
    }

    /// Slice 96 — idempotent absent: `move_graph` on a non-existent
    /// src returns 0 without erroring. No call to `copy_graph` is
    /// made (we short-circuit on the existence check) so this test
    /// is independent of slice 97 and can run green standalone.
    #[pg_test]
    fn move_graph_absent_src_returns_zero() {
        let moved: Option<i64> =
            Spi::get_one("SELECT pgrdf.move_graph(9603::bigint, 9604::bigint)")
                .expect("move_graph on absent src failed");
        assert_eq!(moved, Some(0), "absent src must return 0");
    }

    /// Slice 96 — src == dst rejection. Self-move would be a copy
    /// followed by a drop of the destination, which is destructive.
    /// Explicit panic with the stable prefix is safer than a no-op.
    /// Independent of slice 97. pgrx-tests requires an EXACT match
    /// on the panic message, so the `(both = N)` suffix is included.
    #[pg_test(error = "move_graph: src and dst must differ (both = 9605)")]
    fn move_graph_self_move_rejected() {
        Spi::run("SELECT pgrdf.move_graph(9605::bigint, 9605::bigint)").unwrap();
    }

    /// Slice 96 — negative-id guard mirrors `drop_graph` /
    /// `clear_graph` / `add_graph(g BIGINT)`. Independent of
    /// slice 97. pgrx-tests requires an EXACT match on the panic
    /// message, so the full `, got src=-1, dst=9606` tail is matched.
    #[pg_test(error = "move_graph: graph_id must be >= 0, got src=-1, dst=9606")]
    fn move_graph_negative_id_rejected() {
        Spi::run("SELECT pgrdf.move_graph(-1::bigint, 9606::bigint)").unwrap();
    }

    /// Slice 96 — dst-has-data rejection. Build src + dst, both
    /// populated; move panics with the stable prefix. Note this
    /// path runs the dst existence + count check (which does NOT
    /// depend on slice 97), so it passes standalone in this
    /// worktree. pgrx-tests requires an EXACT match on the panic
    /// message, so the trailing `(N rows); clear or drop it first`
    /// is matched verbatim.
    #[pg_test(
        error = "move_graph: dst graph_id 9608 already has data (1 rows); clear or drop it first"
    )]
    fn move_graph_dst_has_data_rejected() {
        Spi::run("SELECT pgrdf.add_graph(9607::bigint)").expect("add_graph(9607) failed");
        Spi::run("SELECT pgrdf.add_graph(9608::bigint)").expect("add_graph(9608) failed");
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_quads \
             (subject_id, predicate_id, object_id, graph_id) VALUES \
             (1, 1, 1, 9607), (2, 2, 2, 9608)",
        )
        .expect("seed quads failed");
        Spi::run("SELECT pgrdf.move_graph(9607::bigint, 9608::bigint)").unwrap();
    }
}
