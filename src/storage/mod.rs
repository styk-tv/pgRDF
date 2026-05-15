//! Storage layer — shared dictionary, hexastore, bulk loader.
//!
//! Submodules:
//!   * [`dict`] — dictionary CRUD + shmem-aware `put_term_full`.
//!   * [`graphs`] — `_pgrdf_graphs` IRI ↔ graph_id mapping (LLD v0.4
//!     §3.1, schema landed by Phase A slice 120; UDF surface lands
//!     in subsequent slices).
//!   * [`hexastore`] — partitioned `_pgrdf_quads` CRUD + `add_graph`.
//!   * [`loader`] — Turtle ingest with per-call cache + batched
//!     prepared INSERTs (LLD §4.3 phase A).
//!   * [`shmem_cache`] — cross-backend dict cache in Postgres shmem
//!     (LLD §4.1).
//!   * [`stats`] — `pgrdf.stats()` + `pgrdf.shmem_reset()` UDFs.
//!
//! Reference: SPEC.pgRDF.LLD.v0.2 §3, §4.1, §4.3, SPEC.pgRDF.LLD.v0.4
//! §3, and `docs/02-storage.md`.

pub mod dict;
pub mod graphs;
pub mod hexastore;
pub mod loader;
pub mod shmem_cache;
pub mod stats;
