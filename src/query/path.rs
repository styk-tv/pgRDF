//! SPARQL property-path translation — Phase E group E1 (LLD v0.4 §7).
//!
//! Property paths arrive in the spargebra algebra as
//! `GraphPattern::Path { subject, path, object }`, where `path` is a
//! [`PropertyPathExpression`]. The v0.3 translator only handled plain
//! `GraphPattern::Bgp` triples; this module is the dispatch point that
//! lowers a property path back into the existing BGP machinery.
//!
//! ## What E1 ships (LLD v0.4 §7.2 / §7.3)
//!
//! * **Bare predicate** — `NamedNode(p)`. spargebra sometimes wraps an
//!   ordinary predicate as a `Path` when it sits adjacent to a path
//!   operator (or under certain parser productions). It is semantically
//!   identical to the triple `?s p ?o`, so we rewrite it to exactly
//!   that `TriplePattern` and let `pattern_clauses` do the rest.
//! * **Inverse** — `Reverse(NamedNode(p))` = `^p`. Per §7.2 this needs
//!   **no recursion**: `?s ^p ?o` ≡ `?o p ?s`. We rewrite to the same
//!   predicate triple with subject/object **swapped**. Nested reverses
//!   collapse pairwise (`^(^p)` = `p` — note the W3C grammar reserves
//!   the `^^` token for typed-literal datatypes, so a double inverse
//!   is written `^(^p)` and arrives as `Reverse(Reverse(NamedNode))`),
//!   so we fold an even/odd swap count down to a single (possibly
//!   swapped) triple.
//!
//! Because the output is an ordinary [`TriplePattern`], the rewrite
//! composes for free with everything the BGP walker already supports:
//! named-graph scoping (`GRAPH <iri>` / `GRAPH ?g`), multi-pattern BGP
//! joins, OPTIONAL / UNION / MINUS wrappers, and `pgrdf.construct`
//! (which routes its WHERE through the same `parse_select` walker).
//!
//! ## What E1 does NOT ship (deferred — stable preview panics)
//!
//! Recursive operators (`*` / `+` / `?`) need the recursive-CTE
//! machinery (LLD v0.4 §7.2); they land in Phase E groups E2/E3.
//! Alternation (`|`) is a gated stretch goal (group E4). Negated
//! property sets are out of scope for v0.4 entirely. Each panics with
//! a STABLE prefix so downstream tooling can preview the rollout
//! schedule without depending on the (slice-number-bearing) tail —
//! the exact same convention Phase C's per-form UPDATE panics use.

use spargebra::algebra::PropertyPathExpression;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};

/// Stable panic prefix for `+` (one-or-more). Lands in Phase E group
/// E2 (≈ slice 45). Tooling matches on the prefix; the slice tail may
/// shift as the cycle countdown advances.
pub(crate) const PANIC_ONE_OR_MORE: &str =
    "pgrdf: property path operator '+' lands in Phase E group E2 (slice 45)";

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

/// Lower a property-path pattern into an equivalent ordinary
/// [`TriplePattern`] for the E1-supported operator set.
///
/// E1 handles only the non-recursive surface:
///   * bare `NamedNode(p)`            → `subject p object`
///   * `Reverse(NamedNode(p))` (`^p`) → `object p subject`
///   * nested `Reverse` over the above (`^(^p)` = `p`,
///     `^(^(^p))` = `^p`, …) — the swap parity folds down to a
///     single triple.
///
/// `Sequence(p1, p2)` (`p1/p2`) is **rejected** with a clear message:
/// sequence paths are already expressible as a multi-pattern BGP, so
/// E1 deliberately does not desugar them (keeps the E1 surface narrow
/// and avoids minting synthetic join variables that would leak into
/// `SELECT *` projection). E2+ may revisit if a real need appears.
///
/// Every recursive operator / alternation / negated set panics with
/// the corresponding stable prefix above.
pub(crate) fn translate_property_path(
    subject: &TermPattern,
    path: &PropertyPathExpression,
    object: &TermPattern,
) -> TriplePattern {
    // Fold nested `Reverse` wrappers: track whether the net effect is
    // a swap (odd number of reverses) and descend to the innermost
    // non-reverse expression.
    let mut swapped = false;
    let mut cur = path;
    loop {
        match cur {
            PropertyPathExpression::Reverse(inner) => {
                swapped = !swapped;
                cur = inner;
            }
            PropertyPathExpression::NamedNode(p) => {
                let predicate = NamedNodePattern::NamedNode(p.clone());
                // `^p` ≡ `?o p ?s` (LLD v0.4 §7.2): swap the subject /
                // object roles in the emitted triple. Even reverse
                // count (`^(^p)`) collapses back to the plain predicate.
                let (s, o) = if swapped {
                    (object.clone(), subject.clone())
                } else {
                    (subject.clone(), object.clone())
                };
                return TriplePattern {
                    subject: s,
                    predicate,
                    object: o,
                };
            }
            PropertyPathExpression::Sequence(_, _) => {
                // `p1/p2` — already a 2-pattern BGP in user-facing
                // SPARQL; E1 does not desugar (would mint a synthetic
                // join var that pollutes SELECT *). Reject clearly.
                panic!(
                    "pgrdf: sequence property paths (p1/p2) are not a property-path \
                     operator in pgRDF — express them as a multi-pattern BGP \
                     (`{{ ?s p1 ?mid . ?mid p2 ?o }}`)"
                );
            }
            PropertyPathExpression::OneOrMore(_) => panic!("{PANIC_ONE_OR_MORE}"),
            PropertyPathExpression::ZeroOrMore(_) => panic!("{PANIC_ZERO_OR_MORE}"),
            PropertyPathExpression::ZeroOrOne(_) => panic!("{PANIC_ZERO_OR_ONE}"),
            PropertyPathExpression::Alternative(_, _) => panic!("{PANIC_ALTERNATION}"),
            PropertyPathExpression::NegatedPropertySet(_) => panic!("{PANIC_NEGATED}"),
        }
    }
}

/// Is this property-path expression in the E1-supported set (so the
/// executor will lower it to a triple rather than panic)?
///
/// `true`  → bare predicate, `^p`, or nested `^(^…)` over a predicate.
/// `false` → recursive (`*`/`+`/`?`), alternation (`|`), negated set,
///           or sequence (`p1/p2`).
///
/// Used by the parser-side `sparql_parse` analysis so it can flag the
/// unsupported variants in `unsupported_algebra` (parse-time, no
/// panic) — mirroring how Phase C reports not-yet-shipped UPDATE
/// forms. Execution still panics with the stable rollout prefix.
pub(crate) fn is_e1_supported(path: &PropertyPathExpression) -> bool {
    let mut cur = path;
    loop {
        match cur {
            PropertyPathExpression::Reverse(inner) => cur = inner,
            PropertyPathExpression::NamedNode(_) => return true,
            _ => return false,
        }
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

    #[test]
    fn bare_named_node_is_direct_triple() {
        let p = PropertyPathExpression::NamedNode(iri("http://example.org/p"));
        let tp = translate_property_path(&var("s"), &p, &var("o"));
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
        let tp = translate_property_path(&var("s"), &p, &var("o"));
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
        let tp = translate_property_path(&var("s"), &p, &var("o"));
        // `^(^p)` = `p` — no swap.
        assert!(matches!(tp.subject, TermPattern::Variable(ref v) if v.as_str() == "s"));
        assert!(matches!(tp.object, TermPattern::Variable(ref v) if v.as_str() == "o"));
    }

    #[test]
    #[should_panic(expected = "lands in Phase E group E2")]
    fn one_or_more_preview_panics() {
        let p = PropertyPathExpression::OneOrMore(Box::new(PropertyPathExpression::NamedNode(
            iri("http://example.org/p"),
        )));
        let _ = translate_property_path(&var("s"), &p, &var("o"));
    }

    #[test]
    #[should_panic(expected = "lands in Phase E group E3")]
    fn zero_or_more_preview_panics() {
        let p = PropertyPathExpression::ZeroOrMore(Box::new(PropertyPathExpression::NamedNode(
            iri("http://example.org/p"),
        )));
        let _ = translate_property_path(&var("s"), &p, &var("o"));
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
        let _ = translate_property_path(&var("s"), &p, &var("o"));
    }

    #[test]
    #[should_panic(expected = "out of scope for v0.4")]
    fn negated_property_set_panics() {
        let p = PropertyPathExpression::NegatedPropertySet(vec![iri("http://example.org/p")]);
        let _ = translate_property_path(&var("s"), &p, &var("o"));
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
        let _ = translate_property_path(&var("s"), &p, &var("o"));
    }
}
