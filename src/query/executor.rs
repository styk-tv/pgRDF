//! BGP → SQL translator.
//!
//! Phase 2.2 steps 5 + 6: translate a SPARQL SELECT with one or more
//! Basic Graph Pattern triples into a dynamically-generated SQL
//! SELECT over `_pgrdf_quads` joined to `_pgrdf_dictionary`, then
//! return one JSONB row per match.
//!
//! Each BGP pattern gets a `_pgrdf_quads` alias (`q1`, `q2`, …). The
//! first occurrence of a variable records `(alias, column)` as its
//! anchor binding; subsequent occurrences emit equality predicates
//! against that anchor (`q2.subject_id = q1.subject_id`) — that's
//! how shared variables across patterns become INNER JOINs.
//!
//! Constants are resolved to dictionary ids up-front so the dynamic
//! SQL only carries integer constants (never user-supplied IRI
//! strings — that's how the translator avoids SQL injection while
//! still building the query at function-call time).
//!
//! Scope today:
//!   * SELECT only (no CONSTRUCT/ASK/DESCRIBE).
//!   * N BGP triples joined by shared variables.
//!   * FILTER expressions over identity (`=`, `!=`, `sameTerm`),
//!     boolean composition (`&&`, `||`, `!`), term-type predicates
//!     (`isIRI`, `isLiteral`, `isBlank`), and `BOUND` (trivially TRUE
//!     in a BGP context). Numeric ordering / regex / arithmetic are
//!     Phase 3+.
//!   * Constants in any position (subject IRI, predicate IRI,
//!     object IRI or literal).
//!   * Variables in any position.
//!   * Distinct / Reduced / Slice / OrderBy wrappers are walked
//!     through without affecting translation.
//!   * OPTIONAL / UNION / aggregates / paths / VALUES / SERVICE
//!     remain unsupported.
//!
//! Output shape:
//!   `SETOF JSONB` — one row per solution, keys = projected variable
//!   names, values = lexical strings (NULL when the binding maps to
//!   a term id missing from the dictionary).

use crate::storage::dict::term_type;
use pgrx::iter::SetOfIterator;
use pgrx::prelude::*;
use serde_json::{Map, Value};
use spargebra::algebra::{Expression, Function, GraphPattern};
use spargebra::term::{Literal, NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};
use std::collections::HashMap;

/// Execute a SPARQL SELECT query and return one JSONB row per solution.
/// Each row is a JSON object keyed by the projected variable names.
///
/// SQL surface: `pgrdf.sparql(q TEXT) → SETOF JSONB`.
///
/// Invocation typically looks like:
///
/// ```sql
/// SELECT * FROM pgrdf.sparql('SELECT ?s ?p WHERE { ?s foaf:name ?p }');
///   →  {"s": "http://example.com/alice", "p": "Alice"}
///      {"s": "http://example.com/bob",   "p": "Bob"}
/// ```
#[pg_extern]
fn sparql(query: &str) -> SetOfIterator<'static, pgrx::JsonB> {
    let parsed = SparqlParser::new()
        .parse_query(query)
        .unwrap_or_else(|e| panic!("sparql: parse error: {e}"));
    let plan = translate(&parsed);
    let rows = execute(&plan);
    SetOfIterator::new(rows.into_iter())
}

// ─────────────────────────────────────────────────────────────────────
// Plan
// ─────────────────────────────────────────────────────────────────────

struct ExecPlan {
    /// Projected variables, in SELECT-clause order. These are the
    /// JSONB keys in each output row.
    projected: Vec<String>,
    /// Fully-built SQL string with constant dict IDs already
    /// inlined. The translator never passes user IRI strings into
    /// the dynamic SQL — only the SMALL number of integer IDs we
    /// resolved upfront.
    sql: String,
}

fn translate(q: &Query) -> ExecPlan {
    let pattern = match q {
        Query::Select { pattern, .. } => pattern,
        other => panic!("sparql: only SELECT supported in v0.2 (got {other:?})"),
    };
    let (projected, bgp, filters) = unwrap_select(pattern);
    if bgp.is_empty() {
        panic!("sparql: empty BGP");
    }
    let sql = build_bgp_sql(&bgp, &filters, &projected);
    ExecPlan { projected, sql }
}

/// Walk through wrappers we can pass through transparently and
/// surface the projection + BGP shape underneath, collecting any
/// FILTER expressions seen along the way.
fn unwrap_select(p: &GraphPattern) -> (Vec<String>, Vec<TriplePattern>, Vec<Expression>) {
    let mut filters: Vec<Expression> = Vec::new();
    match p {
        GraphPattern::Project { inner, variables } => {
            let vars: Vec<String> = variables.iter().map(|v| v.as_str().to_string()).collect();
            let bgp = extract_bgp_and_filters(inner, &mut filters);
            (vars, bgp, filters)
        }
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => unwrap_select(inner),
        GraphPattern::Slice { inner, .. } | GraphPattern::OrderBy { inner, .. } => {
            unwrap_select(inner)
        }
        GraphPattern::Filter { .. } => {
            // SELECT * with FILTER wrapping the BGP.
            let bgp = extract_bgp_and_filters(p, &mut filters);
            let mut vars: Vec<String> = Vec::new();
            for tp in &bgp {
                push_unique(&mut vars, tp_subject_var(tp));
                push_unique(&mut vars, tp_predicate_var(tp));
                push_unique(&mut vars, tp_object_var(tp));
            }
            (vars, bgp, filters)
        }
        GraphPattern::Bgp { patterns } => {
            // SELECT * — collect variables in the order they appear.
            let mut vars: Vec<String> = Vec::new();
            for tp in patterns {
                push_unique(&mut vars, tp_subject_var(tp));
                push_unique(&mut vars, tp_predicate_var(tp));
                push_unique(&mut vars, tp_object_var(tp));
            }
            (vars, patterns.clone(), filters)
        }
        other => panic!("sparql: unsupported algebra in select wrapper: {other:?}"),
    }
}

fn extract_bgp_and_filters(
    p: &GraphPattern,
    filters: &mut Vec<Expression>,
) -> Vec<TriplePattern> {
    match p {
        GraphPattern::Bgp { patterns } => patterns.clone(),
        GraphPattern::Filter { expr, inner } => {
            filters.push(expr.clone());
            extract_bgp_and_filters(inner, filters)
        }
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => {
            extract_bgp_and_filters(inner, filters)
        }
        GraphPattern::Slice { inner, .. } | GraphPattern::OrderBy { inner, .. } => {
            extract_bgp_and_filters(inner, filters)
        }
        other => panic!("sparql: expected BGP (optionally wrapped in FILTER), got {other:?}"),
    }
}

fn push_unique(out: &mut Vec<String>, name: Option<String>) {
    if let Some(n) = name {
        if !out.contains(&n) {
            out.push(n);
        }
    }
}

fn tp_subject_var(tp: &TriplePattern) -> Option<String> {
    if let TermPattern::Variable(v) = &tp.subject {
        Some(v.as_str().to_string())
    } else {
        None
    }
}
fn tp_predicate_var(tp: &TriplePattern) -> Option<String> {
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        Some(v.as_str().to_string())
    } else {
        None
    }
}
fn tp_object_var(tp: &TriplePattern) -> Option<String> {
    if let TermPattern::Variable(v) = &tp.object {
        Some(v.as_str().to_string())
    } else {
        None
    }
}

// ─────────────────────────────────────────────────────────────────────
// Translation
// ─────────────────────────────────────────────────────────────────────

/// Build a dynamic SQL SELECT for an N-pattern BGP. Each pattern
/// becomes a `_pgrdf_quads qN` clause; shared variables become
/// equality predicates that fold into INNER joins; constants become
/// `qN.col = <resolved_dict_id>` predicates; FILTER expressions
/// become SQL WHERE predicates appended after the join clauses.
fn build_bgp_sql(
    patterns: &[TriplePattern],
    filters: &[Expression],
    projected: &[String],
) -> String {
    /// First-occurrence anchor for each variable: which alias +
    /// which column. Subsequent occurrences emit equality predicates
    /// against this anchor.
    let mut anchors: HashMap<String, (usize, &'static str)> = HashMap::new();
    let mut where_clauses: Vec<String> = Vec::new();
    let mut from_aliases: Vec<String> = Vec::with_capacity(patterns.len());

    for (i, tp) in patterns.iter().enumerate() {
        let qi = i + 1;
        from_aliases.push(format!("pgrdf._pgrdf_quads q{qi}"));

        bind_subject(tp, qi, &mut anchors, &mut where_clauses);
        bind_predicate(tp, qi, &mut anchors, &mut where_clauses);
        bind_object(tp, qi, &mut anchors, &mut where_clauses);
    }

    // FILTER predicates land after the BGP joins, ANDed onto the
    // WHERE clause. translate_filter returns None for any expression
    // shape we don't yet handle — panic in that case rather than
    // silently dropping the filter (which would over-return rows).
    for expr in filters {
        let sql = translate_filter(expr, &anchors).unwrap_or_else(|| {
            panic!("sparql: FILTER expression not translatable: {expr:?}")
        });
        where_clauses.push(sql);
    }

    // Project: each projected var → its anchor alias.column → dict
    // lookup → aliased to the variable name (so SETOF JSONB emission
    // pulls the right column by ordinal).
    let mut select_clauses: Vec<String> = Vec::new();
    for var in projected {
        let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
            panic!("sparql: projected variable ?{var} is not bound in any BGP pattern")
        });
        select_clauses.push(format!(
            "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary
               WHERE id = q{alias_idx}.{col}) AS {alias_v}",
            alias_v = quote_identifier(var),
        ));
    }

    let mut sql = format!(
        "SELECT {sel} FROM {from}",
        sel = select_clauses.join(", "),
        from = from_aliases.join(", "),
    );
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql
}

// ─────────────────────────────────────────────────────────────────────
// FILTER translation
// ─────────────────────────────────────────────────────────────────────

/// Translate a SPARQL FILTER expression into a SQL boolean predicate
/// referencing the BGP's `qN.{subject_id,predicate_id,object_id}`
/// dictionary-id columns. Returns `None` for any expression shape
/// that doesn't have a sound dict-id-only translation — the caller
/// panics in that case rather than silently drop the filter.
///
/// Supported today:
///   * `?a = ?b`, `?a = <iri>`, `?a = "literal"` (and `sameTerm`):
///     compared as dictionary ids. Sound because our dict deduplicates
///     terms by (term_type, lexical, datatype, lang).
///   * `?a != …`: negated identity comparison.
///   * `&&`, `||`, `!`: boolean composition.
///   * `isIRI(?v)`, `isLiteral(?v)`, `isBlank(?v)`: emit a correlated
///     subselect against `_pgrdf_dictionary.term_type`.
///   * `BOUND(?v)`: trivially TRUE in a BGP context (every BGP
///     variable is bound on every row).
///
/// Not yet supported (would return None): numeric ordering
/// (`<`/`>`/`<=`/`>=`), arithmetic, `regex`, `str`, `lang`,
/// `datatype`, `IN`, `EXISTS`, conditional expressions.
fn translate_filter(
    expr: &Expression,
    anchors: &HashMap<String, (usize, &'static str)>,
) -> Option<String> {
    match expr {
        Expression::Equal(a, b) | Expression::SameTerm(a, b) => {
            let l = expr_to_id_sql(a, anchors)?;
            let r = expr_to_id_sql(b, anchors)?;
            Some(format!("({l} = {r})"))
        }
        Expression::And(a, b) => {
            let l = translate_filter(a, anchors)?;
            let r = translate_filter(b, anchors)?;
            Some(format!("({l} AND {r})"))
        }
        Expression::Or(a, b) => {
            let l = translate_filter(a, anchors)?;
            let r = translate_filter(b, anchors)?;
            Some(format!("({l} OR {r})"))
        }
        Expression::Not(inner) => {
            // SPARQL `!=` is `Not(Equal(a,b))` post-normalisation.
            let l = translate_filter(inner, anchors)?;
            Some(format!("(NOT ({l}))"))
        }
        Expression::Bound(v) => {
            // In a pure BGP context every variable is bound on every
            // row — `BOUND(?v)` is `TRUE` if ?v is bound in the BGP
            // and `FALSE` if it isn't projected/anchored at all.
            let name = v.as_str();
            Some(if anchors.contains_key(name) { "TRUE".into() } else { "FALSE".into() })
        }
        Expression::FunctionCall(func, args) => translate_function_call(func, args, anchors),
        _ => None,
    }
}

fn translate_function_call(
    func: &Function,
    args: &[Expression],
    anchors: &HashMap<String, (usize, &'static str)>,
) -> Option<String> {
    match func {
        Function::IsIri => term_type_check(args, anchors, term_type::URI),
        Function::IsBlank => term_type_check(args, anchors, term_type::BLANK_NODE),
        Function::IsLiteral => term_type_check(args, anchors, term_type::LITERAL),
        _ => None,
    }
}

/// `isIRI(?v)` / `isBlank(?v)` / `isLiteral(?v)` → emit a correlated
/// subselect on `_pgrdf_dictionary.term_type`. Only single-argument
/// variable form is supported.
fn term_type_check(
    args: &[Expression],
    anchors: &HashMap<String, (usize, &'static str)>,
    expected: i16,
) -> Option<String> {
    if args.len() != 1 {
        return None;
    }
    let var = match &args[0] {
        Expression::Variable(v) => v.as_str().to_string(),
        _ => return None,
    };
    let &(alias_idx, col) = anchors.get(&var)?;
    Some(format!(
        "((SELECT term_type FROM pgrdf._pgrdf_dictionary
            WHERE id = q{alias_idx}.{col}) = {expected})"
    ))
}

/// Resolve an Expression to a SQL fragment that evaluates to a
/// dictionary id (BIGINT). Variables → `qN.col`; constants → an
/// inlined integer literal (post dict-lookup, defaulting to `-1`
/// when the constant isn't in the dictionary so the predicate
/// reliably returns no rows rather than erroring).
fn expr_to_id_sql(
    e: &Expression,
    anchors: &HashMap<String, (usize, &'static str)>,
) -> Option<String> {
    match e {
        Expression::Variable(v) => {
            let &(alias_idx, col) = anchors.get(v.as_str())?;
            Some(format!("q{alias_idx}.{col}"))
        }
        Expression::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            Some(id.to_string())
        }
        Expression::Literal(l) => {
            let id = lookup_literal_id(l).unwrap_or(-1);
            Some(id.to_string())
        }
        _ => None,
    }
}

fn bind_subject(
    tp: &TriplePattern,
    qi: usize,
    anchors: &mut HashMap<String, (usize, &'static str)>,
    where_clauses: &mut Vec<String>,
) {
    match &tp.subject {
        TermPattern::Variable(v) => bind_var(v.as_str(), qi, "subject_id", anchors, where_clauses),
        TermPattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            where_clauses.push(format!("q{qi}.subject_id = {id}"));
        }
        TermPattern::BlankNode(_) => {
            panic!("sparql: blank-node subject in query not supported")
        }
        TermPattern::Literal(_) => panic!("sparql: literal subject is invalid in RDF"),
        other => panic!("sparql: unsupported subject term {other:?}"),
    }
}

fn bind_predicate(
    tp: &TriplePattern,
    qi: usize,
    anchors: &mut HashMap<String, (usize, &'static str)>,
    where_clauses: &mut Vec<String>,
) {
    match &tp.predicate {
        NamedNodePattern::Variable(v) => {
            bind_var(v.as_str(), qi, "predicate_id", anchors, where_clauses)
        }
        NamedNodePattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            where_clauses.push(format!("q{qi}.predicate_id = {id}"));
        }
    }
}

fn bind_object(
    tp: &TriplePattern,
    qi: usize,
    anchors: &mut HashMap<String, (usize, &'static str)>,
    where_clauses: &mut Vec<String>,
) {
    match &tp.object {
        TermPattern::Variable(v) => bind_var(v.as_str(), qi, "object_id", anchors, where_clauses),
        TermPattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            where_clauses.push(format!("q{qi}.object_id = {id}"));
        }
        TermPattern::Literal(l) => {
            let id = lookup_literal_id(l).unwrap_or(-1);
            where_clauses.push(format!("q{qi}.object_id = {id}"));
        }
        TermPattern::BlankNode(_) => {
            panic!("sparql: blank-node object in query not supported")
        }
        other => panic!("sparql: unsupported object term {other:?}"),
    }
}

/// Either record a variable's first occurrence as the anchor, or
/// emit an equality predicate tying this occurrence to that anchor.
fn bind_var(
    name: &str,
    qi: usize,
    col: &'static str,
    anchors: &mut HashMap<String, (usize, &'static str)>,
    where_clauses: &mut Vec<String>,
) {
    if let Some(&(prev_qi, prev_col)) = anchors.get(name) {
        where_clauses.push(format!("q{qi}.{col} = q{prev_qi}.{prev_col}"));
    } else {
        anchors.insert(name.to_string(), (qi, col));
    }
}

/// Postgres identifier quoting for variable names that happen to
/// collide with reserved words or contain characters that would
/// break parsing. Variables in SPARQL match `[A-Za-z_][\w]*` after
/// the leading `?`, which is identifier-safe, but we double-quote
/// anyway to keep the alias verbatim for JSONB key extraction.
fn quote_identifier(name: &str) -> String {
    let escaped = name.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

// ─────────────────────────────────────────────────────────────────────
// Dict ID lookup (constant terms)
// ─────────────────────────────────────────────────────────────────────

fn lookup_iri_id(iri: &str) -> Option<i64> {
    Spi::get_one_with_args(
        "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary
                  WHERE term_type = 1 AND lexical_value = $1 LIMIT 1)",
        &[iri.into()],
    )
    .ok()
    .flatten()
}

fn lookup_literal_id(lit: &Literal) -> Option<i64> {
    let value = lit.value();
    if let Some(lang) = lit.language() {
        Spi::get_one_with_args(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary
                      WHERE term_type = $1
                        AND lexical_value = $2
                        AND language_tag  = $3
                        AND datatype_iri_id IS NULL
                      LIMIT 1)",
            &[
                term_type::LITERAL.into(),
                value.into(),
                lang.into(),
            ],
        )
        .ok()
        .flatten()
    } else {
        let dt_id = lookup_iri_id(lit.datatype().as_str()).unwrap_or(-1);
        Spi::get_one_with_args(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary
                      WHERE term_type = $1
                        AND lexical_value = $2
                        AND datatype_iri_id = $3
                        AND language_tag IS NULL
                      LIMIT 1)",
            &[
                term_type::LITERAL.into(),
                value.into(),
                dt_id.into(),
            ],
        )
        .ok()
        .flatten()
    }
}

// ─────────────────────────────────────────────────────────────────────
// Execution
// ─────────────────────────────────────────────────────────────────────

fn execute(plan: &ExecPlan) -> Vec<pgrx::JsonB> {
    Spi::connect_mut(|client| {
        let table = client
            .update(plan.sql.as_str(), None, &[])
            .expect("sparql: dynamic SELECT failed");
        let mut rows: Vec<pgrx::JsonB> = Vec::new();
        for row in table {
            let mut obj = Map::new();
            for (i, var) in plan.projected.iter().enumerate() {
                let val: Option<String> = row.get::<String>(i + 1).ok().flatten();
                obj.insert(
                    var.clone(),
                    val.map(Value::String).unwrap_or(Value::Null),
                );
            }
            rows.push(pgrx::JsonB(Value::Object(obj)));
        }
        rows
    })
}

// ─────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// Load three triples then issue a basic 3-var SELECT — every
    /// triple should come back as a solution.
    #[pg_test]
    fn sparql_select_all_three_vars() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:b .
                 ex:a ex:p ex:c .
                 ex:a ex:q ex:b .',
                8_001)",
        )
        .unwrap();

        let n: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT
               FROM pgrdf.sparql('SELECT ?s ?p ?o WHERE { ?s ?p ?o }')",
        )
        .unwrap()
        .unwrap_or(0);
        // Each pgrx #[pg_test] runs in an auto-rollback transaction,
        // so the dictionary + quads only contain what we just loaded
        // — 3 triples.
        assert_eq!(n, 3, "expected 3 triples from the 3 we just loaded, got {n}");
    }

    /// Bound predicate → just the matching triples.
    #[pg_test]
    fn sparql_select_bound_predicate() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" .
                 ex:bob   foaf:name \"Bob\" .
                 ex:alice foaf:age 30 .',
                8_002)",
        )
        .unwrap();

        // 3 distinct subjects, 2 of which have foaf:name. The
        // sparql() UDF returns SETOF JSONB.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n WHERE { ?s foaf:name ?n }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);
    }

    /// IRI subject + variable predicate + variable object → the
    /// triples for that single subject come back.
    #[pg_test]
    fn sparql_select_bound_subject() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:k ex:p1 ex:v1 .
                 ex:k ex:p2 ex:v2 .
                 ex:other ex:p1 ex:v3 .',
                8_003)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?p ?o WHERE { <http://example.com/k> ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);
    }

    /// A query whose predicate hasn't been loaded should return zero
    /// rows, NOT error out. The translator inlines `-1` as the
    /// dict id which no row can match.
    #[pg_test]
    fn sparql_unknown_predicate_returns_zero_rows() {
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?o WHERE { ?s <http://nope.example/never-loaded> ?o }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 0);
    }

    /// Two-pattern BGP sharing ?p as subject: returns only subjects
    /// that have BOTH foaf:name AND foaf:mbox.
    #[pg_test]
    fn sparql_two_pattern_shared_subject() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" ; foaf:mbox <mailto:a@x> .
                 ex:bob   foaf:name \"Bob\"                            .
                 ex:carol foaf:name \"Carol\" ; foaf:mbox <mailto:c@x> .',
                8_004)",
        )
        .unwrap();

        // Alice + Carol have both predicates; Bob only has foaf:name.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?p ?n ?m
                  WHERE { ?p foaf:name ?n . ?p foaf:mbox ?m }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);
    }

    /// FILTER(?n = "Alice") restricts the foaf:name solution set to
    /// the single binding whose literal matches.
    #[pg_test]
    fn sparql_filter_literal_equality() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" .
                 ex:bob   foaf:name \"Bob\"   .
                 ex:carol foaf:name \"Carol\" .',
                8_010)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n WHERE { ?s foaf:name ?n FILTER(?n = \"Alice\") }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 1, "FILTER(?n = \"Alice\") should match one row, got {rows}");
    }

    /// FILTER(?o != "B") — the negation form.
    #[pg_test]
    fn sparql_filter_not_equal() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p \"A\" .
                 ex:b ex:p \"B\" .
                 ex:c ex:p \"C\" .',
                8_011)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?o WHERE { ?s <http://example.com/p> ?o FILTER(?o != \"B\") }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2, "expected 2 rows (A, C), got {rows}");
    }

    /// FILTER(isIRI(?o)) restricts to triples whose object is an IRI.
    #[pg_test]
    fn sparql_filter_is_iri() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:s1 ex:p ex:o1 .
                 ex:s2 ex:p \"literal2\" .
                 ex:s3 ex:p ex:o3 .',
                8_012)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?o WHERE { ?s ?p ?o FILTER(isIRI(?o)) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2, "expected 2 IRI-object rows, got {rows}");
    }

    /// FILTER(isIRI(?o) && ?p = foaf:name) — boolean composition
    /// combining a term-type test and a predicate-identity test.
    #[pg_test]
    fn sparql_filter_boolean_and() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:a foaf:name  \"Alice\"   .
                 ex:a foaf:knows ex:b        .
                 ex:b foaf:name  \"Bob\"     .
                 ex:b foaf:knows ex:c        .',
                8_013)",
        )
        .unwrap();

        // Only foaf:knows triples have an IRI object (foaf:name has a
        // literal object). The AND should retain just those 2 rows.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?p ?o
                  WHERE { ?s ?p ?o FILTER(isIRI(?o) && ?p = foaf:knows) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);
    }

    /// FILTER(?a = ?b) — two variables compared as dict ids. With
    /// a self-loop in the data we should see exactly one row come back.
    #[pg_test]
    fn sparql_filter_var_equals_var() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:x ex:p ex:x .
                 ex:x ex:p ex:y .
                 ex:z ex:p ex:y .',
                8_014)",
        )
        .unwrap();

        // ?s ?p ?o FILTER(?s = ?o) → only the self-loop row.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?p ?o WHERE { ?s ?p ?o FILTER(?s = ?o) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 1);
    }

    /// BOUND(?v) is trivially true for every BGP variable, so it
    /// should not change the row count.
    #[pg_test]
    fn sparql_filter_bound_is_trivially_true() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:b .
                 ex:a ex:p ex:c .',
                8_015)",
        )
        .unwrap();

        let baseline: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        let filtered: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s ?p ?o FILTER(BOUND(?o)) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(baseline, filtered, "BOUND(?o) should not change row count");
    }

    /// Three-pattern BGP exercises chained joins (a → b → c). Same
    /// FOAF setup but we also assert ?p's binding round-trips.
    #[pg_test]
    fn sparql_three_pattern_chain() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice  a foaf:Person ; foaf:name \"Alice\" ; foaf:knows ex:bob .
                 ex:bob    a foaf:Person ; foaf:name \"Bob\"   .',
                8_005)",
        )
        .unwrap();

        // ?a foaf:knows ?b . ?a foaf:name ?an . ?b foaf:name ?bn
        // -> 1 row (alice knows bob, alice has name Alice, bob has name Bob)
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?an ?bn
                  WHERE { ?a foaf:knows ?b .
                          ?a foaf:name  ?an .
                          ?b foaf:name  ?bn }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 1);
    }
}
