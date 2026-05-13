//! Hexastore index helpers.
//!
//! The on-disk shape is six covering indexes over `_pgrdf_quads`
//! (SPO, POS, OSP, plus SOP, PSO, OPS as we add them) using
//! `CREATE INDEX … INCLUDE (is_inferred)` for index-only scans.
//! See SPEC.pgRDF.LLD.v0.2 §3.3.
