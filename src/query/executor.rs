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
//!   * FILTER expressions:
//!     - identity (`=`, `!=`, `sameTerm`) via dict-id comparison,
//!     - boolean composition (`&&`, `||`, `!`),
//!     - term-type predicates (`isIRI`, `isLiteral`, `isBlank`),
//!     - `BOUND` (trivially TRUE in a BGP context),
//!     - numeric ordering (`<`, `>`, `<=`, `>=`) — operand must
//!       resolve to a numeric XSD literal at the SPI layer, else
//!       the row is dropped (NULL comparison),
//!     - `IN (…)` set membership over dict ids,
//!     - `REGEX(?v, "pat", "flags")` against `_pgrdf_dictionary.lexical_value`
//!       (PCRE-style, Postgres `~`/`~*`). `STR(?v)` is a passthrough.
//!     Arithmetic, full string functions, `lang`/`datatype` are
//!     Phase 3 step 3+.
//!   * Constants in any position (subject IRI, predicate IRI,
//!     object IRI or literal).
//!   * Variables in any position.
//!   * `DISTINCT` / `REDUCED` → SELECT DISTINCT in the generated SQL.
//!   * `LIMIT` / `OFFSET` (from spargebra's Slice wrapper) → applied.
//!   * `ORDER BY ?var` / `ORDER BY ASC(?var)` / `ORDER BY DESC(?var)`
//!     → ordered lexicographically on the term's `lexical_value`.
//!     Complex ORDER BY expressions panic.
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
use spargebra::algebra::{Expression, Function, GraphPattern, OrderExpression};
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

#[derive(Default)]
struct ParsedSelect {
    projected: Vec<String>,
    bgp: Vec<TriplePattern>,
    filters: Vec<Expression>,
    distinct: bool,
    /// (variable_name, ascending). Each entry orders by the term's
    /// lexical_value in the given direction. Variables that aren't
    /// projected get an extra (hidden) SELECT-list column.
    order_by: Vec<(String, bool)>,
    limit: Option<usize>,
    offset: usize,
}

fn translate(q: &Query) -> ExecPlan {
    let pattern = match q {
        Query::Select { pattern, .. } => pattern,
        other => panic!("sparql: only SELECT supported in v0.2 (got {other:?})"),
    };
    let ps = parse_select(pattern);
    if ps.bgp.is_empty() {
        panic!("sparql: empty BGP");
    }
    let sql = build_bgp_sql(&ps);
    ExecPlan { projected: ps.projected, sql }
}

/// Walk the algebra wrappers around the SELECT's inner BGP,
/// recording projection, filters, DISTINCT, ORDER BY, LIMIT, OFFSET
/// as we go. The walk terminates at the innermost `Bgp { patterns }`.
fn parse_select(p: &GraphPattern) -> ParsedSelect {
    let mut ps = ParsedSelect::default();
    walk_select(p, &mut ps);
    if ps.projected.is_empty() {
        // SELECT * — collect variables in the order they appear in
        // the BGP. This branch fires when no `Project` wrapper sets
        // an explicit projection.
        for tp in &ps.bgp {
            push_unique(&mut ps.projected, tp_subject_var(tp));
            push_unique(&mut ps.projected, tp_predicate_var(tp));
            push_unique(&mut ps.projected, tp_object_var(tp));
        }
    }
    ps
}

fn walk_select(p: &GraphPattern, ps: &mut ParsedSelect) {
    match p {
        GraphPattern::Project { inner, variables } => {
            if ps.projected.is_empty() {
                ps.projected = variables.iter().map(|v| v.as_str().to_string()).collect();
            }
            walk_select(inner, ps);
        }
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => {
            // REDUCED is a "duplicates may or may not be removed"
            // hint. Implementing it as DISTINCT is a safe
            // over-approximation — the spec allows it.
            ps.distinct = true;
            walk_select(inner, ps);
        }
        GraphPattern::Slice { inner, start, length } => {
            // Slice is OFFSET (start) + LIMIT (length).
            ps.offset = *start;
            ps.limit = *length;
            walk_select(inner, ps);
        }
        GraphPattern::OrderBy { inner, expression } => {
            for oe in expression {
                let (expr, ascending) = match oe {
                    OrderExpression::Asc(e) => (e, true),
                    OrderExpression::Desc(e) => (e, false),
                };
                match expr {
                    Expression::Variable(v) => {
                        ps.order_by.push((v.as_str().to_string(), ascending));
                    }
                    other => panic!(
                        "sparql: ORDER BY supports only variable expressions today (got {other:?})"
                    ),
                }
            }
            walk_select(inner, ps);
        }
        GraphPattern::Filter { expr, inner } => {
            ps.filters.push(expr.clone());
            walk_select(inner, ps);
        }
        GraphPattern::Bgp { patterns } => {
            ps.bgp = patterns.clone();
        }
        other => panic!("sparql: unsupported algebra in select wrapper: {other:?}"),
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
/// become SQL WHERE predicates appended after the join clauses;
/// DISTINCT / ORDER BY / LIMIT / OFFSET land on the outer SELECT.
fn build_bgp_sql(ps: &ParsedSelect) -> String {
    /// First-occurrence anchor for each variable: which alias +
    /// which column. Subsequent occurrences emit equality predicates
    /// against this anchor.
    let mut anchors: HashMap<String, (usize, &'static str)> = HashMap::new();
    let mut where_clauses: Vec<String> = Vec::new();
    let mut from_aliases: Vec<String> = Vec::with_capacity(ps.bgp.len());

    for (i, tp) in ps.bgp.iter().enumerate() {
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
    for expr in &ps.filters {
        let sql = translate_filter(expr, &anchors).unwrap_or_else(|| {
            panic!("sparql: FILTER expression not translatable: {expr:?}")
        });
        where_clauses.push(sql);
    }

    // Project: each projected var → its anchor alias.column → dict
    // lookup → aliased to the variable name (so SETOF JSONB emission
    // pulls the right column by ordinal). Hidden trailing columns are
    // appended for ORDER BY variables that aren't in the projection.
    let mut select_clauses: Vec<String> = Vec::new();
    for var in &ps.projected {
        let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
            panic!("sparql: projected variable ?{var} is not bound in any BGP pattern")
        });
        select_clauses.push(format!(
            "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary
               WHERE id = q{alias_idx}.{col}) AS {alias_v}",
            alias_v = quote_identifier(var),
        ));
    }

    // ORDER BY: each entry maps to an ordinal SELECT-list position.
    // For projected vars: reuse the existing column position. For
    // unprojected vars: append a hidden column and reference it.
    let mut order_clauses: Vec<String> = Vec::new();
    for (idx, (var, ascending)) in ps.order_by.iter().enumerate() {
        let dir = if *ascending { "ASC" } else { "DESC" };
        // NULLS LAST makes ASC + missing-dict-entry behave intuitively
        // (the row with the smallest lexical_value comes first, missing
        // entries sink to the bottom regardless of direction).
        let position = if let Some(pos) = ps.projected.iter().position(|p| p == var) {
            pos + 1
        } else {
            if ps.distinct {
                panic!(
                    "sparql: ORDER BY ?{var} requires ?{var} to be in the SELECT \
                     clause when DISTINCT is used"
                );
            }
            let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
                panic!("sparql: ORDER BY variable ?{var} not bound in any BGP pattern")
            });
            select_clauses.push(format!(
                "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary
                   WHERE id = q{alias_idx}.{col}) AS _pgrdf_order_{idx}",
            ));
            select_clauses.len()
        };
        order_clauses.push(format!("{position} {dir} NULLS LAST"));
    }

    let distinct_kw = if ps.distinct { "DISTINCT " } else { "" };
    let mut sql = format!(
        "SELECT {distinct_kw}{sel} FROM {from}",
        sel = select_clauses.join(", "),
        from = from_aliases.join(", "),
    );
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    if !order_clauses.is_empty() {
        sql.push_str(" ORDER BY ");
        sql.push_str(&order_clauses.join(", "));
    }
    if let Some(limit) = ps.limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    if ps.offset > 0 {
        sql.push_str(&format!(" OFFSET {}", ps.offset));
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
        Expression::Greater(a, b) => translate_numeric_cmp(a, b, anchors, ">"),
        Expression::GreaterOrEqual(a, b) => translate_numeric_cmp(a, b, anchors, ">="),
        Expression::Less(a, b) => translate_numeric_cmp(a, b, anchors, "<"),
        Expression::LessOrEqual(a, b) => translate_numeric_cmp(a, b, anchors, "<="),
        Expression::In(operand, list) => translate_in(operand, list, anchors),
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
        Function::Regex => translate_regex(args, anchors),
        // STR / LANG / DATATYPE etc. aren't callable as top-level
        // FILTER predicates — they're string-yielding functions used
        // inside other comparisons. Handled by the lexical-extraction
        // path (expr_to_lexical_sql) when nested.
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

/// `?a IN (x, y, z)` — emit `dict_id IN (id_x, id_y, id_z)`. Returns
/// None if any list element doesn't have a sound dict-id translation.
fn translate_in(
    operand: &Expression,
    list: &[Expression],
    anchors: &HashMap<String, (usize, &'static str)>,
) -> Option<String> {
    let lhs = expr_to_id_sql(operand, anchors)?;
    if list.is_empty() {
        return Some("FALSE".to_string());
    }
    let ids: Vec<String> = list
        .iter()
        .map(|e| expr_to_id_sql(e, anchors))
        .collect::<Option<Vec<_>>>()?;
    Some(format!("({lhs} IN ({}))", ids.join(",")))
}

/// Resolve an Expression to a SQL fragment that evaluates to a
/// numeric value (Postgres NUMERIC). Used by `<`/`>`/`<=`/`>=` so
/// the comparison is type-safe.
///
/// For variables: subselect on `_pgrdf_dictionary` that returns the
/// cast lexical_value only when the row's datatype is one of the
/// known XSD numeric types. Non-numeric rows yield NULL, so the
/// comparison is NULL and the row is dropped — matching SPARQL's
/// "type error → unbound" semantics.
///
/// For constants: must be a numeric XSD literal at translation time,
/// else `None` (translator panics — the comparison is meaningless).
fn expr_to_numeric_sql(
    e: &Expression,
    anchors: &HashMap<String, (usize, &'static str)>,
) -> Option<String> {
    match e {
        Expression::Variable(v) => {
            let &(alias_idx, col) = anchors.get(v.as_str())?;
            let dt_ids = numeric_datatype_id_list();
            Some(format!(
                "(SELECT CASE WHEN datatype_iri_id IN ({dt_ids})
                              THEN lexical_value::numeric
                              ELSE NULL
                         END
                  FROM pgrdf._pgrdf_dictionary WHERE id = q{alias_idx}.{col})"
            ))
        }
        Expression::Literal(l) => {
            if !is_xsd_numeric_iri(l.datatype().as_str()) {
                return None;
            }
            // Constant: emit the numeric value directly. Reject anything
            // not actually parseable as a number (defensive — spargebra
            // should have caught this).
            let v = l.value();
            if v.parse::<f64>().is_err() {
                return None;
            }
            Some(format!("{v}::numeric"))
        }
        _ => None,
    }
}

fn translate_numeric_cmp(
    a: &Expression,
    b: &Expression,
    anchors: &HashMap<String, (usize, &'static str)>,
    op: &str,
) -> Option<String> {
    let l = expr_to_numeric_sql(a, anchors)?;
    let r = expr_to_numeric_sql(b, anchors)?;
    Some(format!("({l} {op} {r})"))
}

/// `REGEX(?v, "pattern" [, "flags"])` — Postgres regex match on the
/// term's lexical form. `i` flag → case-insensitive (`~*` operator),
/// no flag → case-sensitive (`~`). Other flags are accepted but not
/// translated (Postgres POSIX regex doesn't have direct PCRE-flag
/// parity); we still match without them rather than erroring.
fn translate_regex(
    args: &[Expression],
    anchors: &HashMap<String, (usize, &'static str)>,
) -> Option<String> {
    if !(2..=3).contains(&args.len()) {
        return None;
    }
    let lex = expr_to_lexical_sql(&args[0], anchors)?;
    let pattern = match &args[1] {
        Expression::Literal(l) => l.value().to_string(),
        _ => return None,
    };
    let flags = match args.get(2) {
        Some(Expression::Literal(l)) => l.value().to_string(),
        Some(_) => return None,
        None => String::new(),
    };
    let op = if flags.contains('i') { "~*" } else { "~" };
    let escaped = pattern.replace('\'', "''");
    Some(format!("({lex} {op} '{escaped}')"))
}

/// Resolve an Expression to a SQL fragment that evaluates to the
/// term's lexical form (TEXT). Used by `REGEX` and similar string
/// functions.
fn expr_to_lexical_sql(
    e: &Expression,
    anchors: &HashMap<String, (usize, &'static str)>,
) -> Option<String> {
    match e {
        Expression::Variable(v) => {
            let &(alias_idx, col) = anchors.get(v.as_str())?;
            Some(format!(
                "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary
                   WHERE id = q{alias_idx}.{col})"
            ))
        }
        Expression::Literal(l) => {
            let escaped = l.value().replace('\'', "''");
            Some(format!("'{escaped}'"))
        }
        // STR(?v) is identity for our purposes — every dict entry's
        // lexical_value IS the string form. Pass through.
        Expression::FunctionCall(Function::Str, inner) if inner.len() == 1 => {
            expr_to_lexical_sql(&inner[0], anchors)
        }
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────
// XSD numeric datatype IDs
// ─────────────────────────────────────────────────────────────────────

/// The full set of XSD numeric IRIs we recognise for type-safe
/// ordering comparisons. Anything outside this set is treated as
/// non-numeric — the row is dropped (NULL comparison) rather than
/// the Postgres cast erroring out.
fn xsd_numeric_iris() -> &'static [&'static str] {
    &[
        "http://www.w3.org/2001/XMLSchema#integer",
        "http://www.w3.org/2001/XMLSchema#decimal",
        "http://www.w3.org/2001/XMLSchema#double",
        "http://www.w3.org/2001/XMLSchema#float",
        "http://www.w3.org/2001/XMLSchema#long",
        "http://www.w3.org/2001/XMLSchema#int",
        "http://www.w3.org/2001/XMLSchema#short",
        "http://www.w3.org/2001/XMLSchema#byte",
        "http://www.w3.org/2001/XMLSchema#unsignedLong",
        "http://www.w3.org/2001/XMLSchema#unsignedInt",
        "http://www.w3.org/2001/XMLSchema#unsignedShort",
        "http://www.w3.org/2001/XMLSchema#unsignedByte",
        "http://www.w3.org/2001/XMLSchema#nonPositiveInteger",
        "http://www.w3.org/2001/XMLSchema#nonNegativeInteger",
        "http://www.w3.org/2001/XMLSchema#positiveInteger",
        "http://www.w3.org/2001/XMLSchema#negativeInteger",
    ]
}

fn is_xsd_numeric_iri(iri: &str) -> bool {
    xsd_numeric_iris().contains(&iri)
}

/// Comma-separated SQL list of dict ids for every XSD numeric IRI
/// currently in the dictionary. If none are present, returns `-1`
/// so the IN(...) check trivially matches nothing rather than
/// producing invalid SQL.
fn numeric_datatype_id_list() -> String {
    let ids: Vec<i64> = xsd_numeric_iris()
        .iter()
        .filter_map(|iri| lookup_iri_id(iri))
        .collect();
    if ids.is_empty() {
        "-1".to_string()
    } else {
        ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
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

    /// FILTER(?age > 30) — numeric ordering. Spargebra parses bare
    /// numbers as xsd:integer; we cast lexical_value to NUMERIC when
    /// the dict row's datatype is one of the XSD numeric IRIs.
    #[pg_test]
    fn sparql_filter_numeric_gt() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
                 ex:a ex:age 25 .
                 ex:b ex:age 35 .
                 ex:c ex:age 45 .
                 ex:d ex:age 55 .',
                8_020)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/age> ?age FILTER(?age > 30) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 3, "expected b,c,d (ages > 30), got {rows}");
    }

    /// FILTER(?age >= 35 && ?age <= 45) — composed numeric range.
    #[pg_test]
    fn sparql_filter_numeric_range() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:age 25 .
                 ex:b ex:age 35 .
                 ex:c ex:age 45 .
                 ex:d ex:age 55 .',
                8_021)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/age> ?age FILTER(?age >= 35 && ?age <= 45) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2, "expected b,c (35..=45), got {rows}");
    }

    /// FILTER(?age > 30) with a NON-numeric value — the row's
    /// lexical_value would fail Postgres `::numeric`, so the CASE
    /// drops it to NULL → row excluded.
    #[pg_test]
    fn sparql_filter_numeric_skips_non_numeric() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:val 99 .
                 ex:b ex:val \"hello\" .',
                8_022)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/val> ?v FILTER(?v > 30) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        // Only ex:a (99) qualifies; ex:b is a string literal, NULL on cast.
        assert_eq!(rows, 1);
    }

    /// FILTER(REGEX(?n, "^A")) — case-sensitive regex.
    #[pg_test]
    fn sparql_filter_regex_case_sensitive() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" .
                 ex:adam  foaf:name \"Adam\"  .
                 ex:bob   foaf:name \"Bob\"   .
                 ex:carol foaf:name \"Carol\" .',
                8_023)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s WHERE { ?s foaf:name ?n FILTER(REGEX(?n, \"^A\")) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2, "expected Alice+Adam (start with A), got {rows}");
    }

    /// FILTER(REGEX(STR(?n), "ar", "i")) — STR() passthrough + the
    /// case-insensitive flag → matches Carol AND ar in any case.
    #[pg_test]
    fn sparql_filter_regex_case_insensitive_with_str() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:carol foaf:name \"Carol\" .
                 ex:mark  foaf:name \"Mark\"  .
                 ex:alice foaf:name \"Alice\" .',
                8_024)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s WHERE { ?s foaf:name ?n FILTER(REGEX(STR(?n), \"ar\", \"i\")) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2, "expected Carol+Mark (contain ar/Ar), got {rows}");
    }

    /// FILTER(?s IN (?a, ?b, ?c)) — set membership via dict ids.
    #[pg_test]
    fn sparql_filter_in_set() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p 1 .
                 ex:b ex:p 2 .
                 ex:c ex:p 3 .
                 ex:d ex:p 4 .',
                8_025)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/p> ?v FILTER(?s IN (<http://example.com/a>, <http://example.com/c>)) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);
    }

    /// SELECT DISTINCT — deduplication on the projected variables.
    #[pg_test]
    fn sparql_distinct_dedups() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:x .
                 ex:b ex:p ex:x .
                 ex:c ex:p ex:y .',
                8_030)",
        )
        .unwrap();

        let baseline: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?o WHERE { ?s ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        let with_distinct: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT DISTINCT ?o WHERE { ?s ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(baseline, 3, "expected 3 raw rows, got {baseline}");
        assert_eq!(with_distinct, 2, "expected 2 distinct ?o (x, y), got {with_distinct}");
    }

    /// LIMIT caps the number of rows returned.
    #[pg_test]
    fn sparql_limit_caps_rows() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:p 3 . ex:d ex:p 4 . ex:e ex:p 5 .',
                8_031)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?o WHERE { ?s ?p ?o } LIMIT 3'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 3);
    }

    /// OFFSET skips rows from the start.
    #[pg_test]
    fn sparql_offset_skips_rows() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:p 3 . ex:d ex:p 4 . ex:e ex:p 5 .',
                8_032)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?o WHERE { ?s ?p ?o } ORDER BY ?s OFFSET 2 LIMIT 100'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 3, "5 rows minus offset 2 = 3");
    }

    /// ORDER BY ASC ?n — first result has the alphabetically-first name.
    #[pg_test]
    fn sparql_order_by_asc() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:carol foaf:name \"Carol\" .
                 ex:alice foaf:name \"Alice\" .
                 ex:bob   foaf:name \"Bob\"   .',
                8_033)",
        )
        .unwrap();

        let first: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?n WHERE { ?s foaf:name ?n } ORDER BY ?n LIMIT 1'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(first.0["n"], "Alice");
    }

    /// ORDER BY DESC ?n — first result has the alphabetically-last name.
    #[pg_test]
    fn sparql_order_by_desc() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:carol foaf:name \"Carol\" .
                 ex:alice foaf:name \"Alice\" .
                 ex:bob   foaf:name \"Bob\"   .',
                8_034)",
        )
        .unwrap();

        let first: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?n WHERE { ?s foaf:name ?n } ORDER BY DESC(?n) LIMIT 1'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(first.0["n"], "Carol");
    }

    /// DISTINCT + ORDER BY on a projected variable works.
    #[pg_test]
    fn sparql_distinct_with_order_by() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p \"x\" . ex:b ex:p \"x\" . ex:c ex:p \"y\" . ex:d ex:p \"z\" .',
                8_035)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT DISTINCT ?o WHERE { ?s ?p ?o } ORDER BY ?o'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 3, "x, y, z = 3 distinct");
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
