//! BGP → SQL translator.
//!
//! Phase 2.2 step 5: translate a SPARQL SELECT with a single Basic
//! Graph Pattern into a dynamically-generated SQL SELECT over
//! `_pgrdf_quads` joined to `_pgrdf_dictionary`, then return one
//! JSONB row per match.
//!
//! Scope today (intentionally narrow so the translator can be
//! validated empirically before scaling up):
//!   * SELECT only (no CONSTRUCT/ASK/DESCRIBE).
//!   * Exactly one BGP triple — multi-pattern joins arrive in step 6.
//!   * Constants in any position (subject IRI, predicate IRI,
//!     object IRI or literal).
//!   * Variables in any position.
//!   * Distinct / Reduced / Slice / OrderBy wrappers are walked
//!     through without affecting translation (they don't change
//!     the BGP shape).
//!
//! Output shape:
//!   `SETOF JSONB` where each row is `{"varname": "lexical_value", ...}`
//!   keyed by the projected variable names.

use crate::storage::dict::term_type;
use pgrx::iter::SetOfIterator;
use pgrx::prelude::*;
use serde_json::{Map, Value};
use spargebra::algebra::GraphPattern;
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
    let (projected, bgp) = unwrap_select(pattern);
    if bgp.is_empty() {
        panic!("sparql: empty BGP");
    }
    if bgp.len() > 1 {
        panic!(
            "sparql: multi-pattern BGP not yet supported in step 5 (got {} patterns)",
            bgp.len()
        );
    }
    let sql = build_single_pattern_sql(&bgp[0], &projected);
    ExecPlan { projected, sql }
}

/// Walk through wrappers we can pass through transparently and
/// surface the projection + BGP shape underneath.
fn unwrap_select(p: &GraphPattern) -> (Vec<String>, Vec<TriplePattern>) {
    match p {
        GraphPattern::Project { inner, variables } => {
            let vars: Vec<String> = variables.iter().map(|v| v.as_str().to_string()).collect();
            (vars, extract_bgp(inner))
        }
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => unwrap_select(inner),
        GraphPattern::Slice { inner, .. } | GraphPattern::OrderBy { inner, .. } => {
            unwrap_select(inner)
        }
        GraphPattern::Bgp { patterns } => {
            // SELECT * — collect variables in the order they appear.
            let mut vars: Vec<String> = Vec::new();
            for tp in patterns {
                push_unique(&mut vars, tp_subject_var(tp));
                push_unique(&mut vars, tp_predicate_var(tp));
                push_unique(&mut vars, tp_object_var(tp));
            }
            (vars, patterns.clone())
        }
        other => panic!("sparql: unsupported algebra in select wrapper: {other:?}"),
    }
}

fn extract_bgp(p: &GraphPattern) -> Vec<TriplePattern> {
    match p {
        GraphPattern::Bgp { patterns } => patterns.clone(),
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => extract_bgp(inner),
        GraphPattern::Slice { inner, .. } | GraphPattern::OrderBy { inner, .. } => {
            extract_bgp(inner)
        }
        other => panic!("sparql: expected BGP, got {other:?}"),
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

fn build_single_pattern_sql(tp: &TriplePattern, projected: &[String]) -> String {
    // Resolve every constant position (IRI or literal) to a dict id
    // *now*, so the dynamic SQL only carries integer constants.
    let mut var_to_col: HashMap<String, &'static str> = HashMap::new();
    let mut where_clauses: Vec<String> = Vec::new();

    match &tp.subject {
        TermPattern::Variable(v) => {
            var_to_col.insert(v.as_str().to_string(), "subject_id");
        }
        TermPattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            where_clauses.push(format!("q.subject_id = {id}"));
        }
        TermPattern::BlankNode(_) => {
            panic!("sparql: blank-node subject in query not supported")
        }
        TermPattern::Literal(_) => panic!("sparql: literal subject is invalid in RDF"),
        other => panic!("sparql: unsupported subject term {other:?}"),
    }

    match &tp.predicate {
        NamedNodePattern::Variable(v) => {
            var_to_col.insert(v.as_str().to_string(), "predicate_id");
        }
        NamedNodePattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            where_clauses.push(format!("q.predicate_id = {id}"));
        }
    }

    match &tp.object {
        TermPattern::Variable(v) => {
            var_to_col.insert(v.as_str().to_string(), "object_id");
        }
        TermPattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            where_clauses.push(format!("q.object_id = {id}"));
        }
        TermPattern::Literal(l) => {
            let id = lookup_literal_id(l).unwrap_or(-1);
            where_clauses.push(format!("q.object_id = {id}"));
        }
        TermPattern::BlankNode(_) => {
            panic!("sparql: blank-node object in query not supported")
        }
        other => panic!("sparql: unsupported object term {other:?}"),
    }

    // Project: each var → its quad column → joined to the dictionary
    // for the lexical value. Aliased to the variable name so JSONB
    // emission can pull by ordinal.
    let mut select_clauses: Vec<String> = Vec::new();
    for var in projected {
        let col = var_to_col.get(var).unwrap_or_else(|| {
            panic!("sparql: projected variable ?{var} is not bound in the BGP")
        });
        // Sub-select on dictionary so a non-existent term lands as NULL.
        select_clauses.push(format!(
            "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary WHERE id = q.{col}) AS {alias}",
            col = col,
            alias = quote_identifier(var),
        ));
    }

    let mut sql = format!(
        "SELECT {sel} FROM pgrdf._pgrdf_quads q",
        sel = select_clauses.join(", "),
    );
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql
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
}
