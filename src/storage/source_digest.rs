//! #118 — the loader records the byte digest of what it ingested.
//!
//! Downstream systems pin the file bytes they loaded (a `sourceDigest`
//! recorded beside an adoption). Once triples are in the store nothing can
//! recompute file bytes — so the pin is recorded and consulted by nothing.
//! The loader is the ONLY party that ever sees the input bytes; if it does
//! not record their digest, no one downstream can ever verify one.
//!
//! Contract (the v0.6.31 recheck, stated before the code existed):
//!   - a successful load records `source_sha256` = sha256 of the exact input
//!     bytes, and `source_loads` = 1 on first load;
//!   - a second load updates the digest to the LATEST load's bytes and
//!     increments `source_loads` — `source_loads > 1` self-reports that byte
//!     identity no longer holds for the graph as a whole;
//!   - a graph that was never loaded reads NULL in both columns — "never
//!     recorded" is distinct from every digest.

use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::io::Read;
use std::rc::Rc;

// The DDL rides as an idempotent ALTER so the same block is correct on
// a fresh install (after the graphs table exists) and in the generated
// full-install SQL — the exact #107 lock-columns pattern. The upgrade
// path ships the identical statements in sql/pgrdf--0.6.30--0.6.31.sql.
//
// Both columns are NULL until the first recorded load: "never recorded"
// must stay distinct from every possible digest and every count.
pgrx::extension_sql!(
    r#"
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS source_sha256 TEXT;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS source_loads  INTEGER;
"#,
    name = "graph_source_digest_columns_v0_6_31",
    requires = ["schema_v0_4_0_graphs"],
);

/// Wraps any `Read` and folds every byte that passes through into a
/// SHA-256. The loader is the only party that ever sees the input
/// bytes; this adapter is how it sees them WITHOUT a second read and
/// without a gap between what was hashed and what was parsed — the
/// digest is over the exact bytes the parser consumed.
pub struct HashingReader<R: Read> {
    inner: R,
    hasher: Rc<RefCell<Sha256>>,
}

impl<R: Read> HashingReader<R> {
    /// Returns the wrapping reader plus a handle to the hasher; the
    /// caller finalizes the handle after the parse has consumed the
    /// reader.
    pub fn new(inner: R) -> (Self, Rc<RefCell<Sha256>>) {
        let hasher = Rc::new(RefCell::new(Sha256::new()));
        (
            Self {
                inner,
                hasher: Rc::clone(&hasher),
            },
            hasher,
        )
    }
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        if n > 0 {
            self.hasher.borrow_mut().update(&buf[..n]);
        }
        Ok(n)
    }
}

/// Open the load record at INGEST START: bump `source_loads` and take the
/// graphs-table lock in the canonical early position — BEFORE the parse
/// path acquires any partition/DDL locks on the quads parent.
///
/// LOCK ORDER IS THE POINT. The first full-suite run deadlocked
/// (measured: `deadlock detected`, my late UPDATE waiting on
/// `_pgrdf_graphs` while a concurrent `add_graph`'s partition DDL —
/// which takes strong locks on the quads parent AND, via the partition
/// FK, on the graphs table — waited on locks this transaction already
/// held from its own earlier DDL). A single UPDATE after the parse
/// acquired the graphs lock LAST and completed the cycle. Splitting the
/// record puts the graphs-table acquisition first, where `add_graph`'s
/// own INSERT takes it; the digest write then reuses a lock the
/// transaction already holds and never waits late.
///
/// Rollback keeps the semantics honest: a failed parse panics, aborting
/// the transaction, so the early count and the late digest vanish
/// together — no record ever describes bytes that did not land. A graph
/// with no `_pgrdf_graphs` row updates zero rows, mirroring the lock
/// module's treatment of unregistered graphs.
pub fn begin_source_record(graph_id: i64) {
    pgrx::Spi::run_with_args(
        "UPDATE pgrdf._pgrdf_graphs
            SET source_loads = COALESCE(source_loads, 0) + 1
          WHERE graph_id = $1",
        &[graph_id.into()],
    )
    .expect("source_digest: opening the load record failed");
}

/// Close the load record AFTER a successful ingest: write the digest of
/// the exact bytes the parser consumed (latest-load-wins). The graphs
/// row lock was taken by `begin_source_record`, so this statement waits
/// on nothing late in the transaction.
pub fn finish_source_record(graph_id: i64, hasher: &Rc<RefCell<Sha256>>) {
    let digest = hasher.borrow_mut().finalize_reset();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    pgrx::Spi::run_with_args(
        "UPDATE pgrdf._pgrdf_graphs
            SET source_sha256 = $1
          WHERE graph_id = $2",
        &[hex.into(), graph_id.into()],
    )
    .expect("source_digest: recording the load digest failed");
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;
    use std::io::Write;

    /// Real Turtle (a `@prefix` line), NOT bare N-Triples: `load_turtle`
    /// sniffs one-statement-per-line files as N-Triples and dispatches to the
    /// STAGED loader when the worker pool is preloaded — which the pgrx test
    /// harness does, and staged workers commit their own transactions, which
    /// a `#[pg_test]`'s wrapping transaction can never let happen: the run
    /// HANGS (measured: 31 min at 0% CPU, worker at phase 6 waiting).
    /// Prefixed Turtle keeps the test on the standard funnel path.
    ///
    /// sha256 computed OUTSIDE
    /// this crate with `shasum -a 256`, so the expectation is independent
    /// of whatever the implementation links.
    const FIXTURE_A: &str = "@prefix sd: <urn:sd:> .\nsd:s sd:p sd:o .\n";
    const FIXTURE_A_SHA256: &str =
        "374c0d9b987a7e7aa1548363922f47cf7ae04f1e4f855fb6d8a779ad0e350818";

    /// Two triples — a different byte stream, digest also via `shasum`.
    const FIXTURE_B: &str = "@prefix sd: <urn:sd:> .\nsd:s sd:p sd:o .\nsd:s2 sd:p sd:o2 .\n";
    const FIXTURE_B_SHA256: &str =
        "02516a43ba5a6f5d50dd3f85ea1091643b3b813f9c8f51ecab8b4bc2540fe50c";

    fn write_fixture(name: &str, content: &str) -> String {
        let path = format!("/tmp/pgrdf-source-digest-{name}.ttl");
        let mut f = std::fs::File::create(&path).expect("fixture create failed");
        f.write_all(content.as_bytes())
            .expect("fixture write failed");
        path
    }

    fn source_row(graph_id: i64) -> (Option<String>, Option<i32>) {
        let sha: Option<String> = Spi::get_one_with_args(
            "SELECT source_sha256 FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
            &[graph_id.into()],
        )
        .expect("source_sha256 read failed");
        let loads: Option<i32> = Spi::get_one_with_args(
            "SELECT source_loads FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
            &[graph_id.into()],
        )
        .expect("source_loads read failed");
        (sha, loads)
    }

    /// A file load records the sha256 of the exact bytes it consumed.
    #[pg_test]
    fn load_turtle_records_source_sha256() {
        let path = write_fixture("a", FIXTURE_A);
        Spi::run("SELECT pgrdf.add_graph(981101)").expect("add_graph failed");
        Spi::run(&format!("SELECT pgrdf.load_turtle('{path}', 981101)"))
            .expect("load_turtle failed");

        let (sha, loads) = source_row(981101);
        assert_eq!(sha.as_deref(), Some(FIXTURE_A_SHA256));
        assert_eq!(loads, Some(1));
    }

    /// A second load moves the digest to the LATEST bytes and counts it —
    /// `source_loads > 1` self-reports that whole-graph byte identity is gone.
    #[pg_test]
    fn load_twice_updates_digest_and_counts() {
        let path_a = write_fixture("twice-a", FIXTURE_A);
        let path_b = write_fixture("twice-b", FIXTURE_B);
        Spi::run("SELECT pgrdf.add_graph(981102)").expect("add_graph failed");
        Spi::run(&format!("SELECT pgrdf.load_turtle('{path_a}', 981102)"))
            .expect("first load failed");
        Spi::run(&format!("SELECT pgrdf.load_turtle('{path_b}', 981102)"))
            .expect("second load failed");

        let (sha, loads) = source_row(981102);
        assert_eq!(sha.as_deref(), Some(FIXTURE_B_SHA256));
        assert_eq!(loads, Some(2));
    }

    /// Never loaded ⇒ NULL in both columns. "Never recorded" must stay
    /// distinct from every possible digest.
    #[pg_test]
    fn graph_without_load_reads_null() {
        Spi::run("SELECT pgrdf.add_graph(981103)").expect("add_graph failed");
        let (sha, loads) = source_row(981103);
        assert_eq!(sha, None);
        assert_eq!(loads, None);
    }

    /// The content path (`parse_turtle`) is a load too — the bytes it sees
    /// are the text it was handed, and they get the same record.
    #[pg_test]
    fn parse_turtle_records_source_sha256() {
        Spi::run("SELECT pgrdf.add_graph(981104)").expect("add_graph failed");
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[FIXTURE_A.into(), 981104i64.into()],
        )
        .expect("parse_turtle failed");

        let (sha, loads) = source_row(981104);
        assert_eq!(sha.as_deref(), Some(FIXTURE_A_SHA256));
        assert_eq!(loads, Some(1));
    }
}
