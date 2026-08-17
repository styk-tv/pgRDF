//! #109 — VALUES bound to a graph variable refuses instead of widening.
//!
//! Measured live on 0.6.31 (2026-08-16, re-confirming the 0.6.27 filing):
//! `VALUES ?g { <three nonexistent graphs> } GRAPH ?g { … }` answered for
//! EVERY shape-bearing graph in the store — the binding contributed
//! nothing, and the caller received a confidently unscoped answer with no
//! signal. Survives the #114 guards (no UNION involved). The read-path
//! member of the reports-not-refuses family.
//!
//! Contract (v0.6.32 recheck): a `VALUES` that binds a variable used as a
//! `GRAPH` name RAISES a stable `pgRDF#109` error naming the rewrite
//! (enumerate the graphs as explicit `GRAPH <iri>` groups, or wait for the
//! join in #111's follow-up). A `VALUES` on a plain (non-graph) variable
//! keeps applying — pinned here, measured working on the bench today.

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    fn seed(graph_id: i64) {
        Spi::run(&format!("SELECT pgrdf.add_graph({graph_id})")).expect("add_graph failed");
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix v: <urn:v:> .\nv:a v:p v:x .\nv:b v:p v:y .\n".into(),
                graph_id.into(),
            ],
        )
        .expect("seed parse failed");
    }

    /// The repro, exactly as measured: VALUES on the graph variable must
    /// refuse — never answer over graphs the binding excluded.
    #[pg_test(
        error = "sparql: VALUES binds ?g, which also names a GRAPH scope — the binding is not joined into graph resolution on the SELECT/CONSTRUCT path and the answer would silently widen to every graph (pgRDF#109). Enumerate explicit GRAPH <iri> groups instead. Refusing instead of returning a wrong answer."
    )]
    fn values_on_graph_variable_refuses() {
        seed(982101);
        Spi::run(
            "SELECT * FROM pgrdf.sparql('SELECT ?g ?s WHERE {
                VALUES ?g { <urn:g:one> <urn:g:two> }
                GRAPH ?g { ?s ?p ?o } }')",
        )
        .expect("unreachable: the call above must raise");
    }

    /// Regression pin (passes today, measured on the bench): VALUES on a
    /// plain variable keeps applying inside an explicit GRAPH group.
    #[pg_test]
    fn values_on_plain_variable_still_applies() {
        seed(982102);
        let n: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pgrdf.sparql('SELECT ?s WHERE {
                VALUES ?s { <urn:v:a> }
                GRAPH <urn:pgrdf:graph:982102> { ?s ?p ?o } }')",
        )
        .expect("query failed")
        .expect("count NULL");
        assert_eq!(n, 1, "VALUES on a plain variable must keep restricting");
    }
}
