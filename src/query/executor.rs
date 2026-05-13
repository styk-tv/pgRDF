//! SPARQL algebra → parameterized SQL translator.
//!
//! Per SPEC.pgRDF.LLD.v0.2 §4.2: translate BGPs into a parameterized
//! plan over `_pgrdf_quads`, cache the plan handle via `Spi::prepare`,
//! and parameter-bind on subsequent identical-structure queries to
//! bypass the Postgres parser/planner entirely.
