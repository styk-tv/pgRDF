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
//! Reference: SPEC.pgRDF.LLD.v0.4 §3.1.

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
        let iri: String =
            Spi::get_one("SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 0")
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
        assert_eq!(count, 1, "expected exactly one row for graph_id = 42, got {count}");

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
        let id: i64 = Spi::get_one(
            "SELECT pgrdf.add_graph(50::bigint, 'http://example.org/g50')",
        )
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

        let id: i64 = Spi::get_one(
            "SELECT pgrdf.add_graph(60::bigint, 'http://example.org/g60')",
        )
        .expect("upgrade add_graph(60, iri) failed")
        .expect("upgrade add_graph(60, iri) returned NULL");
        assert_eq!(id, 60);

        let upgraded: Option<String> =
            Spi::get_one("SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 60")
                .expect("upgraded iri lookup failed");
        assert_eq!(upgraded.as_deref(), Some("http://example.org/g60"));

        let count: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_graphs WHERE graph_id = 60",
        )
        .expect("row count failed")
        .expect("row count returned NULL");
        assert_eq!(count, 1, "synthetic upgrade must UPDATE in place, not duplicate");
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
        Spi::run("SELECT pgrdf.add_graph(70::bigint, 'http://example.org/different')")
            .unwrap();
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
}
