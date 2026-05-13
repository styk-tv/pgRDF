//! End-to-end install verification via `cargo pgrx test`.
//!
//! These run inside a pgrx-managed Postgres so we can assert against
//! catalog state directly. Pure-Rust unit tests live next to the
//! code they test, not here.

#[cfg(test)]
mod tests {
    // Placeholder. The real surface lives as `#[pg_test]` in
    // src/lib.rs because pgrx requires #[pg_test] to share a crate
    // with #[pg_extension]. This file documents the test plan:
    //
    //   1. CREATE EXTENSION pgrdf;                          (lib.rs)
    //   2. SELECT pgrdf.version();                          (lib.rs)
    //   3. SELECT count(*) FROM pg_class WHERE relname = '_pgrdf_dictionary';   -- TODO Phase 1
    //   4. SELECT count(*) FROM pg_class WHERE relname LIKE '_pgrdf_quads%';     -- TODO Phase 1
    //   5. SELECT count(*) FROM pg_indexes WHERE indexname LIKE '_pgrdf_idx_%';  -- TODO Phase 1
    //   6. DROP EXTENSION pgrdf;                             -- TODO Phase 1
}
