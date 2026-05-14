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
//!   * OPTIONAL `{ ?s :p ?o }` — translated to `LEFT JOIN
//!     pgrdf._pgrdf_quads qOPT_i ON …`. Restriction: each OPTIONAL
//!     block must be a single triple pattern. Per-block FILTER
//!     (the SPARQL `OPTIONAL { … FILTER(...) }` form) lands in the
//!     ON clause, so the row survives with the optional vars NULL
//!     when the filter rejects.
//!   * `UNION` — each branch becomes its own sub-SELECT with its
//!     own BGP / filters / optionals; the union of all
//!     branch-bound variables is the projection, with unbound vars
//!     emitted as `NULL::TEXT`. Branches are combined with
//!     `UNION ALL`; outer `DISTINCT` / `ORDER BY` / `LIMIT` /
//!     `OFFSET` wrap the union via a derived table.
//!   * `MINUS { ?s :p ?o }` — translated to `WHERE NOT EXISTS
//!     (SELECT 1 FROM pgrdf._pgrdf_quads qMIN_K WHERE …)` keyed
//!     on shared variables. Per spec, MINUS with no shared variables
//!     is a no-op and is elided. Restriction: single-triple right
//!     side (same as OPTIONAL today).
//!   * Aggregates — `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT ?v)`,
//!     `SUM(?v)`, `AVG(?v)`, `MIN(?v)`, `MAX(?v)`, with or without
//!     `GROUP BY ?vars`. SUM/AVG are numeric-aware (non-numeric
//!     literals contribute NULL); MIN/MAX are lexicographic on the
//!     term's `lexical_value`. Aggregate output values come back as
//!     strings in the JSONB row (consistent with the rest of the
//!     surface). HAVING and `GROUP_CONCAT` / `SAMPLE` are Phase 3
//!     backlog.
//!   * Paths / VALUES / BIND / SERVICE remain unsupported.
//!
//! Output shape:
//!   `SETOF JSONB` — one row per solution, keys = projected variable
//!   names, values = lexical strings (NULL when the binding maps to
//!   a term id missing from the dictionary).

use crate::query::plan_cache;
use crate::storage::dict::term_type;
use pgrx::datum::DatumWithOid;
use pgrx::iter::SetOfIterator;
use pgrx::pg_sys::{Oid, PgBuiltInOids};
use pgrx::prelude::*;
use serde_json::{Map, Value};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
};
use spargebra::term::{Literal, NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};
use std::cell::RefCell;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────
// Parameter buffer
//
// Phase 3 step 2 (LLD §4.2): every dict-id constant that used to be
// inlined into the SQL string now becomes a `$N` placeholder. The
// resolved i64 lands here in declaration order; `translate()` clears
// the buffer before each top-level walk and snapshots it into the
// ExecPlan once the walk finishes. Single-threaded per backend so a
// thread_local is sufficient and avoids signature churn through the
// ~3 500 lines of translator.
// ─────────────────────────────────────────────────────────────────────

thread_local! {
    static PARAM_BUF: RefCell<Vec<i64>> = const { RefCell::new(Vec::new()) };
}

fn params_clear() {
    PARAM_BUF.with(|b| b.borrow_mut().clear());
}

fn params_take() -> Vec<i64> {
    PARAM_BUF.with(|b| std::mem::take(&mut *b.borrow_mut()))
}

/// Append `id` to the param buffer and return its `$N` placeholder
/// (1-based, matching Postgres positional-parameter syntax). The
/// translator uses this everywhere a resolved dict id would have
/// previously been baked into the SQL string.
fn id_placeholder(id: i64) -> String {
    PARAM_BUF.with(|b| {
        let mut v = b.borrow_mut();
        v.push(id);
        format!("${}", v.len())
    })
}

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
    /// Parameterised SQL string. Every dict id constant is rendered
    /// as a positional placeholder (`$1`, `$2`, …); the translator
    /// never inlines user-supplied IRI strings directly.
    sql: String,
    /// Resolved dict ids in `$1`-onwards order. Bound at execute
    /// time as `INT8` Datums alongside the cached prepared plan.
    params: Vec<i64>,
}

#[derive(Default)]
struct ParsedSelect {
    projected: Vec<String>,
    // Single-branch state. Empty when union_branches is populated.
    bgp: Vec<TriplePattern>,
    filters: Vec<Expression>,
    /// Each OPTIONAL block — a single triple pattern plus its
    /// optional FILTER. Multiple chained OPTIONALs land here in
    /// left-to-right order; build_bgp_sql emits one LEFT JOIN per.
    optionals: Vec<OptionalBlock>,
    /// Each MINUS block — a list of triple patterns. Translates
    /// to `WHERE NOT EXISTS (SELECT 1 FROM q_min_1, q_min_2, …
    /// WHERE join-shared-vars AND inner-pattern-predicates)`.
    /// Elided if there are no shared vars (SPARQL no-op).
    minuses: Vec<Vec<TriplePattern>>,
    /// GROUP BY variables. Empty when the query has aggregates but
    /// no GROUP BY (the entire result is a single aggregate row),
    /// or when there are no aggregates at all.
    group_vars: Vec<String>,
    /// Aggregate output specs. `output_var` is the SPARQL variable
    /// that holds the value (after the `Extend` rename); each entry
    /// becomes a column in the generated SELECT clause.
    aggregates: Vec<AggregateSpec>,
    /// FILTER expressions that reference aggregate output vars —
    /// these become a SQL `HAVING` clause rather than `WHERE`.
    /// Populated by `parse_select` after the walk completes, by
    /// migrating any filter that names an aggregate alias out of
    /// `ps.filters`.
    having_filters: Vec<Expression>,
    /// BIND specs — `BIND(expr AS ?var)` (or the `(EXPR AS ?var)`
    /// form in SELECT-DISTINCT-aside cases). Each entry becomes an
    /// extra SELECT-list column whose value is the translated
    /// expression. Filtering on a BIND output isn't supported yet.
    binds: Vec<BindSpec>,
    /// When non-empty, this is a UNION query — each entry is a
    /// self-contained branch with its own BGP / filters / optionals.
    /// Single-branch fields (`bgp`, `filters`, `optionals`) remain
    /// empty in that case; build_bgp_sql dispatches on which is set.
    union_branches: Vec<UnionBranch>,
    distinct: bool,
    /// (variable_name, ascending). Each entry orders by the term's
    /// lexical_value in the given direction. Variables that aren't
    /// projected get an extra (hidden) SELECT-list column.
    order_by: Vec<(String, bool)>,
    limit: Option<usize>,
    offset: usize,
}

struct OptionalBlock {
    triple: TriplePattern,
    /// The filter inside `OPTIONAL { … FILTER(...) }`, if any.
    /// Translated into the LEFT JOIN's ON clause so rejected rows
    /// still survive with the optional variables NULL.
    filter: Option<Expression>,
}

struct BindSpec {
    output_var: String,
    expression: Expression,
}

struct AggregateSpec {
    /// The user-facing SPARQL variable that holds this aggregate's
    /// output (post-Extend rename). Empty SetCompare = `$agg_N`
    /// from the algebra synthesis layer until Extend renames it.
    output_var: String,
    func: AggregateFn,
    distinct: bool,
    /// The aggregate's argument variable. `None` only for `COUNT(*)`.
    arg_var: Option<String>,
}

#[derive(Clone)]
enum AggregateFn {
    Count,
    Sum,
    Avg,
    Min,
    Max,
    /// GROUP_CONCAT(?v [; SEPARATOR = "…"]) — Postgres `string_agg`.
    /// Separator defaults to a single space per SPARQL spec.
    GroupConcat { separator: String },
    /// SAMPLE(?v) — "any value from the group". Postgres has no
    /// SAMPLE; we use `MIN(...)` as a deterministic surrogate which
    /// is spec-conformant ("an implementation-defined element").
    Sample,
}

#[derive(Default)]
struct UnionBranch {
    bgp: Vec<TriplePattern>,
    filters: Vec<Expression>,
    optionals: Vec<OptionalBlock>,
    minuses: Vec<Vec<TriplePattern>>,
}

fn translate(q: &Query) -> ExecPlan {
    // Clear before each top-level translation so a previous panic
    // mid-walk can't leak a stale partial-parameter list into this
    // call. Buffer fills as the walk emits `$N` placeholders.
    params_clear();
    match q {
        Query::Select { pattern, .. } => {
            let ps = parse_select(pattern);
            if ps.bgp.is_empty() && ps.union_branches.is_empty() {
                panic!("sparql: empty BGP");
            }
            let sql = build_bgp_sql(&ps);
            ExecPlan {
                projected: ps.projected,
                sql,
                params: params_take(),
            }
        }
        Query::Ask { pattern, .. } => {
            // ASK reuses the SELECT pattern walk but only cares
            // whether the resulting solution sequence is non-empty.
            let mut ps = parse_select(pattern);
            if ps.bgp.is_empty() && ps.union_branches.is_empty() {
                panic!("sparql: ASK with empty BGP");
            }
            // Force a stable single-row projection so build_*_sql
            // doesn't fail looking for projected vars in anchors —
            // we'll discard the inner SELECT below.
            ps.projected = Vec::new();
            ps.distinct = false;
            ps.order_by.clear();
            ps.limit = Some(1);
            ps.offset = 0;
            // Build a probe SELECT that yields any row when the
            // pattern matches; wrap it in EXISTS and cast to text.
            let probe = build_ask_probe_sql(&ps);
            let sql = format!(
                "SELECT CASE WHEN EXISTS ({probe}) THEN 'true' ELSE 'false' END AS \"_ask\""
            );
            ExecPlan {
                projected: vec!["_ask".to_string()],
                sql,
                params: params_take(),
            }
        }
        other => panic!("sparql: query form not supported yet (got {other:?})"),
    }
}

/// Build a probe SELECT for ASK — same machinery as
/// build_single_branch_outer / build_union_sql but with an empty
/// SELECT clause (`SELECT 1`) so the SQL is well-formed before
/// being wrapped in EXISTS().
fn build_ask_probe_sql(ps: &ParsedSelect) -> String {
    if !ps.union_branches.is_empty() {
        let branch_sqls: Vec<String> = ps
            .union_branches
            .iter()
            .map(|b| {
                let mut anchors: HashMap<String, (usize, &'static str)> = HashMap::new();
                let (from_sql, where_clauses) = build_from_and_where(
                    &b.bgp, &b.filters, &b.optionals, &b.minuses, &mut anchors, 0,
                );
                let mut sql = format!("SELECT 1 FROM {from_sql}");
                if !where_clauses.is_empty() {
                    sql.push_str(" WHERE ");
                    sql.push_str(&where_clauses.join(" AND "));
                }
                sql
            })
            .collect();
        return branch_sqls.join(" UNION ALL ");
    }
    let mut anchors: HashMap<String, (usize, &'static str)> = HashMap::new();
    let (from_sql, where_clauses) = build_from_and_where(
        &ps.bgp,
        &ps.filters,
        &ps.optionals,
        &ps.minuses,
        &mut anchors,
        0,
    );
    let mut sql = format!("SELECT 1 FROM {from_sql}");
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql.push_str(" LIMIT 1");
    sql
}

/// Walk the algebra wrappers around the SELECT's inner BGP,
/// recording projection, filters, DISTINCT, ORDER BY, LIMIT, OFFSET
/// as we go. The walk terminates at the innermost `Bgp { patterns }`.
fn parse_select(p: &GraphPattern) -> ParsedSelect {
    let mut ps = ParsedSelect::default();
    walk_select(p, &mut ps);
    // For aggregate queries, any filter that names an aggregate
    // output variable is HAVING (the outer SQL ran the GROUP BY
    // by the time it evaluates the predicate). Split now so
    // build_aggregate_sql can route each filter correctly.
    if !ps.aggregates.is_empty() {
        let agg_names: Vec<String> = ps.aggregates.iter().map(|a| a.output_var.clone()).collect();
        let (having, where_): (Vec<_>, Vec<_>) = std::mem::take(&mut ps.filters)
            .into_iter()
            .partition(|f| expression_references_any(f, &agg_names));
        ps.filters = where_;
        ps.having_filters = having;
    }
    if ps.projected.is_empty() {
        // SELECT * — collect variables in the order they appear in
        // the BGP (or across all UNION branches). This branch fires
        // when no `Project` wrapper sets an explicit projection.
        if !ps.union_branches.is_empty() {
            for branch in &ps.union_branches {
                for tp in &branch.bgp {
                    push_unique(&mut ps.projected, tp_subject_var(tp));
                    push_unique(&mut ps.projected, tp_predicate_var(tp));
                    push_unique(&mut ps.projected, tp_object_var(tp));
                }
                for opt in &branch.optionals {
                    push_unique(&mut ps.projected, tp_subject_var(&opt.triple));
                    push_unique(&mut ps.projected, tp_predicate_var(&opt.triple));
                    push_unique(&mut ps.projected, tp_object_var(&opt.triple));
                }
            }
        } else {
            for tp in &ps.bgp {
                push_unique(&mut ps.projected, tp_subject_var(tp));
                push_unique(&mut ps.projected, tp_predicate_var(tp));
                push_unique(&mut ps.projected, tp_object_var(tp));
            }
            for opt in &ps.optionals {
                push_unique(&mut ps.projected, tp_subject_var(&opt.triple));
                push_unique(&mut ps.projected, tp_predicate_var(&opt.triple));
                push_unique(&mut ps.projected, tp_object_var(&opt.triple));
            }
        }
    }
    ps
}

/// Translate a BIND expression into a SQL fragment that yields a
/// TEXT value (consistent with all other projected columns; the
/// JSONB row emits each cell as a string).
///
/// Supported today:
///   * Literal / NamedNode / Variable (text form).
///   * STR / LANG / DATATYPE / UCASE / LCASE / STRLEN-as-text.
///   * Arithmetic, cast to text.
///   * CONCAT(?a, ?b, …) → Postgres `concat`.
/// Anything else returns None (translator panics with a clear
/// "BIND expression not translatable" message).
fn translate_bind_expression(
    e: &Expression,
    anchors: &HashMap<String, (usize, &'static str)>,
) -> Option<String> {
    // Try text-valued first (literal, NamedNode, STR/LANG/DATATYPE/
    // UCASE/LCASE/STRLEN-derived, plain variable).
    if let Some(s) = expr_to_lexical_sql(e, anchors) {
        return Some(s);
    }
    // Otherwise try numeric — wrap as text so the JSONB output stays
    // a string.
    if let Some(n) = expr_to_numeric_sql(e, anchors) {
        return Some(format!("({n})::text"));
    }
    // CONCAT — variable arg count, string-yielding.
    if let Expression::FunctionCall(Function::Concat, args) = e {
        let parts: Vec<String> = args
            .iter()
            .map(|a| expr_to_lexical_sql(a, anchors))
            .collect::<Option<Vec<_>>>()?;
        return Some(format!("concat({})", parts.join(", ")));
    }
    None
}

/// Does any sub-expression of `e` reference a variable in `names`?
/// Used to migrate filters into a HAVING clause when they name an
/// aggregate output variable.
fn expression_references_any(e: &Expression, names: &[String]) -> bool {
    let any = |inner: &Expression| expression_references_any(inner, names);
    let any_list = |list: &[Expression]| list.iter().any(|x| expression_references_any(x, names));
    match e {
        Expression::Variable(v) => names.iter().any(|n| n == v.as_str()),
        Expression::Bound(v) => names.iter().any(|n| n == v.as_str()),
        Expression::Equal(a, b)
        | Expression::SameTerm(a, b)
        | Expression::Greater(a, b)
        | Expression::GreaterOrEqual(a, b)
        | Expression::Less(a, b)
        | Expression::LessOrEqual(a, b)
        | Expression::And(a, b)
        | Expression::Or(a, b)
        | Expression::Add(a, b)
        | Expression::Subtract(a, b)
        | Expression::Multiply(a, b)
        | Expression::Divide(a, b) => any(a) || any(b),
        Expression::UnaryPlus(a) | Expression::UnaryMinus(a) | Expression::Not(a) => any(a),
        Expression::If(a, b, c) => any(a) || any(b) || any(c),
        Expression::In(op, list) => any(op) || any_list(list),
        Expression::Coalesce(list) => any_list(list),
        Expression::FunctionCall(_, args) => any_list(args),
        Expression::Exists(_) => false,
        Expression::NamedNode(_) | Expression::Literal(_) => false,
    }
}

/// Lower a spargebra `AggregateExpression` into our AggregateSpec.
/// Supported: COUNT(*), COUNT(?v) [DISTINCT], SUM(?v) [DISTINCT],
/// AVG(?v), MIN(?v), MAX(?v). Anything else panics.
fn parse_aggregate(synth_var: &str, agg: &AggregateExpression) -> AggregateSpec {
    match agg {
        AggregateExpression::CountSolutions { distinct } => AggregateSpec {
            output_var: synth_var.to_string(),
            func: AggregateFn::Count,
            distinct: *distinct,
            arg_var: None,
        },
        AggregateExpression::FunctionCall { name, expr, distinct } => {
            let arg_var = match expr {
                Expression::Variable(v) => v.as_str().to_string(),
                other => panic!(
                    "sparql: aggregate over non-variable expression not supported yet (got {other:?})"
                ),
            };
            let func = match name {
                AggregateFunction::Count => AggregateFn::Count,
                AggregateFunction::Sum => AggregateFn::Sum,
                AggregateFunction::Avg => AggregateFn::Avg,
                AggregateFunction::Min => AggregateFn::Min,
                AggregateFunction::Max => AggregateFn::Max,
                AggregateFunction::GroupConcat { separator } => AggregateFn::GroupConcat {
                    separator: separator.clone().unwrap_or_else(|| " ".to_string()),
                },
                AggregateFunction::Sample => AggregateFn::Sample,
                AggregateFunction::Custom(iri) => {
                    panic!("sparql: custom aggregate {iri:?} not supported")
                }
            };
            AggregateSpec {
                output_var: synth_var.to_string(),
                func,
                distinct: *distinct,
                arg_var: Some(arg_var),
            }
        }
    }
}

/// Walk a left-leaning Union tree, pushing each leaf branch.
fn collect_union_branches(p: &GraphPattern, out: &mut Vec<UnionBranch>) {
    match p {
        GraphPattern::Union { left, right } => {
            collect_union_branches(left, out);
            collect_union_branches(right, out);
        }
        _ => out.push(walk_union_branch(p)),
    }
}

/// Walk a single UNION branch, capturing its BGP, filters, and
/// OPTIONALs. Solution modifiers (DISTINCT, ORDER BY, LIMIT, OFFSET)
/// belong on the outer SELECT, not inside a branch — so they aren't
/// expected here.
fn walk_union_branch(p: &GraphPattern) -> UnionBranch {
    let mut ub = UnionBranch::default();
    walk_branch(p, &mut ub);
    ub
}

fn walk_branch(p: &GraphPattern, ub: &mut UnionBranch) {
    match p {
        GraphPattern::Bgp { patterns } => {
            ub.bgp = patterns.clone();
        }
        GraphPattern::Filter { expr, inner } => {
            ub.filters.push(expr.clone());
            walk_branch(inner, ub);
        }
        GraphPattern::LeftJoin { left, right, expression } => {
            walk_branch(left, ub);
            let triple = match right.as_ref() {
                GraphPattern::Bgp { patterns } if patterns.len() == 1 => patterns[0].clone(),
                GraphPattern::Bgp { patterns } => panic!(
                    "sparql: OPTIONAL today only supports a single triple pattern (got {} triples)",
                    patterns.len()
                ),
                other => panic!(
                    "sparql: OPTIONAL today only supports a single triple pattern (got {other:?})"
                ),
            };
            ub.optionals.push(OptionalBlock {
                triple,
                filter: expression.clone(),
            });
        }
        GraphPattern::Minus { left, right } => {
            walk_branch(left, ub);
            let triples = match right.as_ref() {
                GraphPattern::Bgp { patterns } => patterns.clone(),
                _ => panic!("sparql: MINUS right side must be a BGP"),
            };
            ub.minuses.push(triples);
        }
        other => panic!("sparql: unsupported algebra inside UNION branch: {other:?}"),
    }
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
        GraphPattern::LeftJoin { left, right, expression } => {
            // Walk the left arm first — it may itself be another
            // LeftJoin (chained OPTIONALs) or a Filter wrapping a BGP.
            walk_select(left, ps);
            // The right arm is the OPTIONAL group. Today only a
            // single-triple BGP is accepted.
            let triple = match right.as_ref() {
                GraphPattern::Bgp { patterns } if patterns.len() == 1 => patterns[0].clone(),
                GraphPattern::Bgp { patterns } => panic!(
                    "sparql: OPTIONAL today only supports a single triple pattern (got {} triples)",
                    patterns.len()
                ),
                other => panic!(
                    "sparql: OPTIONAL today only supports a single triple pattern (got {other:?})"
                ),
            };
            ps.optionals.push(OptionalBlock {
                triple,
                filter: expression.clone(),
            });
        }
        GraphPattern::Union { left, right } => {
            // Chained `A UNION B UNION C` arrives as a left-leaning
            // Union tree; flatten so every leaf becomes its own branch.
            collect_union_branches(left, &mut ps.union_branches);
            collect_union_branches(right, &mut ps.union_branches);
        }
        GraphPattern::Minus { left, right } => {
            walk_select(left, ps);
            let triples = match right.as_ref() {
                GraphPattern::Bgp { patterns } => patterns.clone(),
                other => panic!(
                    "sparql: MINUS right side must be a BGP (got {other:?})"
                ),
            };
            ps.minuses.push(triples);
        }
        GraphPattern::Group { inner, variables, aggregates } => {
            for v in variables {
                ps.group_vars.push(v.as_str().to_string());
            }
            for (synth_var, agg_expr) in aggregates {
                ps.aggregates.push(parse_aggregate(synth_var.as_str(), agg_expr));
            }
            walk_select(inner, ps);
        }
        GraphPattern::Extend { inner, variable, expression } => {
            // Walk inner FIRST so any Group below has populated
            // ps.aggregates by the time we decide what kind of
            // Extend this is.
            walk_select(inner, ps);
            // Two kinds of Extend matter today:
            //   1) Aggregate rename — `(EXPR AS ?v)` lowers to Extend
            //      wrapping Group, with `expression == Variable($agg_N)`.
            //      Match and rename the matching AggregateSpec.
            //   2) General BIND — `BIND(expr AS ?v)` or `(EXPR AS ?v)`
            //      on a non-aggregate expression. Capture as a BindSpec.
            let new_name = variable.as_str().to_string();
            if let Expression::Variable(v) = expression {
                let synth = v.as_str();
                if let Some(agg) = ps.aggregates.iter_mut().find(|a| a.output_var == synth) {
                    agg.output_var = new_name;
                    return;
                }
            }
            ps.binds.push(BindSpec {
                output_var: new_name,
                expression: expression.clone(),
            });
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

/// Build a dynamic SQL SELECT.
///
/// Single-branch path: builds the BGP + filters + optionals directly
/// via `build_branch_sql`, then layers the solution modifiers
/// (DISTINCT/ORDER BY/LIMIT/OFFSET) onto the same statement so
/// ORDER BY can reference unprojected anchored variables via hidden
/// SELECT-list columns.
///
/// UNION path: builds each branch independently via
/// `build_branch_sql`, combines them with `UNION ALL`, then wraps
/// the union in an outer SELECT for DISTINCT/ORDER BY/LIMIT/OFFSET.
/// ORDER BY on UNION may only reference projected variables (the
/// outer SELECT has no access to a branch's per-alias columns).
fn build_bgp_sql(ps: &ParsedSelect) -> String {
    if !ps.union_branches.is_empty() {
        if !ps.aggregates.is_empty() {
            panic!("sparql: aggregates on top of UNION not supported yet");
        }
        return build_union_sql(ps);
    }
    if !ps.aggregates.is_empty() {
        return build_aggregate_sql(ps);
    }
    build_single_branch_outer(ps)
}

/// Single-branch SELECT — emits FROM + WHERE + SELECT + solution
/// modifiers in one shot, since ORDER BY may use anchored vars
/// even when they aren't projected (via hidden trailing columns).
fn build_single_branch_outer(ps: &ParsedSelect) -> String {
    let mut anchors: HashMap<String, (usize, &'static str)> = HashMap::new();
    let (from_sql, where_clauses) = build_from_and_where(
        &ps.bgp,
        &ps.filters,
        &ps.optionals,
        &ps.minuses,
        &mut anchors,
        0,
    );

    // Project: each projected var → its anchor alias.column → dict
    // lookup → aliased to the variable name (so SETOF JSONB emission
    // pulls the right column by ordinal). Hidden trailing columns
    // are appended for ORDER BY variables that aren't projected.
    // BIND-bound variables emit their expression SQL instead of a
    // dict-lookup.
    let mut select_clauses: Vec<String> = Vec::new();
    for var in &ps.projected {
        if let Some(bind) = ps.binds.iter().find(|b| &b.output_var == var) {
            let expr_sql = translate_bind_expression(&bind.expression, &anchors)
                .unwrap_or_else(|| {
                    panic!(
                        "sparql: BIND expression for ?{var} not translatable: {:?}",
                        bind.expression
                    )
                });
            select_clauses.push(format!("{expr_sql} AS {}", quote_identifier(var)));
            continue;
        }
        let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
            panic!("sparql: projected variable ?{var} is not bound in any BGP pattern")
        });
        select_clauses.push(format!(
            "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary
               WHERE id = q{alias_idx}.{col}) AS {alias_v}",
            alias_v = quote_identifier(var),
        ));
    }

    let mut order_clauses: Vec<String> = Vec::new();
    for (idx, (var, ascending)) in ps.order_by.iter().enumerate() {
        let dir = if *ascending { "ASC" } else { "DESC" };
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
        "SELECT {distinct_kw}{sel} FROM {from_sql}",
        sel = select_clauses.join(", "),
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

/// Aggregate SELECT — single-branch BGP wrapped in SQL GROUP BY +
/// aggregate functions. Output columns: group vars (dict-lookup)
/// + aggregate values (cast to TEXT for consistent JSONB output).
fn build_aggregate_sql(ps: &ParsedSelect) -> String {
    let mut anchors: HashMap<String, (usize, &'static str)> = HashMap::new();
    let (from_sql, where_clauses) = build_from_and_where(
        &ps.bgp,
        &ps.filters,
        &ps.optionals,
        &ps.minuses,
        &mut anchors,
        0,
    );

    // Pre-compute SQL expressions per group variable so SELECT and
    // GROUP BY use the same string (Postgres accepts either repeated
    // expression or an ordinal; we use the expression here).
    let mut group_exprs: Vec<(String, String)> = Vec::new(); // (var, sql-expr)
    for var in &ps.group_vars {
        let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
            panic!("sparql: GROUP BY variable ?{var} not bound in any BGP pattern")
        });
        let expr = format!(
            "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary WHERE id = q{alias_idx}.{col})"
        );
        group_exprs.push((var.clone(), expr));
    }

    // Build SELECT clauses. The output column ordering is the user
    // projection (ps.projected) — for each projected var, either it
    // is a group var (dict-lookup expr) or it is an aggregate
    // output_var (aggregate function).
    let mut select_clauses: Vec<String> = Vec::new();
    for var in &ps.projected {
        if let Some((_, expr)) = group_exprs.iter().find(|(v, _)| v == var) {
            select_clauses.push(format!("{expr} AS {}", quote_identifier(var)));
        } else if let Some(agg) = ps.aggregates.iter().find(|a| &a.output_var == var) {
            select_clauses.push(format!(
                "{}::TEXT AS {}",
                translate_aggregate(agg, &anchors),
                quote_identifier(var)
            ));
        } else {
            panic!(
                "sparql: projected variable ?{var} is neither a GROUP BY variable nor an aggregate output"
            );
        }
    }

    let mut sql = format!(
        "SELECT {sel} FROM {from_sql}",
        sel = select_clauses.join(", "),
    );
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    if !group_exprs.is_empty() {
        let group_by_parts: Vec<&str> = group_exprs.iter().map(|(_, e)| e.as_str()).collect();
        sql.push_str(" GROUP BY ");
        sql.push_str(&group_by_parts.join(", "));
    }

    // HAVING — filters that name aggregate output vars. Variable refs
    // in the filter expand to the underlying SQL aggregate function;
    // group-var refs expand to the group expression.
    if !ps.having_filters.is_empty() {
        let mut having_parts: Vec<String> = Vec::new();
        for expr in &ps.having_filters {
            let sql_pred = translate_filter_with_aggregates(
                expr,
                &anchors,
                &ps.aggregates,
                &group_exprs,
            )
            .unwrap_or_else(|| {
                panic!("sparql: HAVING expression not translatable: {expr:?}")
            });
            having_parts.push(sql_pred);
        }
        sql.push_str(" HAVING ");
        sql.push_str(&having_parts.join(" AND "));
    }

    // ORDER BY / LIMIT / OFFSET on aggregate output: only group vars
    // and aggregate outputs are in scope.
    if !ps.order_by.is_empty() {
        let mut order_parts: Vec<String> = Vec::new();
        for (var, ascending) in &ps.order_by {
            let dir = if *ascending { "ASC" } else { "DESC" };
            if !ps.projected.contains(var) {
                panic!(
                    "sparql: ORDER BY ?{var} on an aggregate query must reference a \
                     projected variable (group var or aggregate output)"
                );
            }
            order_parts.push(format!("{} {dir} NULLS LAST", quote_identifier(var)));
        }
        sql.push_str(&format!(" ORDER BY {}", order_parts.join(", ")));
    }
    if let Some(limit) = ps.limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    if ps.offset > 0 {
        sql.push_str(&format!(" OFFSET {}", ps.offset));
    }
    sql
}

/// Lower a single AggregateSpec into a SQL aggregate expression.
/// SUM/AVG are numeric-aware via a CASE-guarded cast to NUMERIC
/// (matching FILTER numeric semantics — non-numeric rows contribute
/// NULL and are ignored by SUM/AVG aggregates).
fn translate_aggregate(
    agg: &AggregateSpec,
    anchors: &HashMap<String, (usize, &'static str)>,
) -> String {
    let distinct = if agg.distinct { "DISTINCT " } else { "" };
    let lex_subselect = |var: &str| {
        let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
            panic!("sparql: aggregate over ?{var} but variable not bound in any BGP pattern")
        });
        format!(
            "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary WHERE id = q{alias_idx}.{col})"
        )
    };
    match (&agg.func, &agg.arg_var) {
        (AggregateFn::Count, None) => "COUNT(*)".to_string(),
        (AggregateFn::Count, Some(var)) => {
            let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
                panic!("sparql: aggregate COUNT(?{var}) but variable not bound in any BGP pattern")
            });
            format!("COUNT({distinct}q{alias_idx}.{col})")
        }
        (AggregateFn::Sum, Some(var)) => {
            let numeric_expr = numeric_cast_subselect(var, anchors);
            format!("SUM({distinct}{numeric_expr})")
        }
        (AggregateFn::Avg, Some(var)) => {
            let numeric_expr = numeric_cast_subselect(var, anchors);
            format!("AVG({distinct}{numeric_expr})")
        }
        (AggregateFn::Min, Some(var)) => format!("MIN({distinct}{})", lex_subselect(var)),
        (AggregateFn::Max, Some(var)) => format!("MAX({distinct}{})", lex_subselect(var)),
        (AggregateFn::GroupConcat { separator }, Some(var)) => {
            let escaped = separator.replace('\'', "''");
            format!(
                "STRING_AGG({distinct}{}, '{escaped}')",
                lex_subselect(var)
            )
        }
        (AggregateFn::Sample, Some(var)) => format!("MIN({})", lex_subselect(var)),
        (_, None) => panic!("sparql: aggregate over `*` only supported for COUNT"),
    }
}

/// Translate a HAVING filter where some variable references resolve
/// to aggregate output expressions rather than dict-id columns.
/// Supported predicates: identity (`=`, `!=`, `sameTerm`), numeric
/// ordering (`<`, `>`, `<=`, `>=`), boolean composition (`&&`,
/// `||`, `!`). Variable refs may name (1) an aggregate output —
/// expands to the raw SQL aggregate, no `::TEXT`; (2) a group var —
/// expands to its dict-lookup expression cast to numeric / text as
/// the comparison demands.
fn translate_filter_with_aggregates(
    expr: &Expression,
    anchors: &HashMap<String, (usize, &'static str)>,
    aggs: &[AggregateSpec],
    group_exprs: &[(String, String)],
) -> Option<String> {
    let numeric_side = |e: &Expression| -> Option<String> {
        match e {
            Expression::Variable(v) => {
                let name = v.as_str();
                if let Some(agg) = aggs.iter().find(|a| a.output_var == name) {
                    Some(translate_aggregate(agg, anchors))
                } else if let Some((_, gexpr)) = group_exprs.iter().find(|(n, _)| n == name) {
                    Some(format!("({gexpr})::numeric"))
                } else {
                    None
                }
            }
            Expression::Literal(l) => {
                if !is_xsd_numeric_iri(l.datatype().as_str()) {
                    return None;
                }
                if l.value().parse::<f64>().is_err() {
                    return None;
                }
                Some(format!("{}::numeric", l.value()))
            }
            _ => None,
        }
    };
    let text_side = |e: &Expression| -> Option<String> {
        match e {
            Expression::Variable(v) => {
                let name = v.as_str();
                if let Some(agg) = aggs.iter().find(|a| a.output_var == name) {
                    Some(format!("({})::text", translate_aggregate(agg, anchors)))
                } else if let Some((_, gexpr)) = group_exprs.iter().find(|(n, _)| n == name) {
                    Some(gexpr.clone())
                } else {
                    None
                }
            }
            Expression::Literal(l) => {
                let escaped = l.value().replace('\'', "''");
                Some(format!("'{escaped}'"))
            }
            _ => None,
        }
    };
    match expr {
        Expression::Greater(a, b) => Some(format!("({} > {})", numeric_side(a)?, numeric_side(b)?)),
        Expression::GreaterOrEqual(a, b) => {
            Some(format!("({} >= {})", numeric_side(a)?, numeric_side(b)?))
        }
        Expression::Less(a, b) => Some(format!("({} < {})", numeric_side(a)?, numeric_side(b)?)),
        Expression::LessOrEqual(a, b) => {
            Some(format!("({} <= {})", numeric_side(a)?, numeric_side(b)?))
        }
        Expression::Equal(a, b) | Expression::SameTerm(a, b) => {
            Some(format!("({} = {})", text_side(a)?, text_side(b)?))
        }
        Expression::And(a, b) => {
            let l = translate_filter_with_aggregates(a, anchors, aggs, group_exprs)?;
            let r = translate_filter_with_aggregates(b, anchors, aggs, group_exprs)?;
            Some(format!("({l} AND {r})"))
        }
        Expression::Or(a, b) => {
            let l = translate_filter_with_aggregates(a, anchors, aggs, group_exprs)?;
            let r = translate_filter_with_aggregates(b, anchors, aggs, group_exprs)?;
            Some(format!("({l} OR {r})"))
        }
        Expression::Not(inner) => {
            let l = translate_filter_with_aggregates(inner, anchors, aggs, group_exprs)?;
            Some(format!("(NOT ({l}))"))
        }
        _ => None,
    }
}

/// Build the same NUMERIC-cast CASE expression we use for FILTER
/// ordering — restricted to dict rows whose datatype is one of the
/// XSD numeric IRIs, else NULL.
fn numeric_cast_subselect(
    var: &str,
    anchors: &HashMap<String, (usize, &'static str)>,
) -> String {
    let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
        panic!("sparql: numeric aggregate over ?{var} but variable not bound in any BGP pattern")
    });
    let dt_ids = numeric_datatype_id_list();
    format!(
        "(SELECT CASE WHEN datatype_iri_id IN ({dt_ids})
                      THEN lexical_value::numeric
                      ELSE NULL
                 END
          FROM pgrdf._pgrdf_dictionary WHERE id = q{alias_idx}.{col})"
    )
}

/// UNION SELECT — each branch is a complete sub-SELECT that
/// projects the common variable list (NULL for vars it doesn't
/// bind). Outer SELECT layers DISTINCT/ORDER BY/LIMIT/OFFSET.
fn build_union_sql(ps: &ParsedSelect) -> String {
    let branch_sqls: Vec<String> = ps
        .union_branches
        .iter()
        .map(|b| build_branch_sql(b, &ps.projected))
        .collect();
    let union_inner = branch_sqls.join(" UNION ALL ");

    let distinct_kw = if ps.distinct { "DISTINCT " } else { "" };
    let outer_cols = ps
        .projected
        .iter()
        .map(|v| quote_identifier(v))
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "SELECT {distinct_kw}{outer_cols} FROM ({union_inner}) AS _pgrdf_union"
    );

    if !ps.order_by.is_empty() {
        let mut order_parts: Vec<String> = Vec::new();
        for (var, ascending) in &ps.order_by {
            let dir = if *ascending { "ASC" } else { "DESC" };
            if !ps.projected.contains(var) {
                panic!(
                    "sparql: ORDER BY ?{var} on UNION must reference a projected \
                     variable (the outer SELECT can't see branch-local columns)"
                );
            }
            order_parts.push(format!("{} {dir} NULLS LAST", quote_identifier(var)));
        }
        sql.push_str(&format!(" ORDER BY {}", order_parts.join(", ")));
    }
    if let Some(limit) = ps.limit {
        sql.push_str(&format!(" LIMIT {limit}"));
    }
    if ps.offset > 0 {
        sql.push_str(&format!(" OFFSET {}", ps.offset));
    }
    sql
}

/// Build one branch of a UNION as a standalone SELECT statement.
/// Variables not bound by this branch are emitted as `NULL::TEXT`
/// in the SELECT list so the resulting row shape matches every
/// other branch's row shape (required for `UNION ALL`).
fn build_branch_sql(branch: &UnionBranch, projected: &[String]) -> String {
    let mut anchors: HashMap<String, (usize, &'static str)> = HashMap::new();
    let (from_sql, where_clauses) = build_from_and_where(
        &branch.bgp,
        &branch.filters,
        &branch.optionals,
        &branch.minuses,
        &mut anchors,
        0,
    );

    let mut select_clauses: Vec<String> = Vec::new();
    for var in projected {
        let part = if let Some(&(alias_idx, col)) = anchors.get(var) {
            format!(
                "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary
                   WHERE id = q{alias_idx}.{col}) AS {alias_v}",
                alias_v = quote_identifier(var),
            )
        } else {
            format!("NULL::TEXT AS {}", quote_identifier(var))
        };
        select_clauses.push(part);
    }

    let mut sql = format!(
        "SELECT {sel} FROM {from_sql}",
        sel = select_clauses.join(", "),
    );
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }
    sql
}

/// Shared FROM/WHERE builder used by both the single-branch and
/// per-UNION-branch paths. Emits explicit `INNER JOIN`s for
/// mandatory patterns after the first, `LEFT JOIN`s for each
/// OPTIONAL block, and ANDs every filter into the returned
/// `where_clauses` vec. The caller layers SELECT + modifiers.
fn build_from_and_where(
    bgp: &[TriplePattern],
    filters: &[Expression],
    optionals: &[OptionalBlock],
    minuses: &[Vec<TriplePattern>],
    anchors: &mut HashMap<String, (usize, &'static str)>,
    alias_offset: usize,
) -> (String, Vec<String>) {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut from_sql = String::new();
    // Mandatory BGP — pattern 1 in FROM (predicates → WHERE),
    // pattern 2..N as INNER JOIN qN ON (predicates).
    for (i, tp) in bgp.iter().enumerate() {
        let qi = alias_offset + i + 1;
        let mut clauses = pattern_clauses(tp, qi, anchors);
        if i == 0 {
            from_sql.push_str(&format!("pgrdf._pgrdf_quads q{qi}"));
            where_clauses.append(&mut clauses);
        } else {
            let on = if clauses.is_empty() {
                "TRUE".to_string()
            } else {
                clauses.join(" AND ")
            };
            from_sql.push_str(&format!(
                " INNER JOIN pgrdf._pgrdf_quads q{qi} ON ({on})"
            ));
        }
    }
    // OPTIONAL blocks — each becomes a LEFT JOIN whose ON includes
    // the OPTIONAL's inner FILTER (if any). Vars only bound in the
    // OPTIONAL come back NULL when the LEFT JOIN doesn't match.
    let mut next_qi = alias_offset + bgp.len() + 1;
    for opt in optionals {
        let opt_qi = next_qi;
        next_qi += 1;
        let mut clauses = pattern_clauses(&opt.triple, opt_qi, anchors);
        if let Some(filter_expr) = &opt.filter {
            let sql = translate_filter(filter_expr, anchors).unwrap_or_else(|| {
                panic!("sparql: OPTIONAL FILTER not translatable: {filter_expr:?}")
            });
            clauses.push(sql);
        }
        let on = if clauses.is_empty() {
            "TRUE".to_string()
        } else {
            clauses.join(" AND ")
        };
        from_sql.push_str(&format!(
            " LEFT JOIN pgrdf._pgrdf_quads q{opt_qi} ON ({on})"
        ));
    }
    // Top-level / branch-level FILTERs — applied to the joined
    // result. NULL comparisons drop the row (SPARQL "type error →
    // unbound" semantics).
    for expr in filters {
        let sql = translate_filter(expr, anchors).unwrap_or_else(|| {
            panic!("sparql: FILTER expression not translatable: {expr:?}")
        });
        where_clauses.push(sql);
    }
    // MINUS blocks → `NOT EXISTS (SELECT 1 FROM … WHERE shared_vars)`.
    // Per SPARQL spec, MINUS with no shared variables is a no-op and
    // is elided here.
    for minus_triples in minuses {
        if let Some(sql) = translate_minus(minus_triples, anchors, &mut next_qi) {
            where_clauses.push(sql);
        }
    }
    (from_sql, where_clauses)
}

/// Translate a MINUS sub-pattern (N triples) against the outer
/// anchors. Returns `None` if the sub-pattern shares no variables
/// with the outer query (SPARQL spec: MINUS with no shared
/// variables is the identity). Otherwise emits a
/// `NOT EXISTS (SELECT 1 FROM <quads aliases> WHERE …)` sub-SELECT
/// where the WHERE clause carries every triple's predicates +
/// equality predicates joining shared variables back to the outer
/// aliases.
fn translate_minus(
    triples: &[TriplePattern],
    outer_anchors: &HashMap<String, (usize, &'static str)>,
    next_qi: &mut usize,
) -> Option<String> {
    if triples.is_empty() {
        return None;
    }
    // Collect all variables across the MINUS sub-pattern; if none
    // are shared with the outer query, MINUS is a no-op.
    let mut all_vars: Vec<String> = Vec::new();
    for tp in triples {
        if let Some(v) = tp_subject_var(tp) {
            all_vars.push(v);
        }
        if let Some(v) = tp_predicate_var(tp) {
            all_vars.push(v);
        }
        if let Some(v) = tp_object_var(tp) {
            all_vars.push(v);
        }
    }
    if !all_vars.iter().any(|v| outer_anchors.contains_key(v)) {
        return None;
    }
    // Clone outer anchors so shared vars in the sub-pattern emit
    // equality predicates against outer aliases on first occurrence.
    // New vars introduced inside the MINUS get fresh local aliases;
    // shared vars internal to the sub-pattern also tie together.
    let mut local_anchors = outer_anchors.clone();
    let mut all_clauses: Vec<String> = Vec::new();
    let mut from_aliases: Vec<String> = Vec::with_capacity(triples.len());
    for tp in triples {
        let qi = *next_qi;
        *next_qi += 1;
        from_aliases.push(format!("pgrdf._pgrdf_quads q{qi}"));
        let mut clauses = pattern_clauses(tp, qi, &mut local_anchors);
        all_clauses.append(&mut clauses);
    }
    let where_inside = if all_clauses.is_empty() {
        "TRUE".to_string()
    } else {
        all_clauses.join(" AND ")
    };
    let from = from_aliases.join(", ");
    Some(format!(
        "NOT EXISTS (SELECT 1 FROM {from} WHERE {where_inside})"
    ))
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
            // Prefer dict-id equality (sameTerm semantics, single
            // BIGINT compare, hits the dictionary's PK index).
            if let (Some(l), Some(r)) = (expr_to_id_sql(a, anchors), expr_to_id_sql(b, anchors)) {
                return Some(format!("({l} = {r})"));
            }
            // Fall back to lexical comparison so STR(?v) = "x" and
            // LANG(?v) = "en" etc. translate cleanly.
            let l = expr_to_lexical_sql(a, anchors)?;
            let r = expr_to_lexical_sql(b, anchors)?;
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
            // For mandatory anchors qN.col is never NULL (INNER join
            // semantics). For OPTIONAL anchors qOPT_i.col is NULL
            // when the LEFT JOIN didn't match. Emitting `IS NOT NULL`
            // is correct in both cases: TRUE for mandatory, distinguishes
            // matched/unmatched for OPTIONAL. Unknown variable → FALSE.
            let name = v.as_str();
            match anchors.get(name) {
                Some(&(alias_idx, col)) => Some(format!("(q{alias_idx}.{col} IS NOT NULL)")),
                None => Some("FALSE".to_string()),
            }
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
        Function::Contains => translate_string_fn(args, anchors, |s, sub| {
            format!("(strpos({s}, {sub}) > 0)")
        }),
        Function::StrStarts => translate_string_fn(args, anchors, |s, prefix| {
            format!("(left({s}, length({prefix})) = {prefix})")
        }),
        Function::StrEnds => translate_string_fn(args, anchors, |s, suffix| {
            format!("(right({s}, length({suffix})) = {suffix})")
        }),
        // STR / LANG / DATATYPE / UCASE / LCASE / STRLEN etc. aren't
        // boolean — they're text- or numeric-valued and surface inside
        // other comparisons via expr_to_lexical_sql / expr_to_numeric_sql.
        _ => None,
    }
}

/// 2-argument string predicate translator. Both args go through
/// `expr_to_lexical_sql` so they accept variables, literals,
/// `STR(?v)`, etc. The closure builds the SQL boolean expression.
fn translate_string_fn<F>(
    args: &[Expression],
    anchors: &HashMap<String, (usize, &'static str)>,
    builder: F,
) -> Option<String>
where
    F: FnOnce(String, String) -> String,
{
    if args.len() != 2 {
        return None;
    }
    let s = expr_to_lexical_sql(&args[0], anchors)?;
    let needle = expr_to_lexical_sql(&args[1], anchors)?;
    Some(builder(s, needle))
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
            Some(id_placeholder(id))
        }
        Expression::Literal(l) => {
            let id = lookup_literal_id(l).unwrap_or(-1);
            Some(id_placeholder(id))
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
            let v = l.value();
            if v.parse::<f64>().is_err() {
                return None;
            }
            Some(format!("{v}::numeric"))
        }
        // Arithmetic — both sides cast to numeric. NULL propagation
        // means a non-numeric operand drops the row (SPARQL "type
        // error → unbound" semantics).
        Expression::Add(a, b) => Some(format!(
            "({} + {})",
            expr_to_numeric_sql(a, anchors)?,
            expr_to_numeric_sql(b, anchors)?
        )),
        Expression::Subtract(a, b) => Some(format!(
            "({} - {})",
            expr_to_numeric_sql(a, anchors)?,
            expr_to_numeric_sql(b, anchors)?
        )),
        Expression::Multiply(a, b) => Some(format!(
            "({} * {})",
            expr_to_numeric_sql(a, anchors)?,
            expr_to_numeric_sql(b, anchors)?
        )),
        Expression::Divide(a, b) => Some(format!(
            "({} / NULLIF({}, 0))",
            expr_to_numeric_sql(a, anchors)?,
            expr_to_numeric_sql(b, anchors)?
        )),
        Expression::UnaryMinus(a) => {
            Some(format!("(-{})", expr_to_numeric_sql(a, anchors)?))
        }
        Expression::UnaryPlus(a) => expr_to_numeric_sql(a, anchors),
        // STRLEN(?v) → length of lexical_value
        Expression::FunctionCall(Function::StrLen, args) if args.len() == 1 => {
            let lex = expr_to_lexical_sql(&args[0], anchors)?;
            Some(format!("length({lex})::numeric"))
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
        // A NamedNode lexical form is just its IRI. Needed so
        // `DATATYPE(?v) = xsd:integer` and `?p = foaf:name` (when
        // lexical-equality fallback fires) translate cleanly.
        Expression::NamedNode(n) => {
            let escaped = n.as_str().replace('\'', "''");
            Some(format!("'{escaped}'"))
        }
        // STR(?v) is identity — every dict entry's lexical_value IS
        // the string form.
        Expression::FunctionCall(Function::Str, inner) if inner.len() == 1 => {
            expr_to_lexical_sql(&inner[0], anchors)
        }
        // LANG(?v) → language_tag from the dict (empty string when NULL).
        Expression::FunctionCall(Function::Lang, inner) if inner.len() == 1 => {
            if let Expression::Variable(v) = &inner[0] {
                let &(alias_idx, col) = anchors.get(v.as_str())?;
                Some(format!(
                    "COALESCE((SELECT language_tag FROM pgrdf._pgrdf_dictionary
                                WHERE id = q{alias_idx}.{col}), '')"
                ))
            } else {
                None
            }
        }
        // DATATYPE(?v) → IRI of the literal's datatype, resolved via
        // a chained dict lookup (datatype_iri_id → lexical_value).
        Expression::FunctionCall(Function::Datatype, inner) if inner.len() == 1 => {
            if let Expression::Variable(v) = &inner[0] {
                let &(alias_idx, col) = anchors.get(v.as_str())?;
                Some(format!(
                    "(SELECT dt.lexical_value
                        FROM pgrdf._pgrdf_dictionary d
                        JOIN pgrdf._pgrdf_dictionary dt ON dt.id = d.datatype_iri_id
                       WHERE d.id = q{alias_idx}.{col})"
                ))
            } else {
                None
            }
        }
        // UCASE / LCASE — case conversion on the lexical form.
        Expression::FunctionCall(Function::UCase, inner) if inner.len() == 1 => {
            let s = expr_to_lexical_sql(&inner[0], anchors)?;
            Some(format!("upper({s})"))
        }
        Expression::FunctionCall(Function::LCase, inner) if inner.len() == 1 => {
            let s = expr_to_lexical_sql(&inner[0], anchors)?;
            Some(format!("lower({s})"))
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
        // No xsd:numeric IRIs in the dictionary yet — emit a literal
        // -1 placeholder that won't match any real dict id. Routed
        // through id_placeholder so the surrounding SQL string is
        // still parameterised consistently.
        id_placeholder(-1)
    } else {
        ids.iter()
            .map(|i| id_placeholder(*i))
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Process a triple pattern's subject/predicate/object positions
/// against the anchor map and return the equality predicates it
/// introduces. Callers route these to either `WHERE` (the first
/// mandatory pattern) or the `ON` clause of an `INNER JOIN` / `LEFT
/// JOIN` (every subsequent mandatory or optional pattern).
fn pattern_clauses(
    tp: &TriplePattern,
    qi: usize,
    anchors: &mut HashMap<String, (usize, &'static str)>,
) -> Vec<String> {
    let mut clauses = Vec::new();
    bind_subject(tp, qi, anchors, &mut clauses);
    bind_predicate(tp, qi, anchors, &mut clauses);
    bind_object(tp, qi, anchors, &mut clauses);
    clauses
}

fn bind_subject(
    tp: &TriplePattern,
    qi: usize,
    anchors: &mut HashMap<String, (usize, &'static str)>,
    clauses: &mut Vec<String>,
) {
    match &tp.subject {
        TermPattern::Variable(v) => bind_var(v.as_str(), qi, "subject_id", anchors, clauses),
        TermPattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            let p = id_placeholder(id);
            clauses.push(format!("q{qi}.subject_id = {p}"));
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
    clauses: &mut Vec<String>,
) {
    match &tp.predicate {
        NamedNodePattern::Variable(v) => {
            bind_var(v.as_str(), qi, "predicate_id", anchors, clauses)
        }
        NamedNodePattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            let p = id_placeholder(id);
            clauses.push(format!("q{qi}.predicate_id = {p}"));
        }
    }
}

fn bind_object(
    tp: &TriplePattern,
    qi: usize,
    anchors: &mut HashMap<String, (usize, &'static str)>,
    clauses: &mut Vec<String>,
) {
    match &tp.object {
        TermPattern::Variable(v) => bind_var(v.as_str(), qi, "object_id", anchors, clauses),
        TermPattern::NamedNode(n) => {
            let id = lookup_iri_id(n.as_str()).unwrap_or(-1);
            let p = id_placeholder(id);
            clauses.push(format!("q{qi}.object_id = {p}"));
        }
        TermPattern::Literal(l) => {
            let id = lookup_literal_id(l).unwrap_or(-1);
            let p = id_placeholder(id);
            clauses.push(format!("q{qi}.object_id = {p}"));
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
    clauses: &mut Vec<String>,
) {
    if let Some(&(prev_qi, prev_col)) = anchors.get(name) {
        clauses.push(format!("q{qi}.{col} = q{prev_qi}.{prev_col}"));
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
        // Phase 3 step 2 (LLD §4.2): consult the per-backend plan
        // cache before paying for parse + plan. Hit ⇒ reuse the
        // `SPI_keepplan`-promoted statement; miss ⇒ prepare, then
        // promote and stash for next time.
        if !plan_cache::contains(&plan.sql) {
            let arg_oids: Vec<PgOid> =
                vec![PgOid::BuiltIn(PgBuiltInOids::INT8OID); plan.params.len()];
            let prepared = client
                .prepare(plan.sql.as_str(), &arg_oids)
                .expect("sparql: prepare failed")
                .keep();
            plan_cache::insert(plan.sql.clone(), prepared);
            plan_cache::record_miss();
        } else {
            plan_cache::record_hit();
        }

        // SAFETY-adjacent: DatumWithOid::new is unsafe because Rust
        // can't verify the (value, type-oid) pair matches at the
        // type level. We pass INT8 i64 values everywhere — all
        // dict ids — so this contract holds.
        let int8_oid: Oid = PgBuiltInOids::INT8OID.into();
        let datums: Vec<DatumWithOid<'_>> = plan
            .params
            .iter()
            .map(|id| unsafe { DatumWithOid::new(*id, int8_oid) })
            .collect();

        let mut rows: Vec<pgrx::JsonB> = Vec::new();
        plan_cache::with_plan(&plan.sql, |maybe_owned| {
            let owned = maybe_owned.expect("plan must be in cache after insert");
            let table = client
                .update(owned, None, &datums)
                .expect("sparql: prepared SELECT failed");
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
        });
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

    /// BIND with UCASE — derived text column.
    #[pg_test]
    fn sparql_bind_ucase() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:n \"Alice\" .
                 ex:b ex:n \"Bob\"   .',
                8_100)",
        )
        .unwrap();

        let row: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT ?upper
                  WHERE { ?s <http://example.com/n> \"Alice\"
                          BIND(UCASE(\"Alice\") AS ?upper) }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(row.0["upper"], "ALICE");
    }

    /// BIND with arithmetic — derived numeric column (emitted as text).
    #[pg_test]
    fn sparql_bind_arithmetic() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:age 30 .',
                8_101)",
        )
        .unwrap();

        let row: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT ?double
                  WHERE { ?s <http://example.com/age> ?age
                          BIND(?age * 2 AS ?double) }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(row.0["double"], "60");
    }

    /// BIND with CONCAT.
    #[pg_test]
    fn sparql_bind_concat() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:n \"Alice\" .',
                8_102)",
        )
        .unwrap();

        let row: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT ?greeting
                  WHERE { ?s <http://example.com/n> ?n
                          BIND(CONCAT(\"Hi, \", ?n) AS ?greeting) }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(row.0["greeting"], "Hi, Alice");
    }

    /// Arithmetic in FILTER — `?a + ?b > N`.
    #[pg_test]
    fn sparql_filter_arithmetic_add() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:price 10 ; ex:tax 2 .
                 ex:b ex:price 20 ; ex:tax 4 .
                 ex:c ex:price 100 ; ex:tax 25 .',
                8_090)",
        )
        .unwrap();

        // price + tax > 15 → b (24), c (125). 2 rows.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE {
                  ?s <http://example.com/price> ?p .
                  ?s <http://example.com/tax>   ?t
                  FILTER(?p + ?t > 15) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);
    }

    /// Arithmetic — multiplication + division.
    #[pg_test]
    fn sparql_filter_arithmetic_mul_div() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:v 4 . ex:b ex:v 5 . ex:c ex:v 10 .',
                8_091)",
        )
        .unwrap();

        // ?v * 2 > 8 → b, c. ?v / 2 < 3 → a, b.
        let mul: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/v> ?v FILTER(?v * 2 > 8) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        let div: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/v> ?v FILTER(?v / 2 < 3) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(mul, 2);
        assert_eq!(div, 2);
    }

    /// STRLEN in FILTER.
    #[pg_test]
    fn sparql_filter_strlen() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:n1 ex:label \"abc\" .
                 ex:n2 ex:label \"abcdef\" .
                 ex:n3 ex:label \"abcdefghi\" .',
                8_092)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(STRLEN(?l) > 5) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);
    }

    /// CONTAINS / STRSTARTS / STRENDS — string boundary predicates.
    #[pg_test]
    fn sparql_filter_string_predicates() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:n1 ex:label \"hello world\" .
                 ex:n2 ex:label \"goodbye world\" .
                 ex:n3 ex:label \"hello there\" .',
                8_093)",
        )
        .unwrap();

        let contains: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(CONTAINS(?l, \"hello\")) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        let starts: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(STRSTARTS(?l, \"hello\")) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        let ends: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(STRENDS(?l, \"world\")) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(contains, 2);
        assert_eq!(starts, 2);
        assert_eq!(ends, 2);
    }

    /// LANG / DATATYPE in FILTER equality.
    #[pg_test]
    fn sparql_filter_lang_and_datatype() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
                 ex:a ex:p \"plain\" .
                 ex:b ex:p \"english\"@en .
                 ex:c ex:p \"french\"@fr .
                 ex:d ex:p \"42\"^^xsd:integer .',
                8_094)",
        )
        .unwrap();

        let en_only: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/p> ?v FILTER(LANG(?v) = \"en\") }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        let integer: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
                SELECT ?s WHERE { ?s <http://example.com/p> ?v
                                  FILTER(DATATYPE(?v) = xsd:integer) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(en_only, 1);
        assert_eq!(integer, 1);
    }

    /// UCASE / LCASE — case folding for case-insensitive equality.
    #[pg_test]
    fn sparql_filter_case_fold() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p \"Alice\" .
                 ex:b ex:p \"BOB\" .
                 ex:c ex:p \"carol\" .',
                8_095)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s WHERE { ?s <http://example.com/p> ?v FILTER(LCASE(?v) = \"bob\") }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 1);
    }

    /// HAVING — post-aggregate filter on COUNT(?o) > N.
    #[pg_test]
    fn sparql_aggregate_having_count() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:1 . ex:a ex:p ex:2 . ex:a ex:p ex:3 . ex:a ex:p ex:4 .
                 ex:x ex:q ex:y .',
                8_080)",
        )
        .unwrap();

        // Per-predicate COUNT: ex:p → 4, ex:q → 1. HAVING ?n > 2 keeps ex:p.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?p (COUNT(?o) AS ?n)
                  WHERE { ?s ?p ?o }
                GROUP BY ?p HAVING (?n > 2)'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 1);
    }

    /// HAVING with SUM threshold.
    #[pg_test]
    fn sparql_aggregate_having_sum() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:age 30 . ex:b ex:age 25 . ex:c ex:age 40 . ex:d ex:age 20 .',
                8_081)",
        )
        .unwrap();

        // Only ?p with SUM > 50 survives. ex:age sum = 115, passes.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?p (SUM(?v) AS ?t)
                  WHERE { ?s ?p ?v }
                GROUP BY ?p HAVING (?t > 50)'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 1);
    }

    /// GROUP_CONCAT — string aggregation with separator.
    #[pg_test]
    fn sparql_aggregate_group_concat() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p \"x\" . ex:a ex:p \"y\" . ex:a ex:p \"z\" .',
                8_082)",
        )
        .unwrap();

        let row: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT (GROUP_CONCAT(?o; SEPARATOR=\",\") AS ?vals)
                  WHERE { ?s ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap();
        // Order isn't specified by SPARQL but our STRING_AGG with no
        // ORDER BY returns Postgres' encounter order. Sort assertion
        // to keep the test deterministic across runs.
        let mut parts: Vec<&str> = row.0["vals"].as_str().unwrap().split(',').collect();
        parts.sort();
        assert_eq!(parts, vec!["x", "y", "z"]);
    }

    /// SAMPLE returns one of the values from the group (we implement
    /// as `MIN(lexical_value)` — spec says "implementation-defined
    /// element", and MIN is a deterministic choice).
    #[pg_test]
    fn sparql_aggregate_sample() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p \"b\" . ex:a ex:p \"c\" . ex:a ex:p \"d\" .',
                8_083)",
        )
        .unwrap();

        let row: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT (SAMPLE(?o) AS ?one) WHERE { ?s ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap();
        let s = row.0["one"].as_str().unwrap().to_string();
        assert!(["b", "c", "d"].contains(&s.as_str()), "got {s}");
    }

    /// COUNT(*) — total solutions for the BGP.
    #[pg_test]
    fn sparql_aggregate_count_star() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:b .
                 ex:a ex:p ex:c .
                 ex:a ex:q ex:b .',
                8_070)",
        )
        .unwrap();

        let count: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(count.0["n"], "3");
    }

    /// COUNT(DISTINCT ?o) — count distinct object terms.
    #[pg_test]
    fn sparql_aggregate_count_distinct() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:x .
                 ex:b ex:p ex:x .
                 ex:c ex:p ex:y .',
                8_071)",
        )
        .unwrap();

        let dedup: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT (COUNT(DISTINCT ?o) AS ?n) WHERE { ?s ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(dedup.0["n"], "2", "distinct objects = x, y");
    }

    /// COUNT(?o) with GROUP BY ?p — group-and-count per predicate.
    #[pg_test]
    fn sparql_aggregate_group_by() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:x .
                 ex:b ex:p ex:y .
                 ex:c ex:q ex:z .',
                8_072)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?p (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?p'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2, "two distinct predicates ex:p and ex:q");
    }

    /// SUM over numeric values only — non-numeric literals contribute
    /// NULL (so SUM ignores them per SQL semantics).
    #[pg_test]
    fn sparql_aggregate_sum_numeric() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p 10 .
                 ex:b ex:p 20 .
                 ex:c ex:p 30 .
                 ex:d ex:p \"text\" .',
                8_073)",
        )
        .unwrap();

        let sum: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT (SUM(?v) AS ?total) WHERE { ?s <http://example.com/p> ?v }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(sum.0["total"], "60");
    }

    /// AVG over numeric values.
    #[pg_test]
    fn sparql_aggregate_avg_numeric() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p 10 .
                 ex:b ex:p 20 .
                 ex:c ex:p 30 .',
                8_074)",
        )
        .unwrap();

        let avg: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'SELECT (AVG(?v) AS ?mean) WHERE { ?s <http://example.com/p> ?v }'
             )",
        )
        .unwrap()
        .unwrap();
        // Postgres NUMERIC AVG of {10, 20, 30} = 20 (with no decimals
        // when divisor is exact). Accept either "20" or "20.000…".
        let s = avg.0["mean"].as_str().unwrap();
        let v: f64 = s.parse().unwrap();
        assert!((v - 20.0).abs() < 1e-9, "got {s}");
    }

    /// MIN / MAX — lexicographic on the lexical value.
    #[pg_test]
    fn sparql_aggregate_min_max() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:a foaf:name \"Charlie\" .
                 ex:b foaf:name \"Alice\"   .
                 ex:c foaf:name \"Bob\"     .',
                8_075)",
        )
        .unwrap();

        let min: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT (MIN(?n) AS ?lo) WHERE { ?s foaf:name ?n }'
             )",
        )
        .unwrap()
        .unwrap();
        let max: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT (MAX(?n) AS ?hi) WHERE { ?s foaf:name ?n }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(min.0["lo"], "Alice");
        assert_eq!(max.0["hi"], "Charlie");
    }

    /// GROUP BY with multiple aggregates in the same SELECT.
    #[pg_test]
    fn sparql_aggregate_multiple_in_group() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p 10 . ex:b ex:p 20 . ex:c ex:p 30 .
                 ex:d ex:q 5  . ex:e ex:q 15 .',
                8_076)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?p (COUNT(?v) AS ?n) (SUM(?v) AS ?total)
                  WHERE { ?s ?p ?v } GROUP BY ?p'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2, "ex:p (3 rows, sum 60) and ex:q (2 rows, sum 20)");
    }

    /// ASK — matching pattern returns "true", non-matching "false".
    #[pg_test]
    fn sparql_ask_matches() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:b .',
                8_120)",
        )
        .unwrap();

        let yes: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql('ASK { ?s ?p ?o }')",
        )
        .unwrap()
        .unwrap();
        let no: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'ASK { <http://example.com/zz> <http://nope/> <http://example.com/yy> }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(yes.0["_ask"], "true");
        assert_eq!(no.0["_ask"], "false");
    }

    /// ASK with FILTER. Pattern exists but filter rejects → false.
    #[pg_test]
    fn sparql_ask_with_filter() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:age 30 .',
                8_121)",
        )
        .unwrap();

        let young: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                ASK { ?s foaf:age ?a FILTER(?a > 60) }'
             )",
        )
        .unwrap()
        .unwrap();
        let any: pgrx::JsonB = Spi::get_one(
            "SELECT sparql FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                ASK { ?s foaf:age ?a FILTER(?a > 10) }'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(young.0["_ask"], "false");
        assert_eq!(any.0["_ask"], "true");
    }

    /// Multi-triple MINUS — subtract subjects that match BOTH
    /// inner triples (not either separately).
    #[pg_test]
    fn sparql_minus_multi_triple() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" ; foaf:mbox <mailto:a@x> ; foaf:age 30 .
                 ex:bob   foaf:name \"Bob\"   ; foaf:mbox <mailto:b@x> .
                 ex:carol foaf:name \"Carol\" ; foaf:age 25 .
                 ex:dave  foaf:name \"Dave\"  .',
                8_110)",
        )
        .unwrap();

        // MINUS { ?s foaf:mbox _ . ?s foaf:age _ } subtracts subjects
        // that have BOTH. Only alice has both → 3 survive.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s
                  WHERE { ?s foaf:name ?n
                          MINUS { ?s foaf:mbox ?m . ?s foaf:age ?a } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 3);
    }

    /// MINUS — subtracts solutions of the right pattern from the
    /// left, keyed on shared variables. Alice has both name + mbox;
    /// Bob has only name. MINUS { ?s foaf:mbox ?m } drops Alice.
    #[pg_test]
    fn sparql_minus_basic() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" ; foaf:mbox <mailto:a@x> .
                 ex:bob   foaf:name \"Bob\"   .
                 ex:carol foaf:name \"Carol\" ; foaf:mbox <mailto:c@x> .',
                8_060)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n
                  WHERE { ?s foaf:name ?n
                          MINUS { ?s foaf:mbox ?m } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        // Alice and Carol have mbox → dropped. Only Bob remains.
        assert_eq!(rows, 1);
    }

    /// MINUS with no shared variables is a no-op per spec.
    /// Querying for ?s ?p ?o MINUS { ?x ex:other ?y } subtracts
    /// nothing because there's no shared variable.
    #[pg_test]
    fn sparql_minus_no_shared_vars_is_noop() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:b .
                 ex:c ex:p ex:d .',
                8_061)",
        )
        .unwrap();

        let with_minus: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?p ?o WHERE { ?s ?p ?o
                                        MINUS { ?x <http://example.com/other> ?y } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        let without_minus: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?p ?o WHERE { ?s ?p ?o }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(with_minus, without_minus);
    }

    /// Two chained MINUS clauses each emit a NOT EXISTS predicate.
    #[pg_test]
    fn sparql_minus_chained() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" ; foaf:mbox <mailto:a@x> ; foaf:age 30 .
                 ex:bob   foaf:name \"Bob\"   ; foaf:age 25 .
                 ex:carol foaf:name \"Carol\" ; foaf:mbox <mailto:c@x> .
                 ex:dave  foaf:name \"Dave\"  .',
                8_062)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s
                  WHERE { ?s foaf:name ?n
                          MINUS { ?s foaf:mbox ?m }
                          MINUS { ?s foaf:age  ?a } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        // Alice has mbox → drop. Bob has age → drop. Carol has mbox → drop.
        // Dave has neither → keep. 1 row.
        assert_eq!(rows, 1);
    }

    /// MINUS combined with FILTER on the outer query.
    #[pg_test]
    fn sparql_minus_with_outer_filter() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" .
                 ex:bob   foaf:name \"Bob\"   ; foaf:mbox <mailto:b@x> .
                 ex:carol foaf:name \"Carol\" .
                 ex:dave  foaf:name \"Dave\"  .',
                8_063)",
        )
        .unwrap();

        // Persons whose name starts with letter > "B" and who DON'T have mbox.
        // Names without mbox: Alice, Carol, Dave. Names > "B": Carol, Dave.
        // → 2 rows.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s
                  WHERE { ?s foaf:name ?n
                          MINUS { ?s foaf:mbox ?m }
                          FILTER(REGEX(?n, \"^[C-Z]\")) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);
    }

    /// UNION combines two BGPs over the same subject column.
    /// Both `foaf:name` and `foaf:nick` are name-shaped; UNION
    /// gives back any subject that has at least one.
    #[pg_test]
    fn sparql_union_basic() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" .
                 ex:bob   foaf:nick \"Bobby\" .
                 ex:carol foaf:name \"Carol\" ; foaf:nick \"C\" .',
                8_050)",
        )
        .unwrap();

        // Alice: name only. Bob: nick only. Carol: both (so 2 union rows).
        // Total: 4.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n
                  WHERE { { ?s foaf:name ?n }
                          UNION
                          { ?s foaf:nick ?n } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 4);
    }

    /// UNION where branches bind DIFFERENT variables. ?n only in
    /// the left branch, ?m only in the right. The other branch
    /// emits NULL for the missing var.
    #[pg_test]
    fn sparql_union_different_vars() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" .
                 ex:bob   foaf:mbox <mailto:b@x> .',
                8_051)",
        )
        .unwrap();

        // 1 row from each branch. The ?m column is NULL on row 1,
        // the ?n column is NULL on row 2.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n ?m
                  WHERE { { ?s foaf:name ?n }
                          UNION
                          { ?s foaf:mbox ?m } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2);

        // One row has ?n NULL (Bob's mbox), one has ?m NULL (Alice's name).
        let n_null: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n ?m
                  WHERE { { ?s foaf:name ?n } UNION { ?s foaf:mbox ?m } }'
             ) WHERE sparql->'n' = 'null'::jsonb",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(n_null, 1);
    }

    /// Three-way UNION — chained `A UNION B UNION C` flattens to
    /// three branches via collect_union_branches.
    #[pg_test]
    fn sparql_union_three_branches() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:x .
                 ex:b ex:q ex:y .
                 ex:c ex:r ex:z .',
                8_052)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s
                  WHERE { { ?s <http://example.com/p> ?o }
                          UNION
                          { ?s <http://example.com/q> ?o }
                          UNION
                          { ?s <http://example.com/r> ?o } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 3);
    }

    /// UNION + DISTINCT — branch 1 and branch 2 both emit Alice;
    /// DISTINCT dedups.
    #[pg_test]
    fn sparql_union_with_distinct() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" ; foaf:nick \"Alice\" .',
                8_053)",
        )
        .unwrap();

        // Without DISTINCT: 2 rows (?s=alice from each branch).
        // With DISTINCT on ?s,?n: 1 row.
        let raw: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n
                  WHERE { { ?s foaf:name ?n } UNION { ?s foaf:nick ?n } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        let dedup: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT DISTINCT ?s ?n
                  WHERE { { ?s foaf:name ?n } UNION { ?s foaf:nick ?n } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(raw, 2);
        assert_eq!(dedup, 1);
    }

    /// UNION + ORDER BY + LIMIT on the outer wrapper. Must
    /// reference a projected variable.
    #[pg_test]
    fn sparql_union_with_order_limit() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" .
                 ex:carol foaf:nick \"Carol\" .
                 ex:bob   foaf:nick \"Bobby\" .',
                8_054)",
        )
        .unwrap();

        // 3 rows total from UNION; ORDER BY ?n ASC, LIMIT 1 → Alice.
        let first: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?n
                  WHERE { { ?s foaf:name ?n } UNION { ?s foaf:nick ?n } }
                ORDER BY ?n LIMIT 1'
             )",
        )
        .unwrap()
        .unwrap();
        assert_eq!(first.0["n"], "Alice");
    }

    /// OPTIONAL — bind ?mbox when present, NULL otherwise. Bob has
    /// foaf:name but no foaf:mbox; we should still see his row with
    /// `mbox` as JSON null.
    #[pg_test]
    fn sparql_optional_simple() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" ; foaf:mbox <mailto:a@x> .
                 ex:bob   foaf:name \"Bob\"                            .
                 ex:carol foaf:name \"Carol\" ; foaf:mbox <mailto:c@x> .',
                8_040)",
        )
        .unwrap();

        // 3 people total; LEFT JOIN keeps Bob.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n ?m
                  WHERE { ?s foaf:name ?n
                          OPTIONAL { ?s foaf:mbox ?m } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 3, "expected 3 rows (Alice, Bob, Carol), got {rows}");

        // Bob's row has ?m as NULL.
        let rows_with_null_mbox: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n ?m
                  WHERE { ?s foaf:name ?n
                          OPTIONAL { ?s foaf:mbox ?m } }'
             ) WHERE sparql->'m' = 'null'::jsonb",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows_with_null_mbox, 1);
    }

    /// OPTIONAL with FILTER inside — Bob's missing mbox stays NULL;
    /// the filter only prunes the OPTIONAL match, not the row itself.
    #[pg_test]
    fn sparql_optional_with_inner_filter() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:x . ex:a ex:age 25 .
                 ex:b ex:p ex:y . ex:b ex:age 35 .
                 ex:c ex:p ex:z .',
                8_041)",
        )
        .unwrap();

        // 3 ?s ?p ?o rows; OPTIONAL ages with filter `?age >= 30`
        // only adds the b->35 binding. a's age (25) doesn't match
        // → ?age NULL on a. c has no age at all → ?age NULL on c.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?o ?age
                  WHERE { ?s <http://example.com/p> ?o
                          OPTIONAL { ?s <http://example.com/age> ?age FILTER(?age >= 30) } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 3);

        // Only one row should have a non-null age (b=35).
        let rows_with_age: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?o ?age
                  WHERE { ?s <http://example.com/p> ?o
                          OPTIONAL { ?s <http://example.com/age> ?age FILTER(?age >= 30) } }'
             ) WHERE sparql->'age' != 'null'::jsonb",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows_with_age, 1);
    }

    /// Multiple chained OPTIONALs — each becomes its own LEFT JOIN.
    /// Alice has both extras, Bob has none.
    #[pg_test]
    fn sparql_optional_chained() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice foaf:name \"Alice\" ; foaf:mbox <mailto:a@x> ; foaf:age 30 .
                 ex:bob   foaf:name \"Bob\"   .',
                8_042)",
        )
        .unwrap();

        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
                SELECT ?s ?n ?m ?a
                  WHERE { ?s foaf:name ?n
                          OPTIONAL { ?s foaf:mbox ?m }
                          OPTIONAL { ?s foaf:age  ?a } }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 2, "Alice and Bob, both rows survive LEFT JOINs");
    }

    /// OPTIONAL combined with outer FILTER. The outer filter prunes
    /// rows where the OPTIONAL var is NULL.
    #[pg_test]
    fn sparql_optional_with_outer_filter() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p ex:x . ex:a ex:q ex:y .
                 ex:b ex:p ex:x .',
                8_043)",
        )
        .unwrap();

        // Without the outer filter: a + b both come back (LEFT JOIN
        // preserves b). With outer FILTER(BOUND(?r)) → only a.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(
               'SELECT ?s ?r
                  WHERE { ?s <http://example.com/p> ?o
                          OPTIONAL { ?s <http://example.com/q> ?r }
                          FILTER(BOUND(?r)) }'
             )",
        )
        .unwrap()
        .unwrap_or(0);
        // FILTER(BOUND(?r)) prunes b's row (no q match → ?r NULL).
        assert_eq!(rows, 1);
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
