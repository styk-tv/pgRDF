//! #117 — RDFC-1.0 canonical graph digest: identity that survives reload.
//!
//! A byte digest over exported N-Triples identifies one stored copy: blank
//! node labels are minted per parse, so the same source loads to different
//! bytes forever (measured: 531/948 of core's triples ride bnodes; two
//! loads of one file differ only in labels). `pgrdf.graph_digest(graph)`
//! answers identity of MEANING: canonical blank-node relabelling per
//! RDFC-1.0 (W3C), canonical N-Quads serialization, sha256. Algorithm
//! label, per the sealed interface contract: `rdfc-1.0-sha256` — values
//! are NOT comparable with first-degree structural pins, by design.
//!
//! Complexity guard: RDFC-1.0 is worst-case exponential on adversarial
//! automorphic bnode structures; the guard RAISES (never degrades) — the
//! fail-closed direction, as everywhere in this engine.
//!
//! Contract (v0.6.32 rechecks, stated before the code):
//!   - same source parsed into two fresh graphs ⇒ equal `graph_digest`,
//!     while their byte serializations differ (label variance);
//!   - genuinely different graphs ⇒ unequal digests (the conclusive
//!     direction);
//!   - W3C rdf-canon conformance subset passes as regression fixtures.

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// Two anonymous bnodes — each parse mints fresh internal labels, so
    /// byte-level content differs across loads while structure is fixed.
    const BNODE_TTL: &str = "@prefix c: <urn:c:> .\n[ c:p c:o1 ] .\n[ c:p c:o2 ] .\n";

    fn seed(graph_id: i64, ttl: &str) {
        Spi::run(&format!("SELECT pgrdf.add_graph({graph_id})")).expect("add_graph failed");
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), graph_id.into()],
        )
        .expect("seed parse failed");
    }

    fn digest(graph_id: i64) -> String {
        Spi::get_one_with_args("SELECT pgrdf.graph_digest($1)", &[graph_id.into()])
            .expect("graph_digest failed")
            .expect("graph_digest returned NULL")
    }

    /// The core promise: canonical identity survives the reload that a
    /// fork, a spore-germination, or a plain re-load necessarily is.
    #[pg_test]
    fn reload_equality_survives_relabelling() {
        seed(982201, BNODE_TTL);
        seed(982202, BNODE_TTL);
        let d1 = digest(982201);
        let d2 = digest(982202);
        assert_eq!(d1, d2, "isomorphic graphs must share a canonical digest");
        assert_eq!(d1.len(), 64, "sha256 hex");
    }

    /// The conclusive direction: different meaning, different digest.
    #[pg_test]
    fn different_graphs_differ() {
        seed(982203, BNODE_TTL);
        seed(
            982204,
            "@prefix c: <urn:c:> .\n[ c:p c:o1 ] .\n[ c:q c:o2 ] .\n",
        );
        assert_ne!(digest(982203), digest(982204));
    }
}
