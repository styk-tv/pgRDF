//! SPARQL property-path translation — Phase E groups E1 + E2 + E3
//! + E4 (LLD v0.4 §7).
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
//! ## What E3 ships (LLD v0.4 §7.2 / §7.3 + W3C SPARQL 1.1 §9.3)
//!
//! * **Zero-or-one** — `ZeroOrOne(NamedNode(p))` = `p?`, plus the
//!   inverse compositions `^(p?)` / `(^p)?`. NO recursion: `p?` is
//!   the direct `p` (or inverse) edges UNION the **zero-length
//!   path** node-set (W3C §9.3 *ZeroLengthPath*).
//! * **Zero-or-more** — `ZeroOrMore(NamedNode(p))` = `p*`, plus
//!   `^(p*)` / `(^p)*`. `p*` is the E2 cycle-safe recursive `+` walk
//!   (its transitive part) UNION the same zero-length node-set. The
//!   recursive part reuses E2's `CYCLE`-clause termination + the
//!   `pgrdf.path_max_depth` depth guard + the truncation probe (the
//!   zero-length part cannot truncate — it is a single non-recursive
//!   scan).
//!
//! ### W3C SPARQL 1.1 §9.3 zero-length-path semantics
//!
//! The LLD §7.2 sketch (`*` = "union with `SELECT ?s ?s`") is a
//! simplification — exactly as E2 corrected the §7.2 bare-`UNION`
//! cycle sketch to Postgres's `CYCLE` clause, E3 refines the
//! reflexive set to the precise W3C node-set. The zero-length pair
//! set (`{(n,n)}`) an endpoint can match depends on whether that
//! endpoint is **bound** (an IRI in the query) or **unbound** (a
//! variable):
//!
//! * **Bound endpoint** (`<x> p* ?o` / `?s p* <y>` /
//!   `<x> p* <y>`): the bound term's self-pair `(x,x)` holds
//!   **unconditionally** — even if `x` has no `p` edge and even if
//!   `x` is not otherwise a term in the active graph (W3C: the
//!   zero-length path of a fixed term to itself always exists).
//!   Implemented as a `UNION ALL SELECT $x, $x` injected into the
//!   relation; the executor's existing subject/object id binder then
//!   keeps exactly the right rows.
//! * **Unbound endpoint** (`?s p* ?o`, neither bound): the
//!   zero-length pairs are `(n,n)` for every node `n` that is a term
//!   of the **active graph in subject or object position**. pgRDF's
//!   chosen node-set (documented here, citing W3C §9.3): the DISTINCT
//!   union of `subject_id` and `object_id` over the active scope.
//!   W3C also nominally includes nodes appearing only as a predicate,
//!   but in the pgRDF data model a predicate-only IRI is never a
//!   useful path endpoint, and a bare-predicate node-set would make
//!   `?s p* ?o` quadratic in the predicate count for no observable
//!   solutions — so the node-set is scoped to subject∪object of the
//!   active graph(s). When a `GRAPH <iri>` / `GRAPH ?g` scope is
//!   active the node-set is **scoped to that graph's nodes** (a node
//!   that lives only in another graph is NOT in the identity set of
//!   the scoped query).
//!
//! `?`'s zero-length set follows the SAME endpoint-binding rules
//! (W3C `ZeroLengthPath` is shared between `*` and `?`); `?` differs
//! only in that its non-identity part is the single direct `p` edge
//! (no `+` recursion).
//!
//! Because the E1 output is an ordinary [`TriplePattern`] and the
//! E2 / E3 output is a relation exposing `subject_id` / `object_id`
//! columns (the same columns the BGP var-binder reads off a
//! `_pgrdf_quads` alias), all compose for free with everything the
//! BGP walker already supports: named-graph scoping (`GRAPH <iri>` /
//! `GRAPH ?g`), multi-pattern BGP joins, OPTIONAL / UNION / MINUS
//! wrappers, and `pgrdf.construct` (which routes its WHERE through
//! the same `parse_select` walker).
//!
//! ## What E4 ships (LLD v0.4 §7.1 gated stretch / §7.2 / §7.3)
//!
//! * **Alternation** — `Alternative(a, b)` = `a|b`, and the n-ary
//!   nests `a|b|c` (= `Alternative(a, Alternative(b, c))`). Per LLD
//!   §7.2 the base case becomes "a union of per-predicate scans" —
//!   in pgRDF that is exactly `predicate_id IN ($P1, $P2, …)`. The
//!   §7.1-gated stretch ships in full because the refactor IS cheap:
//!   every recursive/optional builder already centralised the single
//!   `predicate_id = $P` clause, so generalising it to a predicate
//!   *set* (`IN (…)`) is a uniform one-line change at each site, not
//!   a translator balloon. Consequently the recursion-composed forms
//!   ship too:
//!   - **`(a|b)+` / `(a|b)*` / `(a|b)?`** — the alternation becomes
//!     the recursive step's predicate SET: the CTE base arm and the
//!     recursive arm both range over `{a,b}` (the depth guard, the
//!     `CYCLE` clause, the truncation probe, and the zero-length
//!     node-set are all predicate-set-agnostic, so they are reused
//!     verbatim).
//!   - **`^(a|b)` / `(^a|^b)`** — `^` composition is uniform (it is
//!     the same `swapped` flag the closure builders already carry),
//!     so the inverse of an alternation = the alternation of the
//!     inverse over the swapped edge.
//!
//!   GATED (still preview-panics, per §7.1's explicit allowance): an
//!   alternation whose arm is NOT a plain (optionally inverted)
//!   predicate — e.g. `(a/b | c)` (sequence arm), `(a+ | b)`
//!   (recursive arm), `(a | (b|c)*)` (nested-recursive arm). These
//!   are exotic; folding them would mean composing a recursive CTE
//!   inside an alternation arm, which IS the translator balloon §7.1
//!   permits gating. They panic with the stable nested-recursive
//!   prefix.
//! * **Materialised-closure no-CTE fallback** — handled in
//!   `executor.rs` (it needs the live dictionary + a probe query):
//!   before emitting the recursive CTE for `+`/`*` over a predicate
//!   that is one of the well-known transitive predicates
//!   (`rdfs:subClassOf` / `rdfs:subPropertyOf` / `owl:sameAs`), if
//!   `_pgrdf_quads` already carries `is_inferred = TRUE` rows for
//!   that predicate in the active scope, the translator falls back
//!   to a direct (non-recursive) BGP-style match — no `WITH
//!   RECURSIVE`, no `CTE Scan` in the plan (§7.2 v0.4 heuristic /
//!   §7.3 acceptance). `?`/`^` are unaffected (no recursion).
//!
//! ## What E4 does NOT ship (deferred — stable preview panics)
//!
//! A recursive path whose inner box is itself recursive / sequence,
//! or an alternation whose arm is non-plain (`(p*)+`, `(p1/p2)?`,
//! `(a/b|c)`, `(a+|b)`), is exotic and would require composing a
//! recursive CTE inside another recursive/alternation context —
//! deferred (LLD §7.1 explicitly permits gating the costly stretch).
//! Negated property sets (`!(...)`) are out of v0.4 scope entirely.
//! Each panics with a STABLE prefix so downstream tooling can
//! preview the rollout schedule without depending on the
//! (slice-number-bearing) tail — the exact same convention Phase C's
//! per-form UPDATE panics use.

use spargebra::algebra::PropertyPathExpression;
use spargebra::term::{NamedNode, NamedNodePattern, TermPattern, TriplePattern};

/// Stable panic prefix for a recursive/alternation path whose inner
/// box / arm is NOT a plain (optionally inverted) predicate — e.g.
/// `(p*)+`, `(p1/p2)?`, `(a/b|c)`, `(a+|b)`. The plain
/// `p+`/`^p+`/`(^p)+` (E2), `p*`/`p?` + inverse (E3), and the
/// alternation forms `a|b`, `(a|b)+`, `(a|b)*`, `(a|b)?`, `^(a|b)`
/// over plain (optionally inverted) predicates (E4) are executable;
/// the exotic nested-recursive / non-plain-arm case is the
/// §7.1-permitted gated remainder.
pub(crate) const PANIC_ONE_OR_MORE_NESTED: &str =
    "pgrdf: nested recursive property path (e.g. `(p*)+`, `(a/b|c)`) is a gated stretch goal (Phase E group E4)";

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
/// The recursive/optional/alternation plans carry a `predicates`
/// **set** (not a single predicate) so the `|` alternation (E4)
/// composes uniformly: a plain `p+`/`p*`/`p?`/`a|b` is just a
/// one-element / multi-element set, and the SQL builders emit
/// `predicate_id IN (…)` (a 1-element `IN` is exactly the old
/// `= $P`). `a|b` over plain (optionally inverted) predicates, and
/// the recursion-composed `(a|b)+` / `(a|b)*` / `(a|b)?`, all reduce
/// to "walk/match a predicate SET", which is the LLD §7.2
/// "union of per-predicate scans" done in one scan.
///
/// * `Triple` — the E1 non-recursive set (bare predicate, `^p`,
///   nested `^(^…)`). Lowered to an ordinary [`TriplePattern`];
///   `executor.rs` pushes it like a BGP triple.
/// * `OneOrMore { predicates, swapped }` — the E2 `+` set
///   (`p+`, `^p+`, `(^p)+`) plus the E4 `(a|b)+` / `^((a|b)+)` set.
///   `predicates` are the resolved IRIs of the predicate set walked
///   (one element for plain `p+`, ≥2 for `(a|b)+`); `swapped` is true
///   when the closure is over the *inverse* edge (subject/object
///   roles flipped — `^p+` ≡ `(^p)+`, the inverse of a transitive
///   closure equals the transitive closure of the inverse).
///   `executor.rs` builds the recursive CTE relation from this.
/// * `ZeroOrMore { predicates, swapped }` — the E3 `*` set
///   (`p*`, `^(p*)`, `(^p)*`) plus the E4 `(a|b)*` set. Same
///   recursive `+` walk PLUS the W3C §9.3 zero-length node-set.
/// * `ZeroOrOne { predicates, swapped }` — the E3 `?` set
///   (`p?`, `^(p?)`, `(^p)?`) plus the E4 `(a|b)?` set. NO recursion
///   — the direct (optionally inverted) edge over the predicate set
///   `UNION` the same W3C §9.3 zero-length node-set.
/// * `Alternation { predicates, swapped }` — the E4 top-level
///   alternation `a|b` (and the n-ary nest `a|b|c`, and `^(a|b)` /
///   `(^a|^b)`). NO recursion, NO zero-length set: it is exactly the
///   non-reflexive single step over the predicate set —
///   `?s (a|b) ?o` ≡ the union of `?s a ?o` and `?s b ?o`. Lowered
///   to a direct-edge relation (`predicate_id IN (…)`), the same
///   shape `?`'s direct arm uses, minus the identity union.
pub(crate) enum PathPlan {
    Triple(Box<TriplePattern>),
    OneOrMore {
        predicates: Vec<NamedNode>,
        swapped: bool,
    },
    ZeroOrMore {
        predicates: Vec<NamedNode>,
        swapped: bool,
    },
    ZeroOrOne {
        predicates: Vec<NamedNode>,
        swapped: bool,
    },
    Alternation {
        predicates: Vec<NamedNode>,
        swapped: bool,
    },
}

/// Fold a (possibly inverted) plain predicate, accumulating any
/// `Reverse` parity. Returns `None` if the expression is NOT a plain
/// (optionally inverted) `NamedNode` — the caller decides whether
/// that is a gate-panic or a different branch.
fn fold_plain_predicate(
    expr: &PropertyPathExpression,
    start_swapped: bool,
) -> Option<(NamedNode, bool)> {
    let mut swapped = start_swapped;
    let mut ic = expr;
    loop {
        match ic {
            PropertyPathExpression::Reverse(b) => {
                swapped = !swapped;
                ic = b;
            }
            PropertyPathExpression::NamedNode(p) => return Some((p.clone(), swapped)),
            _ => return None,
        }
    }
}

/// Flatten an `Alternative(a, b)` tree (the n-ary nest `a|b|c` =
/// `Alternative(a, Alternative(b, c))`) into a flat predicate SET,
/// each arm folded through any `Reverse` parity. ALL arms must share
/// the SAME `swapped` direction: `(a|^b)` would need a per-arm
/// direction (a 2-direction relation), which is the §7.1-permitted
/// gated remainder — return `None` so the caller emits the stable
/// gate panic. The common forms `(a|b)`, `^(a|b)` (= `Reverse` above,
/// so `start_swapped = true` uniformly), and `(^a|^b)` (both arms
/// inverted, uniform) all fold cleanly. Returns `None` if any arm is
/// NOT a plain (optionally inverted) predicate, or the arms disagree
/// on direction.
fn flatten_alternation(
    expr: &PropertyPathExpression,
    start_swapped: bool,
) -> Option<(Vec<NamedNode>, bool)> {
    let mut preds: Vec<NamedNode> = Vec::new();
    let mut dir: Option<bool> = None;
    // Recursive flatten over the (possibly nested) Alternative tree.
    fn walk(
        e: &PropertyPathExpression,
        start_swapped: bool,
        preds: &mut Vec<NamedNode>,
        dir: &mut Option<bool>,
    ) -> bool {
        match e {
            PropertyPathExpression::Alternative(l, r) => {
                walk(l, start_swapped, preds, dir) && walk(r, start_swapped, preds, dir)
            }
            // A plain (optionally inverted) predicate arm.
            other => match fold_plain_predicate(other, start_swapped) {
                Some((p, sw)) => {
                    match dir {
                        None => *dir = Some(sw),
                        Some(d) if *d == sw => {}
                        // Mixed-direction arms (`a|^b`) — gated.
                        Some(_) => return false,
                    }
                    preds.push(p);
                    true
                }
                // A sequence / recursive / nested arm (`a/b|c`,
                // `a+|b`) — the §7.1-permitted gated remainder.
                None => false,
            },
        }
    }
    if walk(expr, start_swapped, &mut preds, &mut dir) && !preds.is_empty() {
        Some((preds, dir.unwrap_or(start_swapped)))
    } else {
        None
    }
}

/// Fold the inner box of a recursive operator (`+`/`*`/`?`) down to
/// its predicate SET. `outer_swapped` is the parity accumulated from
/// any `Reverse` wrappers ABOVE the operator (`^(p+)`); inner
/// `Reverse`s (`(^p)+`) flip it further. The inverse of a
/// recursive/optional closure equals the same closure over the
/// inverse edge, so both fold to one `swapped` flag — identical for
/// `+`, `*`, and `?`. E4: the inner box MAY be an `Alternative` of
/// plain (optionally inverted) predicates (`(a|b)+` / `(a|b)*` /
/// `(a|b)?`) — flattened to the predicate set, the recursive arm
/// then ranges over `predicate_id IN (…)`. A nested-recursive /
/// sequence inner, or a mixed-direction / non-plain alternation arm
/// (`(p*)+`, `(p1/p2)?`, `(a/b|c)`), is the §7.1-permitted gated
/// remainder; it panics with the stable preview prefix.
fn fold_inner_predicates(
    inner: &PropertyPathExpression,
    outer_swapped: bool,
) -> (Vec<NamedNode>, bool) {
    // Single plain (optionally inverted) predicate — the E2/E3 form.
    if let Some((p, sw)) = fold_plain_predicate(inner, outer_swapped) {
        return (vec![p], sw);
    }
    // E4 — `(a|b)+` / `(a|b)*` / `(a|b)?`: the inner box is an
    // alternation of plain (optionally inverted) predicates.
    if matches!(inner, PropertyPathExpression::Alternative(_, _)) {
        if let Some((preds, sw)) = flatten_alternation(inner, outer_swapped) {
            return (preds, sw);
        }
    }
    // `(p*)+`, `(p1/p2)?`, `(a/b|c)+`, `(a+|b)*` — nested recursive
    // / sequence / non-plain-arm. Exotic; the gated E4 remainder.
    panic!("{PANIC_ONE_OR_MORE_NESTED}");
}

/// Classify a property-path pattern into its execution plan, or panic
/// with the stable rollout-preview prefix for a not-yet-shipped
/// operator. `subject` / `object` are the outer term patterns; for
/// the `Triple` plan they are baked into the lowered triple (with the
/// subject/object swap applied for the inverse case), for the
/// `OneOrMore` / `ZeroOrMore` / `ZeroOrOne` plans `executor.rs` binds
/// them against the relation's `src` / `dst` columns.
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
                // E2 `p+` / E4 `(a|b)+`. The inner box folds to a
                // predicate SET (1 elem = plain `+`, ≥2 = alternation
                // step); inner `Reverse` parity folds into `swapped`.
                let (predicates, swapped) = fold_inner_predicates(inner, swapped);
                return PathPlan::OneOrMore {
                    predicates,
                    swapped,
                };
            }
            PropertyPathExpression::ZeroOrMore(inner) => {
                // E3 `p*` / E4 `(a|b)*`. Same inner-box discipline as
                // `+`; reflexive set added by the relation builder.
                let (predicates, swapped) = fold_inner_predicates(inner, swapped);
                return PathPlan::ZeroOrMore {
                    predicates,
                    swapped,
                };
            }
            PropertyPathExpression::ZeroOrOne(inner) => {
                // E3 `p?` / E4 `(a|b)?`. Same inner-box discipline;
                // non-recursive (direct edge ∪ identity).
                let (predicates, swapped) = fold_inner_predicates(inner, swapped);
                return PathPlan::ZeroOrOne {
                    predicates,
                    swapped,
                };
            }
            PropertyPathExpression::Alternative(_, _) => {
                // E4 — top-level `a|b` (n-ary `a|b|c`, `^(a|b)`,
                // `(^a|^b)`). Flatten to the predicate set; a
                // sequence / recursive / mixed-direction arm is the
                // §7.1-permitted gated remainder (stable panic).
                match flatten_alternation(cur, swapped) {
                    Some((predicates, swapped)) => {
                        return PathPlan::Alternation {
                            predicates,
                            swapped,
                        }
                    }
                    None => panic!("{PANIC_ONE_OR_MORE_NESTED}"),
                }
            }
            PropertyPathExpression::NegatedPropertySet(_) => panic!("{PANIC_NEGATED}"),
            PropertyPathExpression::Sequence(_, _) => panic!("{PANIC_SEQUENCE}"),
        }
    }
}

/// Is this property-path expression *executable* under the
/// currently-shipped operator set (E1 lower-to-triple ∪ E2 `+` ∪
/// E3 `*` / `?`)?
///
/// `true`  → bare predicate, `^p`, nested `^(^…)`,
///           `p+`/`p*`/`p?` (and their `^…` inverse compositions)
///           over an optionally-inverted single predicate, OR the
///           E4 alternation forms `a|b` / `(a|b)+` / `(a|b)*` /
///           `(a|b)?` / `^(a|b)` over plain (optionally inverted,
///           uniform-direction) predicates.
/// `false` → negated set, sequence, a `+`/`*`/`?` with a
///           nested-recursive / non-plain-arm inner, or a
///           mixed-direction / non-plain alternation arm
///           (the §7.1-permitted gated E4 remainder).
///
/// Used by `parser.rs` so `sparql_parse` does NOT flag the now-
/// executable forms in `unsupported_algebra` (parse-time, no panic);
/// the genuinely deferred forms still get flagged. Execution of a
/// deferred form panics with the stable rollout-preview prefix.
pub(crate) fn is_executable(path: &PropertyPathExpression) -> bool {
    // True iff `inner` folds (through any `Reverse` wrappers) to a
    // single plain predicate OR an alternation of plain (uniform-
    // direction) predicates — the shared executability rule for the
    // recursive/optional operators (`+`/`*`/`?`).
    fn inner_is_plain_or_alternation(inner: &PropertyPathExpression) -> bool {
        if fold_plain_predicate(inner, false).is_some() {
            return true;
        }
        if matches!(inner, PropertyPathExpression::Alternative(_, _)) {
            return flatten_alternation(inner, false).is_some();
        }
        false
    }
    let mut cur = path;
    loop {
        match cur {
            PropertyPathExpression::Reverse(inner) => cur = inner,
            PropertyPathExpression::NamedNode(_) => return true,
            PropertyPathExpression::OneOrMore(inner)
            | PropertyPathExpression::ZeroOrMore(inner)
            | PropertyPathExpression::ZeroOrOne(inner) => {
                return inner_is_plain_or_alternation(inner)
            }
            // E4 — top-level `a|b` (and `^(a|b)` since we tunnelled
            // through `Reverse` above): executable iff every arm is
            // a plain (optionally inverted, uniform-direction)
            // predicate.
            PropertyPathExpression::Alternative(_, _) => {
                return flatten_alternation(cur, false).is_some()
            }
            _ => return false,
        }
    }
}

/// Parser-facing analysis view of an *executable* property path
/// (E1 lower-to-triple set ∪ E2 `+` ∪ E3 `*` / `?`) as a
/// [`TriplePattern`], WITHOUT running the SQL-side relation lowering
/// (a `+`/`*`/`?` has no triple form for execution — it is a derived
/// relation). For the E1 set this is exactly the lowered triple; for
/// the recursive/optional operators it is `(subject, predicate,
/// object)` with the inverse subject/object swap applied — the
/// predicate is a `NamedNode` (these operators walk a fixed
/// predicate, never a variable), so `collect_vars` /
/// `collect_pattern_vars` see only the subject / object variables,
/// which is correct: `?s p* ?o` binds `?s` and `?o` exactly like a
/// triple would. Returns `None` for a not-yet-executable form (the
/// caller flags it `unsupported_algebra` instead — parse-time, no
/// panic).
pub(crate) fn analysis_triple(
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
) -> Option<TriplePattern> {
    if !is_executable(path) {
        return None;
    }
    // The recursive/optional/alternation operators all bind
    // subject/object like a single (possibly inverted) predicate
    // triple — only the swap direction matters for var collection.
    // The predicate slot is a `NamedNode` (these operators walk a
    // fixed predicate SET, never a variable) so `collect_vars` sees
    // ONLY the subject/object variables — correct regardless of how
    // many predicates the set carries; we use the first as a
    // harmless placeholder (never emitted for a path row).
    let plan_triple = |predicates: Vec<NamedNode>, swapped: bool| {
        let (s, o) = if swapped {
            (object.clone(), subject.clone())
        } else {
            (subject.clone(), object.clone())
        };
        TriplePattern {
            subject: s,
            predicate: NamedNodePattern::NamedNode(
                predicates
                    .into_iter()
                    .next()
                    .expect("non-empty predicate set"),
            ),
            object: o,
        }
    };
    match classify_path(subject, path, object) {
        PathPlan::Triple(tp) => Some(*tp),
        PathPlan::OneOrMore {
            predicates,
            swapped,
        }
        | PathPlan::ZeroOrMore {
            predicates,
            swapped,
        }
        | PathPlan::ZeroOrOne {
            predicates,
            swapped,
        }
        | PathPlan::Alternation {
            predicates,
            swapped,
        } => Some(plan_triple(predicates, swapped)),
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
/// §7.2), also serving the E4 `(a|b)+` alternation step. `pred_match`
/// is the predicate-match SQL fragment using the OUTER `$N`
/// placeholders the caller appended to the param buffer — exactly
/// `predicate_id = $N` for a plain `p+` (one-element set) or
/// `predicate_id IN ($N1, $N2, …)` for `(a|b)+` (the LLD §7.2
/// "union of per-predicate scans" done as a single scan over a
/// predicate set). `probe_pred_match` is the SAME match but written
/// with the probe-local `$1[, $2, …]` placeholders; `probe_params`
/// are the dict ids the probe binds in that order. `graph_placeholder`
/// is the optional `$M` for the `Literal` scope's resolved graph id.
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
///   WHERE <pred_match> [graph predicate]
/// UNION
/// SELECT w.src, q.object_id [, w.gid], w.depth + 1
///   FROM walk w JOIN _pgrdf_quads q ON q.subject_id = w.dst
///   WHERE q.<pred_match> AND w.depth < $MAX [AND same-graph]
/// ```
///
/// `swapped` (the `^p+` / `(^p)+` / `^((a|b)+)` case) flips the edge
/// direction: the base arm reads `object_id, subject_id` and the
/// recursive arm joins `q.object_id = w.dst` projecting
/// `q.subject_id`. `UNION` (not `UNION ALL`) makes cycles terminate
/// (a revisited (src,dst) pair is deduped); `w.depth < $MAX` is the
/// hard depth cap. The predicate-set generalisation is transparent to
/// the cycle clause, the depth guard, and the truncation probe — they
/// are all predicate-match-agnostic.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_one_or_more_relation_sql(
    pred_ids_sql: &str,
    probe_pred_ids_sql: &str,
    probe_params: Vec<i64>,
    graph_placeholder: Option<&str>,
    scope: &PathGraphScope,
    swapped: bool,
    max_depth: i32,
    probe_graph_id: Option<i64>,
) -> PathRelation {
    // Predicate match. `IN (…)` over the resolved dict-id placeholder
    // list — a single-element `IN ($1)` is identical (semantically
    // and to the planner) to `= $1`, so plain `p+` is unchanged and
    // `(a|b)+` just widens the set (LLD §7.2 "union of per-predicate
    // scans" as one scan over the predicate set).
    let base_pred = format!("predicate_id IN ({pred_ids_sql})");
    let rec_pred = format!("q.predicate_id IN ({pred_ids_sql})");
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
            WHERE {base_pred}{base_graph_pred} \
         UNION ALL \
           SELECT w.src, q.{rec_proj_col}{rec_gid}, w.depth + 1 \
             FROM walk w \
             JOIN pgrdf._pgrdf_quads q ON q.{rec_join_col} = w.dst \
            WHERE {rec_pred} \
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
    // Probe predicate match — same `IN (…)` set as the relation, but
    // written with the probe-local `$1[, $2, …]` placeholders the
    // caller bound in `probe_params` order. Plain `p+` is `IN ($1)`
    // (identical to the old `= $1`); `(a|b)+` widens to `IN ($1,$2)`.
    let probe_base_pred = format!("predicate_id IN ({probe_pred_ids_sql})");
    let probe_rec_pred = format!("q.predicate_id IN ({probe_pred_ids_sql})");
    let probe_cont_pred = format!("c.predicate_id IN ({probe_pred_ids_sql})");
    // The probe mirrors the relation's `UNION ALL` + `CYCLE` walk
    // (same cycle-safety), then asks: is there a NON-cycle row at
    // exactly the depth cap whose `dst` still has an outgoing edge
    // in the predicate set? A cycle terminates before the cap (CYCLE
    // clause), so it never produces a `depth == MAX` row → a
    // fully-resolved cyclic query correctly reports NO truncation.
    // Only a genuinely long ACYCLIC path that the cap actually
    // severed fires the probe.
    let probe_sql = format!(
        "SELECT CASE WHEN EXISTS (\
           WITH RECURSIVE {probe_walk_cols} AS (\
             SELECT {base_src}, {base_dst}{p_base_gid}, 1 \
               FROM pgrdf._pgrdf_quads \
              WHERE {probe_base_pred}{probe_base_graph} \
           UNION ALL \
             SELECT w.src, q.{rec_proj_col}{p_rec_gid}, w.depth + 1 \
               FROM pwalk w \
               JOIN pgrdf._pgrdf_quads q ON q.{rec_join_col} = w.dst \
              WHERE {probe_rec_pred} \
                AND w.depth < {max_depth}{probe_rec_graph}\
           ) CYCLE src, dst SET is_cycle USING path \
           SELECT 1 FROM pwalk w \
            WHERE NOT w.is_cycle \
              AND w.depth = {max_depth} \
              AND EXISTS (\
                SELECT 1 FROM pgrdf._pgrdf_quads c \
                 WHERE c.{rec_join_col} = w.dst \
                   AND {probe_cont_pred}{probe_cont_graph}\
              )\
         ) THEN 1::bigint ELSE 0::bigint END"
    );
    // The probe binds ONLY the predicate dict id(s) as `$1[, $2…]`.
    // The Literal scope's resolved graph id is a translate-time
    // integer constant (not user input) and is inlined directly into
    // `probe_base_graph` / `probe_rec_graph` / `probe_cont_graph`
    // above — keeping it out of the param vec keeps the probe's
    // placeholder numbering aligned with `probe_params`.
    // `probe_graph_id` is accepted for call-site symmetry with the
    // main relation builder; it is intentionally not threaded into
    // the param vec (see the inlining above).
    let _ = probe_graph_id;

    PathRelation {
        from_fragment,
        columns,
        probe_sql,
        probe_params,
    }
}

/// The W3C SPARQL 1.1 §9.3 *ZeroLengthPath* node-set, as a SQL
/// `SELECT`ing `(src, dst[, gid])` identity pairs `(n, n[, g])`.
///
/// Two parts, both `UNION`ed into the final relation:
///
/// 1. **Unbound-endpoint node-set** — `(n, n)` for every `n` that is
///    a term of the active scope in subject OR object position. This
///    is what `?s p* ?o` needs (the reflexive pairs over graph
///    nodes). Scoped exactly like the transitive walk: unscoped =
///    all partitions; `GRAPH <iri>` = one resolved graph; `GRAPH ?g`
///    = per named-graph (carries `gid`, excludes the default graph
///    per W3C §13.3 — a node only in another graph is NOT in the
///    identity set of the scoped query).
/// 2. **Bound-endpoint unconditional self-pair** — for `<x> p* …`
///    or `… p* <y>` the bound term's `(x,x)` holds *even if `x` is
///    not a node of the graph at all* (W3C: a fixed term always has
///    a zero-length path to itself). Injected as a constant
///    `SELECT $x, $x` (only for the `AllGraphs` / `Literal` scopes —
///    under `GRAPH ?g` a zero-length path traverses no edge so there
///    is no named graph to bind `?g`; the scoped node-set in part 1
///    already yields the term for every named graph it appears in,
///    which is the spec-correct `?g` binding set).
///
/// `bound_self_pairs` are the resolved dict ids of any *bound* (IRI)
/// endpoints — caller passes the subject id and/or object id when
/// that endpoint is a `NamedNode`. A dict id of `-1` (IRI never
/// interned) still injects `(-1,-1)`; that pair simply never matches
/// a real `subject_id`/`object_id` so it is harmless, and keeps the
/// `<x> p? <x>` "x not in graph" case correct (the binder filters to
/// `src=$x AND dst=$x`, both `-1`, which the injected row satisfies →
/// W3C `ASK { <x> p? <x> }` = true for any `<x>`).
fn zero_length_node_set_sql(scope: &PathGraphScope, bound_self_pairs: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    match scope {
        PathGraphScope::AllGraphs => {
            parts.push(
                "SELECT subject_id AS src, subject_id AS dst FROM pgrdf._pgrdf_quads \
                 UNION SELECT object_id, object_id FROM pgrdf._pgrdf_quads"
                    .to_string(),
            );
            for ph in bound_self_pairs {
                parts.push(format!("SELECT {ph}::bigint AS src, {ph}::bigint AS dst"));
            }
        }
        PathGraphScope::Literal(gid) => {
            // The resolved graph id is a translate-time constant
            // (same inlining discipline the truncation probe uses),
            // so the node-set scopes with a literal predicate.
            parts.push(format!(
                "SELECT subject_id AS src, subject_id AS dst FROM pgrdf._pgrdf_quads \
                  WHERE graph_id = {gid} \
                 UNION SELECT object_id, object_id FROM pgrdf._pgrdf_quads \
                  WHERE graph_id = {gid}"
            ));
            for ph in bound_self_pairs {
                parts.push(format!("SELECT {ph}::bigint AS src, {ph}::bigint AS dst"));
            }
        }
        PathGraphScope::Variable => {
            // Per named-graph identity (carries gid). Excludes the
            // default graph (W3C §13.3 — `?g` ranges over NAMED
            // graphs only). A bound endpoint flows through the SAME
            // scoped node-set: its self-pair binds `?g` to every
            // named graph the term is a node of (and to none if it
            // is in no named graph — spec-correct, `?g` must bind a
            // named graph). So no constant self-pair injection here.
            parts.push(
                "SELECT subject_id AS src, subject_id AS dst, graph_id AS gid \
                   FROM pgrdf._pgrdf_quads WHERE graph_id <> 0 \
                 UNION SELECT object_id, object_id, graph_id \
                   FROM pgrdf._pgrdf_quads WHERE graph_id <> 0"
                    .to_string(),
            );
        }
    }
    parts.join(" UNION ")
}

/// Build the relation for a `*` (zero-or-more) path — LLD v0.4 §7.2,
/// W3C SPARQL 1.1 §9.3. It is the E2 cycle-safe recursive `+` walk
/// (the transitive part) `UNION` the W3C zero-length node-set (the
/// reflexive part). Reuses [`build_one_or_more_relation_sql`] for the
/// transitive arm — same `CYCLE` termination, same depth guard, same
/// truncation probe (the reflexive arm is a single non-recursive scan
/// and cannot truncate, so the probe is unchanged from `+`).
///
/// `bound_self_pairs` carries the resolved dict id placeholders for
/// any *bound* (IRI) endpoint — see [`zero_length_node_set_sql`].
/// `pred_ids_sql` / `probe_pred_ids_sql` / `probe_params` are the
/// predicate-set fragments (E4 `(a|b)*` widens the set; plain `p*`
/// is a 1-element set) — see [`build_one_or_more_relation_sql`].
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_zero_or_more_relation_sql(
    pred_ids_sql: &str,
    probe_pred_ids_sql: &str,
    probe_params: Vec<i64>,
    graph_placeholder: Option<&str>,
    scope: &PathGraphScope,
    swapped: bool,
    max_depth: i32,
    probe_graph_id: Option<i64>,
    bound_self_pairs: &[String],
) -> PathRelation {
    // The transitive part is exactly the `+` relation. Reuse it so
    // there is ONE recursive-CTE + cycle-safety + probe implementation.
    let plus = build_one_or_more_relation_sql(
        pred_ids_sql,
        probe_pred_ids_sql,
        probe_params,
        graph_placeholder,
        scope,
        swapped,
        max_depth,
        probe_graph_id,
    );
    // `+`'s `from_fragment` is a fully parenthesised derived table.
    // Strip its outer parens and `UNION` the zero-length node-set so
    // the whole `*` relation is a single parenthesised subquery the
    // executor aliases exactly like the `+` one. (`+`'s final SELECT
    // is `SELECT DISTINCT src, dst[, gid]` so the column shapes line
    // up for the `UNION`; `UNION` also dedups the reflexive pairs
    // that the transitive walk already produced for cyclic data.)
    let plus_inner = plus
        .from_fragment
        .strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .expect("+ relation from_fragment is parenthesised");
    let zero = zero_length_node_set_sql(scope, bound_self_pairs);
    let from_fragment = format!("({plus_inner} UNION {zero})");
    PathRelation {
        from_fragment,
        columns: plus.columns,
        // Truncation accounting is identical to `+` — only the
        // recursive transitive arm can hit the depth cap.
        probe_sql: plus.probe_sql,
        probe_params: plus.probe_params,
    }
}

/// Build the relation for a `?` (zero-or-one) path — LLD v0.4 §7.2,
/// W3C SPARQL 1.1 §9.3. NON-recursive: the single direct `p`
/// (optionally inverted) edge `UNION` the SAME W3C zero-length
/// node-set `*` uses (W3C `ZeroLengthPath` is shared). No depth
/// guard / truncation probe (there is no recursion to bound), so the
/// returned [`PathRelation`] carries an empty probe.
///
/// `swapped` (the `^(p?)` / `(^p)?` case) flips the direct edge's
/// endpoints — symmetric with `+`/`*`.
pub(crate) fn build_zero_or_one_relation_sql(
    pred_ids_sql: &str,
    graph_placeholder: Option<&str>,
    scope: &PathGraphScope,
    swapped: bool,
    bound_self_pairs: &[String],
) -> PathRelation {
    // Direct-edge arm endpoints (same direction logic as `+`/`*`).
    let (dir_src, dir_dst) = if swapped {
        ("object_id", "subject_id")
    } else {
        ("subject_id", "object_id")
    };
    let (direct_graph_pred, carries_gid, columns): (String, bool, &'static str) = match scope {
        PathGraphScope::AllGraphs => (String::new(), false, "(subject_id, object_id)"),
        PathGraphScope::Literal(_) => {
            let g = graph_placeholder.expect("Literal scope needs a graph placeholder");
            (
                format!(" AND graph_id = {g}"),
                false,
                "(subject_id, object_id)",
            )
        }
        PathGraphScope::Variable => (
            " AND graph_id <> 0".to_string(),
            true,
            "(subject_id, object_id, graph_id)",
        ),
    };
    let direct_gid = if carries_gid { ", graph_id AS gid" } else { "" };
    let zero = zero_length_node_set_sql(scope, bound_self_pairs);
    // Single self-contained parenthesised subquery (no CTE — `?` has
    // no recursion). The direct arm names its columns so the `UNION`
    // with the node-set lines up; the outer `SELECT DISTINCT` dedups
    // the case where the direct edge is also a self-pair (impossible
    // for distinct subject/object but harmless) and matches the `+`
    // relation's distinct projection contract. `predicate_id IN (…)`
    // generalises plain `p?` (1-elem, identical to `= $1`) to the E4
    // `(a|b)?` predicate set.
    let direct = format!(
        "SELECT {dir_src} AS src, {dir_dst} AS dst{direct_gid} \
           FROM pgrdf._pgrdf_quads \
          WHERE predicate_id IN ({pred_ids_sql}){direct_graph_pred}"
    );
    let from_fragment = format!("({direct} UNION {zero})");
    PathRelation {
        from_fragment,
        columns,
        // `?` is non-recursive — nothing can truncate. An empty
        // probe means `collect_truncation_probes` skips it.
        probe_sql: String::new(),
        probe_params: Vec::new(),
    }
}

/// Build the relation for a TOP-LEVEL alternation `a|b` (n-ary
/// `a|b|c`, plus `^(a|b)` / `(^a|^b)` via `swapped`) — LLD v0.4
/// §7.1 (the gated stretch, shipped in E4) / §7.2. This is the
/// **non-reflexive single step** over the predicate SET:
/// `?s (a|b) ?o` ≡ the union of `?s a ?o` and `?s b ?o`. NO
/// recursion (it is not a closure operator), NO zero-length set
/// (alternation is not reflexive — only `*`/`?` add the W3C §9.3
/// identity pairs). It is exactly `?`'s direct arm WITHOUT the
/// identity `UNION` — one scan, `predicate_id IN (…)`, the LLD §7.2
/// "union of per-predicate scans". `swapped` flips the edge for
/// `^(a|b)` (uniform — every arm shares the direction, enforced by
/// [`flatten_alternation`]).
pub(crate) fn build_alternation_relation_sql(
    pred_ids_sql: &str,
    graph_placeholder: Option<&str>,
    scope: &PathGraphScope,
    swapped: bool,
) -> PathRelation {
    let (dir_src, dir_dst) = if swapped {
        ("object_id", "subject_id")
    } else {
        ("subject_id", "object_id")
    };
    let (graph_pred, carries_gid, columns): (String, bool, &'static str) = match scope {
        PathGraphScope::AllGraphs => (String::new(), false, "(subject_id, object_id)"),
        PathGraphScope::Literal(_) => {
            let g = graph_placeholder.expect("Literal scope needs a graph placeholder");
            (
                format!(" AND graph_id = {g}"),
                false,
                "(subject_id, object_id)",
            )
        }
        PathGraphScope::Variable => (
            " AND graph_id <> 0".to_string(),
            true,
            "(subject_id, object_id, graph_id)",
        ),
    };
    let gid = if carries_gid { ", graph_id AS gid" } else { "" };
    let from_fragment = format!(
        "(SELECT DISTINCT {dir_src} AS src, {dir_dst} AS dst{gid} \
            FROM pgrdf._pgrdf_quads \
           WHERE predicate_id IN ({pred_ids_sql}){graph_pred})"
    );
    PathRelation {
        from_fragment,
        columns,
        // Non-recursive single step — nothing can truncate.
        probe_sql: String::new(),
        probe_params: Vec::new(),
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
    /// directly; `+`/`*`/`?` never reach the triple form).
    fn lower_triple(s: &TermPattern, p: &PropertyPathExpression, o: &TermPattern) -> TriplePattern {
        match classify_path(s, p, o) {
            PathPlan::Triple(tp) => *tp,
            PathPlan::OneOrMore { .. }
            | PathPlan::ZeroOrMore { .. }
            | PathPlan::ZeroOrOne { .. }
            | PathPlan::Alternation { .. } => panic!("expected a lower-to-triple plan"),
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
            PathPlan::OneOrMore {
                predicates,
                swapped,
            } => {
                assert_eq!(predicates.len(), 1);
                assert_eq!(predicates[0].as_str(), "http://example.org/p");
                assert!(!swapped, "plain `p+` is not swapped");
            }
            _ => panic!("`p+` must classify as OneOrMore"),
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
            _ => panic!("`^(p+)` is a `+` relation, not a triple"),
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
            _ => panic!("`(^p)+` is a `+` relation, not a triple"),
        }
        assert!(is_executable(&rp));
        assert!(is_executable(&pr));
    }

    #[test]
    fn zero_or_more_classifies_as_star_and_is_executable() {
        // `p*` = ZeroOrMore(NamedNode) — E3, executable.
        let p = PropertyPathExpression::ZeroOrMore(Box::new(PropertyPathExpression::NamedNode(
            iri("http://example.org/p"),
        )));
        match classify_path(&var("s"), &p, &var("o")) {
            PathPlan::ZeroOrMore {
                predicates,
                swapped,
            } => {
                assert_eq!(predicates.len(), 1);
                assert_eq!(predicates[0].as_str(), "http://example.org/p");
                assert!(!swapped, "plain `p*` is not swapped");
            }
            _ => panic!("`p*` must classify as ZeroOrMore"),
        }
        assert!(is_executable(&p), "`p*` is executable from E3");
        // `^(p*)` / `(^p)*` fold to the swapped `*` (inverse of a
        // reflexive-transitive closure = same closure over the
        // inverse — same parity rule as `+`).
        let inv = PropertyPathExpression::Reverse(Box::new(PropertyPathExpression::ZeroOrMore(
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/p",
            ))),
        )));
        match classify_path(&var("s"), &inv, &var("o")) {
            PathPlan::ZeroOrMore { swapped, .. } => assert!(swapped, "`^(p*)` walks the inverse"),
            _ => panic!("`^(p*)` must classify as ZeroOrMore"),
        }
        assert!(is_executable(&inv));
    }

    #[test]
    fn zero_or_one_classifies_as_opt_and_is_executable() {
        // `p?` = ZeroOrOne(NamedNode) — E3, executable, non-recursive.
        let p = PropertyPathExpression::ZeroOrOne(Box::new(PropertyPathExpression::NamedNode(
            iri("http://example.org/p"),
        )));
        match classify_path(&var("s"), &p, &var("o")) {
            PathPlan::ZeroOrOne {
                predicates,
                swapped,
            } => {
                assert_eq!(predicates.len(), 1);
                assert_eq!(predicates[0].as_str(), "http://example.org/p");
                assert!(!swapped, "plain `p?` is not swapped");
            }
            _ => panic!("`p?` must classify as ZeroOrOne"),
        }
        assert!(is_executable(&p), "`p?` is executable from E3");
        // `(^p)?` folds to the swapped `?`.
        let inv =
            PropertyPathExpression::ZeroOrOne(Box::new(PropertyPathExpression::Reverse(Box::new(
                PropertyPathExpression::NamedNode(iri("http://example.org/p")),
            ))));
        match classify_path(&var("s"), &inv, &var("o")) {
            PathPlan::ZeroOrOne { swapped, .. } => assert!(swapped, "`(^p)?` walks the inverse"),
            _ => panic!("`(^p)?` must classify as ZeroOrOne"),
        }
        assert!(is_executable(&inv));
    }

    #[test]
    fn alternation_ships_as_predicate_set() {
        // E4 — top-level `a|b` over plain predicates is executable
        // (LLD §7.1 gated stretch, shipped). Classifies as the
        // non-reflexive `Alternation` plan with BOTH predicates.
        let p = PropertyPathExpression::Alternative(
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/a",
            ))),
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/b",
            ))),
        );
        match classify_path(&var("s"), &p, &var("o")) {
            PathPlan::Alternation {
                predicates,
                swapped,
            } => {
                assert_eq!(predicates.len(), 2);
                assert_eq!(predicates[0].as_str(), "http://example.org/a");
                assert_eq!(predicates[1].as_str(), "http://example.org/b");
                assert!(!swapped, "plain `a|b` is forward");
            }
            _ => panic!("`a|b` must classify as Alternation"),
        }
        assert!(is_executable(&p), "`a|b` ships in E4");
    }

    #[test]
    fn nary_alternation_flattens_to_full_set() {
        // `a|b|c` = Alternative(a, Alternative(b, c)) — the n-ary
        // nest flattens to a 3-element predicate set.
        let p = PropertyPathExpression::Alternative(
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/a",
            ))),
            Box::new(PropertyPathExpression::Alternative(
                Box::new(PropertyPathExpression::NamedNode(iri(
                    "http://example.org/b",
                ))),
                Box::new(PropertyPathExpression::NamedNode(iri(
                    "http://example.org/c",
                ))),
            )),
        );
        match classify_path(&var("s"), &p, &var("o")) {
            PathPlan::Alternation { predicates, .. } => {
                let got: Vec<&str> = predicates.iter().map(|n| n.as_str()).collect();
                assert_eq!(
                    got,
                    vec![
                        "http://example.org/a",
                        "http://example.org/b",
                        "http://example.org/c"
                    ]
                );
            }
            _ => panic!("`a|b|c` must classify as Alternation"),
        }
    }

    #[test]
    fn inverse_alternation_folds_to_swapped() {
        // `^(a|b)` = Reverse(Alternative(a,b)) — inverse of an
        // alternation = alternation of the inverse over the swapped
        // edge (uniform direction, ships).
        let p = PropertyPathExpression::Reverse(Box::new(PropertyPathExpression::Alternative(
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/a",
            ))),
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/b",
            ))),
        )));
        match classify_path(&var("s"), &p, &var("o")) {
            PathPlan::Alternation {
                predicates,
                swapped,
            } => {
                assert_eq!(predicates.len(), 2);
                assert!(swapped, "`^(a|b)` walks the inverse edge");
            }
            _ => panic!("`^(a|b)` must classify as Alternation"),
        }
        assert!(is_executable(&p));
    }

    #[test]
    fn alternation_recursion_composition_classifies() {
        // `(a|b)+` = OneOrMore(Alternative(a,b)) — the alternation
        // becomes the recursive step's predicate SET. Ships in E4.
        let plus =
            PropertyPathExpression::OneOrMore(Box::new(PropertyPathExpression::Alternative(
                Box::new(PropertyPathExpression::NamedNode(iri(
                    "http://example.org/a",
                ))),
                Box::new(PropertyPathExpression::NamedNode(iri(
                    "http://example.org/b",
                ))),
            )));
        match classify_path(&var("s"), &plus, &var("o")) {
            PathPlan::OneOrMore { predicates, .. } => assert_eq!(predicates.len(), 2),
            _ => panic!("`(a|b)+` must classify as OneOrMore over the set"),
        }
        assert!(is_executable(&plus), "`(a|b)+` ships in E4");
        // `(a|b)*` and `(a|b)?` likewise.
        let star =
            PropertyPathExpression::ZeroOrMore(Box::new(PropertyPathExpression::Alternative(
                Box::new(PropertyPathExpression::NamedNode(iri(
                    "http://example.org/a",
                ))),
                Box::new(PropertyPathExpression::NamedNode(iri(
                    "http://example.org/b",
                ))),
            )));
        assert!(matches!(
            classify_path(&var("s"), &star, &var("o")),
            PathPlan::ZeroOrMore { .. }
        ));
        assert!(is_executable(&star), "`(a|b)*` ships in E4");
    }

    #[test]
    #[should_panic(expected = "gated stretch goal")]
    fn alternation_with_sequence_arm_is_gated() {
        // `(a/b | c)` = Alternative(Sequence(a,b), c) — an arm that
        // is itself a sequence. The §7.1-permitted gated remainder.
        let p = PropertyPathExpression::Alternative(
            Box::new(PropertyPathExpression::Sequence(
                Box::new(PropertyPathExpression::NamedNode(iri(
                    "http://example.org/a",
                ))),
                Box::new(PropertyPathExpression::NamedNode(iri(
                    "http://example.org/b",
                ))),
            )),
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/c",
            ))),
        );
        assert!(!is_executable(&p));
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
    #[should_panic(expected = "nested recursive property path")]
    fn nested_recursive_star_panics() {
        // `(a/b)*` = ZeroOrMore(Sequence(...)) — a `*` whose inner
        // box is a SEQUENCE, not a plain (optionally inverted)
        // predicate nor a plain-arm alternation. The §7.1-permitted
        // gated remainder. (`(a|b)*` now SHIPS in E4 — see
        // `alternation_recursion_composition_classifies`.)
        let p = PropertyPathExpression::ZeroOrMore(Box::new(PropertyPathExpression::Sequence(
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/a",
            ))),
            Box::new(PropertyPathExpression::NamedNode(iri(
                "http://example.org/b",
            ))),
        )));
        assert!(!is_executable(&p));
        let _ = classify_path(&var("s"), &p, &var("o"));
    }

    #[test]
    fn star_relation_is_plus_walk_union_zero_length_set() {
        // `p*` unscoped, both-var (no bound self-pairs): the E2 `+`
        // cycle-safe recursive walk UNION the W3C §9.3 node-set
        // (subject∪object of the active scope).
        let r = build_zero_or_more_relation_sql(
            "$1",
            "$1",
            vec![42],
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            None,
            &[],
        );
        // Transitive part is the `+` relation verbatim …
        assert!(r
            .from_fragment
            .contains("WITH RECURSIVE walk(src, dst, depth)"));
        assert!(
            r.from_fragment
                .contains("CYCLE src, dst SET is_cycle USING path"),
            "`*` reuses E2's cycle-safe walk for its transitive part"
        );
        assert!(r.from_fragment.contains("w.depth < 64"), "depth guard");
        // … UNION the zero-length node-set (subject ∪ object).
        assert!(
            r.from_fragment
                .contains("SELECT subject_id AS src, subject_id AS dst FROM pgrdf._pgrdf_quads"),
            "reflexive set over subject nodes"
        );
        assert!(
            r.from_fragment
                .contains("UNION SELECT object_id, object_id"),
            "reflexive set over object nodes"
        );
        assert_eq!(r.columns, "(subject_id, object_id)");
        // Truncation accounting is inherited unchanged from `+`.
        assert_eq!(r.probe_params, vec![42]);
        assert!(r.probe_sql.contains("w.depth = 64"));
    }

    #[test]
    fn star_bound_endpoint_injects_unconditional_self_pair() {
        // `<x> p* ?o` with x bound (placeholder $7): the W3C §9.3
        // bound-endpoint self-pair holds even if x is not a graph
        // node — injected as a constant `SELECT $7,$7`.
        let r = build_zero_or_more_relation_sql(
            "$1",
            "$1",
            vec![42],
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            None,
            &["$7".to_string()],
        );
        assert!(
            r.from_fragment
                .contains("SELECT $7::bigint AS src, $7::bigint AS dst"),
            "unconditional bound-endpoint self-pair"
        );
    }

    #[test]
    fn star_variable_scope_carries_gid_no_constant_self_pair() {
        // `GRAPH ?g` `*`: per-named-graph identity (carries gid,
        // excludes graph 0); a bound endpoint flows through the
        // scoped node-set, so NO constant self-pair even if provided.
        let r = build_zero_or_more_relation_sql(
            "$2",
            "$1",
            vec![9],
            None,
            &PathGraphScope::Variable,
            false,
            32,
            None,
            &["$9".to_string()],
        );
        assert_eq!(r.columns, "(subject_id, object_id, graph_id)");
        assert!(r.from_fragment.contains("q.graph_id = w.gid"));
        assert!(
            r.from_fragment
                .contains("SELECT subject_id AS src, subject_id AS dst, graph_id AS gid"),
            "per-graph reflexive set"
        );
        assert!(
            r.from_fragment.contains("WHERE graph_id <> 0"),
            "named graphs only (W3C §13.3)"
        );
        assert!(
            !r.from_fragment.contains("$9::bigint"),
            "Variable scope does not inject a constant self-pair"
        );
    }

    #[test]
    fn opt_relation_is_direct_edge_union_zero_length_no_probe() {
        // `p?` unscoped: direct `p` edge UNION the SAME zero-length
        // node-set `*` uses. NON-recursive — empty probe.
        let r = build_zero_or_one_relation_sql("$1", None, &PathGraphScope::AllGraphs, false, &[]);
        assert!(
            r.from_fragment
                .contains("SELECT subject_id AS src, object_id AS dst"),
            "direct forward `p` edge"
        );
        assert!(
            r.from_fragment.contains("WHERE predicate_id IN ($1)"),
            "direct arm filters the predicate set (1-elem = old `= $1`)"
        );
        assert!(
            r.from_fragment
                .contains("SELECT subject_id AS src, subject_id AS dst FROM pgrdf._pgrdf_quads"),
            "reflexive node-set shared with `*`"
        );
        assert!(
            !r.from_fragment.contains("WITH RECURSIVE"),
            "`?` is non-recursive"
        );
        assert!(r.probe_sql.is_empty(), "`?` cannot truncate — empty probe");
        assert!(r.probe_params.is_empty());
        assert_eq!(r.columns, "(subject_id, object_id)");

        // Inverse `(^p)?`: direct arm reads object_id → subject_id.
        let ri = build_zero_or_one_relation_sql("$1", None, &PathGraphScope::AllGraphs, true, &[]);
        assert!(ri
            .from_fragment
            .contains("SELECT object_id AS src, subject_id AS dst"));
    }

    #[test]
    fn opt_predicate_set_widens_to_in_list() {
        // `(a|b)?` — the direct arm widens to `IN ($1, $2)` (the E4
        // alternation-recursion composition path).
        let r =
            build_zero_or_one_relation_sql("$1, $2", None, &PathGraphScope::AllGraphs, false, &[]);
        assert!(
            r.from_fragment.contains("WHERE predicate_id IN ($1, $2)"),
            "`(a|b)?` direct arm scans the predicate set"
        );
    }

    #[test]
    fn alternation_relation_is_nonreflexive_single_step() {
        // Top-level `a|b` unscoped: a single non-reflexive step over
        // the predicate set — NO recursion, NO zero-length identity.
        let r = build_alternation_relation_sql("$1, $2", None, &PathGraphScope::AllGraphs, false);
        assert!(
            r.from_fragment
                .contains("SELECT DISTINCT subject_id AS src, object_id AS dst"),
            "forward single step"
        );
        assert!(
            r.from_fragment.contains("WHERE predicate_id IN ($1, $2)"),
            "union of per-predicate scans as one IN-list scan"
        );
        assert!(
            !r.from_fragment.contains("WITH RECURSIVE"),
            "`|` is not a closure — no recursion"
        );
        assert!(
            !r.from_fragment.contains("subject_id AS dst"),
            "`|` is non-reflexive — no identity pairs"
        );
        assert!(r.probe_sql.is_empty(), "non-recursive — empty probe");
        assert_eq!(r.columns, "(subject_id, object_id)");

        // Inverse `^(a|b)`: swapped endpoints.
        let ri = build_alternation_relation_sql("$1, $2", None, &PathGraphScope::AllGraphs, true);
        assert!(ri
            .from_fragment
            .contains("SELECT DISTINCT object_id AS src, subject_id AS dst"));

        // `GRAPH ?g`: per-graph, carries gid.
        let rv = build_alternation_relation_sql("$3", None, &PathGraphScope::Variable, false);
        assert_eq!(rv.columns, "(subject_id, object_id, graph_id)");
        assert!(rv.from_fragment.contains("graph_id <> 0"));
    }

    #[test]
    fn relation_sql_shapes_forward_and_inverse() {
        // Forward `p+`, unscoped: walk subject→object, no graph pred,
        // 2-column relation, UNION (cycle-safe), depth cap present.
        let r = build_one_or_more_relation_sql(
            "$1",
            "$1",
            vec![42],
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            None,
        );
        assert!(r
            .from_fragment
            .contains("WITH RECURSIVE walk(src, dst, depth)"));
        assert!(r.from_fragment.contains("SELECT subject_id, object_id"));
        assert!(
            r.from_fragment.contains("WHERE predicate_id IN ($1)"),
            "1-elem predicate set = old `= $1`"
        );
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
            "$1",
            vec![7],
            None,
            &PathGraphScope::AllGraphs,
            true,
            64,
            None,
        );
        assert!(ri.from_fragment.contains("SELECT object_id, subject_id"));
        assert!(ri.from_fragment.contains("q.subject_id"));

        // GRAPH ?g (Variable) carries gid + same-graph recursive hop.
        let rv = build_one_or_more_relation_sql(
            "$2",
            "$1",
            vec![9],
            None,
            &PathGraphScope::Variable,
            false,
            32,
            None,
        );
        assert_eq!(rv.columns, "(subject_id, object_id, graph_id)");
        assert!(rv.from_fragment.contains("q.graph_id = w.gid"));
        assert!(rv.from_fragment.contains("graph_id <> 0"));
    }

    #[test]
    fn plus_predicate_set_widens_relation_and_probe() {
        // `(a|b)+` — both the recursive CTE and the truncation probe
        // range over the predicate SET (`IN (...)`), not a single id.
        let r = build_one_or_more_relation_sql(
            "$1, $2",
            "$1, $2",
            vec![10, 20],
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            None,
        );
        assert!(
            r.from_fragment.contains("WHERE predicate_id IN ($1, $2)"),
            "base arm scans the predicate set"
        );
        assert!(
            r.from_fragment.contains("q.predicate_id IN ($1, $2)"),
            "recursive arm scans the predicate set"
        );
        assert_eq!(r.probe_params, vec![10, 20]);
        assert!(
            r.probe_sql.contains("predicate_id IN ($1, $2)"),
            "probe binds the full predicate set"
        );
    }
}
