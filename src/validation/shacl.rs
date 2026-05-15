//! SHACL processor wrapper.
//!
//! v0.4 cycle (this body) ships the **real implementation** of
//! `pgrdf.validate(data_graph_id, shapes_graph_id) → JSONB`. The
//! preceding stub (v0.3) is gone.
//!
//! Pipeline:
//!
//! ```text
//!   (data_graph_id)              (shapes_graph_id)
//!         │                            │
//!         ▼                            ▼
//!   rehydrate from _pgrdf_quads + _pgrdf_dictionary
//!         │                            │
//!         ▼                            ▼
//!   serialise to N-Triples text        │
//!         │                            │
//!         ▼                            ▼
//!   InMemoryGraph::from_str            InMemoryGraph::from_str
//!         │                            │
//!         ▼                            ▼
//!   Graph::try_from → GraphValidation  ShaclDataManager::load → IRSchema
//!         │                            │
//!         └───────────┬────────────────┘
//!                     ▼
//!         validator.validate(&schema, &Native) → ValidationReport
//!                     │
//!                     ▼
//!         W3C sh:ValidationReport-shaped JSONB
//! ```
//!
//! Unblocked by:
//! 1. `rudof 0.3.1` (2026-05-12) consolidating `shacl_ast` and
//!    `shacl_validation` into a single `shacl 0.3.x` crate, closing
//!    the `iri_s` → `rudof_iri` half of ERRATA.v0.2 E-009.
//! 2. The `styk-tv/reasonable` fork branch `rdf12-passthrough`
//!    adding a `TermRef::Triple(_)` arm gated behind a new
//!    `rdf-12` passthrough feature, closing the `rdf-12` half of
//!    E-009 (now tracked as ERRATA.v0.4 E-011).
//!
//! Drop the `[patch.crates-io]` block in `Cargo.toml` (and the
//! `features = ["rdf-12"]` on the `reasonable` dep) once
//! `gtfierro/reasonable` merges the upstream PR.

use crate::storage::dict::term_type;
use oxrdf::{BlankNodeRef, LiteralRef, NamedNodeRef, NamedOrBlankNodeRef, TermRef, TripleRef};
use oxttl::NTriplesSerializer;
use pgrx::prelude::*;
use rudof_rdf::rdf_core::term::literal::ConcreteLiteral;
use rudof_rdf::rdf_core::term::Object;
use rudof_rdf::rdf_core::RDFFormat;
use rudof_rdf::rdf_core::SHACLPath;
use rudof_rdf::rdf_impl::{InMemoryGraph, ReaderMode};
use serde_json::{json, Value};
use shacl::types::Severity;
use shacl::validator::processor::{GraphValidation, ShaclProcessor};
use shacl::validator::report::ValidationResult;
use shacl::validator::store::{Graph, ShaclDataManager};
use shacl::validator::ShaclValidationMode;
use std::io::Cursor;
use std::time::Instant;

/// SHACL Core validator.
///
/// SQL: `pgrdf.validate(data_graph_id BIGINT, shapes_graph_id BIGINT) → JSONB`.
///
/// Returns a JSONB payload shaped to mirror the W3C
/// `sh:ValidationReport` structure:
///
/// ```json
/// {
///   "conforms":        <bool>,
///   "results":         [ ValidationResult, ... ],
///   "data_graph_id":   <i64>,
///   "shapes_graph_id": <i64>,
///   "data_triples":    <i64>,
///   "shapes_triples":  <i64>,
///   "elapsed_ms":      <f64>
/// }
/// ```
///
/// Each entry in `results` is shaped:
///
/// ```json
/// {
///   "focusNode":      "<iri-or-bnode-or-literal-encoded>",
///   "resultPath":     "<iri-or-null>",
///   "sourceShape":    "<iri-or-bnode-or-null>",
///   "resultMessage":  "<string-or-null>",
///   "resultSeverity": "sh:Violation|sh:Warning|sh:Info|...",
///   "value":          "<term-encoded-or-null>",
///   "sourceConstraintComponent": "<iri>"
/// }
/// ```
///
/// Validation runs the SHACL Core engine in the rudof `shacl 0.3.x`
/// Native mode. The graphs are rehydrated from `_pgrdf_quads` ↔
/// `_pgrdf_dictionary` (same shape as `pgrdf.materialize`), serialised
/// to N-Triples in-memory, and re-parsed into rudof's `InMemoryGraph`
/// before validation. Validation is in-process; no SPARQL endpoint
/// or external store is contacted.
#[pg_extern]
fn validate(data_graph_id: i64, shapes_graph_id: i64) -> pgrx::JsonB {
    let start = Instant::now();

    // 1. Rehydrate data + shapes graphs as N-Triples text.
    let (data_nt, data_count) = serialise_graph_to_ntriples(data_graph_id);
    let (shapes_nt, shapes_count) = serialise_graph_to_ntriples(shapes_graph_id);

    // 2. Build rudof's in-memory graphs from the N-Triples text.
    let data_im =
        match InMemoryGraph::from_str(&data_nt, &RDFFormat::NTriples, None, &ReaderMode::default())
        {
            Ok(g) => g,
            Err(e) => {
                return pgrx::JsonB(json!({
                    "conforms":        Value::Null,
                    "results":         [],
                    "data_graph_id":   data_graph_id,
                    "shapes_graph_id": shapes_graph_id,
                    "data_triples":    data_count,
                    "shapes_triples":  shapes_count,
                    "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
                    "error":           format!("data graph parse failed: {e}"),
                }));
            }
        };

    let data_graph = match Graph::try_from(data_im) {
        Ok(g) => g,
        Err(e) => {
            return pgrx::JsonB(json!({
                "conforms":        Value::Null,
                "results":         [],
                "data_graph_id":   data_graph_id,
                "shapes_graph_id": shapes_graph_id,
                "data_triples":    data_count,
                "shapes_triples":  shapes_count,
                "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
                "error":           format!("data graph build failed: {e}"),
            }));
        }
    };

    // 3. Compile the shapes graph to a SHACL `IRSchema`.
    let schema = match ShaclDataManager::load(
        &mut Cursor::new(shapes_nt.as_bytes()),
        "pgrdf-shapes",
        &RDFFormat::NTriples,
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            return pgrx::JsonB(json!({
                "conforms":        Value::Null,
                "results":         [],
                "data_graph_id":   data_graph_id,
                "shapes_graph_id": shapes_graph_id,
                "data_triples":    data_count,
                "shapes_triples":  shapes_count,
                "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
                "error":           format!("shapes compile failed: {e}"),
            }));
        }
    };

    // 4. Run validation. Native mode is the in-process engine.
    let mut validator = GraphValidation::new(data_graph);
    let report = match validator.validate(&schema, &ShaclValidationMode::Native) {
        Ok(r) => r,
        Err(e) => {
            return pgrx::JsonB(json!({
                "conforms":        Value::Null,
                "results":         [],
                "data_graph_id":   data_graph_id,
                "shapes_graph_id": shapes_graph_id,
                "data_triples":    data_count,
                "shapes_triples":  shapes_count,
                "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
                "error":           format!("validation failed: {e}"),
            }));
        }
    };

    // 5. Shape the report into JSONB.
    let results_json: Vec<Value> = report.results().iter().map(report_result_to_json).collect();
    pgrx::JsonB(json!({
        "conforms":        report.conforms(),
        "results":         results_json,
        "data_graph_id":   data_graph_id,
        "shapes_graph_id": shapes_graph_id,
        "data_triples":    data_count,
        "shapes_triples":  shapes_count,
        "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
    }))
}

/// Rehydrate one graph from `_pgrdf_quads` JOIN `_pgrdf_dictionary`
/// and serialise it to N-Triples text in memory.
///
/// Mirrors `inference::reasonable::load_base_triples` shape — single
/// SPI scan, all base + inferred rows in the graph included. (Shapes
/// graphs and SHACL Core data graphs are usually pure base; we still
/// take inferred rows in case a caller has run `pgrdf.materialize`
/// first and wants to validate the materialised closure.)
fn serialise_graph_to_ntriples(graph_id: i64) -> (String, i64) {
    let mut count: i64 = 0;
    let mut serializer = NTriplesSerializer::new().for_writer(Vec::<u8>::new());

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
                 WHERE q.graph_id = $1",
                None,
                &[unsafe {
                    pgrx::datum::DatumWithOid::new(
                        graph_id,
                        pgrx::pg_sys::PgBuiltInOids::INT8OID.into(),
                    )
                }],
            )
            .expect("validate: graph rehydrate select failed");
        for row in table {
            let s_type: i16 = row.get(1).ok().flatten().expect("validate: s.term_type");
            let s_val: String = row.get(2).ok().flatten().expect("validate: s.value");
            let p_iri: String = row.get(3).ok().flatten().expect("validate: p.iri");
            let o_type: i16 = row.get(4).ok().flatten().expect("validate: o.term_type");
            let o_val: String = row.get(5).ok().flatten().expect("validate: o.value");
            let o_dt: Option<String> = row.get(6).ok().flatten();
            let o_lang: Option<String> = row.get(7).ok().flatten();

            // Build oxrdf borrow-shaped references and hand them to
            // the N-Triples serialiser. Bad IRIs / blank-node labels
            // are skipped (same defensive shape as
            // `load_base_triples`); they wouldn't have round-tripped
            // through the dict anyway.
            let subject: NamedOrBlankNodeRef<'_> = match s_type {
                term_type::URI => match NamedNodeRef::new(&s_val) {
                    Ok(n) => NamedOrBlankNodeRef::NamedNode(n),
                    Err(_) => continue,
                },
                term_type::BLANK_NODE => match BlankNodeRef::new(&s_val) {
                    Ok(b) => NamedOrBlankNodeRef::BlankNode(b),
                    Err(_) => continue,
                },
                _ => continue, // literal subject — skip; malformed
            };
            let predicate: NamedNodeRef<'_> = match NamedNodeRef::new(&p_iri) {
                Ok(n) => n,
                Err(_) => continue,
            };
            let object: TermRef<'_> = match o_type {
                term_type::URI => match NamedNodeRef::new(&o_val) {
                    Ok(n) => TermRef::NamedNode(n),
                    Err(_) => continue,
                },
                term_type::BLANK_NODE => match BlankNodeRef::new(&o_val) {
                    Ok(b) => TermRef::BlankNode(b),
                    Err(_) => continue,
                },
                _ => {
                    // Literal: language-tagged, datatyped, or simple.
                    // Lang tags survived dictionary ingest (parse_turtle
                    // would have rejected malformed ones), so the
                    // unchecked constructor is safe here.
                    if let Some(ref lang) = o_lang {
                        TermRef::Literal(LiteralRef::new_language_tagged_literal_unchecked(
                            &o_val, lang,
                        ))
                    } else if let Some(ref dt) = o_dt {
                        match NamedNodeRef::new(dt) {
                            Ok(dt_node) => {
                                TermRef::Literal(LiteralRef::new_typed_literal(&o_val, dt_node))
                            }
                            Err(_) => TermRef::Literal(LiteralRef::new_simple_literal(&o_val)),
                        }
                    } else {
                        TermRef::Literal(LiteralRef::new_simple_literal(&o_val))
                    }
                }
            };

            let triple = TripleRef::new(subject, predicate, object);
            if serializer.serialize_triple(triple).is_ok() {
                count += 1;
            }
        }
    });

    let bytes = serializer.finish();
    let text = String::from_utf8(bytes).unwrap_or_default();
    (text, count)
}

/// Map one rudof `ValidationResult` into the JSONB shape the W3C
/// `sh:ValidationReport` describes. Optional fields render as
/// `null`; severity normalises to the canonical `sh:` constants.
fn report_result_to_json(r: &ValidationResult) -> Value {
    let focus_node = encode_object(r.focus_node());
    let result_path = r.path().map(encode_path).unwrap_or(Value::Null);
    let source_shape = r.source().map(encode_object).unwrap_or(Value::Null);
    let value = r.value().map(encode_object).unwrap_or(Value::Null);
    let constraint_component = encode_object(r.constraint_component());

    // Take the first message (any language). The MessageMap may be
    // empty if the engine didn't synthesise a message.
    let message = r
        .message()
        .iter()
        .next()
        .map(|(_lang, msg)| Value::String(msg.clone()))
        .unwrap_or(Value::Null);

    json!({
        "focusNode":      focus_node,
        "resultPath":     result_path,
        "sourceShape":    source_shape,
        "resultMessage":  message,
        "resultSeverity": encode_severity(r.severity()),
        "value":          value,
        "sourceConstraintComponent": constraint_component,
    })
}

/// rudof's `Object` enum → JSON-friendly string.
///
/// IRIs and blank nodes flatten to plain strings (the IRI text, or
/// `_:label` for blanks). Literals render in Turtle-ish form:
/// `"value"`, `"value"@lang`, or `"value"^^<datatype>`.
fn encode_object(obj: &Object) -> Value {
    match obj {
        Object::Iri(iri) => Value::String(iri.as_str().to_string()),
        Object::BlankNode(label) => Value::String(format!("_:{label}")),
        Object::Literal(lit) => Value::String(format_literal(lit)),
        Object::Triple { .. } => {
            // RDF-star nesting — out of scope for SHACL Core. Render
            // a stable placeholder so the JSONB stays well-formed.
            Value::String("<rdf-star-triple>".to_string())
        }
    }
}

fn format_literal(lit: &ConcreteLiteral) -> String {
    match lit {
        ConcreteLiteral::StringLiteral { lexical_form, lang } => match lang {
            Some(l) => format!("\"{lexical_form}\"@{l}"),
            None => format!("\"{lexical_form}\""),
        },
        ConcreteLiteral::DatatypeLiteral {
            lexical_form,
            datatype,
        } => format!("\"{lexical_form}\"^^<{}>", datatype),
        ConcreteLiteral::NumericLiteral(n) => format!("{n}"),
        ConcreteLiteral::DatetimeLiteral(dt) => format!("{}", dt.value()),
        ConcreteLiteral::BooleanLiteral(b) => format!("{b}"),
        ConcreteLiteral::WrongDatatypeLiteral {
            lexical_form,
            datatype,
            ..
        } => format!("\"{lexical_form}\"^^<{}>", datatype),
    }
}

/// SHACL paths flatten to a string. Simple predicate paths render
/// as the IRI; complex paths use SHACLPath's `Display` impl.
fn encode_path(path: &SHACLPath) -> Value {
    match path {
        SHACLPath::Predicate { pred } => Value::String(pred.as_str().to_string()),
        other => Value::String(format!("{other}")),
    }
}

/// Canonical `sh:` constants for severity (see SHACL spec §1.5).
fn encode_severity(sev: &Severity) -> Value {
    let s = match sev {
        Severity::Trace => "sh:Trace",
        Severity::Debug => "sh:Debug",
        Severity::Info => "sh:Info",
        Severity::Warning => "sh:Warning",
        Severity::Violation => "sh:Violation",
        Severity::Generic(iri) => return Value::String(iri.as_str().to_string()),
    };
    Value::String(s.to_string())
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// Conforming data graph against a `sh:NodeShape` with
    /// `sh:property` + `sh:datatype` constraints. The report MUST
    /// claim `conforms: true` and carry zero results.
    #[pg_test]
    fn validate_conforming() {
        let g_data: i64 = 8500;
        let g_shapes: i64 = 8501;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
                 ex:bob a foaf:Person ;
                        foaf:name \"Bob\" ;
                        ex:age \"30\"^^xsd:integer ."
                    .into(),
                g_data.into(),
            ],
        )
        .unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 @prefix sh: <http://www.w3.org/ns/shacl#> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
                 ex:PersonShape a sh:NodeShape ;
                     sh:targetClass foaf:Person ;
                     sh:property [
                         sh:path foaf:name ;
                         sh:minCount 1 ;
                         sh:datatype xsd:string ;
                     ] ;
                     sh:property [
                         sh:path ex:age ;
                         sh:minCount 1 ;
                         sh:datatype xsd:integer ;
                     ] ."
                .into(),
                g_shapes.into(),
            ],
        )
        .unwrap();

        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2)",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["conforms"], serde_json::json!(true));
        assert_eq!(v["data_graph_id"], g_data);
        assert_eq!(v["shapes_graph_id"], g_shapes);
        assert!(v["results"].is_array());
        assert_eq!(v["results"].as_array().unwrap().len(), 0);
    }

    /// Non-conforming data graph — Alice lacks the required
    /// `ex:age`. Report MUST claim `conforms: false` with at least
    /// one violation result whose focusNode is Alice's IRI.
    #[pg_test]
    fn validate_violations() {
        let g_data: i64 = 8510;
        let g_shapes: i64 = 8511;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice a foaf:Person ;
                          foaf:name \"Alice\" ."
                    .into(),
                g_data.into(),
            ],
        )
        .unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 @prefix sh: <http://www.w3.org/ns/shacl#> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
                 ex:PersonShape a sh:NodeShape ;
                     sh:targetClass foaf:Person ;
                     sh:property [
                         sh:path ex:age ;
                         sh:minCount 1 ;
                         sh:datatype xsd:integer ;
                     ] ."
                .into(),
                g_shapes.into(),
            ],
        )
        .unwrap();

        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2)",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["conforms"], serde_json::json!(false));
        let results = v["results"].as_array().expect("results must be array");
        assert!(
            !results.is_empty(),
            "expected at least one violation for Alice"
        );
        let any_alice = results
            .iter()
            .any(|r| r["focusNode"] == "http://example.org/alice");
        assert!(any_alice, "no violation surfaced for ex:alice");
    }

    /// Unknown graphs render zero triple counts and a degenerate
    /// "vacuously conforming" report (no targets ⇒ no failures).
    #[pg_test]
    fn validate_unknown_graphs() {
        let j: pgrx::JsonB = Spi::get_one("SELECT pgrdf.validate(999990::bigint, 999991::bigint)")
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["data_triples"], 0);
        assert_eq!(v["shapes_triples"], 0);
        // No shapes ⇒ no failures ⇒ conforms.
        assert_eq!(v["conforms"], serde_json::json!(true));
    }
}
