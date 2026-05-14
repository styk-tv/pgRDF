//! End-to-end install verification via `cargo pgrx test`.
//!
//! These run inside a pgrx-managed Postgres so we can assert against
//! catalog state directly. Pure-Rust unit tests live next to the
//! code they test, not here.
//!
//! The real surface lives as `#[pg_test]` in `src/lib.rs` because pgrx
//! requires `#[pg_test]` to share a crate with `#[pg_extension]`.
//! Install verification is split across two layers, both shipped:
//!
//!   1. `CREATE EXTENSION pgrdf;` / `SELECT pgrdf.version();` —
//!      covered by `src/lib.rs::tests::test_version_matches_cargo`
//!      (pgrx implicitly creates the extension before any `#[pg_test]`
//!      body runs; the version assertion fails closed if the
//!      extension didn't install).
//!   2. Catalog state (`_pgrdf_dictionary`, `_pgrdf_quads*`,
//!      `_pgrdf_idx_{spo,pos,osp}`) plus `DROP EXTENSION pgrdf;` —
//!      covered by the pg_regress harness at
//!      `tests/regression/sql/00-smoke.sql`, which runs against the
//!      compose Postgres via `just test-regression`.
//!
//! This file is intentionally empty of `#[test]` bodies; it remains
//! as the documented entry point for the install-verification track.

#[cfg(test)]
mod tests {}
