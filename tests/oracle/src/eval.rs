//! spareval evaluation → canonical JSONL rows.
//!
//! Row shapes mirror the engine output that tests/w3c-sparql/run.sh
//! collects from pgRDF:
//!   SELECT — flat objects, every projected variable present per row;
//!            unbound = JSON null (fixture 42 convention). IRIs render
//!            raw, literals render their lexical form, blank nodes
//!            render as "_:label" (the oracle-side marker compare.rs
//!            resolves up to isomorphism).
//!   ASK    — one row {"_ask": "true"|"false"}.
//!   CONSTRUCT / DESCRIBE — structured-term triple rows
//!            {"subject": {...}, "predicate": {...}, "object": {...}}
//!            with "type"/"value"/"datatype"/"language" fields exactly
//!            as the engine emits them (datatype always present on
//!            literals, xsd:string included).

use oxrdf::{Dataset, GraphName, Quad, Term};
use serde_json::{json, Map, Value};
use spareval::{QueryEvaluator, QueryResults};
use spargebra::SparqlParser;

/// Evaluate `query_str` over `data_ttl` (Turtle, default graph) and
/// return canonical rows.
pub fn eval(data_ttl: &str, query_str: &str) -> Result<Vec<Value>, String> {
    let mut dataset = Dataset::new();
    for triple in oxttl::TurtleParser::new().for_slice(data_ttl.as_bytes()) {
        let t = triple.map_err(|e| format!("data parse: {e}"))?;
        dataset.insert(&Quad::new(
            t.subject,
            t.predicate,
            t.object,
            GraphName::DefaultGraph,
        ));
    }
    let query = SparqlParser::new()
        .parse_query(query_str)
        .map_err(|e| format!("query parse: {e}"))?;
    let evaluator = QueryEvaluator::new();
    let results = evaluator
        .prepare(&query)
        .execute(&dataset)
        .map_err(|e| format!("eval: {e}"))?;
    match results {
        QueryResults::Boolean(b) => Ok(vec![json!({"_ask": b.to_string()})]),
        QueryResults::Solutions(iter) => {
            let variables: Vec<_> = iter.variables().to_vec();
            let mut rows = Vec::new();
            for solution in iter {
                let solution = solution.map_err(|e| format!("solution: {e}"))?;
                let mut row = Map::new();
                for var in &variables {
                    let value = match solution.get(var) {
                        Some(term) => Value::String(render_flat(term)),
                        None => Value::Null,
                    };
                    row.insert(var.as_str().to_string(), value);
                }
                rows.push(Value::Object(row));
            }
            Ok(rows)
        }
        QueryResults::Graph(iter) => {
            let mut rows = Vec::new();
            for triple in iter {
                let t = triple.map_err(|e| format!("triple: {e}"))?;
                rows.push(json!({
                    "subject": structured(&Term::from(t.subject)),
                    "predicate": structured(&Term::from(t.predicate)),
                    "object": structured(&t.object),
                }));
            }
            Ok(rows)
        }
    }
}

/// Flat SELECT rendering: raw IRI, literal lexical form, "_:label"
/// blank-node marker (resolved up to isomorphism by compare.rs).
fn render_flat(term: &Term) -> String {
    match term {
        Term::NamedNode(n) => n.as_str().to_string(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
        Term::Literal(l) => l.value().to_string(),
    }
}

/// Structured-term rendering for CONSTRUCT/DESCRIBE rows, matching
/// the engine's shape: datatype always explicit on literals
/// (xsd:string included), "language" present only on lang strings.
fn structured(term: &Term) -> Value {
    match term {
        Term::NamedNode(n) => json!({"type": "iri", "value": n.as_str()}),
        Term::BlankNode(b) => json!({"type": "bnode", "value": b.as_str()}),
        Term::Literal(l) => {
            let mut obj = Map::new();
            obj.insert("type".into(), json!("literal"));
            obj.insert("value".into(), json!(l.value()));
            obj.insert("datatype".into(), json!(l.datatype().as_str()));
            if let Some(lang) = l.language() {
                obj.insert("language".into(), json!(lang));
            }
            Value::Object(obj)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FOAF_DATA: &str = r#"
        @prefix foaf: <http://xmlns.com/foaf/0.1/> .
        <http://ex.com/alice> foaf:name "Alice" ;
                              foaf:mbox <mailto:alice@work> .
        <http://ex.com/bob>   foaf:name "Bob" .
    "#;

    #[test]
    fn select_rows_flat_shape() {
        let rows = eval(
            FOAF_DATA,
            "PREFIX foaf: <http://xmlns.com/foaf/0.1/> \
             SELECT ?s ?n WHERE { ?s foaf:name ?n }",
        )
        .unwrap();
        let mut got = rows;
        got.sort_by_key(|r| r["n"].as_str().unwrap_or_default().to_string());
        assert_eq!(
            got,
            vec![
                json!({"s": "http://ex.com/alice", "n": "Alice"}),
                json!({"s": "http://ex.com/bob", "n": "Bob"}),
            ]
        );
    }

    #[test]
    fn unbound_projected_var_is_null() {
        let rows = eval(
            FOAF_DATA,
            "PREFIX foaf: <http://xmlns.com/foaf/0.1/> \
             SELECT ?n ?m WHERE { ?s foaf:name ?n \
             OPTIONAL { ?s foaf:mbox ?m } }",
        )
        .unwrap();
        let bob = rows
            .iter()
            .find(|r| r["n"] == json!("Bob"))
            .expect("bob row");
        assert_eq!(bob["m"], Value::Null);
        assert!(bob.as_object().unwrap().contains_key("m"));
    }

    #[test]
    fn ask_shape() {
        let t = eval(FOAF_DATA, "ASK { ?s ?p \"Alice\" }").unwrap();
        assert_eq!(t, vec![json!({"_ask": "true"})]);
        let f = eval(FOAF_DATA, "ASK { ?s ?p \"Nobody\" }").unwrap();
        assert_eq!(f, vec![json!({"_ask": "false"})]);
    }

    #[test]
    fn select_bnode_rendered_with_marker() {
        let rows = eval(
            r#"@prefix foaf: <http://xmlns.com/foaf/0.1/> .
               _:someone foaf:name "Nemo" ."#,
            "PREFIX foaf: <http://xmlns.com/foaf/0.1/> \
             SELECT ?s WHERE { ?s foaf:name \"Nemo\" }",
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0]["s"].as_str().unwrap().starts_with("_:"));
    }

    #[test]
    fn construct_rows_structured() {
        let rows = eval(
            r#"@prefix ex: <http://ex.com/> .
               ex:w ex:label "Le Widget"@fr ;
                    ex:count 42 ."#,
            "PREFIX ex: <http://ex.com/> \
             CONSTRUCT { ?s ex:copied ?o } WHERE { ?s ?p ?o }",
        )
        .unwrap();
        let mut got = rows;
        got.sort_by_key(|r| {
            r["object"]["value"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        });
        assert_eq!(
            got,
            vec![
                json!({
                    "subject": {"type": "iri", "value": "http://ex.com/w"},
                    "predicate": {"type": "iri", "value": "http://ex.com/copied"},
                    "object": {"type": "literal", "value": "42",
                                "datatype": "http://www.w3.org/2001/XMLSchema#integer"},
                }),
                json!({
                    "subject": {"type": "iri", "value": "http://ex.com/w"},
                    "predicate": {"type": "iri", "value": "http://ex.com/copied"},
                    "object": {"type": "literal", "value": "Le Widget",
                                "datatype": "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
                                "language": "fr"},
                }),
            ]
        );
    }

    #[test]
    fn plain_string_literal_gets_explicit_xsd_string_datatype() {
        let rows = eval(
            r#"@prefix ex: <http://ex.com/> . ex:a ex:p "plain" ."#,
            "PREFIX ex: <http://ex.com/> \
             CONSTRUCT { ?s ex:q ?o } WHERE { ?s ex:p ?o }",
        )
        .unwrap();
        assert_eq!(
            rows[0]["object"],
            json!({"type": "literal", "value": "plain",
                    "datatype": "http://www.w3.org/2001/XMLSchema#string"})
        );
    }

    #[test]
    fn parse_error_is_reported_not_panicked() {
        assert!(eval(FOAF_DATA, "SELECT WHERE garbage {").is_err());
        assert!(eval("not turtle at all @@@", "ASK { ?s ?p ?o }").is_err());
    }
}
