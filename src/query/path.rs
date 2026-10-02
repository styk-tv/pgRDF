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
pub(crate) const PANIC_ONE_OR_MORE_NESTED: &str = "pgrdf: nested recursive property path (e.g. `(p*)+`, `(a/b|c)`) is a gated stretch goal (Phase E group E4)";

/// Stable panic for negated property sets `!(...)` — out of v0.4 scope.
pub(crate) const PANIC_NEGATED: &str = "pgrdf: negated property sets are out of scope for v0.4";

/// Stable rejection for sequence paths `p1/p2`. They are already a
/// 2-pattern BGP in user-facing SPARQL; E2 keeps E1's stance and does
/// not desugar (would mint a synthetic join var that pollutes
/// `SELECT *`).
pub(crate) const PANIC_SEQUENCE: &str = "pgrdf: sequence property paths (p1/p2) are not a property-path \
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
    if matches!(inner, PropertyPathExpression::Alternative(_, _))
        && let Some((preds, sw)) = flatten_alternation(inner, outer_swapped)
    {
        return (preds, sw);
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
                        };
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
                return inner_is_plain_or_alternation(inner);
            }
            // E4 — top-level `a|b` (and `^(a|b)` since we tunnelled
            // through `Reverse` above): executable iff every arm is
            // a plain (optionally inverted, uniform-direction)
            // predicate.
            PropertyPathExpression::Alternative(_, _) => {
                return flatten_alternation(cur, false).is_some();
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

/// A `+` / `*` / `?` / `|` path lowered to a derived relation that
/// `executor.rs` substitutes for a `_pgrdf_quads` alias in its FROM list:
/// it exposes the same `subject_id` / `object_id` column names a quad
/// alias would, so the var-binder joins it unchanged.
///
/// `from_fragment` is the parenthesised derived table WITHOUT the trailing
/// alias (executor appends `AS q{qi}(...)`). For `+` / `*` it is the walk
/// rendered with no variable seed — a constant endpoint, when the path
/// has one, is already baked in. `walk` lets the emitter re-render it
/// seeded from a variable an earlier pattern bound (SPEC 0.6.37 §3.2) and
/// names the path id the executor checks for truncation.
#[derive(Clone)]
pub(crate) struct PathRelation {
    pub from_fragment: String,
    /// Column list the executor pins on the alias —
    /// `(subject_id, object_id)` for an unscoped / literal-graph
    /// walk, `(subject_id, object_id, graph_id)` when a `GRAPH ?g`
    /// variable scope needs the per-row graph id surfaced.
    pub columns: &'static str,
    /// `Some` for the recursive operators (`+`, `*`), walked by
    /// `pgrdf._path_walk`.
    pub walk: Option<WalkSpec>,
}

/// Which endpoint of a walked path is already bound, as a SQL
/// expression: a dict-id placeholder (`$N`) for a constant, or an
/// earlier alias's column (`q2.subject_id`) for a join-bound variable.
pub(crate) enum Seed<'a> {
    Subject(&'a str),
    Object(&'a str),
}

/// Everything needed to (re-)render a `+` / `*` walk.
#[derive(Clone)]
pub(crate) struct WalkSpec {
    /// Index of this path pattern within its statement; the walk records
    /// a depth cut under it and the executor applies
    /// `pgrdf.on_path_truncation` once per truncated pattern.
    pub path_id: i32,
    /// Dict-id placeholders of the predicate set, e.g. `$3` or `$3, $4`.
    preds_sql: String,
    /// `Literal` scope: the graph-id placeholder.
    graph_sql: Option<String>,
    /// `GRAPH ?g`: walk each named graph separately; never seeded.
    variable_scope: bool,
    /// `^p+` / `(^p)+`: the walk follows edges object → subject.
    swapped: bool,
    max_depth: i32,
    /// `*` (zero-or-more) — adds the W3C §9.3 zero-length pairs.
    reflexive: bool,
    /// The unseeded zero-length node set (for `*` with no bound end).
    zero_unseeded: String,
    /// A constant endpoint is already baked into `from_fragment`.
    pub const_seeded: bool,
}

impl WalkSpec {
    /// Can the emitter seed this walk from a join-bound variable? Not when
    /// a constant already seeds it, and never under `GRAPH ?g`.
    pub(crate) fn seedable(&self) -> bool {
        !self.const_seeded && !self.variable_scope
    }

    /// Render the walk as a parenthesised derived table yielding
    /// `(src, dst[, gid])`. With a seed the walk starts at that node:
    /// forward from a bound subject, backward (edges reversed) from a
    /// bound object — `src` / `dst` keep the path's direction either way.
    pub(crate) fn render(&self, seed: Option<Seed<'_>>) -> String {
        let (seed_expr, from_object) = match &seed {
            None => ("NULL::bigint".to_string(), false),
            // COALESCE: a seed column can be NULL at run time (a variable
            // only an earlier OPTIONAL bound). NULL must mean "no node" —
            // as an unbound join variable equals nothing — never
            // "unseeded", which would walk the whole graph per row.
            Some(Seed::Subject(e)) => (format!("COALESCE({e}, -1)::bigint"), false),
            Some(Seed::Object(e)) => (format!("COALESCE({e}, -1)::bigint"), true),
        };
        // `GRAPH ?g` walks are never seeded (the seed's graph would have
        // to be the walk's graph); they walk every named graph.
        let seeded = seed.is_some() && !self.variable_scope;
        let seed_expr = if seeded {
            seed_expr
        } else {
            "NULL::bigint".to_string()
        };
        let from_object = seeded && from_object;
        let follow_inverse = self.swapped ^ from_object;
        let graph = self
            .graph_sql
            .as_deref()
            .map(|g| format!("{g}::bigint"))
            .unwrap_or_else(|| "NULL::bigint".to_string());
        let (src, dst) = if from_object {
            ("w.reached", "w.start")
        } else {
            ("w.start", "w.reached")
        };
        let gid = if self.variable_scope { ", w.gid" } else { "" };
        let walk = format!(
            "SELECT {src} AS src, {dst} AS dst{gid} FROM pgrdf._path_walk(\
             ARRAY[{preds}]::bigint[], {graph}, {seed_expr}, {follow_inverse}, \
             {max}, {id}, {per_graph}) w",
            preds = self.preds_sql,
            max = self.max_depth,
            id = self.path_id,
            per_graph = self.variable_scope,
        );
        if !self.reflexive {
            return format!("({walk})");
        }
        let zero = if seeded {
            // A bound endpoint x contributes (x, x) unconditionally
            // (W3C §9.3), even when x is not a node of the graph.
            format!("SELECT {seed_expr} AS src, {seed_expr} AS dst")
        } else {
            self.zero_unseeded.clone()
        };
        format!("({walk} UNION {zero})")
    }
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

/// Build the relation for a `+` path (also the E4 `(a|b)+` step): a
/// breadth-first `pgrdf._path_walk` (SPEC 0.6.37 §3.2, issue #138) in
/// place of the old `UNION ALL` + `CYCLE … USING path` CTE, which
/// enumerated every simple path up to the depth cap.
///
/// `pred_ids_sql` is the predicate-set placeholder list (`$3` or
/// `$3, $4`), `graph_placeholder` the `Literal` scope's graph-id
/// placeholder, `max_depth` the `pgrdf.path_max_depth` read once at
/// translate time, `path_id` this pattern's index within the statement.
/// `const_seed` bakes a constant endpoint in; otherwise the emitter may
/// re-render the walk seeded from a join-bound variable.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_one_or_more_relation_sql(
    pred_ids_sql: &str,
    graph_placeholder: Option<&str>,
    scope: &PathGraphScope,
    swapped: bool,
    max_depth: i32,
    path_id: i32,
    const_seed: Option<Seed<'_>>,
) -> PathRelation {
    walk_relation(
        pred_ids_sql,
        graph_placeholder,
        scope,
        swapped,
        max_depth,
        path_id,
        const_seed,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn walk_relation(
    pred_ids_sql: &str,
    graph_placeholder: Option<&str>,
    scope: &PathGraphScope,
    swapped: bool,
    max_depth: i32,
    path_id: i32,
    const_seed: Option<Seed<'_>>,
    reflexive: bool,
) -> PathRelation {
    let variable_scope = matches!(scope, PathGraphScope::Variable);
    let graph_sql = match scope {
        PathGraphScope::Literal(_) => Some(
            graph_placeholder
                .expect("Literal scope needs a graph placeholder")
                .to_string(),
        ),
        _ => None,
    };
    let columns = if variable_scope {
        "(subject_id, object_id, graph_id)"
    } else {
        "(subject_id, object_id)"
    };
    let const_seeded = const_seed.is_some() && !variable_scope;
    let spec = WalkSpec {
        path_id,
        preds_sql: pred_ids_sql.to_string(),
        graph_sql,
        variable_scope,
        swapped,
        max_depth,
        reflexive,
        zero_unseeded: if reflexive {
            zero_length_node_set_sql(scope, &[])
        } else {
            String::new()
        },
        const_seeded,
    };
    PathRelation {
        from_fragment: spec.render(const_seed),
        columns,
        walk: Some(spec),
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

/// Build the relation for a `*` (zero-or-more) path — W3C SPARQL 1.1
/// §9.3: the `+` walk `UNION` the zero-length pairs. Unseeded, those are
/// the node set of the active scope; seeded from a bound endpoint x, the
/// single pair (x, x), which holds even when x is not a node of the
/// graph. The zero-length arm is a plain scan and cannot truncate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_zero_or_more_relation_sql(
    pred_ids_sql: &str,
    graph_placeholder: Option<&str>,
    scope: &PathGraphScope,
    swapped: bool,
    max_depth: i32,
    path_id: i32,
    const_seed: Option<Seed<'_>>,
) -> PathRelation {
    walk_relation(
        pred_ids_sql,
        graph_placeholder,
        scope,
        swapped,
        max_depth,
        path_id,
        const_seed,
        true,
    )
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
        walk: None,
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
        walk: None,
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
        // `p*` unscoped, both-var: the breadth-first walk (no CTE, no
        // path enumeration) UNION the W3C §9.3 node-set.
        let r = build_zero_or_more_relation_sql(
            "$1",
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            0,
            None,
        );
        assert!(
            r.from_fragment.contains(
                "pgrdf._path_walk(ARRAY[$1]::bigint[], NULL::bigint, NULL::bigint, false, 64, 0, false)"
            ),
            "unseeded forward walk: {}",
            r.from_fragment
        );
        assert!(!r.from_fragment.contains("WITH RECURSIVE"));
        assert!(!r.from_fragment.contains("CYCLE"), "no path enumeration");
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
        assert_eq!(r.walk.as_ref().map(|w| w.path_id), Some(0));
    }

    #[test]
    fn star_bound_endpoint_seeds_walk_and_self_pair() {
        // `<x> p* ?o` with x bound ($7): the walk starts at x and the
        // zero-length arm is the single pair (x, x) — W3C §9.3 holds it
        // even when x is not a graph node.
        let r = build_zero_or_more_relation_sql(
            "$1",
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            3,
            Some(Seed::Subject("$7")),
        );
        assert!(
            r.from_fragment
                .contains("NULL::bigint, COALESCE($7, -1)::bigint, false, 64, 3, false)"),
            "seeded forward from the bound subject: {}",
            r.from_fragment
        );
        assert!(
            r.from_fragment.contains(
                "UNION SELECT COALESCE($7, -1)::bigint AS src, COALESCE($7, -1)::bigint AS dst"
            ),
            "unconditional bound-endpoint self-pair"
        );
        assert!(r.walk.as_ref().unwrap().const_seeded);
    }

    #[test]
    fn plus_seeded_from_object_walks_backward() {
        // `?s p+ <x>`: start at x and follow edges object → subject;
        // the columns keep the path's own direction (src = reached).
        let r = build_one_or_more_relation_sql(
            "$1",
            Some("$2"),
            &PathGraphScope::Literal(5),
            false,
            8,
            1,
            Some(Seed::Object("$9")),
        );
        assert!(
            r.from_fragment
                .contains("SELECT w.reached AS src, w.start AS dst")
        );
        assert!(
            r.from_fragment.contains(
                "ARRAY[$1]::bigint[], $2::bigint, COALESCE($9, -1)::bigint, true, 8, 1, false)"
            ),
            "{}",
            r.from_fragment
        );
        // `^p+` seeded from the object follows edges forward again.
        let s = build_one_or_more_relation_sql(
            "$1",
            Some("$2"),
            &PathGraphScope::Literal(5),
            true,
            8,
            1,
            Some(Seed::Object("$9")),
        );
        assert!(
            s.from_fragment
                .contains("COALESCE($9, -1)::bigint, false, 8, 1, false)")
        );
    }

    #[test]
    fn star_variable_scope_walks_per_graph_unseeded() {
        // `GRAPH ?g` `*`: walks each named graph (per_graph = true),
        // carries gid, excludes graph 0 — and is never seeded, even when
        // an endpoint is bound (the seed's graph is not the walk's).
        let r = build_zero_or_more_relation_sql(
            "$2",
            None,
            &PathGraphScope::Variable,
            false,
            32,
            0,
            Some(Seed::Subject("$9")),
        );
        assert_eq!(r.columns, "(subject_id, object_id, graph_id)");
        assert!(r.from_fragment.contains(", w.gid FROM pgrdf._path_walk("));
        assert!(
            r.from_fragment
                .contains("NULL::bigint, false, 32, 0, true)")
        );
        assert!(
            r.from_fragment
                .contains("SELECT subject_id AS src, subject_id AS dst, graph_id AS gid"),
            "per-graph reflexive set"
        );
        assert!(r.from_fragment.contains("WHERE graph_id <> 0"));
        assert!(!r.from_fragment.contains("$9"));
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
        assert!(r.walk.is_none(), "`?` is not walked and cannot truncate");
        assert_eq!(r.columns, "(subject_id, object_id)");

        // Inverse `(^p)?`: direct arm reads object_id → subject_id.
        let ri = build_zero_or_one_relation_sql("$1", None, &PathGraphScope::AllGraphs, true, &[]);
        assert!(
            ri.from_fragment
                .contains("SELECT object_id AS src, subject_id AS dst")
        );
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
        assert!(r.walk.is_none(), "non-recursive — not walked");
        assert_eq!(r.columns, "(subject_id, object_id)");

        // Inverse `^(a|b)`: swapped endpoints.
        let ri = build_alternation_relation_sql("$1, $2", None, &PathGraphScope::AllGraphs, true);
        assert!(
            ri.from_fragment
                .contains("SELECT DISTINCT object_id AS src, subject_id AS dst")
        );

        // `GRAPH ?g`: per-graph, carries gid.
        let rv = build_alternation_relation_sql("$3", None, &PathGraphScope::Variable, false);
        assert_eq!(rv.columns, "(subject_id, object_id, graph_id)");
        assert!(rv.from_fragment.contains("graph_id <> 0"));
    }

    #[test]
    fn relation_sql_shapes_forward_and_inverse() {
        // Forward `p+`, unscoped and unseeded: one breadth-first walk
        // from every source; no recursive CTE, no path enumeration.
        let r = build_one_or_more_relation_sql(
            "$1",
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            0,
            None,
        );
        assert!(
            r.from_fragment.contains(
                "(SELECT w.start AS src, w.reached AS dst FROM pgrdf._path_walk(\
                 ARRAY[$1]::bigint[], NULL::bigint, NULL::bigint, false, 64, 0, false) w)"
            ),
            "{}",
            r.from_fragment
        );
        assert!(!r.from_fragment.contains("UNION"), "`+` is non-reflexive");
        assert_eq!(r.columns, "(subject_id, object_id)");
        assert!(!r.walk.as_ref().unwrap().const_seeded);

        // Inverse `^p+`: the walk follows edges object → subject.
        let ri = build_one_or_more_relation_sql(
            "$1",
            None,
            &PathGraphScope::AllGraphs,
            true,
            64,
            0,
            None,
        );
        assert!(
            ri.from_fragment
                .contains("NULL::bigint, true, 64, 0, false)")
        );

        // GRAPH ?g (Variable): per-graph walk carrying gid.
        let rv = build_one_or_more_relation_sql(
            "$2",
            None,
            &PathGraphScope::Variable,
            false,
            32,
            2,
            None,
        );
        assert_eq!(rv.columns, "(subject_id, object_id, graph_id)");
        assert!(rv.from_fragment.contains(", w.gid FROM pgrdf._path_walk("));
        assert!(rv.from_fragment.contains("false, 32, 2, true)"));
    }

    #[test]
    fn plus_predicate_set_widens_walk() {
        // `(a|b)+` — the walk ranges over the predicate SET.
        let r = build_one_or_more_relation_sql(
            "$1, $2",
            None,
            &PathGraphScope::AllGraphs,
            false,
            64,
            0,
            None,
        );
        assert!(
            r.from_fragment
                .contains("pgrdf._path_walk(ARRAY[$1, $2]::bigint[]"),
            "the walk follows every predicate of the set"
        );
    }
}
