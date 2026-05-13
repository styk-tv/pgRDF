//! pgRDF — Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning.
//!
//! Module map (mirrors SPEC.pgRDF.LLD.v0.2 §4):
//!   storage    — shmem dictionary + partitioned hexastore + COPY BINARY loader
//!   query      — SPARQL parser + BGP-to-prepared-SQL translator + plan cache
//!   inference  — reasonable (OWL 2 RL) materialization
//!   validation — SHACL validation reports

use pgrx::prelude::*;

::pgrx::pg_module_magic!();

pub mod inference;
pub mod query;
pub mod storage;
pub mod validation;

/// Returns the extension version. Smoke surface used by the install
/// verification: `SELECT pgrdf.version();` should return the version
/// declared in `Cargo.toml`.
#[pg_extern]
fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

extension_sql_file!("../sql/schema_v0_2_0.sql", name = "schema_v0_2_0");

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn test_version_matches_cargo() {
        assert_eq!(crate::version(), env!("CARGO_PKG_VERSION"));
    }
}

#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
