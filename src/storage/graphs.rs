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
//! Reference: SPEC.pgRDF.LLD.v0.4 §3.1, §3.2, §5.1.

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
    let total: i64 = Spi::get_one(&format!(
        "SELECT count(*)::bigint FROM pgrdf.{part_name}"
    ))
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

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
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
        let iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(991100::bigint)").expect("graph_iri lookup failed");
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
        Spi::run(
            "CREATE TABLE pgrdf._pgrdf_quads_g992200 \
             PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN (992200)",
        )
        .expect("manual partition creation failed");
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
        let iri: Option<String> =
            Spi::get_one("SELECT pgrdf.graph_iri(992200::bigint)").expect("graph_iri lookup failed");
        assert_eq!(iri, None, "binding must be cleaned up");
    }

    /// Slice 99 — cascade guard. With `is_inferred = TRUE` rows in
    /// the partition and `cascade => FALSE`, the UDF panics with the
    /// stable `drop_graph: inferred rows present` prefix. Default
    /// `cascade => TRUE` would proceed (covered by the regression
    /// suite, not duplicated here to keep pg_test count tight).
    #[pg_test(error = "drop_graph: inferred rows present")]
    fn drop_graph_cascade_false_blocks_inferred() {
        Spi::run(
            "CREATE TABLE pgrdf._pgrdf_quads_g993300 \
             PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN (993300)",
        )
        .expect("manual partition creation failed");
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
}
