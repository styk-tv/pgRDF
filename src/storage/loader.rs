//! Bulk ingestion via `COPY _pgrdf_quads FROM STDIN (FORMAT BINARY)`.
//!
//! See SPEC.pgRDF.LLD.v0.2 §4.3. The loader parses N-Triples/Turtle,
//! resolves IDs against the shmem dictionary, then streams binary
//! tuples through Postgres' COPY interface for millions-of-rows-per-second
//! ingestion.
