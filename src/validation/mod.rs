//! SHACL validation layer.
//!
//! See SPEC.pgRDF.LLD.v0.2 §2 and docs/05-validation.md. The v0.2 LLD
//! references `shacl-rust`; we replace it with `shacl_validation`
//! (see specs/ERRATA.v0.2.md).
//!
//! Submodules:
//! - `shacl` — the rudof `shacl 0.3.x`-backed validator behind
//!   `pgrdf.validate(d, s, 'native' | 'sparql')`.
//! - `pgrdf_sparql` — TH-12 scaffold for the upcoming pgRDF-native
//!   SHACL-SPARQL execution path (`mode => 'pgrdf'`, Track H
//!   Architecture-1). Not yet wired into `validate()`; landing in
//!   TH-11 → TH-8.

pub mod pgrdf_sparql;
pub mod shacl;
