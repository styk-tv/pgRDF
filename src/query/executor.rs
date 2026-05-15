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
//! Constants are resolved to dictionary ids up-front and bound as
//! positional `$N` parameters (LLD §4.2) — the generated SQL never
//! carries user-supplied IRI strings or inlined dict ids, which is
//! how the translator avoids SQL injection while still building the
//! query shape at function-call time.
//!
//! Scope today:
//!   * SELECT and ASK. CONSTRUCT / DESCRIBE remain unsupported
//!     (CONSTRUCT lands in v0.4 — see SPEC.pgRDF.LLD.v0.4.md §6).
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
//!     `SUM(?v)`, `AVG(?v)`, `MIN(?v)`, `MAX(?v)`,
//!     `GROUP_CONCAT(?v [; SEPARATOR = "…"])` (via Postgres
//!     `string_agg`), `SAMPLE(?v)` (`MIN(...)` surrogate), with or
//!     without `GROUP BY ?vars`. SUM/AVG are numeric-aware
//!     (non-numeric literals contribute NULL); MIN/MAX are
//!     type-aware (numeric on numeric datatypes, lexical otherwise).
//!     HAVING is supported — filters that name an aggregate output
//!     migrate from WHERE to HAVING during `parse_select`.
//!     Aggregate output values come back as strings in the JSONB
//!     row (consistent with the rest of the surface).
//!   * `BIND(expr AS ?v)` — captured as a BindSpec and rendered as
//!     an extra SELECT-list column via `translate_bind_expression`.
//!     Filtering on a BIND output is not yet supported.
//!   * `GRAPH <iri> { … }` — literal-IRI named-graph scope.
//!     Translate-time IRI → `graph_id` resolution via
//!     `_pgrdf_graphs.iri`; unresolved IRI binds to the sentinel
//!     `-1` (zero rows, spec-correct "no solutions").
//!   * `GRAPH ?g { … }` — variable named-graph scope.
//!     `build_from_and_where` JOINs `_pgrdf_graphs g{scope_id}` and
//!     constrains every triple alias inside the GRAPH block to that
//!     join's `graph_id`. INNER JOIN matches W3C SPARQL 1.1 §13.3
//!     semantics — only graphs in the IRI mapping bind ?g.
//!   * GRAPH composition (slice 112). The graph constraint is
//!     PER-PATTERN, not per-query. A `GraphScope` is attached to each
//!     triple pattern, each OPTIONAL triple, and each MINUS block.
//!     GRAPH inside OPTIONAL / UNION / MINUS scopes only those
//!     triples; OPTIONAL / MINUS inside a GRAPH inherits the outer
//!     scope. Distinct GRAPH blocks get distinct scope ids; multiple
//!     blocks binding the same `?g` variable are tied together with a
//!     `graph_id` equality so the projected variable stays consistent.
//!   * Property paths, inline VALUES, SERVICE remain unsupported
//!     (paths in v0.4 §7; VALUES + DESCRIBE in §4-deferred backlog).
//!
//! Output shape:
//!   `SETOF JSONB` — one row per solution, keys = projected variable
//!   names, values = lexical strings (NULL when the binding maps to
//!   a term id missing from the dictionary).

use crate::query::plan_cache;
use crate::storage::dict::{put_term_full, term_type};
use pgrx::datum::DatumWithOid;
use pgrx::iter::SetOfIterator;
use pgrx::pg_sys::{Oid, PgBuiltInOids};
use pgrx::prelude::*;
use serde_json::{json, Map, Value};
use spargebra::algebra::{
    AggregateExpression, AggregateFunction, Expression, Function, GraphPattern, OrderExpression,
};
use spargebra::term::{
    GraphName, GraphNamePattern, GroundTerm, Literal, NamedNodePattern, NamedOrBlankNode,
    QuadPattern, Term, TermPattern, TriplePattern,
};
use spargebra::{GraphUpdateOperation, Query, SparqlParser, Update};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

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

/// Execute a SPARQL query (SELECT / ASK / UPDATE) and return one JSONB
/// row per solution. For SELECT/ASK each row is a JSON object keyed by
/// the projected variable names (ASK uses the `_ask` sentinel key).
/// For UPDATE forms (Phase C slice 84+) a single summary row of shape
/// `{"_update": {...}}` is returned — see LLD v0.4 §4.2.
///
/// SQL surface: `pgrdf.sparql(q TEXT) → SETOF JSONB`.
///
/// Invocation typically looks like:
///
/// ```sql
/// SELECT * FROM pgrdf.sparql('SELECT ?s ?p WHERE { ?s foaf:name ?p }');
///   →  {"s": "http://example.com/alice", "p": "Alice"}
///      {"s": "http://example.com/bob",   "p": "Bob"}
///
/// SELECT * FROM pgrdf.sparql('INSERT DATA { <s> <p> <o> }');
///   →  {"_update": {"form": "INSERT_DATA", "triples_inserted": 1, …}}
/// ```
///
/// Detection strategy: try `parse_query` first (SELECT / ASK / CONSTRUCT
/// / DESCRIBE). On parse failure, retry as `parse_update` (the UPDATE
/// surface — INSERT DATA, DELETE DATA, DELETE/INSERT WHERE, lifecycle
/// algebra). If both fail, propagate the *query* parser error message
/// via the stable `sparql: parse error:` prefix (slice #63 contract).
/// This ordering keeps current SELECT/ASK behaviour untouched.
#[pg_extern]
fn sparql(query: &str) -> SetOfIterator<'static, pgrx::JsonB> {
    let parser = SparqlParser::new();
    match parser.parse_query(query) {
        Ok(parsed) => {
            let plan = translate(&parsed);
            let rows = execute(&plan);
            SetOfIterator::new(rows.into_iter())
        }
        Err(query_err) => {
            // Fallback: maybe it's an UPDATE form. spargebra splits
            // the SPARQL grammar — query forms vs. update forms — into
            // two parse entry points, so we have to try both before
            // declaring the input invalid. Build a fresh parser; the
            // builder isn't `Copy` and the previous `parse_query` call
            // consumed it.
            match SparqlParser::new().parse_update(query) {
                Ok(update) => {
                    let rows = execute_update(&update);
                    SetOfIterator::new(rows.into_iter())
                }
                // Surface the *query*-side parse error — that's the
                // existing slice #63 contract scraped by downstream
                // tooling. The update-side error is informational and
                // would only confuse callers who wrote a malformed
                // SELECT.
                Err(_) => panic!("sparql: parse error: {query_err}"),
            }
        }
    }
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

/// Slice 112: per-pattern graph scope. A `GraphScope` decorates each
/// triple pattern, each OPTIONAL triple, and each MINUS block so the
/// SQL builder can emit the correct `graph_id` constraint for that
/// alias independently — composing GRAPH with OPTIONAL / UNION /
/// MINUS arrives by attaching the scope at the triple level rather
/// than the query level.
///
/// `Literal(id)` is the literal-IRI form (slice 114): the graph_id
/// resolved at translate time via `_pgrdf_graphs.iri`. Each triple
/// in scope gets `qN.graph_id = $K`.
///
/// `Variable { name, scope_id }` is the variable form (slice 113):
/// `name` is the SPARQL variable bound to the graph IRI, and
/// `scope_id` is a fresh integer per GRAPH block instance. Triples
/// in the same scope share a JOIN to `pgrdf._pgrdf_graphs g{scope_id}`
/// and equate their `graph_id` to that join's `graph_id`. Multiple
/// scopes binding the same variable name get tied together with
/// `g{scope_a}.graph_id = g{scope_b}.graph_id` so the projected
/// `?g` is consistent across them.
#[derive(Clone, Debug)]
enum GraphScope {
    Literal(i64),
    Variable { name: String, scope_id: usize },
}

/// A triple pattern with optional graph scope. Used by mandatory
/// BGPs (one per alias) and by the MINUS triple list (one per
/// triple — the GRAPH block can wrap part of the MINUS body).
#[derive(Clone)]
struct ScopedTriple {
    triple: TriplePattern,
    scope: Option<GraphScope>,
}

/// A MINUS block — a list of triples plus the (shared, since MINUS
/// today is a single inner BGP) graph scope. When the entire MINUS
/// sits inside a `GRAPH <iri> { MINUS { … } }`, every triple in the
/// block inherits the scope. The block-level field is the form
/// slice 112 needs; the per-triple form will only diverge when MINUS
/// admits nested GRAPH blocks (deferred).
#[derive(Clone)]
struct MinusBlock {
    triples: Vec<TriplePattern>,
    scope: Option<GraphScope>,
}

#[derive(Default)]
struct ParsedSelect {
    projected: Vec<String>,
    // Single-branch state. Empty when union_branches is populated.
    bgp: Vec<ScopedTriple>,
    filters: Vec<Expression>,
    /// Each OPTIONAL block — a single triple pattern plus its
    /// optional FILTER. Multiple chained OPTIONALs land here in
    /// left-to-right order; build_bgp_sql emits one LEFT JOIN per.
    optionals: Vec<OptionalBlock>,
    /// Each MINUS block — a list of triple patterns. Translates
    /// to `WHERE NOT EXISTS (SELECT 1 FROM q_min_1, q_min_2, …
    /// WHERE join-shared-vars AND inner-pattern-predicates)`.
    /// Elided if there are no shared vars (SPARQL no-op).
    minuses: Vec<MinusBlock>,
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
    /// Slice 112: monotonically-increasing scope-id counter used to
    /// mint a fresh `Variable { scope_id }` whenever the walk enters
    /// a new `GRAPH ?g { … }` block. Held at the query level so the
    /// alias numbering inside `build_from_and_where` is stable per
    /// parse — every Variable scope across UNION branches, OPTIONAL,
    /// and MINUS blocks gets a distinct integer.
    graph_scope_counter: usize,
}

struct OptionalBlock {
    triple: TriplePattern,
    /// The filter inside `OPTIONAL { … FILTER(...) }`, if any.
    /// Translated into the LEFT JOIN's ON clause so rejected rows
    /// still survive with the optional variables NULL.
    filter: Option<Expression>,
    /// Slice 112: graph scope that applies to this OPTIONAL's
    /// triple alias. `None` (= scan every partition) when the
    /// OPTIONAL sits outside any GRAPH; carries the scope of the
    /// enclosing `GRAPH <iri> { … }` or `GRAPH ?g { … }` when the
    /// OPTIONAL nests inside one, OR a brand-new scope when the
    /// OPTIONAL itself is `OPTIONAL { GRAPH … { … } }`.
    scope: Option<GraphScope>,
}

struct BindSpec {
    output_var: String,
    expression: Expression,
}

struct AggregateSpec {
    /// The user-facing SPARQL variable that holds this aggregate's
    /// output (post-Extend rename). Starts as the synthetic
    /// `$agg_N` / hex-blob name spargebra emits in the algebra
    /// synthesis layer; `Extend` rewrites it to the AS-alias.
    output_var: String,
    /// Every variable name spargebra used internally to reference
    /// this aggregate, including the original synthetic name (kept
    /// even after `Extend` renames `output_var`). Inline aggregates
    /// in HAVING (`HAVING(SUM(?v) > c)`) reference the synthetic
    /// name rather than the user alias — without this list the
    /// HAVING filter wouldn't find its aggregate and would fall
    /// through to the non-aggregate-aware FILTER translator,
    /// producing "FILTER expression not translatable".
    synth_aliases: Vec<String>,
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
    GroupConcat {
        separator: String,
    },
    /// SAMPLE(?v) — "any value from the group". Postgres has no
    /// SAMPLE; we use `MIN(...)` as a deterministic surrogate which
    /// is spec-conformant ("an implementation-defined element").
    Sample,
}

#[derive(Default)]
struct UnionBranch {
    bgp: Vec<ScopedTriple>,
    filters: Vec<Expression>,
    optionals: Vec<OptionalBlock>,
    minuses: Vec<MinusBlock>,
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
                let (from_sql, where_clauses, _plan) = build_from_and_where(
                    &b.bgp,
                    &b.filters,
                    &b.optionals,
                    &b.minuses,
                    &mut anchors,
                    0,
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
    let (from_sql, where_clauses, _plan) = build_from_and_where(
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
        // An aggregate is "referenced" by either its user-facing
        // output_var (post-Extend rename) OR any of the synthetic
        // names spargebra used internally — inline aggregates in
        // HAVING fall into the latter case.
        let agg_names: Vec<String> = ps
            .aggregates
            .iter()
            .flat_map(|a| {
                std::iter::once(a.output_var.clone()).chain(a.synth_aliases.iter().cloned())
            })
            .collect();
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
                for st in &branch.bgp {
                    push_unique(&mut ps.projected, tp_subject_var(&st.triple));
                    push_unique(&mut ps.projected, tp_predicate_var(&st.triple));
                    push_unique(&mut ps.projected, tp_object_var(&st.triple));
                    // Slice 112: SELECT * surfaces the graph var
                    // bound by any Variable scope inside the branch,
                    // even when the inner BGP never anchors it.
                    push_unique(&mut ps.projected, scope_var_name(&st.scope));
                }
                for opt in &branch.optionals {
                    push_unique(&mut ps.projected, tp_subject_var(&opt.triple));
                    push_unique(&mut ps.projected, tp_predicate_var(&opt.triple));
                    push_unique(&mut ps.projected, tp_object_var(&opt.triple));
                    push_unique(&mut ps.projected, scope_var_name(&opt.scope));
                }
                for m in &branch.minuses {
                    push_unique(&mut ps.projected, scope_var_name(&m.scope));
                }
            }
        } else {
            for st in &ps.bgp {
                push_unique(&mut ps.projected, tp_subject_var(&st.triple));
                push_unique(&mut ps.projected, tp_predicate_var(&st.triple));
                push_unique(&mut ps.projected, tp_object_var(&st.triple));
                push_unique(&mut ps.projected, scope_var_name(&st.scope));
            }
            for opt in &ps.optionals {
                push_unique(&mut ps.projected, tp_subject_var(&opt.triple));
                push_unique(&mut ps.projected, tp_predicate_var(&opt.triple));
                push_unique(&mut ps.projected, tp_object_var(&opt.triple));
                push_unique(&mut ps.projected, scope_var_name(&opt.scope));
            }
            for m in &ps.minuses {
                push_unique(&mut ps.projected, scope_var_name(&m.scope));
            }
        }
    }
    ps
}

/// Return the SPARQL variable name bound by a Variable scope, if any.
/// Helper for the SELECT * projection enrichment in `parse_select`.
fn scope_var_name(scope: &Option<GraphScope>) -> Option<String> {
    match scope {
        Some(GraphScope::Variable { name, .. }) => Some(name.clone()),
        _ => None,
    }
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
/// AVG(?v), MIN(?v), MAX(?v), GROUP_CONCAT(?v [; SEPARATOR = "…"]),
/// SAMPLE(?v). Custom IRI aggregates panic.
fn parse_aggregate(synth_var: &str, agg: &AggregateExpression) -> AggregateSpec {
    match agg {
        AggregateExpression::CountSolutions { distinct } => AggregateSpec {
            output_var: synth_var.to_string(),
            synth_aliases: vec![synth_var.to_string()],
            func: AggregateFn::Count,
            distinct: *distinct,
            arg_var: None,
        },
        AggregateExpression::FunctionCall {
            name,
            expr,
            distinct,
        } => {
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
                synth_aliases: vec![synth_var.to_string()],
                func,
                distinct: *distinct,
                arg_var: Some(arg_var),
            }
        }
    }
}

/// Walk a single UNION branch. `current_scope` is the GRAPH scope in
/// effect for triples discovered at this level (inherited from an
/// enclosing GRAPH block, if any) — `None` at the branch root.
fn walk_branch(
    p: &GraphPattern,
    ub: &mut UnionBranch,
    current_scope: Option<&GraphScope>,
    scope_counter: &mut usize,
) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                ub.bgp.push(ScopedTriple {
                    triple: tp.clone(),
                    scope: current_scope.cloned(),
                });
            }
        }
        GraphPattern::Filter { expr, inner } => {
            ub.filters.push(expr.clone());
            walk_branch(inner, ub, current_scope, scope_counter);
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            walk_branch(left, ub, current_scope, scope_counter);
            // The right arm of OPTIONAL today is a single triple, OR
            // a single GRAPH wrapping a single triple (slice 112).
            let (triple, opt_scope) = extract_optional_triple(right, current_scope, scope_counter);
            ub.optionals.push(OptionalBlock {
                triple,
                filter: expression.clone(),
                scope: opt_scope,
            });
        }
        GraphPattern::Minus { left, right } => {
            walk_branch(left, ub, current_scope, scope_counter);
            let (triples, minus_scope) = extract_minus_triples(right, current_scope, scope_counter);
            ub.minuses.push(MinusBlock {
                triples,
                scope: minus_scope,
            });
        }
        GraphPattern::Graph { name, inner } => {
            // `GRAPH` block inside a UNION branch. Mint a scope and
            // walk the inner with it bound — every triple discovered
            // inside this block carries the scope. Note: the inner
            // may itself contain LeftJoin / Minus, in which case the
            // OPTIONAL / MINUS triples inherit this scope (W3C §13.3).
            let scope = make_scope(name, scope_counter);
            walk_branch(inner, ub, Some(&scope), scope_counter);
        }
        other => panic!("sparql: unsupported algebra inside UNION branch: {other:?}"),
    }
}

/// Mint a GraphScope from a SPARQL `GraphPattern::Graph` name. Literal
/// IRI → translate-time resolution to a `graph_id`; variable → a fresh
/// scope_id paired with the variable name.
fn make_scope(name: &NamedNodePattern, scope_counter: &mut usize) -> GraphScope {
    match name {
        NamedNodePattern::NamedNode(node) => {
            let iri = node.as_str().to_string();
            let resolved = lookup_graph_id(&iri).unwrap_or(-1);
            GraphScope::Literal(resolved)
        }
        NamedNodePattern::Variable(v) => {
            *scope_counter += 1;
            GraphScope::Variable {
                name: v.as_str().to_string(),
                scope_id: *scope_counter,
            }
        }
    }
}

/// Extract the single triple of an OPTIONAL's right arm, peeling off
/// a wrapping `GRAPH` to produce the triple's scope. The wrapping
/// scope (if any) wins over the surrounding `current_scope` — that's
/// the `OPTIONAL { GRAPH <g> { ?s :p ?o } }` shape.
fn extract_optional_triple(
    right: &GraphPattern,
    current_scope: Option<&GraphScope>,
    scope_counter: &mut usize,
) -> (TriplePattern, Option<GraphScope>) {
    match right {
        GraphPattern::Graph { name, inner } => {
            let scope = make_scope(name, scope_counter);
            // Inner of an `OPTIONAL { GRAPH … { … } }` is a single-triple BGP.
            match inner.as_ref() {
                GraphPattern::Bgp { patterns } if patterns.len() == 1 => {
                    (patterns[0].clone(), Some(scope))
                }
                GraphPattern::Bgp { patterns } => panic!(
                    "sparql: OPTIONAL today only supports a single triple pattern (got {} triples)",
                    patterns.len()
                ),
                other => panic!(
                    "sparql: OPTIONAL today only supports a single triple pattern (got {other:?})"
                ),
            }
        }
        GraphPattern::Bgp { patterns } if patterns.len() == 1 => {
            (patterns[0].clone(), current_scope.cloned())
        }
        GraphPattern::Bgp { patterns } => panic!(
            "sparql: OPTIONAL today only supports a single triple pattern (got {} triples)",
            patterns.len()
        ),
        other => {
            panic!("sparql: OPTIONAL today only supports a single triple pattern (got {other:?})")
        }
    }
}

/// Extract the triple list of a MINUS's right arm, peeling off a
/// wrapping `GRAPH` to produce the block's scope. As with OPTIONAL,
/// `MINUS { GRAPH <g> { … } }` overrides the surrounding scope.
fn extract_minus_triples(
    right: &GraphPattern,
    current_scope: Option<&GraphScope>,
    scope_counter: &mut usize,
) -> (Vec<TriplePattern>, Option<GraphScope>) {
    match right {
        GraphPattern::Graph { name, inner } => {
            let scope = make_scope(name, scope_counter);
            match inner.as_ref() {
                GraphPattern::Bgp { patterns } => (patterns.clone(), Some(scope)),
                _ => panic!("sparql: MINUS right side must be a BGP"),
            }
        }
        GraphPattern::Bgp { patterns } => (patterns.clone(), current_scope.cloned()),
        _ => panic!("sparql: MINUS right side must be a BGP"),
    }
}

fn walk_select(p: &GraphPattern, ps: &mut ParsedSelect) {
    walk_select_scoped(p, ps, None);
}

/// Walk the SELECT pattern with the GRAPH scope currently in effect
/// (inherited from an enclosing `GRAPH … { … }`, if any). The scope
/// is `None` at the query root and propagates down through Filter /
/// LeftJoin's left arm / Minus's left arm / etc.
fn walk_select_scoped(p: &GraphPattern, ps: &mut ParsedSelect, current_scope: Option<&GraphScope>) {
    match p {
        GraphPattern::Project { inner, variables } => {
            if ps.projected.is_empty() {
                ps.projected = variables.iter().map(|v| v.as_str().to_string()).collect();
            }
            walk_select_scoped(inner, ps, current_scope);
        }
        GraphPattern::Distinct { inner } | GraphPattern::Reduced { inner } => {
            ps.distinct = true;
            walk_select_scoped(inner, ps, current_scope);
        }
        GraphPattern::Slice {
            inner,
            start,
            length,
        } => {
            ps.offset = *start;
            ps.limit = *length;
            walk_select_scoped(inner, ps, current_scope);
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
            walk_select_scoped(inner, ps, current_scope);
        }
        GraphPattern::Filter { expr, inner } => {
            ps.filters.push(expr.clone());
            walk_select_scoped(inner, ps, current_scope);
        }
        GraphPattern::LeftJoin {
            left,
            right,
            expression,
        } => {
            // Walk the left arm first — it may itself be another
            // LeftJoin (chained OPTIONALs) or a Filter wrapping a BGP.
            walk_select_scoped(left, ps, current_scope);
            // OPTIONAL's right arm: peel a wrapping GRAPH to get the
            // optional's effective scope. `OPTIONAL { GRAPH … { … } }`
            // overrides the outer scope; plain `OPTIONAL { … }`
            // inherits `current_scope` (W3C §13.3).
            let (triple, opt_scope) =
                extract_optional_triple(right, current_scope, &mut ps.graph_scope_counter);
            ps.optionals.push(OptionalBlock {
                triple,
                filter: expression.clone(),
                scope: opt_scope,
            });
        }
        GraphPattern::Union { left, right } => {
            // Chained `A UNION B UNION C` arrives as a left-leaning
            // Union tree; flatten so every leaf becomes its own branch.
            // Each branch carries its own scope state — when the UNION
            // sits INSIDE a GRAPH the scope inherits into every branch.
            collect_union_branches_scoped(
                left,
                &mut ps.union_branches,
                current_scope,
                &mut ps.graph_scope_counter,
            );
            collect_union_branches_scoped(
                right,
                &mut ps.union_branches,
                current_scope,
                &mut ps.graph_scope_counter,
            );
        }
        GraphPattern::Minus { left, right } => {
            walk_select_scoped(left, ps, current_scope);
            let (triples, minus_scope) =
                extract_minus_triples(right, current_scope, &mut ps.graph_scope_counter);
            ps.minuses.push(MinusBlock {
                triples,
                scope: minus_scope,
            });
        }
        GraphPattern::Group {
            inner,
            variables,
            aggregates,
        } => {
            for v in variables {
                ps.group_vars.push(v.as_str().to_string());
            }
            for (synth_var, agg_expr) in aggregates {
                ps.aggregates
                    .push(parse_aggregate(synth_var.as_str(), agg_expr));
            }
            walk_select_scoped(inner, ps, current_scope);
        }
        GraphPattern::Extend {
            inner,
            variable,
            expression,
        } => {
            walk_select_scoped(inner, ps, current_scope);
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
            for tp in patterns {
                ps.bgp.push(ScopedTriple {
                    triple: tp.clone(),
                    scope: current_scope.cloned(),
                });
            }
        }
        GraphPattern::Graph { name, inner } => {
            // Slice 112: a GRAPH block mints a scope and walks
            // `inner` with that scope bound — every triple emerging
            // from the walk carries the scope, including triples
            // inside an OPTIONAL / MINUS that is itself inside the
            // GRAPH block. Nested GRAPH blocks override per W3C §13.3.
            let scope = make_scope(name, &mut ps.graph_scope_counter);
            walk_select_scoped(inner, ps, Some(&scope));
        }
        other => panic!("sparql: unsupported algebra in select wrapper: {other:?}"),
    }
}

/// Union flattener that propagates the GRAPH scope down to each
/// branch (so `GRAPH <g> { { ... } UNION { ... } }` works).
fn collect_union_branches_scoped(
    p: &GraphPattern,
    out: &mut Vec<UnionBranch>,
    current_scope: Option<&GraphScope>,
    scope_counter: &mut usize,
) {
    match p {
        GraphPattern::Union { left, right } => {
            collect_union_branches_scoped(left, out, current_scope, scope_counter);
            collect_union_branches_scoped(right, out, current_scope, scope_counter);
        }
        _ => {
            let mut ub = UnionBranch::default();
            walk_branch(p, &mut ub, current_scope, scope_counter);
            out.push(ub);
        }
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
    let (from_sql, where_clauses, plan) = build_from_and_where(
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
            let expr_sql =
                translate_bind_expression(&bind.expression, &anchors).unwrap_or_else(|| {
                    panic!(
                        "sparql: BIND expression for ?{var} not translatable: {:?}",
                        bind.expression
                    )
                });
            select_clauses.push(format!("{expr_sql} AS {}", quote_identifier(var)));
            continue;
        }
        // Slice 112: a projected variable bound by `GRAPH ?g { … }`
        // emits the IRI from the `g{scope_id}` JOIN added in
        // build_from_and_where. The anchor scope is the FIRST scope
        // (mandatory before optional) that binds the variable.
        if let Some(scope_id) = plan.projection_scope(var) {
            select_clauses.push(format!("g{scope_id}.iri AS {}", quote_identifier(var)));
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
            // Slice 112: ORDER BY ?g on a graph-var emits a hidden
            // trailing column pulling the anchor scope's g{S}.iri.
            if let Some(scope_id) = plan.projection_scope(var) {
                select_clauses.push(format!("g{scope_id}.iri AS _pgrdf_order_{idx}"));
                select_clauses.len()
            } else {
                let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
                    panic!("sparql: ORDER BY variable ?{var} not bound in any BGP pattern")
                });
                select_clauses.push(format!(
                    "(SELECT lexical_value FROM pgrdf._pgrdf_dictionary
                       WHERE id = q{alias_idx}.{col}) AS _pgrdf_order_{idx}",
                ));
                select_clauses.len()
            }
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
    let (from_sql, where_clauses, plan) = build_from_and_where(
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
    // Slice 112: a GROUP BY ?g where ?g is the variable bound by a
    // `GRAPH ?g { … }` block resolves to that scope's `g{S}.iri`
    // rather than a dict lookup.
    let mut group_exprs: Vec<(String, String)> = Vec::new(); // (var, sql-expr)
    for var in &ps.group_vars {
        if let Some(scope_id) = plan.projection_scope(var) {
            group_exprs.push((var.clone(), format!("g{scope_id}.iri")));
            continue;
        }
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
            let sql_pred =
                translate_filter_with_aggregates(expr, &anchors, &ps.aggregates, &group_exprs)
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
        format!("(SELECT lexical_value FROM pgrdf._pgrdf_dictionary WHERE id = q{alias_idx}.{col})")
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
        // Type-aware MIN/MAX: SPARQL 1.1 §17.4 says the
        // aggregate uses the value's "natural order" — numeric
        // for xsd:numeric literals, lexicographic otherwise.
        // COALESCE picks numeric when any row in the group has a
        // numeric datatype (numeric_cast_subselect returns NULL for
        // non-numeric rows, so MIN/MAX over numeric yields the
        // numeric extreme; when every row is non-numeric the
        // numeric aggregate is NULL and COALESCE falls back to
        // lex_subselect). Mixed groups prefer numeric — SPARQL
        // leaves mixed-type behaviour implementation-defined.
        (AggregateFn::Min, Some(var)) => {
            let lex = lex_subselect(var);
            let num = numeric_cast_subselect(var, anchors);
            format!("COALESCE(MIN({distinct}{num})::text, MIN({distinct}{lex}))")
        }
        (AggregateFn::Max, Some(var)) => {
            let lex = lex_subselect(var);
            let num = numeric_cast_subselect(var, anchors);
            format!("COALESCE(MAX({distinct}{num})::text, MAX({distinct}{lex}))")
        }
        (AggregateFn::GroupConcat { separator }, Some(var)) => {
            let escaped = separator.replace('\'', "''");
            format!("STRING_AGG({distinct}{}, '{escaped}')", lex_subselect(var))
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
    // Look up an aggregate by either its current output_var or any
    // of the synthetic names spargebra used internally.
    let find_agg = |name: &str| -> Option<&AggregateSpec> {
        aggs.iter()
            .find(|a| a.output_var == name || a.synth_aliases.iter().any(|s| s == name))
    };
    let numeric_side = |e: &Expression| -> Option<String> {
        match e {
            Expression::Variable(v) => {
                let name = v.as_str();
                if let Some(agg) = find_agg(name) {
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
                if let Some(agg) = find_agg(name) {
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
fn numeric_cast_subselect(var: &str, anchors: &HashMap<String, (usize, &'static str)>) -> String {
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
    let mut sql = format!("SELECT {distinct_kw}{outer_cols} FROM ({union_inner}) AS _pgrdf_union");

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
    let (from_sql, where_clauses, plan) = build_from_and_where(
        &branch.bgp,
        &branch.filters,
        &branch.optionals,
        &branch.minuses,
        &mut anchors,
        0,
    );

    let mut select_clauses: Vec<String> = Vec::new();
    for var in projected {
        // Slice 112: ?g bound by a Variable scope in this branch
        // projects from that scope's g{S}.iri. Branches that don't
        // bind ?g (legitimate per-branch shape for
        // `{ GRAPH ?g {…} } UNION { ?s ?p ?o }`) emit NULL::TEXT,
        // which the outer UNION ALL row reconciles.
        let part = if let Some(scope_id) = plan.projection_scope(var) {
            format!("g{scope_id}.iri AS {}", quote_identifier(var))
        } else if let Some(&(alias_idx, col)) = anchors.get(var) {
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

/// Per-Variable-scope state tracked across BGP / OPTIONAL / MINUS
/// during one call to `build_from_and_where`. The set of unique
/// `scope_id`s that need a JOIN to `_pgrdf_graphs g{scope_id}` is
/// determined by scanning the mandatory BGP + OPTIONALs; MINUS
/// scopes stay internal to their NOT EXISTS subquery.
///
/// `name_anchor` maps the variable name to the FIRST scope_id seen
/// in this builder's context. The projection layer uses that anchor
/// to render `?g` as `g{anchor_scope_id}.iri`; later scopes binding
/// the same name emit a `g{later}.graph_id = g{anchor}.graph_id`
/// equality so the projected value is consistent.
#[derive(Default)]
struct ScopePlan {
    /// Scope ids that have at least one mandatory-BGP triple — these
    /// get an `INNER JOIN _pgrdf_graphs g{S}` JOIN anchored on the
    /// first BGP alias that carries this scope.
    mandatory_join_ids: Vec<usize>,
    /// Scope ids that ONLY appear inside OPTIONALs — these get a
    /// `LEFT JOIN _pgrdf_graphs g{S}` JOIN, so an unmatched OPTIONAL
    /// leaves `?g` unbound (NULL) rather than dropping the outer row.
    optional_join_ids: Vec<usize>,
    /// scope_id → first BGP alias (qN) carrying that scope. The
    /// `_pgrdf_graphs g{S}` join anchors on `q{first_qi}.graph_id`,
    /// and every other triple alias in scope S equates its
    /// `graph_id` to the same anchor — so the JOIN's textual order
    /// (graphs joined AFTER mandatory triples) is unambiguous.
    mandatory_first_qi: HashMap<usize, usize>,
    /// scope_id → opt-triple alias (qOPT_i) for scopes born inside
    /// an OPTIONAL. The LEFT JOIN to `_pgrdf_graphs g{S}` anchors on
    /// this alias.
    optional_first_qi: HashMap<usize, usize>,
    /// Maps variable name to its anchor scope_id (the first scope_id
    /// in this context binding that name). Used both for projection
    /// and for consistency equality between sibling scopes.
    name_anchor: HashMap<String, usize>,
    /// (scope_id, anchor_scope_id) pairs that need
    /// `g{scope_id}.graph_id = g{anchor_scope_id}.graph_id` so two
    /// GRAPH blocks binding the same `?g` agree on which graph wins.
    /// Only emitted for non-anchor scopes (scope_id != anchor).
    cross_scope_consistency: Vec<(usize, usize)>,
}

impl ScopePlan {
    /// Build the plan by scanning every BGP + OPTIONAL scope. MINUS
    /// scopes are handled inside `translate_minus`.
    fn build(bgp: &[ScopedTriple], optionals: &[OptionalBlock], alias_offset: usize) -> Self {
        let mut plan = ScopePlan::default();
        // Mandatory-BGP scopes first so the "first scope binding ?g
        // wins" rule favours the mandatory side.
        for (i, st) in bgp.iter().enumerate() {
            let qi = alias_offset + i + 1;
            if let Some(GraphScope::Variable { name, scope_id }) = &st.scope {
                plan.register_mandatory(name.clone(), *scope_id, qi);
            }
        }
        let first_opt_qi = alias_offset + bgp.len() + 1;
        for (i, opt) in optionals.iter().enumerate() {
            let this_qi = first_opt_qi + i;
            if let Some(GraphScope::Variable { name, scope_id }) = &opt.scope {
                plan.register_optional(name.clone(), *scope_id, this_qi);
            }
        }
        plan
    }

    fn register_mandatory(&mut self, name: String, scope_id: usize, qi: usize) {
        if !self.mandatory_join_ids.contains(&scope_id) {
            self.mandatory_join_ids.push(scope_id);
            self.mandatory_first_qi.insert(scope_id, qi);
        }
        self.register_name(name, scope_id);
    }

    fn register_optional(&mut self, name: String, scope_id: usize, qi: usize) {
        // Skip if already counted as mandatory (i.e. an OPTIONAL
        // inheriting the outer mandatory scope).
        if self.mandatory_join_ids.contains(&scope_id) {
            return;
        }
        if !self.optional_join_ids.contains(&scope_id) {
            self.optional_join_ids.push(scope_id);
            self.optional_first_qi.insert(scope_id, qi);
        }
        self.register_name(name, scope_id);
    }

    fn register_name(&mut self, name: String, scope_id: usize) {
        match self.name_anchor.get(&name).copied() {
            Some(anchor) if anchor != scope_id => {
                if !self
                    .cross_scope_consistency
                    .iter()
                    .any(|&(s, a)| s == scope_id && a == anchor)
                {
                    self.cross_scope_consistency.push((scope_id, anchor));
                }
            }
            Some(_) => {} // same scope already anchored.
            None => {
                self.name_anchor.insert(name, scope_id);
            }
        }
    }

    /// Look up the scope_id whose JOIN's `g{S}.iri` column should be
    /// the projection value for a given SPARQL variable name. Returns
    /// the anchor scope_id if `name` is bound by any Variable scope
    /// in this builder context.
    fn projection_scope(&self, name: &str) -> Option<usize> {
        self.name_anchor.get(name).copied()
    }

    /// First BGP alias for a Variable scope_id, used as the anchor
    /// for both the `_pgrdf_graphs` JOIN and the per-triple graph_id
    /// equality on the second-and-later triples of the same scope.
    fn first_qi_for(&self, scope_id: usize) -> Option<usize> {
        self.mandatory_first_qi
            .get(&scope_id)
            .or_else(|| self.optional_first_qi.get(&scope_id))
            .copied()
    }
}

/// Shared FROM/WHERE builder used by both the single-branch and
/// per-UNION-branch paths. Emits explicit `INNER JOIN`s for
/// mandatory patterns after the first, `LEFT JOIN`s for each
/// OPTIONAL block, and ANDs every filter into the returned
/// `where_clauses` vec. The caller layers SELECT + modifiers.
///
/// Slice 112: graph constraints are PER-PATTERN. Each `ScopedTriple`
/// carries its own `GraphScope`; the builder pre-scans BGP +
/// OPTIONAL scopes to produce a `ScopePlan` describing which scopes
/// need INNER vs LEFT joins to `_pgrdf_graphs`. The returned
/// `ScopePlan` is used by the caller's SELECT-list builder so a
/// projected `?g` resolves to the right `g{S}.iri`.
fn build_from_and_where(
    bgp: &[ScopedTriple],
    filters: &[Expression],
    optionals: &[OptionalBlock],
    minuses: &[MinusBlock],
    anchors: &mut HashMap<String, (usize, &'static str)>,
    alias_offset: usize,
) -> (String, Vec<String>, ScopePlan) {
    let plan = ScopePlan::build(bgp, optionals, alias_offset);
    let mut where_clauses: Vec<String> = Vec::new();
    let mut from_sql = String::new();
    // Mandatory BGP — pattern 1 in FROM (predicates → WHERE),
    // pattern 2..N as INNER JOIN qN ON (predicates). Each triple's
    // graph constraint references either a Literal id ($K) or — for
    // Variable scope — the first qN in the same scope (no
    // _pgrdf_graphs reference yet; the JOIN to that table is
    // appended after the mandatory triples).
    for (i, st) in bgp.iter().enumerate() {
        let qi = alias_offset + i + 1;
        let mut clauses = pattern_clauses(&st.triple, qi, anchors);
        if let Some(scope) = &st.scope {
            scope_constraint_clauses_anchor_q(scope, qi, &plan, &mut clauses);
        }
        if i == 0 {
            from_sql.push_str(&format!("pgrdf._pgrdf_quads q{qi}"));
            where_clauses.append(&mut clauses);
        } else {
            let on = if clauses.is_empty() {
                "TRUE".to_string()
            } else {
                clauses.join(" AND ")
            };
            from_sql.push_str(&format!(" INNER JOIN pgrdf._pgrdf_quads q{qi} ON ({on})"));
        }
    }
    // INNER JOIN to `_pgrdf_graphs g{S}` — one per Variable scope
    // that has at least one mandatory-BGP triple. Anchored on
    // `q{first_qN_in_S}.graph_id` (which is in scope by this point)
    // so the join is unambiguous. INNER matches W3C §13.3: a
    // mandatory `?g` MUST bind to a graph in the IRI mapping.
    for &scope_id in &plan.mandatory_join_ids {
        let anchor_qi = plan
            .first_qi_for(scope_id)
            .expect("ScopePlan: every mandatory scope has a first_qi");
        from_sql.push_str(&format!(
            " INNER JOIN pgrdf._pgrdf_graphs g{scope_id} \
             ON (g{scope_id}.graph_id = q{anchor_qi}.graph_id)"
        ));
    }
    // Cross-scope consistency: two mandatory GRAPH blocks binding
    // the same `?g` must agree on the graph. Emit
    // `g{later}.graph_id = g{anchor}.graph_id` so the projected
    // value is consistent. Optional-side scopes carry their own
    // alias and are handled below.
    for &(scope_id, anchor) in &plan.cross_scope_consistency {
        let later_mandatory = plan.mandatory_join_ids.contains(&scope_id);
        let anchor_mandatory = plan.mandatory_join_ids.contains(&anchor);
        if later_mandatory && anchor_mandatory {
            where_clauses.push(format!("g{scope_id}.graph_id = g{anchor}.graph_id"));
        }
    }
    // OPTIONAL blocks — each becomes a LEFT JOIN whose ON includes
    // the OPTIONAL's inner FILTER (if any). Vars only bound in the
    // OPTIONAL come back NULL when the LEFT JOIN doesn't match.
    let mut next_qi = alias_offset + bgp.len() + 1;
    let mut emitted_left_join_scopes: Vec<usize> = Vec::new();
    for opt in optionals {
        let opt_qi = next_qi;
        next_qi += 1;
        let mut clauses = pattern_clauses(&opt.triple, opt_qi, anchors);
        if let Some(scope) = &opt.scope {
            scope_constraint_clauses_anchor_q(scope, opt_qi, &plan, &mut clauses);
        }
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
        // OPTIONAL-born Variable scopes get a LEFT JOIN to
        // `_pgrdf_graphs g{S}` so an unmatched OPTIONAL leaves both
        // qOPT_i.* and g{S}.iri NULL (?g unbound).
        if let Some(GraphScope::Variable { scope_id, .. }) = &opt.scope {
            if plan.optional_join_ids.contains(scope_id)
                && !emitted_left_join_scopes.contains(scope_id)
            {
                from_sql.push_str(&format!(
                    " LEFT JOIN pgrdf._pgrdf_graphs g{scope_id} \
                     ON (g{scope_id}.graph_id = q{opt_qi}.graph_id)"
                ));
                emitted_left_join_scopes.push(*scope_id);
            }
        }
    }
    // Top-level / branch-level FILTERs — applied to the joined
    // result. NULL comparisons drop the row (SPARQL "type error →
    // unbound" semantics).
    for expr in filters {
        let sql = translate_filter(expr, anchors)
            .unwrap_or_else(|| panic!("sparql: FILTER expression not translatable: {expr:?}"));
        where_clauses.push(sql);
    }
    // MINUS blocks → `NOT EXISTS (SELECT 1 FROM … WHERE shared_vars)`.
    // Per SPARQL spec, MINUS with no shared variables is a no-op and
    // is elided here.
    for minus in minuses {
        if let Some(sql) = translate_minus(minus, anchors, &mut next_qi) {
            where_clauses.push(sql);
        }
    }
    (from_sql, where_clauses, plan)
}

/// Emit the graph-id constraint clauses for a triple alias whose
/// scope is `scope`. Literal scopes add `qN.graph_id = $K` (with the
/// resolved id as a positional parameter); Variable scopes add
/// `qN.graph_id = q{first_qN_in_scope}.graph_id` — anchored on the
/// SCOPE's first BGP alias (per ScopePlan) so the JOIN to
/// `_pgrdf_graphs g{S}` (which lands after all mandatory triples in
/// the textual SQL) doesn't need to be visible at the time this
/// clause is emitted.
///
/// For the FIRST qN of a scope (qN == first_qN_in_scope), no
/// equality is emitted — the alias IS the anchor.
fn scope_constraint_clauses_anchor_q(
    scope: &GraphScope,
    qi: usize,
    plan: &ScopePlan,
    clauses: &mut Vec<String>,
) {
    match scope {
        GraphScope::Literal(gid) => {
            let p = id_placeholder(*gid);
            clauses.push(format!("q{qi}.graph_id = {p}"));
        }
        GraphScope::Variable { scope_id, .. } => {
            let anchor_qi = plan.first_qi_for(*scope_id).unwrap_or(qi);
            if anchor_qi != qi {
                clauses.push(format!("q{qi}.graph_id = q{anchor_qi}.graph_id"));
            }
        }
    }
}

/// Translate a MINUS sub-pattern (N triples) against the outer
/// anchors. Returns `None` if the sub-pattern shares no variables
/// with the outer query (SPARQL spec: MINUS with no shared
/// variables is the identity). Otherwise emits a
/// `NOT EXISTS (SELECT 1 FROM <quads aliases> WHERE …)` sub-SELECT
/// where the WHERE clause carries every triple's predicates +
/// equality predicates joining shared variables back to the outer
/// aliases.
///
/// Slice 112: the MINUS block carries its own `GraphScope`. A
/// Literal scope adds `qN.graph_id = $K` per inner alias. A Variable
/// scope adds a `_pgrdf_graphs g{S}` row into the NOT EXISTS FROM
/// and constrains every inner alias to `g{S}.graph_id`. The Variable
/// scope's `?g` is internal to the MINUS — it doesn't surface to the
/// outer projection (a fresh GRAPH inside MINUS binds locally;
/// MINUS-inherited scope is keyed by the outer `g{S}` and would have
/// already been registered by the BGP/OPTIONAL scan).
fn translate_minus(
    minus: &MinusBlock,
    outer_anchors: &HashMap<String, (usize, &'static str)>,
    next_qi: &mut usize,
) -> Option<String> {
    let triples = &minus.triples;
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
    // First qN inside the MINUS for the Variable scope, if any —
    // used as the anchor for both the `_pgrdf_graphs g{S}` row in
    // the NOT EXISTS FROM and the per-triple graph_id equality.
    let mut minus_first_qi: Option<usize> = None;
    for (i, tp) in triples.iter().enumerate() {
        let qi = *next_qi;
        *next_qi += 1;
        from_aliases.push(format!("pgrdf._pgrdf_quads q{qi}"));
        if i == 0 {
            minus_first_qi = Some(qi);
        }
        let mut clauses = pattern_clauses(tp, qi, &mut local_anchors);
        match &minus.scope {
            Some(GraphScope::Literal(gid)) => {
                let p = id_placeholder(*gid);
                clauses.push(format!("q{qi}.graph_id = {p}"));
            }
            Some(GraphScope::Variable { .. }) => {
                if let Some(first_qi) = minus_first_qi {
                    if qi != first_qi {
                        clauses.push(format!("q{qi}.graph_id = q{first_qi}.graph_id"));
                    }
                }
            }
            None => {}
        }
        all_clauses.append(&mut clauses);
    }
    // Variable scope inside MINUS → join `_pgrdf_graphs g{S}` so
    // the NOT EXISTS subquery only matches rows whose graph is in
    // the IRI mapping (consistent with mandatory-side semantics).
    // The scope is internal to the subquery; the outer projection
    // never references it.
    if let Some(GraphScope::Variable { scope_id, .. }) = &minus.scope {
        if let Some(first_qi) = minus_first_qi {
            from_aliases.push(format!("pgrdf._pgrdf_graphs g{scope_id}"));
            all_clauses.push(format!("g{scope_id}.graph_id = q{first_qi}.graph_id"));
        }
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
///     terms by (term_type, lexical, datatype, lang). Falls back to
///     a lexical compare when STR / LANG / DATATYPE wrappers force it.
///   * `?a != …`: negated identity comparison.
///   * `&&`, `||`, `!`: boolean composition.
///   * Numeric ordering `<` / `>` / `<=` / `>=` — via
///     `translate_numeric_cmp` with XSD-numeric-aware NUMERIC casts.
///   * `?a IN (x, y, z)` — dict-id `IN` list via `translate_in`.
///   * `isIRI(?v)`, `isLiteral(?v)`, `isBlank(?v)`: emit a correlated
///     subselect against `_pgrdf_dictionary.term_type`.
///   * `BOUND(?v)`: trivially TRUE for mandatory anchors;
///     `qOPT_i.col IS NOT NULL` for OPTIONAL anchors.
///   * `REGEX(?v, "pat" [, "flags"])` and the string predicates
///     CONTAINS / STRSTARTS / STRENDS — dispatched through
///     `translate_function_call`.
///
/// Not yet supported (would return None): `EXISTS` / `NOT EXISTS`
/// inside FILTER, conditional `IF` expressions. Arithmetic appears
/// inside numeric ordering (handled there) but is not yet a
/// boolean-yielding top-level FILTER operator on its own.
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
        Function::Contains => {
            translate_string_fn(args, anchors, |s, sub| format!("(strpos({s}, {sub}) > 0)"))
        }
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
/// dictionary id (BIGINT). Variables → `qN.col`; constants →
/// a `$N` parameter placeholder bound to the resolved dict id (or
/// `-1` when the constant isn't in the dictionary, so the predicate
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
        Expression::UnaryMinus(a) => Some(format!("(-{})", expr_to_numeric_sql(a, anchors)?)),
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
        NamedNodePattern::Variable(v) => bind_var(v.as_str(), qi, "predicate_id", anchors, clauses),
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

/// Resolve a graph IRI to its integer `graph_id` via
/// `_pgrdf_graphs.iri`. Returns `None` if the IRI is not bound;
/// callers turn that into the sentinel `-1` so the generated SQL
/// produces zero rows (spec-correct "no solutions"). Wrapped in a
/// scalar `SELECT (subquery)` so SPI stays on the "exactly one row"
/// path — same idiom slice 116 / slice 115 use for their lookups.
fn lookup_graph_id(iri: &str) -> Option<i64> {
    Spi::get_one_with_args(
        "SELECT (SELECT graph_id FROM pgrdf._pgrdf_graphs
                  WHERE iri = $1 LIMIT 1)",
        &[iri.into()],
    )
    .ok()
    .flatten()
}

/// Dictionary lookup (no interning) for a `spargebra::term::GroundTerm`.
/// Returns the existing dictionary id, or `None` if the term has never
/// been written. Used by `DELETE DATA` (slice 83) where the absence of
/// any term is a spec-correct no-op rather than an "allocate and then
/// fail to delete" round-trip.
///
/// `GroundTerm::Triple` (RDF-star, gated behind spargebra's
/// `sparql-12` feature) is unreachable in this build — the variant
/// only exists when the feature is enabled, which our Cargo.toml does
/// not turn on. Listed here for grep-completeness should the feature
/// ever come on.
fn lookup_ground_term_id(t: &GroundTerm) -> Option<i64> {
    match t {
        GroundTerm::NamedNode(n) => lookup_iri_id(n.as_str()),
        GroundTerm::Literal(lit) => lookup_literal_id(lit),
        #[allow(unreachable_patterns)]
        _ => None,
    }
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
            &[term_type::LITERAL.into(), value.into(), lang.into()],
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
            &[term_type::LITERAL.into(), value.into(), dt_id.into()],
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
            let owned = maybe_owned.expect("sparql: plan must be in cache after insert");
            let table = client
                .update(owned, None, &datums)
                .expect("sparql: prepared SELECT failed");
            for row in table {
                let mut obj = Map::new();
                for (i, var) in plan.projected.iter().enumerate() {
                    let val: Option<String> = row.get::<String>(i + 1).ok().flatten();
                    obj.insert(var.clone(), val.map(Value::String).unwrap_or(Value::Null));
                }
                rows.push(pgrx::JsonB(Value::Object(obj)));
            }
        });
        rows
    })
}

// ─────────────────────────────────────────────────────────────────────
// SPARQL UPDATE execution (Phase C slice 84 — foundation)
//
// `pgrdf.sparql(q)` routes UPDATE forms here after `parse_query` fails
// and `parse_update` succeeds. The dispatcher walks the Update's
// `Vec<GraphUpdateOperation>` and either materialises the operation
// (INSERT DATA today) or panics with a "lands in slice NN" message for
// the variants the per-form follow-up slices will land.
//
// Return shape: a single summary row with one key `_update`, paralleling
// the v0.3 `_ask` sentinel for ASK queries (LLD v0.4 §4.2). Callers
// discriminate on the leading JSONB key.
// ─────────────────────────────────────────────────────────────────────

/// Walk an `spargebra::Update` and dispatch each operation to its
/// per-form implementation. Returns a single-row `Vec<pgrx::JsonB>`
/// carrying the `_update` summary; the caller wraps it in
/// `SetOfIterator`. Wall-clock timing starts at function entry so the
/// dictionary internment + partition setup cost is visible to operators.
fn execute_update(update: &Update) -> Vec<pgrx::JsonB> {
    let start = std::time::Instant::now();
    let mut triples_inserted: i64 = 0;
    let mut triples_deleted: i64 = 0;
    let mut graphs_touched: HashSet<i64> = HashSet::new();
    // Form discriminator strategy: if every operation in the Update
    // shares the same variant name, the summary's `form` field carries
    // that name (the single-form case — INSERT DATA, DELETE DATA, etc.).
    // If two or more variants appear (e.g. a future
    // `DELETE DATA { … } ; INSERT DATA { … }` composition), `form`
    // collapses to `"MIXED"` so callers know they need to look at the
    // per-op detail (today via `pgrdf.sparql_parse(q)`; a richer
    // per-op summary array is a v0.4.4+ follow-up). EMPTY when there
    // are no operations at all (degenerate but well-formed).
    let mut form: &'static str = "EMPTY";

    for (i, op) in update.operations.iter().enumerate() {
        let op_name = update_op_name(op);
        if i == 0 {
            form = op_name;
        } else if form != op_name {
            form = "MIXED";
        }
        match op {
            GraphUpdateOperation::InsertData { data } => {
                for quad in data {
                    let g_id = resolve_or_allocate_graph(&quad.graph_name);
                    let s_id = intern_subject(&quad.subject);
                    let p_id = intern_named_node(quad.predicate.as_str());
                    let o_id = intern_object(&quad.object);
                    insert_quad(s_id, p_id, o_id, g_id);
                    triples_inserted += 1;
                    graphs_touched.insert(g_id);
                }
            }
            GraphUpdateOperation::DeleteData { data } => {
                // DELETE DATA — ground quads, no variables. Per LLD
                // v0.4 §4.1 the form is set-semantic: removing a
                // triple that isn't in the store is a spec-correct
                // no-op (not an error). We honour that by looking up
                // each term in the dictionary WITHOUT interning. If
                // any of (subject, predicate, object) is missing
                // from `_pgrdf_dictionary`, the quad cannot possibly
                // be present in `_pgrdf_quads` — skip it. Same for an
                // unbound named graph IRI: the partition wouldn't
                // exist anyway, so the DELETE produces zero rows.
                //
                // `graphs_touched` records the graph EVEN ON NO-OP —
                // it carries the operator's INTENT, mirroring how
                // INSERT DATA reports a graph it just allocated even
                // if the quad was already present (idempotent path).
                // The triple-count counter is the source of truth
                // for "did anything change"; `graphs_touched` is the
                // scope summary.
                for ground_quad in data {
                    let g_id = match &ground_quad.graph_name {
                        GraphName::DefaultGraph => Some(0_i64),
                        GraphName::NamedNode(n) => lookup_graph_id(n.as_str()),
                    };
                    let Some(g_id) = g_id else {
                        // Named graph IRI not bound — nothing to delete.
                        continue;
                    };
                    // GroundQuad.subject is `NamedNode` (no blank
                    // nodes in DELETE DATA per the SPARQL 1.1 grammar
                    // — spargebra enforces this at parse time). Same
                    // for predicate. Object can be NamedNode or
                    // Literal (Triple is sparql-12-feature, off by
                    // default in our spargebra build).
                    let s_id = lookup_iri_id(ground_quad.subject.as_str());
                    let p_id = lookup_iri_id(ground_quad.predicate.as_str());
                    let o_id = lookup_ground_term_id(&ground_quad.object);
                    let (Some(s), Some(p), Some(o)) = (s_id, p_id, o_id) else {
                        // At least one term not in the dictionary —
                        // the quad cannot exist. Spec-correct no-op.
                        graphs_touched.insert(g_id);
                        continue;
                    };
                    let n: i64 = Spi::get_one_with_args(
                        "WITH d AS (
                            DELETE FROM pgrdf._pgrdf_quads
                             WHERE subject_id   = $1
                               AND predicate_id = $2
                               AND object_id    = $3
                               AND graph_id     = $4
                           RETURNING 1)
                         SELECT count(*)::bigint FROM d",
                        &[s.into(), p.into(), o.into(), g_id.into()],
                    )
                    .unwrap_or_else(|e| panic!("sparql: UPDATE: DELETE quad failed: {e}"))
                    .unwrap_or(0);
                    triples_deleted += n;
                    graphs_touched.insert(g_id);
                }
            }
            GraphUpdateOperation::DeleteInsert {
                delete,
                insert,
                using,
                pattern,
            } => {
                // Triage by which template halves are present. spargebra
                // 0.4.6 models `delete` and `insert` as Vec<…> (not
                // Option<Vec<…>>) so an absent half is the empty vec.
                let has_delete = !delete.is_empty();
                let has_insert = !insert.is_empty();
                match (has_delete, has_insert) {
                    (true, true) => {
                        // Combined DELETE … INSERT … WHERE — the
                        // "modify" form. The substring `UPDATE form
                        // 'DELETE/INSERT WHERE' lands` is the locked
                        // prefix scraped by slice 84's regression
                        // (test 93's `update-delete-insert-where-lands`
                        // expectation). Keep the substring contiguous.
                        panic!(
                            "sparql: UPDATE form 'DELETE/INSERT WHERE' lands in slice 77 \
                             (combined modify form)"
                        )
                    }
                    (true, false) => {
                        panic!("sparql: UPDATE form 'DELETE WHERE' (without INSERT) lands in slice 78")
                    }
                    (false, true) => {
                        // Pure INSERT WHERE — slice 82.
                        if using.is_some() {
                            panic!(
                                "sparql: INSERT WHERE template feature 'USING / USING NAMED' \
                                 not yet supported"
                            );
                        }
                        let (n_inserted, graphs) = execute_insert_where(insert, pattern);
                        triples_inserted += n_inserted;
                        for g in graphs {
                            graphs_touched.insert(g);
                        }
                    }
                    (false, false) => {
                        // spargebra never emits an empty `DeleteInsert`
                        // — the parser would reject it as a syntax
                        // error. Treat as a no-op for completeness.
                    }
                }
            }
            GraphUpdateOperation::Load { .. } => {
                panic!("sparql: UPDATE form 'LOAD' is out of scope for v0.4 (see LLD v0.4 §14)")
            }
            GraphUpdateOperation::Clear { .. } => {
                panic!("sparql: UPDATE form 'CLEAR GRAPH' lands in slice 71")
            }
            GraphUpdateOperation::Create { .. } => {
                panic!("sparql: UPDATE form 'CREATE GRAPH' lands in slice 70")
            }
            GraphUpdateOperation::Drop { .. } => {
                panic!("sparql: UPDATE form 'DROP GRAPH' lands in slice 69")
            }
        }
    }

    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    // Resolve each touched graph_id back to its IRI for the summary.
    // We hand back the IRIs (not raw graph_ids) so the JSONB shape
    // stays portable across pg_dump round-trips and matches the LLD
    // v0.4 §4.2 example.
    let mut graphs: Vec<String> = graphs_touched
        .iter()
        .filter_map(|g| lookup_graph_iri(*g))
        .collect();
    graphs.sort();

    let summary = json!({
        "_update": {
            "form":             form,
            "triples_inserted": triples_inserted,
            "triples_deleted":  triples_deleted,
            "graphs_touched":   graphs,
            "elapsed_ms":       elapsed_ms,
        }
    });
    vec![pgrx::JsonB(summary)]
}

/// Discriminator for the JSONB `form` field in the `_update` summary.
/// Mirrors the SPARQL surface from LLD v0.4 §4.1. Per-form slices will
/// extend the dispatch table; the variant string stays stable so it
/// can be matched on by downstream callers.
fn update_op_name(op: &GraphUpdateOperation) -> &'static str {
    match op {
        GraphUpdateOperation::InsertData { .. } => "INSERT_DATA",
        GraphUpdateOperation::DeleteData { .. } => "DELETE_DATA",
        // Slice 82 narrows the DeleteInsert label by template-half
        // presence: pure-INSERT-WHERE reports `INSERT_WHERE`, pure-
        // DELETE-WHERE will report `DELETE_WHERE` once slice 78 ships,
        // and the combined modify form keeps the legacy
        // `DELETE_INSERT_WHERE` discriminator for slice 77.
        GraphUpdateOperation::DeleteInsert { delete, insert, .. } => {
            match (!delete.is_empty(), !insert.is_empty()) {
                (false, true) => "INSERT_WHERE",
                (true, false) => "DELETE_WHERE",
                _ => "DELETE_INSERT_WHERE",
            }
        }
        GraphUpdateOperation::Load { .. } => "LOAD",
        GraphUpdateOperation::Clear { .. } => "CLEAR",
        GraphUpdateOperation::Create { .. } => "CREATE",
        GraphUpdateOperation::Drop { .. } => "DROP",
    }
}

/// Resolve an `InsertData` quad's `graph_name` field to a `graph_id`:
///   - `DefaultGraph` → 0 (the default partition; always exists post
///     `CREATE EXTENSION`).
///   - `NamedNode(iri)` → existing `graph_id` if the IRI is already
///     bound; otherwise allocate a fresh `graph_id` via
///     `pgrdf.add_graph(iri TEXT)` (slice 118), matching the LLD v0.4
///     §4.1 "Unknown IRIs auto-allocate (default behaviour, matching
///     `add_graph(iri)`)" contract.
fn resolve_or_allocate_graph(g: &GraphName) -> i64 {
    match g {
        GraphName::DefaultGraph => 0,
        GraphName::NamedNode(n) => {
            let iri = n.as_str();
            if let Some(id) = lookup_graph_id(iri) {
                return id;
            }
            Spi::get_one_with_args::<i64>("SELECT pgrdf.add_graph($1::text)", &[iri.into()])
                .unwrap_or_else(|e| panic!("sparql: UPDATE: add_graph({iri}) failed: {e}"))
                .expect("sparql: UPDATE: add_graph returned NULL (impossible)")
        }
    }
}

/// Intern an IRI (URI term) into `_pgrdf_dictionary` and return its id.
/// Routes through `put_term_full` so we pick up the shmem cache hit
/// path (LLD §4.1) and stage commit-deferred publish on insert.
fn intern_named_node(iri: &str) -> i64 {
    put_term_full(iri, term_type::URI, None, None)
}

/// Intern a quad subject (IRI or blank node) into the dictionary.
fn intern_subject(s: &NamedOrBlankNode) -> i64 {
    match s {
        NamedOrBlankNode::NamedNode(n) => intern_named_node(n.as_str()),
        NamedOrBlankNode::BlankNode(b) => {
            put_term_full(b.as_str(), term_type::BLANK_NODE, None, None)
        }
    }
}

/// Intern a quad object (IRI / blank node / literal) into the
/// dictionary. Literal datatype IRIs are themselves interned first so
/// the literal row can reference them by id, matching the existing
/// Turtle-loader convention in `src/storage/loader.rs::object_to_id`.
fn intern_object(t: &Term) -> i64 {
    match t {
        Term::NamedNode(n) => intern_named_node(n.as_str()),
        Term::BlankNode(b) => put_term_full(b.as_str(), term_type::BLANK_NODE, None, None),
        Term::Literal(lit) => {
            let lang = lit.language();
            let datatype_id = if lang.is_some() {
                None
            } else {
                Some(intern_named_node(lit.datatype().as_str()))
            };
            put_term_full(lit.value(), term_type::LITERAL, datatype_id, lang)
        }
        #[allow(unreachable_patterns)]
        _ => panic!("sparql: UPDATE: unsupported object term (RDF-star not in v0.4 scope)"),
    }
}

/// INSERT one resolved quad into `pgrdf._pgrdf_quads`. The destination
/// partition for `graph_id = g` is ensured up front (a no-op when
/// already present).
///
/// `_pgrdf_quads` has no `UNIQUE` constraint on `(subject_id,
/// predicate_id, object_id, graph_id)` — the hexastore indexes are
/// covering, not unique, by design (the bulk Turtle loader appends
/// without dedup checks for perf). To honour LLD v0.4 §4's
/// "INSERT DATA is set-semantics" contract on re-runs of the same
/// statement, we route the INSERT through a `WHERE NOT EXISTS`
/// guard against the existing row (base OR inferred). Cost: one
/// index probe against the SPO covering index per inserted triple.
/// `ON CONFLICT DO NOTHING` would have been the cleaner shape, but
/// it requires a unique constraint Postgres can attach to — which
/// `_pgrdf_quads` deliberately doesn't have.
fn insert_quad(s_id: i64, p_id: i64, o_id: i64, g_id: i64) {
    // Auto-create the partition for non-default graphs. graph_id = 0
    // always has the default partition seeded at CREATE EXTENSION
    // (slice 120 contract), so we skip the call to avoid a redundant
    // `pgrdf.add_graph(0)` round-trip.
    if g_id != 0 {
        Spi::run_with_args("SELECT pgrdf.add_graph($1::bigint)", &[g_id.into()])
            .unwrap_or_else(|e| panic!("sparql: UPDATE: add_graph({g_id}) failed: {e}"));
    }
    Spi::run_with_args(
        "INSERT INTO pgrdf._pgrdf_quads \
              (subject_id, predicate_id, object_id, graph_id, is_inferred) \
         SELECT $1, $2, $3, $4, false \
          WHERE NOT EXISTS ( \
             SELECT 1 FROM pgrdf._pgrdf_quads \
              WHERE subject_id = $1 \
                AND predicate_id = $2 \
                AND object_id = $3 \
                AND graph_id = $4)",
        &[s_id.into(), p_id.into(), o_id.into(), g_id.into()],
    )
    .unwrap_or_else(|e| panic!("sparql: UPDATE: INSERT quad failed: {e}"));
}

/// Reverse-lookup a `graph_id` to its IRI for the `graphs_touched`
/// summary array. Returns `None` for the default graph (id = 0) by
/// convention — operators reading the summary see `"DEFAULT"` as the
/// absence of a named-graph IRI rather than the synthetic seed row
/// `urn:pgrdf:graph:0` (which IS bound, but is not user-visible).
fn lookup_graph_iri(g: i64) -> Option<String> {
    if g == 0 {
        return Some("DEFAULT".to_string());
    }
    Spi::get_one_with_args(
        "SELECT (SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = $1 LIMIT 1)",
        &[g.into()],
    )
    .ok()
    .flatten()
}

// ─────────────────────────────────────────────────────────────────────
// SPARQL UPDATE — INSERT { template } WHERE { pattern } (Phase C slice 82)
//
// Pattern-driven insertion. For each solution row of the WHERE pattern
// the template's variables substitute and the resulting concrete quads
// land in `_pgrdf_quads`.
//
// Strategy A (per slice 82 brief — single-pass via SPI):
//   1. parse_select(pattern) — re-uses the v0.3 SELECT walker, gets us
//      a `ParsedSelect` with the BGP, FILTERs, OPTIONALs, MINUSes, and
//      (implicitly via SELECT *) the full set of bound variables.
//   2. Build a custom SELECT SQL that returns each template-referenced
//      variable as a `BIGINT` (the q{N}.{subject_id|predicate_id|
//      object_id} dict id), NOT a TEXT lexical. This keeps internment
//      lossless — the binding's term_type / datatype / lang stay
//      attached to the existing dict row.
//   3. For each binding row, instantiate the template's `QuadPattern`s:
//      constants intern through the existing helpers; variable refs
//      resolve via the per-row BIGINT map. Each instantiated quad goes
//      through `insert_quad` (the same WHERE NOT EXISTS guard slice 84
//      installed for INSERT DATA's set-semantics).
//
// Limitations locked for slice 82 (deferred to follow-up slices):
//   - Template GRAPH scope: the `QuadPattern.graph_name` must be
//     `GraphNamePattern::DefaultGraph` OR a literal `NamedNode`. A
//     variable graph (e.g. `INSERT { GRAPH ?g { … } }`) panics with
//     the stable `INSERT WHERE template feature` prefix — that variant
//     lands with slice 76 (graph-scoped INSERT WHERE).
//   - Template variables MUST be bound by the WHERE pattern; an
//     unbound variable in the template panics with the same stable
//     prefix. SPARQL spec says unbound vars yield no triple for that
//     solution, but slice 82 trades silent-skip for fail-fast so
//     authoring mistakes surface early; the spec-conformant skip
//     lands as an enhancement when CONSTRUCT does (Track 4).
//   - Aggregates / GROUP BY / SELECT-modifiers in the WHERE pattern
//     are syntactically valid SPARQL but produce variable scoping
//     that's outside the §4.1 INSERT WHERE intent — `parse_select`
//     happily walks them, but the SQL builder would emit columns
//     that don't carry a dict id. Slice 82 panics on the aggregate
//     case via the same stable prefix.
// ─────────────────────────────────────────────────────────────────────

/// Translate + execute one `INSERT { template } WHERE { pattern }`
/// operation. Returns `(triples_inserted, graphs_touched)` so the
/// caller can fold into the `_update` summary row.
fn execute_insert_where(
    template: &[QuadPattern],
    pattern: &GraphPattern,
) -> (i64, HashSet<i64>) {
    // Collect template-referenced variables so we know which columns
    // the WHERE-SELECT must return as dict ids.
    let template_vars = collect_template_vars(template);

    // Re-use the SELECT walker for the WHERE pattern. The walker
    // populates ps.bgp + ps.filters + ps.optionals + ps.minuses (and
    // any UNION branches) — exactly the §13 algebra we need.
    params_clear();
    let mut ps = parse_select(pattern);
    if !ps.aggregates.is_empty() || !ps.group_vars.is_empty() {
        panic!(
            "sparql: INSERT WHERE template feature 'aggregate/GROUP BY in WHERE' not yet supported"
        );
    }
    if !ps.union_branches.is_empty() {
        // UNION in WHERE produces solutions from disjoint branches —
        // the SQL shape (UNION ALL of per-branch SELECTs) doesn't
        // share anchor aliases across branches, which would force
        // per-branch template instantiation. Out of scope for 82.
        panic!("sparql: INSERT WHERE template feature 'UNION in WHERE' not yet supported");
    }
    if ps.bgp.is_empty() {
        panic!("sparql: INSERT WHERE requires a non-empty WHERE pattern");
    }
    // We don't honour DISTINCT / ORDER BY / LIMIT / OFFSET from the
    // walker (the spec doesn't admit them on WHERE-of-UPDATE), but
    // parse_select happily picks them up if a user wraps the pattern
    // in solution modifiers. Strip them so the emitted SQL stays a
    // pure FROM+WHERE shape.
    ps.distinct = false;
    ps.order_by.clear();
    ps.limit = None;
    ps.offset = 0;
    ps.projected.clear();
    ps.binds.clear();

    // Emit FROM + WHERE; keep the anchors map so we know which q{N}.col
    // each variable resolved to.
    let mut anchors: HashMap<String, (usize, &'static str)> = HashMap::new();
    let (from_sql, where_clauses, _plan) = build_from_and_where(
        &ps.bgp,
        &ps.filters,
        &ps.optionals,
        &ps.minuses,
        &mut anchors,
        0,
    );

    // Build the projection: one column per template variable, casting
    // through the anchored q{N}.{col} so the row hands back BIGINTs we
    // can feed straight to insert_quad without re-internment.
    let mut select_clauses: Vec<String> = Vec::new();
    let mut col_order: Vec<String> = Vec::new();
    for var in &template_vars {
        let &(alias_idx, col) = anchors.get(var).unwrap_or_else(|| {
            panic!(
                "sparql: INSERT WHERE template feature 'unbound template variable ?{var}' \
                 not yet supported (every template variable must appear in the WHERE BGP)"
            )
        });
        select_clauses.push(format!(
            "q{alias_idx}.{col} AS {alias_v}",
            alias_v = quote_identifier(var),
        ));
        col_order.push(var.clone());
    }
    // Fallback when the template has no variables at all (degenerate
    // — equivalent to INSERT DATA gated by a WHERE existence check).
    // SELECT a constant so the binding count still drives the
    // per-row insert.
    if select_clauses.is_empty() {
        select_clauses.push("1 AS _pgrdf_unit".to_string());
    }

    let mut sql = format!(
        "SELECT {sel} FROM {from_sql}",
        sel = select_clauses.join(", "),
    );
    if !where_clauses.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&where_clauses.join(" AND "));
    }

    let params = params_take();

    // Resolve template constants once — the dict id stays stable
    // across binding rows. Variables resolve per row.
    let mut triples_inserted: i64 = 0;
    let mut graphs_touched: HashSet<i64> = HashSet::new();

    Spi::connect_mut(|client| {
        let arg_oids: Vec<PgOid> =
            vec![PgOid::BuiltIn(PgBuiltInOids::INT8OID); params.len()];
        let prepared = client
            .prepare(sql.as_str(), &arg_oids)
            .expect("sparql: INSERT WHERE: prepare failed");
        let int8_oid: Oid = PgBuiltInOids::INT8OID.into();
        // SAFETY: every WHERE-SELECT param is a dict id (i64) — see
        // params_push / id_placeholder. The (value, type-oid) pair
        // is well-formed by construction.
        let datums: Vec<DatumWithOid<'_>> = params
            .iter()
            .map(|id| unsafe { DatumWithOid::new(*id, int8_oid) })
            .collect();
        let table = client
            .select(&prepared, None, &datums)
            .expect("sparql: INSERT WHERE: WHERE-SELECT failed");
        for row in table {
            // Build the (var → dict-id) map for this binding. NULL
            // bindings (an OPTIONAL that didn't match for a template
            // var) skip the insert — spec-conformant "no triple
            // emitted for unbound template vars in that solution".
            let mut binding: HashMap<&str, Option<i64>> = HashMap::new();
            let mut skip_row = false;
            for (i, var) in col_order.iter().enumerate() {
                // SPI is 1-based on column index.
                let v: Option<i64> = row
                    .get::<i64>(i + 1)
                    .ok()
                    .flatten();
                if v.is_none() {
                    // Template variable unbound in this solution —
                    // skip the entire row's instantiation. This
                    // matches the W3C §4.2 "Template Group" rule:
                    // "if any binding is missing for a template
                    // variable, no triple is added for that group".
                    skip_row = true;
                }
                binding.insert(var.as_str(), v);
            }
            if skip_row {
                continue;
            }
            for qp in template {
                let (s_id, p_id, o_id, g_id) =
                    instantiate_template_quad(qp, &binding);
                insert_quad(s_id, p_id, o_id, g_id);
                triples_inserted += 1;
                graphs_touched.insert(g_id);
            }
        }
    });

    (triples_inserted, graphs_touched)
}

/// Walk the template QuadPatterns and collect every Variable name
/// referenced across subject / predicate / object / graph_name slots.
/// Order is first-appearance (stable across runs) so SQL columns are
/// reproducible; HashSet would be order-randomised.
fn collect_template_vars(template: &[QuadPattern]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let push = |name: &str, out: &mut Vec<String>, seen: &mut HashSet<String>| {
        if seen.insert(name.to_string()) {
            out.push(name.to_string());
        }
    };
    for qp in template {
        if let TermPattern::Variable(v) = &qp.subject {
            push(v.as_str(), &mut out, &mut seen);
        }
        if let NamedNodePattern::Variable(v) = &qp.predicate {
            push(v.as_str(), &mut out, &mut seen);
        }
        if let TermPattern::Variable(v) = &qp.object {
            push(v.as_str(), &mut out, &mut seen);
        }
        if let GraphNamePattern::Variable(v) = &qp.graph_name {
            push(v.as_str(), &mut out, &mut seen);
        }
    }
    out
}

/// Resolve a template `QuadPattern` to `(s, p, o, g)` concrete dict
/// ids by combining (a) constants interned through the existing
/// helpers and (b) variables looked up in the per-row binding.
///
/// Unbound variables panic before reaching here — the WHERE-SELECT
/// row-loop screens for them via `skip_row` so a real solution row
/// is always fully bound across template vars.
fn instantiate_template_quad(
    qp: &QuadPattern,
    binding: &HashMap<&str, Option<i64>>,
) -> (i64, i64, i64, i64) {
    let s_id = match &qp.subject {
        TermPattern::Variable(v) => binding
            .get(v.as_str())
            .and_then(|x| *x)
            .unwrap_or_else(|| {
                panic!("sparql: INSERT WHERE: subject variable ?{} unbound (internal)", v.as_str())
            }),
        TermPattern::NamedNode(n) => intern_named_node(n.as_str()),
        TermPattern::BlankNode(b) => {
            // Per SPARQL UPDATE §4.1.3, every blank node in an INSERT
            // template introduces a FRESH node for each solution row.
            // We synthesise a unique label per (binding-row, blank-
            // label) pair so set-semantics still applies but distinct
            // template invocations don't collapse onto the same node.
            // For slice 82 we render that as `<original>:<row-uuid>`
            // — except we don't have a row uuid handy, and the
            // semantic is delicate enough to warrant its own slice.
            // Defer.
            panic!(
                "sparql: INSERT WHERE template feature 'blank node in template ({})' \
                 not yet supported",
                b.as_str()
            )
        }
        TermPattern::Literal(_) => panic!("sparql: literal subject is invalid in RDF"),
        other => panic!("sparql: INSERT WHERE template: unsupported subject term {other:?}"),
    };
    let p_id = match &qp.predicate {
        NamedNodePattern::Variable(v) => binding
            .get(v.as_str())
            .and_then(|x| *x)
            .unwrap_or_else(|| {
                panic!("sparql: INSERT WHERE: predicate variable ?{} unbound (internal)", v.as_str())
            }),
        NamedNodePattern::NamedNode(n) => intern_named_node(n.as_str()),
    };
    let o_id = match &qp.object {
        TermPattern::Variable(v) => binding
            .get(v.as_str())
            .and_then(|x| *x)
            .unwrap_or_else(|| {
                panic!("sparql: INSERT WHERE: object variable ?{} unbound (internal)", v.as_str())
            }),
        TermPattern::NamedNode(n) => intern_named_node(n.as_str()),
        TermPattern::BlankNode(b) => {
            panic!(
                "sparql: INSERT WHERE template feature 'blank node in template ({})' \
                 not yet supported",
                b.as_str()
            )
        }
        TermPattern::Literal(lit) => intern_object(&Term::Literal(lit.clone())),
        other => panic!("sparql: INSERT WHERE template: unsupported object term {other:?}"),
    };
    let g_id = match &qp.graph_name {
        GraphNamePattern::DefaultGraph => 0,
        GraphNamePattern::NamedNode(n) => resolve_or_allocate_graph(&GraphName::NamedNode(n.clone())),
        GraphNamePattern::Variable(v) => {
            panic!(
                "sparql: INSERT WHERE template feature 'variable GRAPH ?{}' not yet supported \
                 (lands with slice 76 graph-scoped INSERT WHERE)",
                v.as_str()
            )
        }
    };
    (s_id, p_id, o_id, g_id)
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
                8001)",
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
        assert_eq!(
            n, 3,
            "expected 3 triples from the 3 we just loaded, got {n}"
        );
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
                8002)",
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
                8003)",
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
    /// rows, NOT error out. The translator binds `-1` as the
    /// parameterised dict id sentinel which no row can match.
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
                8004)",
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
                8010)",
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
        assert_eq!(
            rows, 1,
            "FILTER(?n = \"Alice\") should match one row, got {rows}"
        );
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
                8011)",
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
                8012)",
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
                8013)",
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
                8014)",
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
                8020)",
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
                8021)",
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
                8022)",
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
                8023)",
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
                8024)",
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
                8025)",
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
                8100)",
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
                8101)",
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
                8102)",
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
                8090)",
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
                8091)",
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
                8092)",
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
                8093)",
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
                8094)",
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
                8095)",
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
                8080)",
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
                8081)",
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
                8082)",
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
                8083)",
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
                8070)",
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
                8071)",
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
                8072)",
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
                8073)",
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
                8074)",
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
                8075)",
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
                8076)",
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
                8120)",
        )
        .unwrap();

        let yes: pgrx::JsonB = Spi::get_one("SELECT sparql FROM pgrdf.sparql('ASK { ?s ?p ?o }')")
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
                8121)",
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
                8110)",
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
                8060)",
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
                8061)",
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
                8062)",
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
                8063)",
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
                8050)",
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
                8051)",
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
                8052)",
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
                8053)",
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
                8054)",
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
                8040)",
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
                8041)",
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
                8042)",
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
                8043)",
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
                8030)",
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
        assert_eq!(
            with_distinct, 2,
            "expected 2 distinct ?o (x, y), got {with_distinct}"
        );
    }

    /// LIMIT caps the number of rows returned.
    #[pg_test]
    fn sparql_limit_caps_rows() {
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://example.com/> .
                 ex:a ex:p 1 . ex:b ex:p 2 . ex:c ex:p 3 . ex:d ex:p 4 . ex:e ex:p 5 .',
                8031)",
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
                8032)",
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
                8033)",
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
                8034)",
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
                8035)",
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
                8015)",
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
                8005)",
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

    /// Slice 114 — `GRAPH <iri> { … }` scopes the BGP to a single
    /// named graph. Two graphs (501, 502) each carry one triple; a
    /// `GRAPH <iri>` query against either's IRI returns exactly that
    /// graph's triple, and an unresolved IRI returns zero rows
    /// (sentinel `graph_id = -1` matches no real partition).
    ///
    /// Uses direct INSERT into `_pgrdf_graphs` + manual partition
    /// creation to bypass `add_graph`'s parallelism flake — pgrx
    /// runs `#[pg_test]`s in parallel and the IRI-keyed overload's
    /// SHARE ROW EXCLUSIVE lock has been flaky under contention.
    #[pg_test]
    fn sparql_graph_literal_iri_scopes_to_graph() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (501, 'http://example.org/test-g1'), \
                    (502, 'http://example.org/test-g2')",
        )
        .unwrap();
        Spi::run(
            "CREATE TABLE pgrdf._pgrdf_quads_test501 \
             PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN (501)",
        )
        .unwrap();
        Spi::run(
            "CREATE TABLE pgrdf._pgrdf_quads_test502 \
             PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN (502)",
        )
        .unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:a ex:p \"in-501\" .', 501)",
        )
        .unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:b ex:p \"in-502\" .', 502)",
        )
        .unwrap();

        let count_g1: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?o WHERE { GRAPH <http://example.org/test-g1> { ?s ex:p ?o } }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            count_g1, 1,
            "GRAPH <g1> should surface exactly its 1 triple"
        );

        let count_g2: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?o WHERE { GRAPH <http://example.org/test-g2> { ?s ex:p ?o } }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            count_g2, 1,
            "GRAPH <g2> should surface exactly its 1 triple"
        );

        let count_unresolved: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?o WHERE { GRAPH <http://example.org/none> { ?s ex:p ?o } }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            count_unresolved, 0,
            "unresolved GRAPH IRI must bind to the -1 sentinel → zero rows"
        );
    }

    /// Slice 113 — `GRAPH ?g { … }` projects ?g as the IRI from
    /// `_pgrdf_graphs.iri` via an INNER JOIN. Two graphs (511, 512)
    /// each carry one triple; the variable-form query returns two
    /// rows whose ?g bindings are the two distinct IRIs, and the
    /// multi-triple BGP path enforces the shared-graph constraint
    /// (qN.graph_id = q1.graph_id for N≥2) so cross-graph subject
    /// joins never surface.
    ///
    /// Same scaffolding as the slice-114 test: direct INSERT into
    /// `_pgrdf_graphs` + manual partition creation, bypassing
    /// `add_graph`'s parallelism flake under pgrx's parallel test
    /// harness.
    #[pg_test]
    fn sparql_graph_variable_projects_iri() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (511, 'http://example.org/test-v1'), \
                    (512, 'http://example.org/test-v2')",
        )
        .unwrap();
        Spi::run(
            "CREATE TABLE pgrdf._pgrdf_quads_test511 \
             PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN (511)",
        )
        .unwrap();
        Spi::run(
            "CREATE TABLE pgrdf._pgrdf_quads_test512 \
             PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN (512)",
        )
        .unwrap();
        // Two triples in g1 (so the multi-triple BGP can match a
        // shared subject), two in g2.
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:a ex:name \"NameA\" . \
                  ex:a ex:age \"30\" .', 511)",
        )
        .unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:b ex:name \"NameB\" . \
                  ex:b ex:age \"25\" .', 512)",
        )
        .unwrap();

        // Two rows — one per graph — with ?g bound to the IRI.
        let row_count: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?g ?n WHERE { GRAPH ?g { ?s ex:name ?n } }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            row_count, 2,
            "GRAPH ?g {{ ?s ex:name ?n }} binds 2 rows (one per graph)"
        );

        // Both graph IRIs surface as ?g; assert by counting distinct
        // matches against the seed IRIs.
        let matched: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?g ?n WHERE { GRAPH ?g { ?s ex:name ?n } }') AS s(j) \
             WHERE (s.j->>'g') IN \
               ('http://example.org/test-v1', 'http://example.org/test-v2')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            matched, 2,
            "both rows must project ?g as the named-graph IRI from _pgrdf_graphs"
        );

        // Multi-triple inner BGP: `?s ex:name ?n . ?s ex:age ?a` —
        // both triples must share a graph_id. Each graph carries a
        // matched pair (NameA/30, NameB/25), so we get exactly 2
        // rows. A cross-graph stitch (NameA paired with 25 in g2,
        // say) would balloon the count.
        let multi_rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?g ?n ?a WHERE { GRAPH ?g { ?s ex:name ?n . ?s ex:age ?a } }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            multi_rows, 2,
            "multi-triple inner BGP must share graph_id — 1 row per graph, no cross-graph stitches"
        );
    }

    /// Slice 112 — composition of GRAPH with OPTIONAL. The outer
    /// BGP is scoped to one literal graph; the OPTIONAL wraps a
    /// GRAPH block over a DIFFERENT literal graph. The OPTIONAL
    /// scopes only its own triple — the outer is unaffected — and
    /// unmatched OPTIONALs leave their projected var NULL while the
    /// outer row survives (LEFT JOIN semantics).
    ///
    /// Setup: g1 (id 521) has alice ex:p "p1-alice". g2 (id 522)
    /// has alice ex:q "q2-alice". g3 (id 523) has bob ex:p only.
    /// Query: outer GRAPH g1 binds alice/p1; OPTIONAL GRAPH g2
    /// binds ex:q for alice → q2-alice. Subject join across the two
    /// GRAPH scopes is the SAME ?s (alice), so the OPTIONAL fires.
    ///
    /// Direct INSERT into `_pgrdf_graphs` + manual partitions to
    /// bypass `add_graph`'s parallel-test flake, matching the
    /// pattern slices 114 + 113 used.
    #[pg_test]
    fn sparql_graph_composition_with_optional() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (521, 'http://example.org/test-c1'), \
                    (522, 'http://example.org/test-c2'), \
                    (523, 'http://example.org/test-c3')",
        )
        .unwrap();
        for gid in [521, 522, 523] {
            Spi::run(&format!(
                "CREATE TABLE pgrdf._pgrdf_quads_test{gid} \
                 PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN ({gid})",
            ))
            .unwrap();
        }
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:alice ex:p \"p1-alice\" .', 521)",
        )
        .unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:alice ex:q \"q2-alice\" .', 522)",
        )
        .unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:bob ex:p \"p3-bob\" .', 523)",
        )
        .unwrap();

        // Outer GRAPH <c1> binds alice's ?o = "p1-alice"; OPTIONAL
        // GRAPH <c2> { ?s ex:q ?v } binds ?v = "q2-alice" because
        // ?s = alice appears in both graphs' subjects.
        let row_count: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?s ?o ?v WHERE { \
                  GRAPH <http://example.org/test-c1> { ?s ex:p ?o } \
                  OPTIONAL { GRAPH <http://example.org/test-c2> { ?s ex:q ?v } } \
                }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(row_count, 1, "outer GRAPH <c1> yields one row (alice)");

        let optional_match: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?s ?o ?v WHERE { \
                  GRAPH <http://example.org/test-c1> { ?s ex:p ?o } \
                  OPTIONAL { GRAPH <http://example.org/test-c2> { ?s ex:q ?v } } \
                }') AS s(j) \
             WHERE (s.j->>'v') = 'q2-alice'",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            optional_match, 1,
            "OPTIONAL {{ GRAPH <c2> {{ ?s ex:q ?v }} }} must bind ?v from c2"
        );

        // Outer GRAPH <c3> binds bob's ?o; OPTIONAL GRAPH <c2>
        // doesn't bind ?v (bob has no ex:q anywhere). Row survives,
        // ?v IS NULL.
        let unbound: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?s ?o ?v WHERE { \
                  GRAPH <http://example.org/test-c3> { ?s ex:p ?o } \
                  OPTIONAL { GRAPH <http://example.org/test-c2> { ?s ex:q ?v } } \
                }') AS s(j) \
             WHERE (s.j->>'v') IS NULL",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            unbound, 1,
            "unmatched OPTIONAL {{ GRAPH … }} must leave ?v NULL without dropping outer row"
        );
    }

    /// Slice 112 — composition of GRAPH with UNION. Each branch
    /// carries its own literal GRAPH scope; the per-pattern scope
    /// design means the two branches don't accidentally share
    /// constraints.
    #[pg_test]
    fn sparql_graph_composition_with_union() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (531, 'http://example.org/test-u1'), \
                    (532, 'http://example.org/test-u2')",
        )
        .unwrap();
        for gid in [531, 532] {
            Spi::run(&format!(
                "CREATE TABLE pgrdf._pgrdf_quads_test{gid} \
                 PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN ({gid})",
            ))
            .unwrap();
        }
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:alice ex:p \"p1-alice\" .', 531)",
        )
        .unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:bob ex:p \"p2-bob\" .', 532)",
        )
        .unwrap();

        // `{ GRAPH <u1> { ?s ex:p ?o } } UNION { GRAPH <u2> { ?s ex:p ?o } }`
        // — 2 rows total, one per branch.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?o WHERE { \
                  { GRAPH <http://example.org/test-u1> { ?s ex:p ?o } } \
                  UNION \
                  { GRAPH <http://example.org/test-u2> { ?s ex:p ?o } } \
                }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            rows, 2,
            "GRAPH inside UNION branches yields one row per branch (each scoped independently)"
        );
    }

    /// Slice 112 — composition of GRAPH with MINUS. The outer query
    /// is a bare BGP (no GRAPH); the MINUS body is wrapped in
    /// GRAPH <iri>, so MINUS only subtracts rows whose subject
    /// appears in THAT graph's ex:q.
    #[pg_test]
    fn sparql_graph_composition_with_minus() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (541, 'http://example.org/test-m1')",
        )
        .unwrap();
        Spi::run(
            "CREATE TABLE pgrdf._pgrdf_quads_test541 \
             PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN (541)",
        )
        .unwrap();
        // m1: alice has ex:q ; default graph (id 0): alice + bob have ex:p.
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:alice ex:q \"q-alice-m1\" .', 541)",
        )
        .unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:alice ex:p \"p-alice-default\" . \
                  ex:bob   ex:p \"p-bob-default\" .', 0)",
        )
        .unwrap();

        // Outer: every ex:p row from default → alice + bob.
        // MINUS GRAPH <m1> { ?s ex:q ?o2 } subtracts subjects whose
        // ex:q is in m1 — alice. So bob survives, alice doesn't.
        let after_minus: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?s ?o WHERE { \
                  ?s ex:p ?o \
                  MINUS { GRAPH <http://example.org/test-m1> { ?s ex:q ?o2 } } \
                }') AS s(j) \
             WHERE (s.j->>'s') = 'http://example.org/bob'",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            after_minus, 1,
            "bob (no ex:q in m1) must survive MINUS GRAPH <m1>"
        );

        let alice_dropped: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?s ?o WHERE { \
                  ?s ex:p ?o \
                  MINUS { GRAPH <http://example.org/test-m1> { ?s ex:q ?o2 } } \
                }') AS s(j) \
             WHERE (s.j->>'s') = 'http://example.org/alice'",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            alice_dropped, 0,
            "alice (has ex:q in m1) must be dropped by MINUS GRAPH <m1>"
        );
    }

    /// Slice 112 — OPTIONAL inside GRAPH ?g. Both triples MUST come
    /// from the same graph; the outer GRAPH ?g scope propagates
    /// into the OPTIONAL's triple via inherited scope (no fresh
    /// scope_id minted), so the OPTIONAL's `q{opt}.graph_id`
    /// equates to the BGP's first qN graph_id.
    #[pg_test]
    fn sparql_optional_inside_graph_variable() {
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
             VALUES (551, 'http://example.org/test-v1'), \
                    (552, 'http://example.org/test-v2')",
        )
        .unwrap();
        for gid in [551, 552] {
            Spi::run(&format!(
                "CREATE TABLE pgrdf._pgrdf_quads_test{gid} \
                 PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN ({gid})",
            ))
            .unwrap();
        }
        // v1: alice has both ex:p and ex:q. v2: bob has only ex:p.
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:alice ex:p \"p-v1\" . \
                  ex:alice ex:q \"q-v1\" .', 551)",
        )
        .unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(\
                 '@prefix ex: <http://example.org/> . \
                  ex:bob ex:p \"p-v2\" .', 552)",
        )
        .unwrap();

        // GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } } → 2 rows
        // (one per graph). The v1 row has ?v = "q-v1"; the v2 row's
        // OPTIONAL is unmatched (?v IS NULL).
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?g ?o ?v WHERE { \
                  GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } } \
                }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            rows, 2,
            "GRAPH ?g {{ ?s p ?o OPTIONAL {{ ?s q ?v }} }} yields 2 rows (one per graph)"
        );

        let v1_paired: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?g ?o ?v WHERE { \
                  GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } } \
                }') AS s(j) \
             WHERE (s.j->>'g') = 'http://example.org/test-v1' \
               AND (s.j->>'v') = 'q-v1'",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            v1_paired, 1,
            "v1 row pairs ?o + ?v from the same graph (no cross-graph stitch)"
        );

        let v2_unmatched: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?g ?o ?v WHERE { \
                  GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } } \
                }') AS s(j) \
             WHERE (s.j->>'g') = 'http://example.org/test-v2' \
               AND (s.j->>'v') IS NULL",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(
            v2_unmatched, 1,
            "v2's OPTIONAL inherits the outer GRAPH ?g scope; v2 has no ex:q so ?v stays NULL"
        );
    }

    // ─────────────────────────────────────────────────────────────────
    // Phase C slice 84 — SPARQL UPDATE foundation + INSERT DATA
    // ─────────────────────────────────────────────────────────────────

    /// `INSERT DATA { <s> <p> <o> }` against the default graph: the
    /// triple appears in `_pgrdf_quads_g0` and a follow-up SELECT
    /// retrieves it.
    #[pg_test]
    fn sparql_update_insert_data_default_graph() {
        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }')",
        )
        .unwrap()
        .unwrap();
        let summary = &j.0["_update"];
        assert_eq!(summary["form"], "INSERT_DATA");
        assert_eq!(summary["triples_inserted"], 1);
        assert_eq!(summary["triples_deleted"], 0);
        let graphs = summary["graphs_touched"].as_array().unwrap();
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0], "DEFAULT");

        // Round-trip — SELECT against the same triple should find it.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'SELECT ?s ?p ?o WHERE { ?s ?p ?o }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 1, "the INSERT DATA triple must be queryable");
    }

    /// `INSERT DATA { GRAPH <iri> { … } }` against a fresh named
    /// graph: the IRI is auto-allocated, the partition is created,
    /// and the triple lands in the partition.
    #[pg_test]
    fn sparql_update_insert_data_named_graph() {
        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { GRAPH <http://example.org/g1> { \
                  <http://example.org/a> <http://example.org/b> <http://example.org/c> \
                } }')",
        )
        .unwrap()
        .unwrap();
        let summary = &j.0["_update"];
        assert_eq!(summary["form"], "INSERT_DATA");
        assert_eq!(summary["triples_inserted"], 1);
        let graphs = summary["graphs_touched"].as_array().unwrap();
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0], "http://example.org/g1");

        // The triple should live in the named graph's partition.
        let in_graph: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads \
               WHERE graph_id = pgrdf.graph_id('http://example.org/g1')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(in_graph, 1);
    }

    /// The `_update` summary row carries the LLD v0.4 §4.2 shape
    /// (form / triples_inserted / triples_deleted / graphs_touched /
    /// elapsed_ms). Sanity-check the key set and the types.
    #[pg_test]
    fn sparql_update_returns_update_summary_shape() {
        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { <http://example.org/a> <http://example.org/b> \"hi\" }')",
        )
        .unwrap()
        .unwrap();
        let summary = j.0.get("_update").expect("_update key must be present");
        assert!(summary.get("form").is_some(), "form key missing");
        assert!(summary.get("triples_inserted").is_some());
        assert!(summary.get("triples_deleted").is_some());
        assert!(summary.get("graphs_touched").is_some());
        assert!(summary.get("elapsed_ms").is_some());
        assert!(
            summary["elapsed_ms"].as_f64().unwrap_or(-1.0) >= 0.0,
            "elapsed_ms must be a non-negative number"
        );
    }

    /// Idempotency — issuing the same `INSERT DATA` twice must not
    /// duplicate the row. The second call reports
    /// `triples_inserted = 1` (we count attempted inserts, not net
    /// row delta), but `ON CONFLICT DO NOTHING` keeps the table at
    /// one row.
    #[pg_test]
    fn sparql_update_insert_data_idempotent_on_repeat() {
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }')",
        )
        .unwrap();
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }')",
        )
        .unwrap();
        let n: i64 = Spi::get_one("SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads")
            .unwrap()
            .unwrap_or(0);
        assert_eq!(n, 1, "INSERT DATA must be set-semantic (no duplicates)");
    }

    // ─────────────────────────────────────────────────────────────────
    // Phase C slice 83 — SPARQL UPDATE DELETE DATA
    // ─────────────────────────────────────────────────────────────────

    /// `DELETE DATA { … }` removes an existing triple from the default
    /// graph. The `_update` summary reports `triples_deleted = 1`,
    /// `form = "DELETE_DATA"`, and the post-delete SELECT count drops
    /// by one.
    #[pg_test]
    fn sparql_update_delete_data_removes_existing() {
        // Seed three triples via INSERT DATA so we can verify both
        // the count of deletions and the count of survivors.
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { \
                  <http://example.org/a> <http://example.org/p> <http://example.org/v1> . \
                  <http://example.org/a> <http://example.org/p> <http://example.org/v2> . \
                  <http://example.org/a> <http://example.org/p> <http://example.org/v3> . \
                }')",
        )
        .unwrap();

        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'DELETE DATA { <http://example.org/a> <http://example.org/p> <http://example.org/v2> }')",
        )
        .unwrap()
        .unwrap();
        let summary = &j.0["_update"];
        assert_eq!(summary["form"], "DELETE_DATA");
        assert_eq!(summary["triples_inserted"], 0);
        assert_eq!(summary["triples_deleted"], 1);
        let graphs = summary["graphs_touched"].as_array().unwrap();
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0], "DEFAULT");

        let remaining: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'SELECT ?o WHERE { <http://example.org/a> <http://example.org/p> ?o }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(remaining, 2, "the two un-deleted triples must still be queryable");

        // Same triple deleted twice — the second call is a no-op.
        let j2: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'DELETE DATA { <http://example.org/a> <http://example.org/p> <http://example.org/v2> }')",
        )
        .unwrap()
        .unwrap();
        assert_eq!(j2.0["_update"]["triples_deleted"], 0);
    }

    /// DELETE DATA against a triple whose terms aren't in the
    /// dictionary at all — spec-correct no-op, never errors.
    #[pg_test]
    fn sparql_update_delete_data_missing_term_is_noop() {
        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'DELETE DATA { <http://example.org/never> <http://example.org/seen> <http://example.org/before> }')",
        )
        .unwrap()
        .unwrap();
        let summary = &j.0["_update"];
        assert_eq!(summary["form"], "DELETE_DATA");
        assert_eq!(summary["triples_deleted"], 0);
        assert_eq!(summary["triples_inserted"], 0);

        // Mixed case — half the terms exist (from a prior insert),
        // the other half don't. Still no-op (the full quad isn't
        // in `_pgrdf_quads`).
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { <http://example.org/a> <http://example.org/p> <http://example.org/b> }')",
        )
        .unwrap();
        let j2: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'DELETE DATA { <http://example.org/a> <http://example.org/p> <http://example.org/c> }')",
        )
        .unwrap()
        .unwrap();
        // <http://example.org/a> and <http://example.org/p> are in
        // the dictionary; <http://example.org/c> is NOT — full quad
        // therefore can't exist, no-op.
        assert_eq!(j2.0["_update"]["triples_deleted"], 0);

        // The original triple still survives the no-op attempt.
        let rows: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'SELECT ?o WHERE { <http://example.org/a> <http://example.org/p> ?o }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(rows, 1, "the original quad must survive a no-op DELETE");
    }

    /// `DELETE DATA { GRAPH <iri> { … } }` scopes the removal to the
    /// named graph: a same-shape triple in the default graph is NOT
    /// touched, and the per-graph partition row count drops.
    #[pg_test]
    fn sparql_update_delete_data_named_graph() {
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { \
                  <http://example.org/a> <http://example.org/p> <http://example.org/b> . \
                  GRAPH <http://example.org/g1> { \
                    <http://example.org/a> <http://example.org/p> <http://example.org/b> \
                  } \
                }')",
        )
        .unwrap();

        // Both partitions carry one row.
        let before_default: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads WHERE graph_id = 0",
        )
        .unwrap()
        .unwrap_or(0);
        let before_g1: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads \
               WHERE graph_id = pgrdf.graph_id('http://example.org/g1')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(before_default, 1);
        assert_eq!(before_g1, 1);

        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'DELETE DATA { GRAPH <http://example.org/g1> { \
                  <http://example.org/a> <http://example.org/p> <http://example.org/b> \
                } }')",
        )
        .unwrap()
        .unwrap();
        let summary = &j.0["_update"];
        assert_eq!(summary["form"], "DELETE_DATA");
        assert_eq!(summary["triples_deleted"], 1);
        let graphs = summary["graphs_touched"].as_array().unwrap();
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0], "http://example.org/g1");

        // Default graph row untouched; named graph drops to zero.
        let after_default: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads WHERE graph_id = 0",
        )
        .unwrap()
        .unwrap_or(0);
        let after_g1: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads \
               WHERE graph_id = pgrdf.graph_id('http://example.org/g1')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(after_default, 1, "default-graph copy survives a named-graph DELETE");
        assert_eq!(after_g1, 0, "the named-graph copy is gone");
    }

    // ─────────────────────────────────────────────────────────────────
    // Phase C slice 82 — SPARQL UPDATE INSERT WHERE pattern-driven
    // ─────────────────────────────────────────────────────────────────

    /// Happy path. Seed two `rdf:type ex:Person` triples via INSERT
    /// DATA, then `INSERT { ?x ex:tag "person" } WHERE { ?x rdf:type
    /// ex:Person }` — exactly two new triples land, one per solution.
    #[pg_test]
    fn sparql_update_insert_where_happy_path() {
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'PREFIX ex:  <http://example.org/> \
                PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
                INSERT DATA { \
                  ex:alice rdf:type ex:Person . \
                  ex:bob   rdf:type ex:Person \
                }')",
        )
        .unwrap();

        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'PREFIX ex:  <http://example.org/> \
                PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> \
                INSERT { ?x ex:tag \"person\" } WHERE { ?x rdf:type ex:Person }')",
        )
        .unwrap()
        .unwrap();
        let summary = &j.0["_update"];
        assert_eq!(summary["form"], "INSERT_WHERE");
        assert_eq!(
            summary["triples_inserted"], 2,
            "two solution rows ⇒ two template instantiations"
        );
        assert_eq!(summary["triples_deleted"], 0);

        // Verify the new triples are queryable. Both subjects should
        // carry the new ex:tag "person" assertion.
        let tagged: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?s WHERE { ?s ex:tag \"person\" }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(tagged, 2, "both subjects should carry the new tag");
    }

    /// Zero-match no-op. INSERT WHERE against a pattern that returns
    /// no solutions reports `triples_inserted = 0` and the table
    /// remains empty of the template-produced rows.
    #[pg_test]
    fn sparql_update_insert_where_zero_match_noop() {
        // Seed one unrelated triple so the database isn't pristine.
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { <http://example.org/a> <http://example.org/b> \"hi\" }')",
        )
        .unwrap();
        let before: i64 =
            Spi::get_one("SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads")
                .unwrap()
                .unwrap_or(-1);

        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'PREFIX foaf: <http://xmlns.com/foaf/0.1/> \
                PREFIX ex:   <http://example.org/> \
                INSERT { ?x ex:name ?n } WHERE { ?x foaf:name ?n }')",
        )
        .unwrap()
        .unwrap();
        let summary = &j.0["_update"];
        assert_eq!(summary["form"], "INSERT_WHERE");
        assert_eq!(summary["triples_inserted"], 0);
        let after: i64 = Spi::get_one("SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads")
            .unwrap()
            .unwrap_or(-1);
        assert_eq!(
            before, after,
            "INSERT WHERE with no matches must not touch the quads table"
        );
    }

    /// Multi-triple template + multi-row solution. Two solution rows
    /// × three template quads = six new triples. Lock the cross-
    /// product so set-semantics on the WHERE NOT EXISTS guard
    /// doesn't accidentally swallow distinct rows.
    #[pg_test]
    fn sparql_update_insert_where_multi_triple_template() {
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'PREFIX ex:  <http://example.org/> \
                INSERT DATA { ex:a ex:label \"A\" . ex:b ex:label \"B\" }')",
        )
        .unwrap();

        let j: pgrx::JsonB = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                INSERT { ?s ex:tag1 \"t1\" . ?s ex:tag2 \"t2\" . ?s ex:lbl ?l } \
                WHERE  { ?s ex:label ?l }')",
        )
        .unwrap()
        .unwrap();
        let summary = &j.0["_update"];
        assert_eq!(
            summary["triples_inserted"], 6,
            "2 rows × 3 template quads = 6 inserted"
        );
        // Round-trip the two ?l bindings projected via the template.
        let bound_labels: i64 = Spi::get_one(
            "SELECT count(*)::BIGINT FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                SELECT ?s ?l WHERE { ?s ex:lbl ?l }')",
        )
        .unwrap()
        .unwrap_or(0);
        assert_eq!(bound_labels, 2);
    }

    /// Negative — unbound template variable panics with the stable
    /// `INSERT WHERE template feature 'unbound template variable` prefix.
    /// Downstream tooling routes on this for partial-translatability.
    #[pg_test(
        error = "sparql: INSERT WHERE template feature 'unbound template variable ?z' not yet supported"
    )]
    fn sparql_update_insert_where_unbound_template_var_panics() {
        Spi::run(
            "SELECT * FROM pgrdf.sparql(\
               'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }')",
        )
        .unwrap();
        let _: Option<pgrx::JsonB> = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'PREFIX ex: <http://example.org/> \
                INSERT { ?x ex:tag ?z } WHERE { ?x ?p ?o }')",
        )
        .ok()
        .flatten();
    }

    /// Negative — combined DELETE+INSERT WHERE still panics with the
    /// slice-77 prefix (the contiguous substring slice 84's regression
    /// test 93 locks must still appear post-slice-82).
    #[pg_test(
        error = "sparql: UPDATE form 'DELETE/INSERT WHERE' lands in slice 77"
    )]
    fn sparql_update_delete_insert_combined_still_panics() {
        let _: Option<pgrx::JsonB> = Spi::get_one(
            "SELECT * FROM pgrdf.sparql(\
               'DELETE { ?s ?p ?o } INSERT { ?s ?p \"new\" } WHERE { ?s ?p ?o }')",
        )
        .ok()
        .flatten();
    }
}
