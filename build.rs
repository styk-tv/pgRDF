//! Build script — build identity only.
//!
//! `PGRDF_BUILD_ID` is read with `option_env!` in `src/lib.rs`, which is a
//! COMPILE-TIME lookup. Without the line below, cargo has no reason to
//! recompile when the variable changes, so a rebuild would silently keep the
//! previous build id. That is worse than having none: it would report a build
//! that is not the one loaded, which is the exact failure this function exists
//! to prevent.

fn main() {
    println!("cargo:rerun-if-env-changed=PGRDF_BUILD_ID");
}
