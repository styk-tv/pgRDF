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

use pgrx::prelude::*;

/// Refuse (22P05) a parsed query or update that holds U+0000 in a term.
///
/// A SPARQL string literal may spell the character as `\u0000`;
/// PostgreSQL `text` cannot hold it. Left alone it went two ways. Where
/// the term reached SQL text, SPI failed with a Rust `NulError` dump
/// under XX000. Where it reached a bound parameter nothing failed at
/// all: `INSERT DATA` stored a term with a NUL in it.
///
/// `entry` prefixes the message, `text` is the request as received and
/// `parsed` its algebra. Only two escapes produce the character, so a
/// request without them passes without a second look. With one, the
/// algebra decides: the escape may sit in a comment or follow an escaped
/// backslash, and then no term holds U+0000.
pub(crate) fn refuse_nul_term(entry: &str, text: &str, parsed: &impl std::fmt::Debug) {
    if may_spell_nul(text) && debug_holds_nul(parsed) {
        crate::refuse(
            PgSqlErrorCode::ERRCODE_UNTRANSLATABLE_CHARACTER,
            format!(
                "{entry}: a term contains U+0000 (written \\u0000), \
                 which PostgreSQL text cannot hold"
            ),
        );
    }
}

/// Whether `text` could denote U+0000 once parsed: the 4- or 8-digit
/// escape, or the character itself.
fn may_spell_nul(text: &str) -> bool {
    text.contains("\\u0000") || text.contains("\\U00000000") || text.contains('\0')
}

/// Whether any string inside `value` holds U+0000, read off its `Debug`
/// rendering. `Debug` writes that character as `\0` and a backslash as
/// `\\`, so walking the escapes pairwise tells the two apart — and
/// covers every string of the algebra, whichever variant carries it,
/// without a hand-written walk that a new variant would slip past.
fn debug_holds_nul(value: &impl std::fmt::Debug) -> bool {
    let dump = format!("{value:?}");
    let mut chars = dump.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.next() == Some('0') {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod unit {
    use super::*;
    use spargebra::SparqlParser;

    fn query_holds_nul(q: &str) -> bool {
        let parsed = SparqlParser::new()
            .parse_query(q)
            .expect("test query parses");
        may_spell_nul(q) && debug_holds_nul(&parsed)
    }

    #[test]
    fn nul_is_found_wherever_a_string_sits() {
        assert!(debug_holds_nul(&"a\0b"));
        assert!(debug_holds_nul(&vec![Some(("x", "\0"))]));
        assert!(query_holds_nul(r#"SELECT ?s WHERE { ?s ?p "a\u0000b" }"#));
        assert!(query_holds_nul(
            r#"SELECT ?s WHERE { ?s ?p "\U00000000"@en }"#
        ));
        assert!(query_holds_nul(
            r#"SELECT (GROUP_CONCAT(?o; separator="\u0000") AS ?x) WHERE { ?s ?p ?o }"#
        ));
        assert!(query_holds_nul(
            r#"SELECT ?s WHERE { ?s ?p ?o FILTER(REGEX(STR(?o), "\u0000")) }"#
        ));
    }

    #[test]
    fn a_backslash_before_a_zero_is_not_nul() {
        // backslash, then the digit zero
        assert!(!debug_holds_nul(&"a\\0b"));
        assert!(!debug_holds_nul(&"\\\\0"));
        // two backslashes then `u0000`: an escaped backslash and five
        // ordinary characters
        assert!(!query_holds_nul(r#"SELECT ?s WHERE { ?s ?p "a\\u0000b" }"#));
        // the escape in a comment is in no term
        assert!(!query_holds_nul("SELECT ?s WHERE { ?s ?p ?o } # \\u0000"));
        assert!(!query_holds_nul(r#"SELECT ?s WHERE { ?s ?p "0" }"#));
    }
}

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

    /// Text that is not SPARQL is the caller's error: 42601 at every
    /// entry point that parses.
    #[pg_test]
    fn syntax_errors_refuse_42601() {
        for (sql, prefix) in [
            (
                "SELECT count(*) FROM pgrdf.sparql('SELEC ?s WHERE { ?s ?p ?o }')",
                "sparql: parse error: ",
            ),
            (
                "SELECT pgrdf.sparql_sql('SELEC ?s')",
                "sparql: parse error: ",
            ),
            ("SELECT pgrdf.sparql_parse('SELEC ?s')", "sparql_parse: "),
            (
                "SELECT count(*) FROM pgrdf.describe('SELEC ?s')",
                "pgrdf.describe: parse error: ",
            ),
        ] {
            let got = outcome(sql);
            assert_eq!(got.0, "42601", "{sql}: {}", got.1);
            assert!(got.1.starts_with(prefix), "{sql}: {}", got.1);
        }
    }

    /// A request that parses but is not valid for the function called,
    /// or that RDF forbids, is 22023.
    #[pg_test]
    fn invalid_requests_refuse_22023() {
        for (sql, fragment) in [
            (
                "SELECT count(*) FROM pgrdf.describe('SELECT ?s WHERE { ?s ?p ?o }')",
                "pgrdf.describe: not a DESCRIBE query",
            ),
            (
                "SELECT count(*) FROM pgrdf.construct('CONSTRUCT WHERE { ?s ?p _:b }')",
                "WHERE-shorthand prohibits blank nodes in the pattern",
            ),
            (
                "SELECT count(*) FROM pgrdf.construct(\
                 'CONSTRUCT { \"lit\" ?p ?o } WHERE { ?s ?p ?o }')",
                "literal not allowed in subject/predicate position",
            ),
        ] {
            assert_outcome(outcome(sql), "22023", fragment, sql);
        }
    }

    /// `"\u0000"` in a query or update is a term PostgreSQL text cannot
    /// hold. It refuses 22P05 before anything runs — no Rust `NulError`
    /// dump, and no row written with a NUL in it.
    #[pg_test]
    fn nul_escape_refuses_22p05_and_writes_nothing() {
        seed("urn:tr:nul");
        let dict_before: i64 = Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary")
            .unwrap()
            .unwrap();
        for q in [
            r#"SELECT ?s WHERE { ?s ?p "a\u0000b" }"#,
            r#"SELECT ?s WHERE { ?s ?p "\U00000000" }"#,
            r#"SELECT ?s WHERE { ?s ?p ?o FILTER(CONTAINS(STR(?o), "\u0000")) }"#,
            r#"SELECT (CONCAT(STR(?o), "\u0000") AS ?x) WHERE { ?s ?p ?o }"#,
            r#"SELECT ?s WHERE { VALUES ?o { "a\u0000b" } ?s ?p ?o }"#,
            r#"ASK { ?s ?p "\u0000"@en }"#,
            r#"INSERT DATA { <urn:tr:a> <urn:tr:p> "a\u0000b" }"#,
            r#"DELETE DATA { <urn:tr:a> <urn:tr:p> "a\u0000b" }"#,
            r#"INSERT { ?s <urn:tr:q> "a\u0000b" } WHERE { ?s <urn:tr:p> ?o }"#,
            r#"DELETE { ?s ?p ?o } WHERE { ?s ?p ?o FILTER(?o = "\u0000") }"#,
        ] {
            assert_outcome(sparql(q), "22P05", "U+0000", q);
        }
        for call in [
            r#"SELECT count(*) FROM pgrdf.construct($q$CONSTRUCT { ?s ?p "a\u0000b" } WHERE { ?s ?p ?o }$q$)"#,
            r#"SELECT count(*) FROM pgrdf.describe($q$DESCRIBE ?s WHERE { ?s ?p "\u0000" }$q$)"#,
            r#"SELECT pgrdf.sparql_sql($q$SELECT ?s WHERE { ?s ?p "\u0000" }$q$)"#,
            r#"SELECT pgrdf.sparql_parse($q$SELECT ?s WHERE { ?s ?p "\u0000" }$q$)"#,
        ] {
            assert_outcome(outcome(call), "22P05", "U+0000", call);
        }
        let dict_after: i64 = Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary")
            .unwrap()
            .unwrap();
        assert_eq!(dict_after, dict_before, "a refused request interns nothing");

        // The six characters `\u0000` are not the escape when the
        // backslash is itself escaped, and a comment is not a term.
        for q in [
            r#"SELECT ?s WHERE { ?s ?p "a\\u0000b" }"#,
            "SELECT ?s WHERE { ?s ?p ?o } # \\u0000",
        ] {
            let (state, msg) = sparql(q);
            assert_eq!(state, "00000", "{q}: {msg}");
        }
    }

    /// A derived table carries at most 64 distinct variables. Past that
    /// the request hits a limit of the implementation: 54000.
    #[pg_test]
    fn projection_arity_limit_refuses_54000() {
        let vars: Vec<String> = (0..65).map(|i| format!("?v{i}")).collect();
        let vals: Vec<String> = (0..65).map(|i| i.to_string()).collect();
        let q = format!(
            "SELECT ?s WHERE {{ ?s ?p ?o VALUES ({}) {{ ({}) }} }}",
            vars.join(" "),
            vals.join(" ")
        );
        assert_outcome(
            sparql(&q),
            "54000",
            "sparql: derived-table arity exceeded 64 distinct variables",
            "65-variable VALUES",
        );
        let within = format!(
            "SELECT ?s WHERE {{ ?s ?p ?o VALUES ({}) {{ ({}) }} }}",
            vars[..64].join(" "),
            vals[..64].join(" ")
        );
        let (state, msg) = sparql(&within);
        assert_eq!(state, "00000", "64 variables are within the limit: {msg}");
    }
    fn quads_in(graph_id: i64) -> i64 {
        Spi::get_one_with_args(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = $1",
            &[graph_id.into()],
        )
        .unwrap()
        .unwrap()
    }

    /// An UPDATE template pgRDF cannot instantiate refuses whether or
    /// not the WHERE pattern matches anything: the outcome is a property
    /// of the request. Each shape runs twice — over a pattern that
    /// matches nothing and over one that matches — and must refuse the
    /// same way both times, leaving the store untouched.
    #[pg_test]
    fn update_template_refusals_do_not_depend_on_the_data() {
        let g = seed("urn:tr:tpl");
        let total_before: i64 = Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads")
            .unwrap()
            .unwrap();
        // (template-and-form with a `{W}` slot for the WHERE predicate,
        //  SQLSTATE, message fragment)
        let shapes = [
            (
                "INSERT { _:b <urn:tr:q> ?o } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "INSERT WHERE template feature 'blank node in template (b)'",
            ),
            (
                "INSERT { ?s <urn:tr:q> _:b } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "INSERT WHERE template feature 'blank node in template (b)'",
            ),
            (
                "INSERT { GRAPH ?o { ?s <urn:tr:q> ?o } } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "INSERT WHERE template feature 'variable GRAPH ?o'",
            ),
            (
                "INSERT { \"lit\" <urn:tr:q> ?o } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "22023",
                "sparql: literal subject is invalid in RDF",
            ),
            (
                "DELETE { GRAPH ?o { ?s <urn:tr:p> ?o } } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "DELETE WHERE template feature 'variable GRAPH ?o'",
            ),
            // A constant the dictionary has never seen used to end the
            // row's instantiation before the GRAPH check was reached.
            (
                "DELETE { GRAPH ?o { <urn:tr:never-seen> <urn:tr:p> ?o } } \
                 WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "DELETE WHERE template feature 'variable GRAPH ?o'",
            ),
            (
                "DELETE { \"lit\" <urn:tr:p> ?o } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "22023",
                "sparql: literal subject is invalid in RDF",
            ),
            (
                "DELETE { ?s <urn:tr:p> ?o } INSERT { _:b <urn:tr:q> ?o } \
                 WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "INSERT WHERE template feature 'blank node in template (b)'",
            ),
            (
                "DELETE { GRAPH ?o { ?s <urn:tr:p> ?o } } INSERT { ?s <urn:tr:q> ?o } \
                 WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "DELETE WHERE template feature 'variable GRAPH ?o'",
            ),
        ];
        for (shape, state, fragment) in shapes {
            for (label, predicate) in [("no match", "<urn:tr:nomatch>"), ("match", "<urn:tr:p>")] {
                let q = shape.replace("{W}", predicate);
                assert_outcome(sparql(&q), state, fragment, &format!("{label}: {q}"));
            }
        }
        let total_after: i64 = Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads")
            .unwrap()
            .unwrap();
        assert_eq!(total_after, total_before, "no refused UPDATE wrote a quad");
        assert_eq!(quads_in(g), 3);
    }

    /// CLEAR and DROP of a graph that does not exist: 42704. CREATE of
    /// one that does: 42710. `SILENT` suppresses each, as W3C SPARQL 1.1
    /// Update §3.2 requires.
    #[pg_test]
    fn graph_management_refusals_are_typed_and_silent_is_honoured() {
        let g = seed("urn:tr:mgmt");
        assert_outcome(
            sparql("CLEAR GRAPH <urn:tr:absent>"),
            "42704",
            "sparql: CLEAR GRAPH <urn:tr:absent>: graph not bound",
            "CLEAR",
        );
        assert_outcome(
            sparql("DROP GRAPH <urn:tr:absent>"),
            "42704",
            "sparql: DROP GRAPH <urn:tr:absent>: graph not bound",
            "DROP",
        );
        assert_outcome(
            sparql("CREATE GRAPH <urn:tr:mgmt>"),
            "42710",
            "sparql: CREATE GRAPH <urn:tr:mgmt>: graph already exists",
            "CREATE",
        );
        for q in [
            "CLEAR SILENT GRAPH <urn:tr:absent>",
            "DROP SILENT GRAPH <urn:tr:absent>",
            "CREATE SILENT GRAPH <urn:tr:mgmt>",
        ] {
            let (state, msg) = sparql(q);
            assert_eq!(state, "00000", "{q}: {msg}");
        }
        assert_eq!(
            quads_in(g),
            3,
            "none of the six touched the graph's triples"
        );
    }

    /// `graph_integrity` of a graph id nobody registered: 42704.
    #[pg_test]
    fn graph_integrity_of_unknown_graph_refuses_42704() {
        assert_outcome(
            outcome("SELECT pgrdf.graph_integrity(970998)"),
            "42704",
            "graph_integrity: no graph with id 970998 (see pgrdf._pgrdf_graphs)",
            "graph_integrity",
        );
    }
}
