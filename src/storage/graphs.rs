//! Named-graph IRI ↔ graph_id mapping.
//!
//! Phase A slice 120 lands the `_pgrdf_graphs` system table (LLD
//! v0.4 §3.1) via `sql/schema_v0_4_0_graphs.sql`. UDF surface
//! (`pgrdf.add_graph(iri)`, `pgrdf.graph_id(iri)`, `pgrdf.graph_iri(id)`,
//! plus the dual-arg `pgrdf.add_graph(id, iri)` overload) lands in
//! slices 118-115; the existing integer-keyed
//! [`super::hexastore::add_graph`] retains its v0.3 signature until
//! slice 117 wires the synthetic-IRI binding.
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
}
