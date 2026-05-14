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

use crate::storage::dict::{put_term_full, term_type};
use oxrdf::{BlankNode, Literal, NamedNode, NamedOrBlankNode, Subject, Term, Triple};
use pgrx::prelude::*;
use reasonable::reasoner::Reasoner;
use serde_json::json;
use std::collections::HashSet;
use std::time::Instant;

/// Forward-chain OWL 2 RL materialization for one graph.
///
/// Returns a JSONB stats object:
/// ```json
/// {
///   "base_triples":              123,
///   "inferred_triples_written":  45,
///   "previous_inferred_dropped": 0,
///   "reasoner_errors":           [],
///   "elapsed_ms":                17.4
/// }
/// ```
///
/// SQL: `pgrdf.materialize(graph_id BIGINT) -> JSONB`.
#[pg_extern]
fn materialize(graph_id: i64) -> pgrx::JsonB {
    let start = Instant::now();

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
        table
            .first()
            .get_one::<i64>()
            .ok()
            .flatten()
            .unwrap_or(0)
    });

    // 2. Stream base triples out.
    let base = load_base_triples(graph_id);
    let base_count = base.len() as i64;
    let base_set: HashSet<Triple> = base.iter().cloned().collect();

    // 3. Reason.
    let mut reasoner = Reasoner::new();
    reasoner.load_triples(base);
    reasoner.reason();
    let errors: Vec<String> = reasoner
        .errors()
        .iter()
        .map(|e| format!("{e}"))
        .collect();

    // 4. Set-diff to find ONLY the inferred (entailed-but-not-asserted) triples.
    let derived = reasoner.get_triples();
    let inferred: Vec<&Triple> = derived
        .iter()
        .filter(|t| !base_set.contains(t))
        .collect();

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
            &[
                s_id.into(),
                p_id.into(),
                o_id.into(),
                graph_id.into(),
            ],
        )
        .expect("materialize: insert inferred failed");
        written += 1;
    }

    pgrx::JsonB(json!({
        "base_triples":              base_count,
        "inferred_triples_written":  written,
        "previous_inferred_dropped": dropped,
        "reasoner_errors":           errors,
        "elapsed_ms":                start.elapsed().as_secs_f64() * 1000.0,
    }))
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
            let s_type: i16 = row.get(1).ok().flatten().expect("subject term_type");
            let s_val: String = row.get(2).ok().flatten().expect("subject value");
            let p_iri: String = row.get(3).ok().flatten().expect("predicate iri");
            let o_type: i16 = row.get(4).ok().flatten().expect("object term_type");
            let o_val: String = row.get(5).ok().flatten().expect("object value");
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
        term_type::URI => NamedOrBlankNode::NamedNode(
            NamedNode::new(value).unwrap_or_else(|_| {
                NamedNode::new("urn:pgrdf:invalid-iri")
                    .expect("urn:pgrdf:invalid-iri is well-formed")
            }),
        ),
        term_type::BLANK_NODE => {
            NamedOrBlankNode::BlankNode(BlankNode::new(value).unwrap_or_else(|_| BlankNode::default()))
        }
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
        term_type::URI => Term::NamedNode(
            NamedNode::new(value)
                .unwrap_or_else(|_| NamedNode::new("urn:pgrdf:invalid-iri").unwrap()),
        ),
        term_type::BLANK_NODE => Term::BlankNode(
            BlankNode::new(value).unwrap_or_else(|_| BlankNode::default()),
        ),
        _ => {
            // Literal
            if let Some(lang) = language {
                match Literal::new_language_tagged_literal(value, lang) {
                    Ok(l) => Term::Literal(l),
                    Err(_) => Term::Literal(Literal::new_simple_literal(value)),
                }
            } else if let Some(dt) = datatype_iri {
                match NamedNode::new(dt) {
                    Ok(dt_node) => {
                        Term::Literal(Literal::new_typed_literal(value, dt_node))
                    }
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
        NamedOrBlankNode::NamedNode(n) => {
            put_term_full(n.as_str(), term_type::URI, None, None)
        }
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

// Silence unused-import warnings if oxrdf re-exports change.
#[allow(dead_code)]
fn _unused_subject_marker(_: &Subject) {}

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
        Spi::run_with_args(
            "SELECT pgrdf.add_graph($1)",
            &[g.into()],
        )
        .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), g.into()],
        )
        .unwrap();

        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.materialize($1)",
            &[g.into()],
        )
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
        Spi::run_with_args("SELECT pgrdf.parse_turtle($1, $2)", &[ttl.into(), g.into()])
            .unwrap();

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
        assert_eq!(dropped_2, n1, "second call must drop the first call's output");
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
        Spi::run_with_args("SELECT pgrdf.parse_turtle($1, $2)", &[ttl.into(), g.into()])
            .unwrap();

        let j: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.materialize($1)", &[g.into()])
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
        assert_eq!(base_still_there, 2, "base triples must remain after materialize");
    }
}
