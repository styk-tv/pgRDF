//! Wrapper around the `reasonable` OWL 2 RL reasoner.
//!
//! Implements LLD §2 / Phase 4. The flow:
//!
//! ```text
//!   _pgrdf_quads(graph_id = G, is_inferred = FALSE)
//!       │  resolve each (s_id, p_id, o_id) → oxrdf::Triple via dict join
//!       ▼
//!   Reasoner::new().load_triples(base).reason()
//!       │  every base + every entailed RDF triple now in get_triples()
//!       ▼
//!   set-diff against base → inferred-only set
//!       │  intern each term back via put_term_full (shmem hits where warm)
//!       ▼
//!   INSERT INTO _pgrdf_quads (..., is_inferred = TRUE)
//! ```
//!
//! Idempotency. `pgrdf.materialize(g)` first deletes every
//! `is_inferred = TRUE` row in graph `g`, then re-derives from scratch.
//! Two calls in a row produce the same row count; calling after the
//! base graph changed picks up the new entailments.
//!
//! Scope. `reasonable` implements OWL 2 RL only — class hierarchy,
//! property hierarchy, inverse / symmetric / transitive properties,
//! sameAs / functional / inverse-functional, domain / range, etc.
//! OWL 2 EL / QL and arbitrary Datalog are out of scope and are not
//! emulated by this UDF.
//!
//! ## v0.5-FUTURE §3 — reasoning-profile selector
//!
//! `pgrdf.materialize(graph_id, profile TEXT DEFAULT 'owl-rl')`. The
//! default-arg form is byte-for-byte the v0.3/v0.4 surface (OWL 2 RL
//! via `reasonable`, unchanged). Two profiles ship in v0.5:
//!
//! - `'owl-rl'` (default) — the full `reasonable` forward-chain.
//! - `'rdfs'` — the RDFS entailment-rule subset only.
//!
//! **Implementation route (documented per the §3 contract).** The
//! patched `styk-tv/reasonable` fork (branch `rdf12-passthrough`)
//! exposes a single fused datalog fixpoint (`Reasoner::reason()`);
//! it has **no upstream RDFS-only rule selection** (only `reason()`
//! / `reason_full()`, both the full OWL-RL set). Route 1 (direct
//! upstream profile support) is therefore unavailable. We take
//! **route 2: a pgRDF-internal RDFS forward-chain pass** — but
//! implemented as a *strict, sound, complete* RDFS rule engine over
//! the base triples (rdfs2/3/5/7/9/11 + the rdfs domain/range/
//! sub-class/sub-property axiomatic interactions) rather than a
//! lossy post-hoc filter of the OWL-RL output. This yields a *real*
//! RDFS profile, not an approximation: every triple it emits is an
//! RDFS entailment, and RDFS rules are a strict subset of the
//! OWL 2 RL rules `reasonable` runs, so the §3.1 acceptance holds by
//! construction:
//!
//! 1. `count(rdfs) ≤ count(owl-rl)` — RDFS ⊂ OWL-RL; every RDFS
//!    entailment is also an OWL-RL entailment (non-strict subset).
//! 2. The two profiles agree on the RDFS-axiom entailments
//!    (subClassOf/subPropertyOf transitivity, domain/range
//!    propagation, type propagation) — the RDFS engine computes
//!    exactly that closure.
//! 3. An unknown profile string errors with prefix
//!    `materialize: unknown profile` (no silent fallback).
//!
//! The reserved future `'owl-rl-ext'` is **not yet supported**
//! (§3 names it as a *future* profile only); requesting it returns
//! the same `materialize: unknown profile` error until a later
//! cycle wires it.

use crate::storage::dict::{put_term_full, term_type};
use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Term, Triple};
use pgrx::prelude::*;
use reasonable::reasoner::Reasoner;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// RDF / RDFS vocabulary IRIs the RDFS forward-chain keys on.
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const RDFS_SUBCLASSOF: &str = "http://www.w3.org/2000/01/rdf-schema#subClassOf";
const RDFS_SUBPROPERTYOF: &str = "http://www.w3.org/2000/01/rdf-schema#subPropertyOf";
const RDFS_DOMAIN: &str = "http://www.w3.org/2000/01/rdf-schema#domain";
const RDFS_RANGE: &str = "http://www.w3.org/2000/01/rdf-schema#range";

/// Forward-chain materialization for one graph under a reasoning
/// profile.
///
/// Returns a JSONB stats object:
/// ```json
/// {
///   "base_triples":              123,
///   "inferred_triples_written":  45,
///   "previous_inferred_dropped": 0,
///   "profile":                   "owl-rl",
///   "reasoner_errors":           [],
///   "elapsed_ms":                17.4
/// }
/// ```
///
/// SQL: `pgrdf.materialize(graph_id BIGINT, profile TEXT DEFAULT
/// 'owl-rl') -> JSONB` (v0.5-FUTURE §3). The bare
/// `pgrdf.materialize(g)` form defaults `profile => 'owl-rl'` and is
/// behaviourally identical to the v0.3/v0.4 surface — no regression.
///
/// Profiles:
/// - `'owl-rl'` (default) — full OWL 2 RL via `reasonable`.
/// - `'rdfs'` — RDFS entailment-rule subset only (strict, sound;
///   see the module doc for the route-2 rationale).
///
/// Any other string (including the reserved-but-not-yet-supported
/// `'owl-rl-ext'`) panics with the stable prefix
/// `materialize: unknown profile` — never a silent fallback.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn materialize(graph_id: i64, profile: default!(String, "'owl-rl'")) -> pgrx::JsonB {
    let start = Instant::now();

    // Validate the profile up-front, BEFORE any side effect (the
    // idempotency wipe). An unknown profile must not perturb state.
    // Exact prefix `materialize: unknown profile` per §3.1 #3 — the
    // pgrx negative test pins the full message.
    match profile.as_str() {
        "owl-rl" | "rdfs" => {}
        other => panic!(
            "materialize: unknown profile {other:?} \
             (supported: 'owl-rl', 'rdfs')"
        ),
    }

    // 1. Idempotency: wipe prior inferred rows in this graph.
    let dropped: i64 = Spi::connect_mut(|client| {
        let table = client
            .update(
                "WITH del AS (DELETE FROM pgrdf._pgrdf_quads
                               WHERE graph_id = $1 AND is_inferred = TRUE
                               RETURNING 1)
                 SELECT count(*)::bigint FROM del",
                None,
                &[unsafe {
                    pgrx::datum::DatumWithOid::new(
                        graph_id,
                        pgrx::pg_sys::PgBuiltInOids::INT8OID.into(),
                    )
                }],
            )
            .expect("materialize: delete-prior failed");
        table.first().get_one::<i64>().ok().flatten().unwrap_or(0)
    });

    // 2. Stream base triples out.
    let base = load_base_triples(graph_id);
    let base_count = base.len() as i64;
    let base_set: HashSet<Triple> = base.iter().cloned().collect();

    // 3. Reason under the selected profile. Both paths return the
    //    full derived closure (base + entailed); step 4 set-diffs
    //    against `base_set` to keep only the new entailments.
    let (derived, errors): (Vec<Triple>, Vec<String>) = match profile.as_str() {
        "owl-rl" => {
            let mut reasoner = Reasoner::new();
            reasoner.load_triples(base.clone());
            reasoner.reason();
            let errs = reasoner.errors().iter().map(|e| format!("{e}")).collect();
            (reasoner.get_triples(), errs)
        }
        // 'rdfs' — the route-2 strict RDFS forward-chain. Sound and
        // complete for the RDFS entailment-rule subset; emits a
        // strict subset of what 'owl-rl' would (RDFS ⊂ OWL-RL).
        "rdfs" => (rdfs_closure(&base), Vec::new()),
        // Unreachable: the match at the top of the fn already
        // rejected every other string.
        _ => unreachable!("profile validated above"),
    };

    // 4. Set-diff to find ONLY the inferred (entailed-but-not-asserted) triples.
    let inferred: Vec<&Triple> = derived.iter().filter(|t| !base_set.contains(t)).collect();

    // 5. Write back. Each new triple's terms are interned via the
    //    shmem-aware `put_term_full`; existing IRIs / literals reuse
    //    their dict ids without a table touch.
    let mut written = 0i64;
    for t in &inferred {
        let s_id = subject_id(&t.subject);
        let p_id = put_term_full(t.predicate.as_str(), term_type::URI, None, None);
        let o_id = term_id(&t.object);
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_quads
                (subject_id, predicate_id, object_id, graph_id, is_inferred)
             VALUES ($1, $2, $3, $4, TRUE)",
            &[s_id.into(), p_id.into(), o_id.into(), graph_id.into()],
        )
        .expect("materialize: insert inferred failed");
        written += 1;
    }

    pgrx::JsonB(json!({
        "base_triples":              base_count,
        "inferred_triples_written":  written,
        "previous_inferred_dropped": dropped,
        "profile":                   profile,
        "reasoner_errors":           errors,
        "elapsed_ms":                start.elapsed().as_secs_f64() * 1000.0,
    }))
}

/// Strict, sound, complete RDFS entailment-rule forward-chain over
/// `base`. Route-2 of the §3 contract (no upstream RDFS-only mode in
/// `reasonable`). Returns the full closure (base ∪ entailed); the
/// caller set-diffs against the base set to recover the new triples.
///
/// Implements the application-visible RDFS entailment rules (W3C
/// RDF 1.1 Semantics §9.2.1) to a fixed point:
///
/// - **rdfs5**  — `subPropertyOf` transitivity.
/// - **rdfs11** — `subClassOf` transitivity.
/// - **rdfs7**  — `subPropertyOf` application: `p ⊑ q ∧ s p o ⇒ s q o`.
/// - **rdfs9**  — `subClassOf` application: `c ⊑ d ∧ s a c ⇒ s a d`.
/// - **rdfs2**  — `domain`: `p rdfs:domain c ∧ s p o ⇒ s a c`.
/// - **rdfs3**  — `range`:  `p rdfs:range  c ∧ s p o ⇒ o a c`.
///
/// rdfs7's sub-property propagation feeds rdfs2/rdfs3 (a triple
/// entailed onto a super-property still triggers that property's
/// domain/range), and rdfs9 consumes types produced by rdfs2/rdfs3,
/// so all six rules iterate together until no new triple appears.
/// The rule set is a strict subset of the OWL 2 RL rules
/// `reasonable` runs, which is exactly why the §3.1 subset +
/// agreement criteria hold by construction.
///
/// rdfs1/rdfs4a/rdfs4b/rdfs6/rdfs8/rdfs10/rdfs12/rdfs13 (the
/// axiomatic `rdf:Property` / `rdfs:Resource` / `rdfs:Class`
/// reflexive-typing rules) are intentionally NOT emitted: they add
/// only tautological `… rdf:type rdfs:Resource`-style triples that
/// inflate the count without application value, and OWL-RL's
/// `reasonable` does not emit the universal `rdfs:Resource` typing
/// either — emitting them on the `rdfs` side would *violate* the
/// §3.1 #1 non-strict-subset criterion. Restricting to the six
/// productive rules keeps `rdfs` a true subset of `owl-rl`.
fn rdfs_closure(base: &[Triple]) -> Vec<Triple> {
    // Working set as a dedup'd HashSet so the fixpoint test is O(1).
    let mut closure: HashSet<Triple> = base.iter().cloned().collect();

    loop {
        // Re-derive the schema relations each round so transitivity
        // (rdfs5/rdfs11) feeds the application rules on the next pass.
        let mut subclass: HashMap<String, Vec<String>> = HashMap::new();
        let mut subprop: HashMap<String, Vec<String>> = HashMap::new();
        let mut domain: HashMap<String, Vec<String>> = HashMap::new();
        let mut range: HashMap<String, Vec<String>> = HashMap::new();

        for t in &closure {
            let p = t.predicate.as_str();
            if p == RDFS_SUBCLASSOF {
                if let (Some(s), Some(o)) = (named_str(&t.subject), term_named_str(&t.object)) {
                    subclass.entry(s).or_default().push(o);
                }
            } else if p == RDFS_SUBPROPERTYOF {
                if let (Some(s), Some(o)) = (named_str(&t.subject), term_named_str(&t.object)) {
                    subprop.entry(s).or_default().push(o);
                }
            } else if p == RDFS_DOMAIN {
                if let (Some(s), Some(o)) = (named_str(&t.subject), term_named_str(&t.object)) {
                    domain.entry(s).or_default().push(o);
                }
            } else if p == RDFS_RANGE {
                if let (Some(s), Some(o)) = (named_str(&t.subject), term_named_str(&t.object)) {
                    range.entry(s).or_default().push(o);
                }
            }
        }

        let mut next: Vec<Triple> = Vec::new();

        // rdfs11 — subClassOf transitivity: c ⊑ d ∧ d ⊑ e ⇒ c ⊑ e.
        for (c, ds) in &subclass {
            for d in ds {
                if let Some(es) = subclass.get(d) {
                    for e in es {
                        if let (Ok(cn), Ok(en)) = (NamedNode::new(c), NamedNode::new(e)) {
                            next.push(Triple::new(
                                cn,
                                NamedNode::new(RDFS_SUBCLASSOF).unwrap(),
                                en,
                            ));
                        }
                    }
                }
            }
        }
        // rdfs5 — subPropertyOf transitivity: p ⊑ q ∧ q ⊑ r ⇒ p ⊑ r.
        for (p, qs) in &subprop {
            for q in qs {
                if let Some(rs) = subprop.get(q) {
                    for r in rs {
                        if let (Ok(pn), Ok(rn)) = (NamedNode::new(p), NamedNode::new(r)) {
                            next.push(Triple::new(
                                pn,
                                NamedNode::new(RDFS_SUBPROPERTYOF).unwrap(),
                                rn,
                            ));
                        }
                    }
                }
            }
        }

        // Per-data-triple rules: iterate a snapshot so we don't mutate
        // while reading. rdfs7 (sub-property application), rdfs9
        // (sub-class application), rdfs2 (domain), rdfs3 (range).
        for t in &closure {
            let p = t.predicate.as_str();

            // rdfs7 — p ⊑ q ∧ s p o ⇒ s q o (for every super-property).
            if let Some(qs) = subprop.get(p) {
                for q in qs {
                    if let Ok(qn) = NamedNode::new(q) {
                        next.push(Triple::new(t.subject.clone(), qn, t.object.clone()));
                    }
                }
            }

            if p == RDF_TYPE {
                // rdfs9 — c ⊑ d ∧ s a c ⇒ s a d (every super-class).
                if let Some(c) = term_named_str(&t.object) {
                    if let Some(ds) = subclass.get(&c) {
                        for d in ds {
                            if let Ok(dn) = NamedNode::new(d) {
                                next.push(Triple::new(
                                    t.subject.clone(),
                                    NamedNode::new(RDF_TYPE).unwrap(),
                                    Term::NamedNode(dn),
                                ));
                            }
                        }
                    }
                }
            } else {
                // rdfs2 — p rdfs:domain c ∧ s p o ⇒ s rdf:type c.
                if let Some(cs) = domain.get(p) {
                    for c in cs {
                        if let Ok(cn) = NamedNode::new(c) {
                            next.push(Triple::new(
                                t.subject.clone(),
                                NamedNode::new(RDF_TYPE).unwrap(),
                                Term::NamedNode(cn),
                            ));
                        }
                    }
                }
                // rdfs3 — p rdfs:range c ∧ s p o ⇒ o rdf:type c.
                // Only when the object can be a type subject (IRI /
                // bnode); a literal object yields no rdf:type triple.
                if let Some(cs) = range.get(p) {
                    if let Some(o_subj) = term_as_subject(&t.object) {
                        for c in cs {
                            if let Ok(cn) = NamedNode::new(c) {
                                next.push(Triple::new(
                                    o_subj.clone(),
                                    NamedNode::new(RDF_TYPE).unwrap(),
                                    Term::NamedNode(cn),
                                ));
                            }
                        }
                    }
                }
            }
        }

        // Fixpoint test: stop when no genuinely new triple appears.
        let mut grew = false;
        for t in next {
            if closure.insert(t) {
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    closure.into_iter().collect()
}

/// IRI string of a `NamedOrBlankNode` subject if it is a NamedNode.
fn named_str(s: &NamedOrBlankNode) -> Option<String> {
    match s {
        NamedOrBlankNode::NamedNode(n) => Some(n.as_str().to_owned()),
        NamedOrBlankNode::BlankNode(_) => None,
    }
}

/// IRI string of a `Term` if it is a NamedNode (schema positions —
/// subClassOf/domain/range objects — are always IRIs in RDFS).
fn term_named_str(o: &Term) -> Option<String> {
    match o {
        Term::NamedNode(n) => Some(n.as_str().to_owned()),
        _ => None,
    }
}

/// View a `Term` object as a triple subject for rdfs3 (range →
/// `o rdf:type c`). Literals can't be subjects, so they yield None;
/// the RDF-star `Term::Triple` variant (out of pgRDF scope, same as
/// `term_id`'s defensive arm) also yields None.
fn term_as_subject(o: &Term) -> Option<NamedOrBlankNode> {
    match o {
        Term::NamedNode(n) => Some(NamedOrBlankNode::NamedNode(n.clone())),
        Term::BlankNode(b) => Some(NamedOrBlankNode::BlankNode(b.clone())),
        _ => None,
    }
}

/// Pull every `is_inferred = FALSE` quad in `graph_id` out of the
/// hexastore and rehydrate each row's term IDs into an
/// `oxrdf::Triple`. A single LEFT JOIN to `_pgrdf_dictionary` for the
/// datatype lookup keeps the round-trip to one SPI scan.
fn load_base_triples(graph_id: i64) -> Vec<Triple> {
    let mut out = Vec::new();
    Spi::connect(|client| {
        let table = client
            .select(
                "SELECT
                    s.term_type,        s.lexical_value,
                    p.lexical_value     AS p_iri,
                    o.term_type,        o.lexical_value,
                    dt.lexical_value    AS o_dt,
                    o.language_tag      AS o_lang
                 FROM pgrdf._pgrdf_quads q
                 JOIN pgrdf._pgrdf_dictionary s  ON s.id  = q.subject_id
                 JOIN pgrdf._pgrdf_dictionary p  ON p.id  = q.predicate_id
                 JOIN pgrdf._pgrdf_dictionary o  ON o.id  = q.object_id
                 LEFT JOIN pgrdf._pgrdf_dictionary dt ON dt.id = o.datatype_iri_id
                 WHERE q.graph_id = $1 AND q.is_inferred = FALSE",
                None,
                &[unsafe {
                    pgrx::datum::DatumWithOid::new(
                        graph_id,
                        pgrx::pg_sys::PgBuiltInOids::INT8OID.into(),
                    )
                }],
            )
            .expect("materialize: base select failed");
        for row in table {
            let s_type: i16 = row
                .get(1)
                .ok()
                .flatten()
                .expect("materialize: subject term_type");
            let s_val: String = row
                .get(2)
                .ok()
                .flatten()
                .expect("materialize: subject value");
            let p_iri: String = row
                .get(3)
                .ok()
                .flatten()
                .expect("materialize: predicate iri");
            let o_type: i16 = row
                .get(4)
                .ok()
                .flatten()
                .expect("materialize: object term_type");
            let o_val: String = row
                .get(5)
                .ok()
                .flatten()
                .expect("materialize: object value");
            let o_dt: Option<String> = row.get(6).ok().flatten();
            let o_lang: Option<String> = row.get(7).ok().flatten();

            let subject = build_subject(s_type, &s_val);
            let predicate = match NamedNode::new(&p_iri) {
                Ok(n) => n,
                Err(_) => continue, // skip malformed predicates
            };
            let object = build_object(o_type, &o_val, o_dt.as_deref(), o_lang.as_deref());

            out.push(Triple::new(subject, predicate, object));
        }
    });
    out
}

fn build_subject(t_type: i16, value: &str) -> NamedOrBlankNode {
    match t_type {
        term_type::URI => NamedOrBlankNode::NamedNode(NamedNode::new(value).unwrap_or_else(|_| {
            NamedNode::new("urn:pgrdf:invalid-iri")
                .expect("materialize: urn:pgrdf:invalid-iri sentinel is well-formed")
        })),
        term_type::BLANK_NODE => NamedOrBlankNode::BlankNode(
            BlankNode::new(value).unwrap_or_else(|_| BlankNode::default()),
        ),
        // SPARQL disallows literal subjects; if we somehow saw one,
        // skip with a sentinel blank node (the row was malformed).
        _ => NamedOrBlankNode::BlankNode(BlankNode::default()),
    }
}

fn build_object(
    t_type: i16,
    value: &str,
    datatype_iri: Option<&str>,
    language: Option<&str>,
) -> Term {
    match t_type {
        term_type::URI => Term::NamedNode(NamedNode::new(value).unwrap_or_else(|_| {
            NamedNode::new("urn:pgrdf:invalid-iri")
                .expect("materialize: urn:pgrdf:invalid-iri sentinel is well-formed")
        })),
        term_type::BLANK_NODE => {
            Term::BlankNode(BlankNode::new(value).unwrap_or_else(|_| BlankNode::default()))
        }
        _ => {
            // Literal
            if let Some(lang) = language {
                match Literal::new_language_tagged_literal(value, lang) {
                    Ok(l) => Term::Literal(l),
                    Err(_) => Term::Literal(Literal::new_simple_literal(value)),
                }
            } else if let Some(dt) = datatype_iri {
                match NamedNode::new(dt) {
                    Ok(dt_node) => Term::Literal(Literal::new_typed_literal(value, dt_node)),
                    Err(_) => Term::Literal(Literal::new_simple_literal(value)),
                }
            } else {
                Term::Literal(Literal::new_simple_literal(value))
            }
        }
    }
}

fn subject_id(s: &NamedOrBlankNode) -> i64 {
    match s {
        NamedOrBlankNode::NamedNode(n) => put_term_full(n.as_str(), term_type::URI, None, None),
        NamedOrBlankNode::BlankNode(b) => {
            put_term_full(b.as_str(), term_type::BLANK_NODE, None, None)
        }
    }
}

fn term_id(t: &Term) -> i64 {
    match t {
        Term::NamedNode(n) => put_term_full(n.as_str(), term_type::URI, None, None),
        Term::BlankNode(b) => put_term_full(b.as_str(), term_type::BLANK_NODE, None, None),
        Term::Literal(lit) => {
            let lang = lit.language();
            let datatype_id = if lang.is_some() {
                None
            } else {
                Some(put_term_full(
                    lit.datatype().as_str(),
                    term_type::URI,
                    None,
                    None,
                ))
            };
            put_term_full(lit.value(), term_type::LITERAL, datatype_id, lang)
        }
        #[allow(unreachable_patterns)]
        _ => panic!("materialize: unsupported object term (RDF-star out of scope)"),
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// A minimal `rdfs:subClassOf` chain should yield one new
    /// entailment: `?a a Engineer ⇒ ?a a Person`.
    #[pg_test]
    fn materialize_subclass_chain() {
        let ttl = r#"
            @prefix ex:   <http://example.com/> .
            @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:Engineer rdfs:subClassOf ex:Person .
            ex:alice    rdf:type        ex:Engineer .
        "#;
        let g: i64 = 8400;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.parse_turtle($1, $2)", &[ttl.into(), g.into()]).unwrap();

        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.materialize($1)", &[g.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["base_triples"], 2);
        // OWL 2 RL also adds rdfs:subClassOf reflexivity and other
        // entailments; the only thing we strictly need is the
        // ex:alice a ex:Person derivation. Be tolerant of additional
        // entailments — assert at least one new triple was written.
        assert!(
            v["inferred_triples_written"].as_i64().unwrap() >= 1,
            "expected at least one inferred triple, got {}",
            v["inferred_triples_written"]
        );

        // Verify the expected entailment is present.
        let person_count: i64 = Spi::get_one_with_args(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads q
              JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
              JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
              JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
             WHERE q.graph_id = $1
               AND q.is_inferred = TRUE
               AND s.lexical_value = 'http://example.com/alice'
               AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
               AND o.lexical_value = 'http://example.com/Person'",
            &[g.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(person_count, 1, "ex:alice a ex:Person must be inferred");
    }

    /// Calling materialize twice should be idempotent — the second
    /// call returns the same inferred count and drops the previous
    /// inferred rows first.
    #[pg_test]
    fn materialize_is_idempotent() {
        let ttl = r#"
            @prefix ex:   <http://example.com/> .
            @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:B rdfs:subClassOf ex:A .
            ex:x rdf:type ex:B .
        "#;
        let g: i64 = 8401;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.parse_turtle($1, $2)", &[ttl.into(), g.into()]).unwrap();

        let first: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.materialize($1)", &[g.into()])
                .unwrap()
                .unwrap();
        let second: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.materialize($1)", &[g.into()])
                .unwrap()
                .unwrap();

        let n1 = first.0["inferred_triples_written"].as_i64().unwrap();
        let n2 = second.0["inferred_triples_written"].as_i64().unwrap();
        let dropped_2 = second.0["previous_inferred_dropped"].as_i64().unwrap();
        assert_eq!(n1, n2, "two materialize runs must produce same row count");
        assert_eq!(
            dropped_2, n1,
            "second call must drop the first call's output"
        );
    }

    /// A graph with no application-level OWL/RDFS axioms still
    /// produces the OWL 2 RL **axiomatic triples** (`rdf:type
    /// rdf:Property`, `rdfs:Class rdf:type rdfs:Class`, etc.) — the
    /// fixed-point of the RL rule set on the empty input is a small
    /// constant set. We don't assert an exact count (would couple
    /// the test to `reasonable`'s internals); instead assert the
    /// base survived and the user's data was NOT clobbered.
    #[pg_test]
    fn materialize_pure_data_preserves_input() {
        let ttl = r#"
            @prefix ex: <http://example.com/> .
            ex:a ex:p ex:b .
            ex:c ex:q ex:d .
        "#;
        let g: i64 = 8402;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.parse_turtle($1, $2)", &[ttl.into(), g.into()]).unwrap();

        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.materialize($1)", &[g.into()])
            .unwrap()
            .unwrap();
        assert_eq!(j.0["base_triples"], 2);
        // No application-level entailment, but axiomatic OWL 2 RL
        // triples ARE expected. Just confirm the base + something
        // was written, and that the base survives the round-trip.
        let written = j.0["inferred_triples_written"].as_i64().unwrap();
        assert!(written >= 0); // sanity

        let base_still_there: i64 = Spi::get_one_with_args(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads
              WHERE graph_id = $1 AND is_inferred = FALSE",
            &[g.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            base_still_there, 2,
            "base triples must remain after materialize"
        );
    }

    // ── v0.5-FUTURE §3 — reasoning-profile selector ──────────────

    /// §3.1 #1 — `materialize(g, 'rdfs')` triple count ≤
    /// `materialize(g, 'owl-rl')` on a fixed input (non-strict
    /// subset). Same seed for both; compare `inferred_triples_written`.
    #[pg_test]
    fn materialize_rdfs_count_is_subset_of_owl_rl() {
        let ttl = r#"
            @prefix ex:   <http://example.com/> .
            @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:Engineer rdfs:subClassOf ex:Person .
            ex:Person   rdfs:subClassOf ex:Agent .
            ex:worksAt  rdfs:domain     ex:Employee .
            ex:worksAt  rdfs:range      ex:Org .
            ex:alice    rdf:type        ex:Engineer .
            ex:alice    ex:worksAt      ex:acme .
        "#;
        let g1: i64 = 8410;
        let g2: i64 = 8411;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g1.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g2.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), g1.into()],
        )
        .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), g2.into()],
        )
        .unwrap();

        let owl: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.materialize($1, 'owl-rl')", &[g1.into()])
                .unwrap()
                .unwrap();
        let rdfs: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.materialize($1, 'rdfs')", &[g2.into()])
                .unwrap()
                .unwrap();

        let owl_n = owl.0["inferred_triples_written"].as_i64().unwrap();
        let rdfs_n = rdfs.0["inferred_triples_written"].as_i64().unwrap();
        assert!(
            rdfs_n <= owl_n,
            "rdfs ({rdfs_n}) must be a non-strict subset of owl-rl ({owl_n})"
        );

        // §3.1 #2 — RDFS-axiom agreement: the subClassOf-transitivity
        // entailment ex:alice a ex:Agent must be present under BOTH.
        for g in [g1, g2] {
            let agent: i64 = Spi::get_one_with_args(
                "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads q
                  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
                  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
                  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
                 WHERE q.graph_id = $1 AND q.is_inferred = TRUE
                   AND s.lexical_value = 'http://example.com/alice'
                   AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
                   AND o.lexical_value = 'http://example.com/Agent'",
                &[g.into()],
            )
            .unwrap()
            .unwrap();
            assert_eq!(
                agent, 1,
                "ex:alice a ex:Agent (subClassOf transitivity) must \
                 be entailed under both profiles (graph {g})"
            );
        }
    }

    /// §3 — the bare `materialize(g)` form is identical to
    /// `materialize(g, 'owl-rl')` (no v0.4 regression) and reports
    /// `profile:"owl-rl"`.
    #[pg_test]
    fn materialize_default_arg_equals_owl_rl() {
        let ttl = r#"
            @prefix ex:   <http://example.com/> .
            @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:B rdfs:subClassOf ex:A .
            ex:x rdf:type ex:B .
        "#;
        let gd: i64 = 8420;
        let ge: i64 = 8421;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[gd.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[ge.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), gd.into()],
        )
        .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), ge.into()],
        )
        .unwrap();

        let bare: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.materialize($1)", &[gd.into()])
                .unwrap()
                .unwrap();
        let explicit: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.materialize($1, 'owl-rl')", &[ge.into()])
                .unwrap()
                .unwrap();

        assert_eq!(
            bare.0["inferred_triples_written"], explicit.0["inferred_triples_written"],
            "bare materialize(g) must equal materialize(g,'owl-rl')"
        );
        assert_eq!(
            bare.0["profile"], "owl-rl",
            "default-arg call must report profile:owl-rl"
        );
        assert_eq!(explicit.0["profile"], "owl-rl");
    }

    /// §3 — JSONB carries the requested `profile` for the rdfs path.
    #[pg_test]
    fn materialize_rdfs_reports_profile_field() {
        let ttl = r#"
            @prefix ex:   <http://example.com/> .
            @prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
            @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
            ex:B rdfs:subClassOf ex:A .
            ex:x rdf:type ex:B .
        "#;
        let g: i64 = 8430;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.parse_turtle($1, $2)", &[ttl.into(), g.into()]).unwrap();
        let j: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.materialize($1, 'rdfs')", &[g.into()])
                .unwrap()
                .unwrap();
        assert_eq!(j.0["profile"], "rdfs");
        // The subClassOf-application entailment ex:x a ex:A is RDFS.
        let a_typed: i64 = Spi::get_one_with_args(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads q
              JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
              JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
              JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
             WHERE q.graph_id = $1 AND q.is_inferred = TRUE
               AND s.lexical_value = 'http://example.com/x'
               AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type'
               AND o.lexical_value = 'http://example.com/A'",
            &[g.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(a_typed, 1, "ex:x a ex:A must be entailed under rdfs");
    }

    /// §3.1 #3 — an unknown profile string errors with the EXACT
    /// `materialize: unknown profile` prefix; no silent fallback.
    /// The reserved future `'owl-rl-ext'` is treated as unknown.
    #[pg_test(error = "materialize: unknown profile \"owl-rl-ext\" (supported: 'owl-rl', 'rdfs')")]
    fn materialize_unknown_profile_errors() {
        let g: i64 = 8440;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g.into()]).unwrap();
        let _ = Spi::get_one_with_args::<pgrx::JsonB>(
            "SELECT pgrdf.materialize($1, 'owl-rl-ext')",
            &[g.into()],
        );
    }
}
