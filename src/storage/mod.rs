//! Storage layer — shared dictionary, hexastore, bulk loader.
//!
//! Submodules:
//!   * [`dict`] — dictionary CRUD + shmem-aware `put_term_full`.
//!   * [`graphs`] — `_pgrdf_graphs` IRI ↔ graph_id mapping (LLD v0.4
//!     §3.1, schema landed by Phase A slice 120; UDF surface lands
//!     in subsequent slices).
//!   * [`hexastore`] — partitioned `_pgrdf_quads` CRUD + `add_graph`.
//!   * [`partition`] — serialised `_pgrdf_quads` partition DDL (xact
//!     advisory lock; restores parallel test threads, see module docs).
//!   * [`loader`] — Turtle ingest with per-call cache + batched
//!     prepared INSERTs (LLD §4.3 phase A).
//!   * [`construct_ingest`] — round-trip pair for `pgrdf.construct`:
//!     `put_construct_row` / `put_construct_rows` decode structured
//!     JSONB rows back into the dictionary + hexastore (LLD v0.4 §6.3;
//!     Phase D slice 53).
//!   * [`shmem_cache`] — cross-backend dict cache in Postgres shmem
//!     (LLD §4.1).
//!   * [`stats`] — `pgrdf.stats()` + `pgrdf.shmem_reset()` UDFs.
//!
//! Reference: SPEC.pgRDF.LLD.v0.2 §3, §4.1, §4.3, SPEC.pgRDF.LLD.v0.4
//! §3, §6.3, and `docs/02-storage.md`.

pub mod construct_ingest;
pub mod dict;
pub mod graphs;
pub mod hexastore;
pub mod loader;
pub mod loader_ta11;
pub mod partition;
pub mod shmem_cache;
pub mod stats;
