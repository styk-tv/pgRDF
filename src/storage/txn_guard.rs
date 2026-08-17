//! #123 — the staged loader refuses in uncommittable transactions.
//!
//! Measured (v0.6.31 cycle, 31 minutes at 0% CPU): a one-statement-per-line
//! file sniffs as N-Triples, the preloaded worker pool reports ready, and
//! `load_turtle` hands the file to the staged loader — whose workers commit
//! their own transactions, inside a caller transaction that can never allow
//! it. Coordinator waits on workers, workers wait on the caller's locks:
//! silent, indefinite. The reports-not-refuses family, in the loader.
//!
//! Contract (the v0.6.32 recheck, stated before the code):
//!   - `load_turtle`'s auto-dispatch FALLS BACK to the standard parser
//!     inside a transaction block — auto-selection never picks a path that
//!     cannot work;
//!   - a DIRECT `load_turtle_staged_run` inside a transaction block RAISES
//!     a stable `pgRDF#123` error naming the constraint and the rewrite.
//!
//! Every test collars itself with `statement_timeout` so a regression hangs
//! for seconds, not the 31 minutes the discovery cost.

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;
    use std::io::Write;

    /// Bare one-statement-per-line — sniffs as N-Triples, which is the
    /// exact shape that dispatched the original hang.
    fn write_nt_fixture(name: &str) -> String {
        let path = format!("/tmp/pgrdf-txn-guard-{name}.nt");
        let mut f = std::fs::File::create(&path).expect("fixture create failed");
        f.write_all(b"<urn:tg:s> <urn:tg:p> <urn:tg:o> .\n")
            .expect("fixture write failed");
        path
    }

    /// The 31-minute hang, collared: inside a transaction block the
    /// N-Triples sniff must fall back to the standard parser and complete.
    #[pg_test]
    fn ntriples_sniff_falls_back_in_txn_block() {
        Spi::run("SET statement_timeout = '8s'").expect("timeout collar failed");
        let path = write_nt_fixture("fallback");
        Spi::run("SELECT pgrdf.add_graph(982001)").expect("add_graph failed");
        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.load_turtle($1, 982001)",
            &[path.as_str().into(), 982001i64.into()],
        )
        .expect("load_turtle failed")
        .expect("load_turtle returned NULL");
        assert_eq!(n, 1);
    }

    /// A direct staged call in a transaction block refuses with the
    /// rewrite — it must never wait on workers that cannot proceed.
    #[pg_test(
        error = "pgRDF#123: the staged loader commits per phase and cannot run inside a transaction block; call it as a single statement, or use pgrdf.load_turtle / pgrdf.parse_turtle here"
    )]
    fn staged_run_refuses_in_txn_block() {
        Spi::run("SET statement_timeout = '8s'").expect("timeout collar failed");
        let path = write_nt_fixture("refuse");
        Spi::run("SELECT pgrdf.add_graph(982002)").expect("add_graph failed");
        Spi::run(&format!(
            "SELECT pgrdf.load_turtle_staged_run('{path}', 982002)"
        ))
        .expect("unreachable: the call above must raise");
    }
}
