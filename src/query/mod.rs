//! SPARQL → prepared-SQL execution engine.
//!
//! See SPEC.pgRDF.LLD.v0.2 §4.2 and docs/03-query.md.

pub mod executor;
pub mod guc;
pub mod parser;
pub mod path;
pub mod plan_cache;
