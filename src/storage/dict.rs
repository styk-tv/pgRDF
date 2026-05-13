//! Shared dictionary manager.
//!
//! Per SPEC.pgRDF.LLD.v0.2 §4.1: an instance-wide RwLock<LruCache<u64, i64>>
//! backed by pgrx::shmem so all Postgres backends share the cache. Read flow:
//! hash RdfTerm → u64 → shmem cache → fall back to _pgrdf_dictionary via Spi.
//!
//! TODO(phase-2): implement Init/Lookup/Insert against pgrx::shmem.
