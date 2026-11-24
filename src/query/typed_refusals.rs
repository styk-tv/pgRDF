//! #159 — typed refusals on the query path.
//!
//! When `pgrdf.sparql`, `construct`, `describe`, `sparql_sql` or
//! `sparql_parse` fails, the SQLSTATE says who has to act:
//!
//! | class             | SQLSTATE        | meaning                                        |
//! |-------------------|-----------------|------------------------------------------------|
//! | unsupported shape | `0A000`         | valid SPARQL that pgRDF does not execute       |
//! | limit             | `54000`         | the request exceeds a bound of the engine      |
//! | request error     | `22xxx`/`42xxx` | the request is wrong, or names what is absent  |
//! | invariant         | `XX000`         | pgRDF or the store is broken — never a request |
//!
//! A refusal depends on the request, never on the data: the same text
//! refuses over an empty store and over a full one. The one input that
//! is not the text is the graph catalogue — a request naming a graph
//! that does not exist (`42704`) or already does (`42710`).
//!
//! Deliberate refusals go through `crate::refuse`; what is left as
//! `panic!` / `expect` / `unreachable!` on the query path is an
//! invariant, and reaching one is a bug to report. The tests below pin
//! one SQLSTATE per refusal family.

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// Run `sql` in a PL/pgSQL subtransaction and report how it ended:
    /// `("00000", "")` on success, else `(SQLSTATE, message)`. The
    /// subtransaction is rolled back on error, so the caller may keep
    /// using SPI afterwards — which is what lets a test assert that a
    /// refused request wrote nothing.
    fn outcome(sql: &str) -> (String, String) {
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.refusal_of(q text) RETURNS text \
             LANGUAGE plpgsql AS $$ DECLARE st text; m text; BEGIN \
               EXECUTE q; RETURN '00000|'; \
             EXCEPTION WHEN OTHERS THEN \
               GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, m = MESSAGE_TEXT; \
               RETURN st || '|' || m; END $$",
        )
        .unwrap();
        let got: String = Spi::get_one_with_args("SELECT pg_temp.refusal_of($1)", &[sql.into()])
            .unwrap()
            .unwrap();
        let (st, msg) = got.split_once('|').unwrap();
        (st.to_string(), msg.to_string())
    }

    /// `outcome` of running `q` through `pgrdf.sparql` to the last row.
    fn sparql(q: &str) -> (String, String) {
        outcome(&format!(
            "SELECT count(*) FROM pgrdf.sparql($sparql${q}$sparql$)"
        ))
    }

    #[track_caller]
    fn assert_outcome(got: (String, String), state: &str, fragment: &str, what: &str) {
        assert_eq!(got.0, state, "{what}: SQLSTATE {} — {}", got.0, got.1);
        assert!(
            got.1.contains(fragment),
            "{what}: message {:?} lacks {fragment:?}",
            got.1
        );
    }

    /// One named graph with a two-hop chain and a number, plus one
    /// default-graph triple. Returns the named graph's id.
    fn seed(iri: &str) -> i64 {
        let g: i64 = Spi::get_one_with_args("SELECT pgrdf.add_graph($1)", &[iri.into()])
            .unwrap()
            .unwrap();
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "<urn:tr:a> <urn:tr:p> <urn:tr:b> . <urn:tr:b> <urn:tr:p> \"x\" . \
                 <urn:tr:a> <urn:tr:n> 5 ."
                    .into(),
                g.into(),
            ],
        )
        .unwrap();
        Spi::get_one::<i64>("SELECT pgrdf.parse_turtle('<urn:tr:d> <urn:tr:p> <urn:tr:e> .', 0)")
            .unwrap();
        g
    }

    fn quads_in(graph_id: i64) -> i64 {
        Spi::get_one_with_args(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = $1",
            &[graph_id.into()],
        )
        .unwrap()
        .unwrap()
    }

    /// Every shape pgRDF parses but does not execute refuses 0A000 —
    /// one request per message family. None of these depends on what
    /// the store holds: the same request refuses over an empty store.
    #[pg_test]
    fn unsupported_shapes_refuse_0a000() {
        const P: &str = "<urn:tr:p>";
        let cases: Vec<(String, &str)> = vec![
            // query forms and empty groups
            ("DESCRIBE <urn:tr:a>".into(), "use pgrdf.describe(q)"),
            (
                "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }".into(),
                "query form not supported yet",
            ),
            ("SELECT * WHERE { }".into(), "sparql: empty BGP"),
            ("ASK { }".into(), "sparql: ASK with empty BGP"),
            // variables no pattern binds
            (
                "SELECT ?zz WHERE { ?s ?p ?o }".into(),
                "projected variable ?zz is not bound in any BGP pattern",
            ),
            (
                "SELECT ?s WHERE { ?s ?p ?o } ORDER BY ?zz".into(),
                "ORDER BY variable ?zz not bound in any BGP pattern",
            ),
            (
                "SELECT ?zz (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?zz".into(),
                "GROUP BY variable ?zz not bound in any BGP pattern",
            ),
            (
                "SELECT ?s (COUNT(?zz) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "aggregate COUNT(?zz) but variable not bound",
            ),
            (
                "SELECT ?s (SUM(?zz) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "numeric aggregate over ?zz but variable not bound",
            ),
            (
                "SELECT ?s (GROUP_CONCAT(?zz) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "aggregate over ?zz but variable not bound",
            ),
            // ORDER BY
            (
                "SELECT ?s WHERE { ?s ?p ?o } ORDER BY (<urn:x:fn>(?o))".into(),
                "ORDER BY expression not translatable",
            ),
            (
                "SELECT DISTINCT ?s WHERE { ?s ?p ?o } ORDER BY (STR(?o))".into(),
                "ORDER BY over an expression is not supported together with DISTINCT",
            ),
            (
                "SELECT DISTINCT ?s WHERE { ?s ?p ?o } ORDER BY ?o".into(),
                "requires ?o to be in the SELECT clause when DISTINCT is used",
            ),
            (
                "SELECT ?s (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s ORDER BY (STR(?s))"
                    .into(),
                "ORDER BY expression sort keys are not supported on an aggregate query",
            ),
            (
                "SELECT ?s (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s ORDER BY ?p".into(),
                "ORDER BY ?p on an aggregate query must reference a projected variable",
            ),
            (
                format!(
                    "SELECT ?s WHERE {{ {{ ?s {P} ?o }} UNION {{ ?s <urn:tr:n> ?o }} }} ORDER BY ?o"
                ),
                "ORDER BY ?o on UNION must reference a projected variable",
            ),
            (
                format!(
                    "SELECT ?s (COUNT(?o) AS ?n) WHERE {{ {{ ?s {P} ?o }} UNION \
                     {{ ?s <urn:tr:n> ?o }} }} GROUP BY ?s ORDER BY ?o"
                ),
                "on an aggregate-over-UNION query must reference a projected variable",
            ),
            // grouping and aggregates
            (
                "SELECT (?s AS ?t) (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "?t is neither a GROUP BY variable nor an aggregate output",
            ),
            (
                "SELECT (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } HAVING (<urn:x:fn>(?n))".into(),
                "HAVING expression not translatable",
            ),
            (
                format!(
                    "SELECT (COUNT(?o) AS ?n) WHERE {{ {{ ?s {P} ?o }} UNION \
                     {{ ?s <urn:tr:n> ?o }} }} HAVING (<urn:x:fn>(?n))"
                ),
                "HAVING expression not translatable",
            ),
            (
                "SELECT ?s (COUNT(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "COUNT over expression not translatable",
            ),
            (
                "SELECT ?s (SUM(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "SUM over expression not numerically translatable",
            ),
            (
                "SELECT ?s (MIN(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "MIN over expression not translatable",
            ),
            (
                "SELECT ?s (MAX(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "MAX over expression not translatable",
            ),
            (
                "SELECT ?s (GROUP_CONCAT(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s"
                    .into(),
                "GROUP_CONCAT over expression not translatable",
            ),
            (
                "SELECT ?s (SAMPLE(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "SAMPLE over expression not translatable",
            ),
            (
                format!(
                    "SELECT ?g (SUM(STRLEN(STR(?g))) AS ?n) WHERE {{ {{ GRAPH ?g {{ ?s {P} ?o }} }} \
                     UNION {{ GRAPH ?g {{ ?s <urn:tr:n> ?o }} }} }} GROUP BY ?g"
                ),
                "UNION graph-text-lane variable ?g not supported yet",
            ),
            (
                format!(
                    "SELECT ?g (COUNT(?s) AS ?n) WHERE {{ {{ GRAPH ?g {{ ?s {P} ?o }} }} \
                     UNION {{ ?s <urn:tr:n> ?g }} }} GROUP BY ?g"
                ),
                "GRAPH-scoped in one UNION branch but term-bound in another",
            ),
            // OPTIONAL
            (
                format!(
                    "SELECT ?s WHERE {{ ?s {P} ?o OPTIONAL {{ ?o {P} ?x \
                     {{ SELECT ?y WHERE {{ ?y ?q ?z }} }} }} }}"
                ),
                "OPTIONAL group contains an unsupported sub-pattern",
            ),
            (
                format!(
                    "SELECT ?s WHERE {{ ?s {P} ?o OPTIONAL {{ ?o {P} ?x . \
                     GRAPH ?g {{ ?x {P} ?y . ?y {P} ?z }} }} }}"
                ),
                "introduced inside a multi-triple OPTIONAL is not yet supported",
            ),
            // VALUES on a graph variable (#109)
            (
                "SELECT ?g ?s WHERE { VALUES ?g { <urn:tr:g1> } GRAPH ?g { ?s ?p ?o } }".into(),
                "pgRDF#109",
            ),
            // UPDATE shapes refused before the WHERE runs
            (
                format!(
                    "INSERT {{ ?s {P} ?n }} WHERE {{ SELECT ?s (COUNT(?o) AS ?n) \
                     WHERE {{ ?s {P} ?o }} GROUP BY ?s }}"
                ),
                "INSERT WHERE template feature 'aggregate/GROUP BY in WHERE'",
            ),
            (
                format!(
                    "DELETE {{ ?s {P} ?n }} WHERE {{ SELECT ?s (COUNT(?o) AS ?n) \
                     WHERE {{ ?s {P} ?o }} GROUP BY ?s }}"
                ),
                "DELETE WHERE template feature 'aggregate/GROUP BY in WHERE'",
            ),
            (
                format!(
                    "DELETE {{ ?s {P} ?o }} INSERT {{ ?s {P} ?n }} WHERE {{ SELECT ?s \
                     (COUNT(?o) AS ?n) WHERE {{ ?s {P} ?o }} GROUP BY ?s }}"
                ),
                "DELETE/INSERT WHERE template feature 'aggregate/GROUP BY in WHERE'",
            ),
            (
                format!("INSERT {{ ?s {P} ?zz }} WHERE {{ ?s {P} ?o }}"),
                "INSERT WHERE template feature 'unbound template variable ?zz'",
            ),
            (
                format!("DELETE {{ ?s {P} ?zz }} WHERE {{ ?s {P} ?o }}"),
                "DELETE WHERE template feature 'unbound template variable ?zz'",
            ),
            (
                format!("DELETE {{ ?s {P} ?zz }} INSERT {{ ?s {P} ?o }} WHERE {{ ?s {P} ?o }}"),
                "DELETE/INSERT WHERE template feature 'unbound template variable ?zz'",
            ),
        ];
        for (q, fragment) in &cases {
            assert_outcome(sparql(q), "0A000", fragment, q);
        }
        // The same class at the other entry point.
        for (q, fragment) in [
            (
                "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o } ORDER BY ?s",
                "DISTINCT / ORDER BY / GROUP BY / aggregates not supported",
            ),
            (
                "CONSTRUCT { ?s ?p ?o } WHERE { SELECT DISTINCT ?s ?p ?o WHERE { ?s ?p ?o } }",
                "DISTINCT / ORDER BY / GROUP BY / aggregates not supported",
            ),
            (
                "CONSTRUCT { ?s ?p ?x } WHERE { ?s ?p ?o BIND(ISIRI(?o) AS ?x) }",
                "BIND expression for template ?x not translatable",
            ),
        ] {
            let got = outcome(&format!(
                "SELECT count(*) FROM pgrdf.construct($sparql${q}$sparql$)"
            ));
            assert_outcome(got, "0A000", fragment, q);
        }
    }

    /// #114: a group-level FILTER beside a UNION refuses 0A000, the
    /// message is the one consumers already match on, and the refusal
    /// still counts in `filter_clauses_dropped`.
    #[pg_test]
    fn filter_over_union_refuses_0a000_and_still_counts() {
        let before = crate::storage::shmem_cache::snapshot().filter_clauses_dropped;
        let got = sparql(
            "SELECT ?c WHERE { { ?c a <urn:t:A> } UNION { ?c a <urn:t:B> } \
             FILTER(?c = <urn:t:x>) }",
        );
        assert_eq!(got.0, "0A000", "{}", got.1);
        assert_eq!(
            got.1,
            "sparql: group-level FILTER alongside a UNION is not applied on the \
             SELECT/CONSTRUCT path (pgRDF#114) — it would be silently dropped and widen the \
             result set. Move the FILTER into every UNION branch (or restructure the query). \
             Refusing instead of returning a wrong answer."
        );
        let after = crate::storage::shmem_cache::snapshot().filter_clauses_dropped;
        assert_eq!(after, before + 1, "the refusal must count");
    }

    /// `ISIRI()` and the other boolean-valued forms in a SELECT
    /// expression are not translated to a value; that is an unsupported
    /// shape (0A000), not an engine fault. The same test inside FILTER
    /// keeps working.
    #[pg_test]
    fn boolean_valued_select_expressions_refuse_0a000() {
        seed("urn:tr:bool");
        for expr in [
            "ISIRI(?o)",
            "ISURI(?o)",
            "ISBLANK(?o)",
            "ISLITERAL(?o)",
            "ISNUMERIC(?o)",
            "BOUND(?o)",
            "sameTerm(?s, ?o)",
            "?s = ?o",
            "REGEX(STR(?o), \"a\")",
        ] {
            let q = format!("SELECT ({expr} AS ?x) WHERE {{ ?s ?p ?o }}");
            assert_outcome(
                sparql(&q),
                "0A000",
                "BIND expression for ?x not translatable",
                &q,
            );
        }
        let (state, msg) = sparql("SELECT ?s WHERE { ?s ?p ?o FILTER(ISIRI(?o)) }");
        assert_eq!(state, "00000", "ISIRI in FILTER is supported: {msg}");
    }

    /// A FILTER pgRDF cannot translate refuses with the same class
    /// wherever it sits: top level, as an OPTIONAL's join condition, or
    /// inside a nested OPTIONAL.
    #[pg_test]
    fn untranslatable_filter_refuses_0a000_inside_optional_too() {
        let top = sparql("SELECT ?s WHERE { ?s <urn:tr:p> ?o FILTER(<urn:x:fn>(?o)) }");
        assert_outcome(
            top,
            "0A000",
            "FILTER expression not translatable",
            "top level",
        );
        let join = sparql(
            "SELECT ?s WHERE { ?s <urn:tr:p> ?o \
             OPTIONAL { ?o <urn:tr:p> ?x FILTER(<urn:x:fn>(?x)) } }",
        );
        assert_outcome(
            join,
            "0A000",
            "OPTIONAL join FILTER not translatable",
            "join",
        );
        let nested = sparql(
            "SELECT ?s WHERE { ?s <urn:tr:p> ?o OPTIONAL { ?o <urn:tr:p> ?x \
             OPTIONAL { ?x <urn:tr:p> ?y FILTER(<urn:x:fn>(?y)) } } }",
        );
        assert_eq!(nested.0, "0A000", "nested: {}", nested.1);
    }
}
