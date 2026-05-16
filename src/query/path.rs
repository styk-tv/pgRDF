//! SPARQL property-path translation — Phase E groups E1 + E2
//! (LLD v0.4 §7).
//!
//! Property paths arrive in the spargebra algebra as
//! `GraphPattern::Path { subject, path, object }`, where `path` is a
//! [`PropertyPathExpression`]. The v0.3 translator only handled plain
//! `GraphPattern::Bgp` triples; this module is the dispatch point that
//! lowers a property path back into the existing BGP machinery (E1
//! non-recursive surface) OR emits a recursive-CTE-derived relation
//! the BGP builder joins like an ordinary `_pgrdf_quads` alias (E2
//! `+`).
//!
//! ## Module boundary (the path.rs carve)
//!
//! Phase E group E1 created this module as the property-path home
//! (the executor never carried path-translation code — E1 built it
//! here from the start). E2 keeps the carve discipline: ALL
//! property-path SQL generation — the recursive-CTE builder, the
//! truncation probe, the classifier, the preview-panic emitters —
//! lives here. `executor.rs` only calls into `path::…` and threads
//! the resulting [`PathRelation`] through its existing FROM/WHERE
//! builder. This keeps `executor.rs` from re-growing the
//! ~3 500-line translator and de-risks E3/E4.
//!
//! ## What E1 ships (LLD v0.4 §7.2 / §7.3)
//!
//! * **Bare predicate** — `NamedNode(p)`. spargebra sometimes wraps
//!   an ordinary predicate as a `Path` when it sits adjacent to a
//!   path operator (or under certain parser productions). It is
//!   semantically identical to the triple `?s p ?o`, so we rewrite
//!   it to exactly that `TriplePattern` and let `pattern_clauses` do
//!   the rest.
//! * **Inverse** — `Reverse(NamedNode(p))` = `^p`. Per §7.2 this
//!   needs **no recursion**: `?s ^p ?o` ≡ `?o p ?s`. We rewrite to
//!   the same predicate triple with subject/object **swapped**.
//!   Nested reverses collapse pairwise (`^(^p)` = `p` — the W3C
//!   grammar reserves the `^^` token for typed-literal datatypes, so
//!   a double inverse is written `^(^p)` and arrives as
//!   `Reverse(Reverse(NamedNode))`), so we fold an even/odd swap
//!   count down to a single (possibly swapped) triple.
//!
//! ## What E2 ships (LLD v0.4 §7.2 / §7.3)
//!
//! * **One-or-more** — `OneOrMore(NamedNode(p))` = `p+`, plus the
//!   inverse compositions `^p+` (`Reverse(OneOrMore(NamedNode))`) and
//!   `(^p)+` (`OneOrMore(Reverse(NamedNode))`) — semantically the
//!   same "transitive non-reflexive closure of (possibly inverted)
//!   `p`". E2 emits the `WITH RECURSIVE walk(src, dst, depth)` CTE
//!   from LLD v0.4 §7.2 as a derived FROM relation. Cycle-safety
//!   uses Postgres's `CYCLE src, dst SET is_cycle USING path` clause
//!   (PG14+): the spec sketch's bare `UNION` cannot dedup a cycle
//!   because the working tuple carries `depth` (so `(a,b,1)` and
//!   `(a,b,4)` are distinct rows and a cycle would spin to the depth
//!   cap). `UNION ALL` + `CYCLE` stops extending a path the moment a
//!   `(src,dst)` pair repeats on it — the spec's "natural cycle
//!   handling" intent, done correctly. The recursive arm's
//!   `WHERE w.depth < $MAX_DEPTH` still enforces the
//!   `pgrdf.path_max_depth` depth guard for genuinely-long ACYCLIC
//!   paths (truncate, never error — §7.2). A per-`+` probe query
//!   detects whether the cap was actually hit (a cycle terminates
//!   before the cap, so it never false-reports) so
//!   `pgrdf.stats().path_depth_truncations` reflects it.
//!
//! Because the E1 output is an ordinary [`TriplePattern`] and the E2
//! output is a relation exposing `subject_id` / `object_id` columns
//! (the same columns the BGP var-binder reads off a `_pgrdf_quads`
//! alias), both compose for free with everything the BGP walker
//! already supports: named-graph scoping (`GRAPH <iri>` /
//! `GRAPH ?g`), multi-pattern BGP joins, OPTIONAL / UNION / MINUS
//! wrappers, and `pgrdf.construct` (which routes its WHERE through
//! the same `parse_select` walker).
//!
//! ## What E2 does NOT ship (deferred — stable preview panics)
//!
//! `*` / `?` need the reflexive base case (LLD v0.4 §7.2); they land
//! in Phase E group E3. Alternation (`|`) and the materialised-
//! closure no-CTE optimisation are group E4 (the `|` arm is gated).
//! A `+` whose inner box is itself recursive / alternation /
//! sequence (`(p*)+`, `(a|b)+`, `(p1/p2)+`) is exotic and lands with
//! the nested-recursive consolidation in group E4. Negated property
//! sets are out of v0.4 scope entirely. Each panics with a STABLE
//! prefix so downstream tooling can preview the rollout schedule
//! without depending on the (slice-number-bearing) tail — the exact
//! same convention Phase C's per-form UPDATE panics use.

use spargebra::algebra::PropertyPathExpression;
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern};

/// Stable panic prefix for `+` (one-or-more) whose inner box is NOT
/// a plain (optionally inverted) predicate — e.g. `(p*)+`. The plain
/// `p+` / `^p+` / `(^p)+` forms are executable from E2; the nested-
/// recursive case lands with the group E4 consolidation.
pub(crate) const PANIC_ONE_OR_MORE_NESTED: &str =
    "pgrdf: nested recursive property path (e.g. `(p*)+`) lands in Phase E group E4";

/// Stable panic prefix for `*` (zero-or-more). Lands in Phase E group
/// E3 (≈ slice 40).
pub(crate) const PANIC_ZERO_OR_MORE: &str =
    "pgrdf: property path operator '*' lands in Phase E group E3 (slice 40)";

/// Stable panic prefix for `?` (zero-or-one). Lands in Phase E group
/// E3 (≈ slice 40).
pub(crate) const PANIC_ZERO_OR_ONE: &str =
    "pgrdf: property path operator '?' lands in Phase E group E3 (slice 40)";

/// Stable panic for alternation `|` — a gated stretch goal (group E4).
pub(crate) const PANIC_ALTERNATION: &str =
    "pgrdf: property path alternation '|' is a gated stretch goal (Phase E group E4)";

/// Stable panic for negated property sets `!(...)` — out of v0.4 scope.
pub(crate) const PANIC_NEGATED: &str = "pgrdf: negated property sets are out of scope for v0.4";

/// Stable rejection for sequence paths `p1/p2`. They are already a
/// 2-pattern BGP in user-facing SPARQL; E2 keeps E1's stance and does
/// not desugar (would mint a synthetic join var that pollutes
/// `SELECT *`).
pub(crate) const PANIC_SEQUENCE: &str =
    "pgrdf: sequence property paths (p1/p2) are not a property-path \
     operator in pgRDF — express them as a multi-pattern BGP \
     (`{ ?s p1 ?mid . ?mid p2 ?o }`)";

/// How a [`PropertyPathExpression`] lowers for execution.
///
/// * `Triple` — the E1 non-recursive set (bare predicate, `^p`,
///   nested `^(^…)`). Lowered to an ordinary [`TriplePattern`];
///   `executor.rs` pushes it like a BGP triple.
/// * `OneOrMore { predicate, swapped }` — the E2 `+` set
///   (`p+`, `^p+`, `(^p)+`). `predicate` is the resolved IRI of the
///   single predicate walked; `swapped` is true when the closure is
///   over the *inverse* edge (subject/object roles flipped — `^p+`
///   ≡ `(^p)+`, the inverse of a transitive closure equals the
///   transitive closure of the inverse). `executor.rs` builds the
///   recursive CTE relation from this.
pub(crate) enum PathPlan {
    Triple(Box<TriplePattern>),
    OneOrMore { predicate: NamedNode, swapped: bool },
}

/// Classify a property-path pattern into its execution plan, or panic
/// with the stable rollout-preview prefix for a not-yet-shipped
/// operator. `subject` / `object` are the outer term patterns; for
/// the `Triple` plan they are baked into the lowered triple (with the
/// subject/object swap applied for the inverse case), for the
/// `OneOrMore` plan `executor.rs` binds them against the CTE
/// relation's `src` / `dst` columns.
pub(crate) fn classify_path(
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
) -> PathPlan {
    // Top-level `Reverse` wrappers fold by parity into a single
    // `swapped` flag. `^(p+)` (= `Reverse(OneOrMore(NamedNode))`) and
    // `(^p)+` (= `OneOrMore(Reverse(NamedNode))`) are semantically
    // identical (inverse of a transitive closure = transitive closure
    // of the inverse), so both collapse to the same plan.
    let mut swapped = false;
    let mut cur = path;
    loop {
        match cur {
            PropertyPathExpression::Reverse(inner) => {
                swapped = !swapped;
                cur = inner;
            }
            PropertyPathExpression::NamedNode(p) => {
                // E1 non-recursive surface — lower to a triple.
                let predicate = NamedNodePattern::NamedNode(p.clone());
                let (s, o) = if swapped {
                    (object.clone(), subject.clone())
                } else {
                    (subject.clone(), object.clone())
                };
                return PathPlan::Triple(Box::new(TriplePattern {
                    subject: s,
                    predicate,
                    object: o,
                }));
            }
            PropertyPathExpression::OneOrMore(inner) => {
                // E2 — `+`. The inner box must (for E2) be a plain
                // predicate or an inverted predicate; fold any inner
                // `Reverse` parity into the same `swapped` flag.
                let mut inner_swapped = swapped;
                let mut ic = inner.as_ref();
                loop {
                    match ic {
                        PropertyPathExpression::Reverse(b) => {
                            inner_swapped = !inner_swapped;
                            ic = b;
                        }
                        PropertyPathExpression::NamedNode(p) => {
                            return PathPlan::OneOrMore {
                                predicate: p.clone(),
                                swapped: inner_swapped,
                            };
                        }
                        // `(p*)+`, `(a|b)+`, `(p1/p2)+` — nested
                        // recursive / alternation / sequence inner.
                        // Exotic; lands with the E4 consolidation.
                        _ => panic!("{PANIC_ONE_OR_MORE_NESTED}"),
                    }
                }
            }
            PropertyPathExpression::ZeroOrMore(_) => panic!("{PANIC_ZERO_OR_MORE}"),
            PropertyPathExpression::ZeroOrOne(_) => panic!("{PANIC_ZERO_OR_ONE}"),
            PropertyPathExpression::Alternative(_, _) => panic!("{PANIC_ALTERNATION}"),
            PropertyPathExpression::NegatedPropertySet(_) => panic!("{PANIC_NEGATED}"),
            PropertyPathExpression::Sequence(_, _) => panic!("{PANIC_SEQUENCE}"),
        }
    }
}

/// Is this property-path expression *executable* under the
/// currently-shipped operator set (E1 lower-to-triple ∪ E2 `+`)?
///
/// `true`  → bare predicate, `^p`, nested `^(^…)`, OR `p+` / `^p+` /
///           `(^p)+` (one-or-more over an optionally-inverted
///           predicate).
/// `false` → `*` / `?` (E3), `|` (E4), negated set, sequence, or a
///           `+` with a nested-recursive inner (E4).
///
/// Used by `parser.rs` so `sparql_parse` does NOT flag the now-
/// executable forms in `unsupported_algebra` (parse-time, no panic);
/// the genuinely deferred forms still get flagged. Execution of a
/// deferred form panics with the stable rollout-preview prefix.
pub(crate) fn is_executable(path: &PropertyPathExpression) -> bool {
    let mut cur = path;
    loop {
        match cur {
            PropertyPathExpression::Reverse(inner) => cur = inner,
            PropertyPathExpression::NamedNode(_) => return true,
            PropertyPathExpression::OneOrMore(inner) => {
                // `+` is executable iff its inner box folds (through
                // any `Reverse` wrappers) down to a plain predicate.
                let mut ic = inner.as_ref();
                loop {
                    match ic {
                        PropertyPathExpression::Reverse(b) => ic = b,
                        PropertyPathExpression::NamedNode(_) => return true,
                        _ => return false,
                    }
                }
            }
            _ => return false,
        }
    }
}

/// Parser-facing analysis view of an *executable* property path
/// (E1 lower-to-triple set ∪ E2 `+`) as a [`TriplePattern`], WITHOUT
/// running the SQL-side relation lowering (a `+` has no triple form
/// for execution — it is a recursive CTE). For the E1 set this is
/// exactly the
/// lowered triple; for `+` it is `(subject, predicate, object)` with
/// the inverse subject/object swap applied — the predicate is a
/// `NamedNode` (a `+` walks a fixed predicate, never a variable), so
/// `collect_vars` / `collect_pattern_vars` see only the subject /
/// object variables, which is correct: `?s p+ ?o` binds `?s` and
/// `?o` exactly like a triple would. Returns `None` for a
/// not-yet-executable form (the caller flags it `unsupported_algebra`
/// instead — parse-time, no panic).
pub(crate) fn analysis_triple(
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
) -> Option<TriplePattern> {
    if !is_executable(path) {
        return None;
    }
    match classify_path(subject, path, object) {
        PathPlan::Triple(tp) => Some(*tp),
        PathPlan::OneOrMore { predicate, swapped } => {
            let (s, o) = if swapped {
                (object.clone(), subject.clone())
            } else {
                (subject.clone(), object.clone())
            };
            Some(TriplePattern {
                subject: s,
                predicate: NamedNodePattern::NamedNode(predicate),
                object: o,
            })
        }
    }
}

/// A `+` (one-or-more) path lowered to a recursive-CTE-derived
/// relation. `executor.rs` substitutes this for a `_pgrdf_quads`
/// alias in its FROM list: the relation exposes the same
/// `subject_id` / `object_id` column names a quad alias would, so the
/// existing var-binder (`bind_var` on `q{qi}.subject_id` /
/// `q{qi}.object_id`) joins it unchanged.
///
/// `from_fragment` is the parenthesised derived table WITHOUT the
/// trailing alias (executor appends `AS q{qi}(...)`). `probe_sql` is
/// the standalone truncation-detection query (run post-execution; if
/// it returns `t`, the depth guard actually cut a path → bump
/// `path_depth_truncations`). `probe_params` are the `$N` dict ids
/// the probe binds, in order.
#[derive(Clone)]
pub(crate) struct PathRelation {
    pub from_fragment: String,
    /// Column list the executor pins on the alias —
    /// `(subject_id, object_id)` for an unscoped / literal-graph
    /// walk, `(subject_id, object_id, graph_id)` when a `GRAPH ?g`
    /// variable scope needs the per-row graph id surfaced.
    pub columns: &'static str,
    pub probe_sql: String,
    pub probe_params: Vec<i64>,
}

/// Graph-scope flavour the recursive CTE must honour. Mirrors the
/// three `GraphScope` shapes `executor.rs` already threads through
/// the BGP builder, reduced to what the CTE needs.
pub(crate) enum PathGraphScope {
    /// Unscoped BGP — slice-112 semantic: scan ALL graphs (default +
    /// named). The CTE applies no `graph_id` predicate; edges may
    /// span graphs (documented, matches how E1's `^` handled an
    /// unscoped pattern).
    AllGraphs,
    /// `GRAPH <iri> { … }` — every hop constrained to one resolved
    /// `graph_id` (`-1` sentinel when the IRI is unbound → zero rows,
    /// spec-correct "no solutions").
    Literal(i64),
    /// `GRAPH ?g { … }` — the whole walk stays inside ONE named
    /// graph; the CTE carries `graph_id` so the recursive hop can
    /// require `q.graph_id = w.gid`, and the executor joins
    /// `_pgrdf_graphs` on the surfaced column for `?g`. Named graphs
    /// only (W3C SPARQL 1.1 §13.3): the base arm excludes
    /// `graph_id = 0`.
    Variable,
}

/// Build the recursive-CTE-derived relation for a `+` path (LLD v0.4
/// §7.2). `predicate_placeholder` is the `$N` placeholder for the
/// resolved predicate dict id (the caller appended it to the param
/// buffer at the correct ordinal); `graph_placeholder` is the
/// optional `$M` for the `Literal` scope's resolved graph id.
/// `max_depth` is `query::guc::path_max_depth()` (read once at
/// translate time — the depth guard is a hard cap baked into the
/// recursive arm's `WHERE`).
///
/// The CTE matches LLD v0.4 §7.2 (adapted to pgRDF's
/// `_pgrdf_quads(subject_id, predicate_id, object_id, graph_id)`
/// schema and dict-id placeholders):
///
/// ```text
/// SELECT subject_id, object_id [, graph_id], 1 FROM _pgrdf_quads
///   WHERE predicate_id = $P [graph predicate]
/// UNION
/// SELECT w.src, q.object_id [, w.gid], w.depth + 1
///   FROM walk w JOIN _pgrdf_quads q ON q.subject_id = w.dst
///   WHERE q.predicate_id = $P AND w.depth < $MAX [AND same-graph]
/// ```
///
/// `swapped` (the `^p+` / `(^p)+` case) flips the edge direction:
/// the base arm reads `object_id, subject_id` and the recursive arm
/// joins `q.object_id = w.dst` projecting `q.subject_id`. `UNION`
/// (not `UNION ALL`) makes cycles terminate (a revisited (src,dst)
/// pair is deduped); `w.depth < $MAX` is the hard depth cap.
pub(crate) fn build_one_or_more_relation_sql(
    predicate_placeholder: &str,
    graph_placeholder: Option<&str>,
    scope: &PathGraphScope,
    swapped: bool,
    max_depth: i32,
    probe_predicate_id: i64,
    probe_graph_id: Option<i64>,
) -> PathRelation {
    // Edge endpoints depend on direction. Forward `p+`: walk
    // subject → object. Inverse `^p+`: walk object → subject.
    let (base_src, base_dst, rec_join_col, rec_proj_col) = if swapped {
        ("object_id", "subject_id", "object_id", "subject_id")
    } else {
        ("subject_id", "object_id", "subject_id", "object_id")
    };

    // Per-scope graph predicates + whether the CTE carries a `gid`
    // column (only the `Variable` scope needs it surfaced).
    let (base_graph_pred, rec_graph_pred, carries_gid, columns): (
        String,
        String,
        bool,
        &'static str,
    ) = match scope {
        PathGraphScope::AllGraphs => (
            String::new(),
            String::new(),
            false,
            "(subject_id, object_id)",
        ),
        PathGraphScope::Literal(_) => {
            let g = graph_placeholder.expect("Literal scope needs a graph placeholder");
            (
                format!(" AND graph_id = {g}"),
                format!(" AND q.graph_id = {g}"),
                false,
                "(subject_id, object_id)",
            )
        }
        PathGraphScope::Variable => (
            // Named graphs only (W3C §13.3): exclude the default
            // graph from the base arm so `?g` never binds graph 0.
            " AND graph_id <> 0".to_string(),
            // Recursive hop must stay in the SAME named graph the
            // base row started in.
            " AND q.graph_id = w.gid".to_string(),
            true,
            "(subject_id, object_id, graph_id)",
        ),
    };

    let base_gid = if carries_gid { ", graph_id" } else { "" };
    let rec_gid = if carries_gid { ", w.gid" } else { "" };
    let walk_cols = if carries_gid {
        "walk(src, dst, gid, depth)"
    } else {
        "walk(src, dst, depth)"
    };
    let final_cols = if carries_gid {
        "src, dst, gid"
    } else {
        "src, dst"
    };

    // The whole relation is a self-contained parenthesised subquery
    // with its OWN `WITH RECURSIVE` (Postgres allows a CTE local to a
    // derived table) — no top-level WITH plumbing in executor.rs, so
    // every non-path query is byte-identical to before.
    //
    // Cycle handling (LLD v0.4 §7.2 intent — "natural cycle
    // handling"): the spec sketch used bare `UNION`, but the working
    // tuple has to carry `depth` for the guard, and `UNION` dedups on
    // the FULL row — so `(a,b,1)` and `(a,b,4)` are distinct and a
    // cycle would spin up to the depth cap (O(MAX) work + a spurious
    // truncation report). Postgres's `CYCLE src, dst SET is_cycle
    // USING path` clause (PG14+) is the correct mechanism: it stops
    // extending a path the moment a `(src, dst)` pair repeats ON THAT
    // PATH, so a cycle terminates after one lap regardless of the
    // depth cap. `UNION ALL` is required by the CYCLE clause; the
    // final `SELECT DISTINCT … WHERE NOT is_cycle` drops the
    // cycle-closing marker row and dedups the (src,dst) projection.
    // The depth cap stays as the bound for genuinely-long ACYCLIC
    // paths (the truncation case).
    let from_fragment = format!(
        "(WITH RECURSIVE {walk_cols} AS (\
           SELECT {base_src}, {base_dst}{base_gid}, 1 \
             FROM pgrdf._pgrdf_quads \
            WHERE predicate_id = {predicate_placeholder}{base_graph_pred} \
         UNION ALL \
           SELECT w.src, q.{rec_proj_col}{rec_gid}, w.depth + 1 \
             FROM walk w \
             JOIN pgrdf._pgrdf_quads q ON q.{rec_join_col} = w.dst \
            WHERE q.predicate_id = {predicate_placeholder} \
              AND w.depth < {max_depth}{rec_graph_pred}\
         ) CYCLE src, dst SET is_cycle USING path \
         SELECT DISTINCT {final_cols} FROM walk WHERE NOT is_cycle)"
    );

    // ─── Truncation probe (LLD v0.4 §7.2 depth-guard accounting) ──
    //
    // Precise detector: did ANY walk row land at `depth == $MAX`
    // whose `dst` still has an outgoing `$P` edge (in the active
    // graph scope) — i.e. the guard cut a path that could have
    // continued? This NEVER under-counts: if a continuation exists
    // past the cap the EXISTS fires. It can only slightly OVER-count
    // (the benign §7.2-permitted case where the continuation node
    // was already reached via a shorter path); over-counting is
    // explicitly acceptable, claiming-complete-when-truncated is not.
    //
    // The probe rebuilds the same bounded walk, then asks the
    // continuation question. It is a standalone scalar
    // `SELECT CASE WHEN EXISTS(…) THEN 1 ELSE 0 END` returning a
    // BIGINT — `executor.rs` reads it with the same `.select(...,
    // Some(1), ...)` + `get::<i64>(1)` idiom every other scalar probe
    // in this file uses (avoids any bool text-vs-typed ambiguity).
    let probe_walk_cols = if carries_gid {
        "pwalk(src, dst, gid, depth)"
    } else {
        "pwalk(src, dst, depth)"
    };
    let (probe_base_graph, probe_rec_graph, probe_cont_graph): (String, String, String) =
        match scope {
            PathGraphScope::AllGraphs => (String::new(), String::new(), String::new()),
            PathGraphScope::Literal(gid) => (
                format!(" AND graph_id = {gid}"),
                format!(" AND q.graph_id = {gid}"),
                format!(" AND c.graph_id = {gid}"),
            ),
            PathGraphScope::Variable => (
                " AND graph_id <> 0".to_string(),
                " AND q.graph_id = w.gid".to_string(),
                " AND c.graph_id = w.gid".to_string(),
            ),
        };
    let p_base_gid = if carries_gid { ", graph_id" } else { "" };
    let p_rec_gid = if carries_gid { ", w.gid" } else { "" };
    // The probe mirrors the relation's `UNION ALL` + `CYCLE` walk
    // (same cycle-safety), then asks: is there a NON-cycle row at
    // exactly the depth cap whose `dst` still has an outgoing `$P`
    // edge? A cycle terminates before the cap (CYCLE clause), so it
    // never produces a `depth == MAX` row → a fully-resolved cyclic
    // query correctly reports NO truncation. Only a genuinely long
    // ACYCLIC path that the cap actually severed fires the probe.
    let probe_sql = format!(
        "SELECT CASE WHEN EXISTS (\
           WITH RECURSIVE {probe_walk_cols} AS (\
             SELECT {base_src}, {base_dst}{p_base_gid}, 1 \
               FROM pgrdf._pgrdf_quads \
              WHERE predicate_id = $1{probe_base_graph} \
           UNION ALL \
             SELECT w.src, q.{rec_proj_col}{p_rec_gid}, w.depth + 1 \
               FROM pwalk w \
               JOIN pgrdf._pgrdf_quads q ON q.{rec_join_col} = w.dst \
              WHERE q.predicate_id = $1 \
                AND w.depth < {max_depth}{probe_rec_graph}\
           ) CYCLE src, dst SET is_cycle USING path \
           SELECT 1 FROM pwalk w \
            WHERE NOT w.is_cycle \
              AND w.depth = {max_depth} \
              AND EXISTS (\
                SELECT 1 FROM pgrdf._pgrdf_quads c \
                 WHERE c.{rec_join_col} = w.dst \
                   AND c.predicate_id = $1{probe_cont_graph}\
              )\
         ) THEN 1::bigint ELSE 0::bigint END"
    );
    // The probe binds ONLY the predicate dict id as `$1`. The
    // Literal scope's resolved graph id is a translate-time integer
    // constant (not user input) and is inlined directly into
    // `probe_base_graph` / `probe_rec_graph` / `probe_cont_graph`
    // above — keeping it out of the param vec keeps the probe's
    // single-`$1` shape uniform across all three scope flavours.
    // `probe_graph_id` is accepted for call-site symmetry with the
    // main relation builder; it is intentionally not threaded into
    // the param vec (see the inlining above).
    let _ = probe_graph_id;
    let probe_params = vec![probe_predicate_id];

    PathRelation {
        from_fragment,
        columns,
        probe_sql,
        probe_params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spargebra::term::{NamedNode, Variable};

    fn var(name: &str) -> TermPattern {
        TermPattern::Variable(Variable::new(name).unwrap())
    }
    fn iri(s: &str) -> NamedNode {
        NamedNode::new(s).unwrap()
    }
    /// Classify and unwrap the E1 lower-to-triple plan (test-only —
    /// the executor uses `classify_path` + `scoped_triple_from_path`
    /// directly; `+` never reaches the triple form).
    fn lower_triple(s: &TermPattern, p: &PropertyPathExpression, o: &TermPattern) -> TriplePattern {
        match classify_path(s, p, o) {
            PathPlan::Triple(tp) => *tp,
            PathPlan::OneOrMore { .. } => panic!("expected a lower-to-triple plan"),
        }
    }

    #[test]
    fn bare_named_node_is_direct_triple() {
        let p = PropertyPathExpression::NamedNode(iri("http://example.org/p"));
        let tp = lower_triple(&var("s"), &p, &var("o"));
        assert!(matches!(tp.subject, TermPattern::Variable(ref v) if v.as_str() == "s"));
        assert!(matches!(tp.object, TermPattern::Variable(ref v) if v.as_str() == "o"));
        assert!(
            matches!(tp.predicate, NamedNodePattern::NamedNode(ref n) if n.as_str() == "http://example.org/p")
        );
    }

    #[test]
    fn reverse_swaps_subject_object() {
        let p = PropertyPathExpression::Reverse(Box::new(PropertyPathExpression::NamedNode(iri(
            "http://example.org/p",
        ))));
        let tp = lower_triple(&var("s"), &p, &var("o"));
        // `?s ^p ?o` ≡ `?o p ?s` — subject is the original object.
        assert!(matches!(tp.subject, TermPattern::Variable(ref v) if v.as_str() == "o"));
        assert!(matches!(tp.object, TermPattern::Variable(ref v) if v.as_str() == "s"));
    }

    #[test]
    fn double_reverse_is_plain_predicate() {
        let inner = PropertyPathExpression::NamedNode(iri("http://example.org/p"));
        let p = PropertyPathExpression::Reverse(Box::new(PropertyPathExpression::Reverse(
            Box::new(inner),
        )));
        let tp = lower_triple(&var("s"), &p, &var("o"));
        // `^(^p)` = `p` — no swap.
        assert!(matches!(tp.subject, TermPattern::Variable(ref v) if v.as_str() == "s"));
        assert!(matches!(tp.object, TermPattern::Variable(ref v) if v.as_str() == "o"));
    }

    #[test]
    fn one_or_more_classifies_as_plus_not_triple() {
        let p = PropertyPathExpression::OneOrMore(Box::new(PropertyPathExpression::NamedNode(
            iri("http://example.org/p"),
        )));
        match classify_path(&var("s"), &p, &var("o")) {
            PathPlan::OneOrMore { predicate, swapped } => {
                assert_eq!(predicate.as_str(), "http://example.org/p");
                assert!(!swapped, "plain `p+` is not swapped");
            }
            PathPlan::Triple(_) => panic!("`p+` must not lower to a triple"),
        }
        assert!(is_executable(&p), "`p+` is executable from E2");
    }

    #[test]
    fn inverse_of_plus_folds_to_swapped() {
        // `^(p+)` = Reverse(OneOrMore(NamedNode)).
        let rp =
            PropertyPathExpression::Reverse(Box::new(PropertyPathExpression::OneOrMore(Box::new(
                PropertyPathExpression::NamedNode(iri("http://example.org/p")),
            ))));
        match classify_path(&var("s"), &rp, &var("o")) {
            PathPlan::OneOrMore { swapped, .. } => {
                assert!(swapped, "`^(p+)` walks the inverse edge")
            }
            PathPlan::Triple(_) => panic!("`^(p+)` is a `+` relation, not a triple"),
        }
        // `(^p)+` = OneOrMore(Reverse(NamedNode)) — same semantics.
        let pr =
            PropertyPathExpression::OneOrMore(Box::new(PropertyPathExpression::Reverse(Box::new(
                PropertyPathExpression::NamedNode(iri("http://example.org/p")),
            ))));
        match classify_path(&var("s"), &pr, &var("o")) {
            PathPlan::OneOrMore { swapped, .. } => {
                assert!(swapped, "`(^p)+` walks the inverse edge")
            }
            PathPlan::Triple(_) => panic!("`(^p)+` is a `+` relation, not a triple"),
        }
        assert!(is_executable(&rp));
        assert!(is_executable(&pr));
    }

    #[test]
    #[should_panic(expected = "lands in Phase E group E3")]
    fn zero_or_more_preview_panics() {
        let p = PropertyPathExpression::ZeroOrMore(Box::new(PropertyPathExpression::NamedNode(
            iri("http://example.org/p"),
        )));
        let _ = classify_path(&var("s"), &p, &var("o"));
    }

    #[test]
    #[should_panic(expected = "lands in Phase E group E3")]
    fn zero_or_one_preview_panics() {
        let p = PropertyPathExpression::ZeroOrOne(Box::new(PropertyPathExpression::NamedNode(
            iri("http://example.org/p"),
        )));
        let _ = classify_path(&var("s"), &p, &var("o"));
    }

    #[test]
    #[should_panic(expected = "gated stretch goal")]
    fn alternation_preview_panics() {
        let p = PropertyPathExpression::Alternative(
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/a",
            ))),
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/b",
            ))),
        );
        let _ = classify_path(&var("s"), &p, &var("o"));
    }

    #[test]
    #[should_panic(expected = "out of scope for v0.4")]
    fn negated_property_set_panics() {
        let p = PropertyPathExpression::NegatedPropertySet(vec![iri("http://example.org/p")]);
        let _ = classify_path(&var("s"), &p, &var("o"));
    }

    #[test]
    #[should_panic(expected = "multi-pattern BGP")]
    fn sequence_path_rejected() {
        let p = PropertyPathExpression::Sequence(
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/a",
            ))),
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/b",
            ))),
        );
        let _ = classify_path(&var("s"), &p, &var("o"));
    }

    #[test]
    #[should_panic(expected = "nested recursive property path")]
    fn nested_recursive_plus_panics() {
        // `(p*)+` = OneOrMore(ZeroOrMore(NamedNode)).
        let p = PropertyPathExpression::OneOrMore(Box::new(PropertyPathExpression::ZeroOrMore(
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/p",
            ))),
        )));
        let _ = classify_path(&var("s"), &p, &var("o"));
        assert!(!is_executable(&p));
    }

    #[test]
    fn relation_sql_shapes_forward_and_inverse() {
        // Forward `p+`, unscoped: walk subject→object, no graph pred,
        // 2-column relation, UNION (cycle-safe), depth cap present.
        let r = build_one_or_more_relation_sql(
            "$1",
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            42,
            None,
        );
        assert!(r
            .from_fragment
            .contains("WITH RECURSIVE walk(src, dst, depth)"));
        assert!(r.from_fragment.contains("SELECT subject_id, object_id"));
        // Cycle-safe termination via Postgres `CYCLE` (UNION ALL is
        // required by the CYCLE clause; the final WHERE NOT is_cycle
        // drops the cycle-closing marker).
        assert!(
            r.from_fragment.contains(" UNION ALL "),
            "UNION ALL required by the CYCLE clause"
        );
        assert!(
            r.from_fragment
                .contains("CYCLE src, dst SET is_cycle USING path"),
            "CYCLE clause terminates cyclic walks"
        );
        assert!(
            r.from_fragment.contains("WHERE NOT is_cycle"),
            "drop the cycle-closing marker row"
        );
        assert!(r.from_fragment.contains("w.depth < 64"), "depth guard cap");
        assert_eq!(r.columns, "(subject_id, object_id)");
        assert_eq!(r.probe_params, vec![42]);
        assert!(r.probe_sql.contains("w.depth = 64"));

        // Inverse `^p+`: base arm reads object_id, subject_id.
        let ri = build_one_or_more_relation_sql(
            "$1",
            None,
            &PathGraphScope::AllGraphs,
            true,
            64,
            7,
            None,
        );
        assert!(ri.from_fragment.contains("SELECT object_id, subject_id"));
        assert!(ri.from_fragment.contains("q.subject_id"));

        // GRAPH ?g (Variable) carries gid + same-graph recursive hop.
        let rv = build_one_or_more_relation_sql(
            "$2",
            None,
            &PathGraphScope::Variable,
            false,
            32,
            9,
            None,
        );
        assert_eq!(rv.columns, "(subject_id, object_id, graph_id)");
        assert!(rv.from_fragment.contains("q.graph_id = w.gid"));
        assert!(rv.from_fragment.contains("graph_id <> 0"));
    }
}
