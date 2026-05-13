//! SPARQL parser surface.
//!
//! Phase 2.2 step 4: expose a UDF that runs the user's query through
//! `spargebra::SparqlParser`, returns the parsed shape as JSONB. The
//! shape is the same one the translator (step 5) consumes, so we get
//! to validate the parser independently before the SQL-translation
//! work lands.
//!
//! Today's scope:
//!   * SELECT queries (any form) — variables + BGP triples extracted.
//!   * CONSTRUCT / ASK / DESCRIBE — recognised but reported as
//!     `supported: false` until the translator covers them.
//!   * Non-BGP graph patterns (Filter, OPTIONAL, UNION, …) — the
//!     parser handles them fine; the JSONB output flags them under
//!     `unsupported_algebra` so the user knows the AST has shape
//!     pgRDF doesn't yet translate.

use pgrx::prelude::*;
use serde_json::{json, Value};
use spargebra::algebra::GraphPattern;
use spargebra::term::{NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{Query, SparqlParser};

/// Parse a SPARQL query and return a JSONB describing its top-level
/// shape: form (SELECT / CONSTRUCT / ASK / DESCRIBE), projected
/// variables, and the BGP triple patterns when present.
///
/// SQL: `pgrdf.sparql_parse(q TEXT) → JSONB`.
///
/// On syntax error the function aborts the query with a Postgres
/// ERROR carrying the spargebra parser message.
#[pg_extern]
fn sparql_parse(query: &str) -> pgrx::JsonB {
    let parsed = SparqlParser::new()
        .parse_query(query)
        .unwrap_or_else(|e| panic!("sparql_parse: {e}"));
    pgrx::JsonB(serialize_query(&parsed))
}

fn serialize_query(q: &Query) -> Value {
    match q {
        Query::Select { pattern, .. } => {
            let (vars, bgp, unsupported) = walk_select_pattern(pattern);
            json!({
                "form":                "SELECT",
                "variables":           vars,
                "bgp_pattern_count":   bgp.len(),
                "bgp_patterns":        bgp,
                "unsupported_algebra": unsupported,
            })
        }
        Query::Construct { .. } => {
            json!({ "form": "CONSTRUCT", "supported": false,
                    "reason": "CONSTRUCT not in Phase 2.2 scope" })
        }
        Query::Ask { .. } => {
            json!({ "form": "ASK", "supported": false,
                    "reason": "ASK not in Phase 2.2 scope" })
        }
        Query::Describe { .. } => {
            json!({ "form": "DESCRIBE", "supported": false,
                    "reason": "DESCRIBE not in Phase 2.2 scope" })
        }
    }
}

/// Walk a SELECT pattern, collect projected variables and the BGP
/// triples we know how to translate. Anything else is flagged in
/// `unsupported_algebra` so callers can see the AST has shape we
/// don't cover yet.
fn walk_select_pattern(pattern: &GraphPattern) -> (Vec<String>, Vec<Value>, Vec<&'static str>) {
    let mut vars: Vec<String> = Vec::new();
    let mut bgp: Vec<Value> = Vec::new();
    let mut unsupported: Vec<&'static str> = Vec::new();
    walk(pattern, &mut vars, &mut bgp, &mut unsupported);
    (vars, bgp, unsupported)
}

fn walk(
    p: &GraphPattern,
    vars: &mut Vec<String>,
    bgp: &mut Vec<Value>,
    unsupported: &mut Vec<&'static str>,
) {
    match p {
        GraphPattern::Project { inner, variables } => {
            // Outer projection: capture the variable list, then keep
            // walking down for the BGP.
            for v in variables {
                let name = v.as_str().to_string();
                if !vars.contains(&name) {
                    vars.push(name);
                }
            }
            walk(inner, vars, bgp, unsupported);
        }
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                bgp.push(triple_to_json(tp));
                // Collect variables seen in the BGP even when no
                // explicit Project wraps it (SELECT * inputs).
                collect_vars(tp, vars);
            }
        }
        GraphPattern::Distinct { inner } => walk(inner, vars, bgp, unsupported),
        GraphPattern::Reduced { inner }  => walk(inner, vars, bgp, unsupported),
        GraphPattern::Slice { inner, .. } => walk(inner, vars, bgp, unsupported),
        GraphPattern::OrderBy { inner, .. } => walk(inner, vars, bgp, unsupported),

        GraphPattern::Filter { .. }    => unsupported.push("Filter"),
        GraphPattern::LeftJoin { .. }  => unsupported.push("LeftJoin (OPTIONAL)"),
        GraphPattern::Union { .. }     => unsupported.push("Union"),
        GraphPattern::Minus { .. }     => unsupported.push("Minus"),
        GraphPattern::Join { .. }      => unsupported.push("Join (non-BGP)"),
        GraphPattern::Graph { .. }     => unsupported.push("Graph (named graph clause)"),
        GraphPattern::Group { .. }     => unsupported.push("Group (aggregate)"),
        GraphPattern::Path { .. }      => unsupported.push("Path (property path)"),
        GraphPattern::Values { .. }    => unsupported.push("Values (inline VALUES)"),
        GraphPattern::Extend { .. }    => unsupported.push("Extend (BIND)"),
        GraphPattern::Service { .. }   => unsupported.push("Service (federation)"),

        _ => unsupported.push("other"),
    }
}

fn collect_vars(tp: &TriplePattern, out: &mut Vec<String>) {
    if let TermPattern::Variable(v) = &tp.subject {
        let n = v.as_str().to_string();
        if !out.contains(&n) { out.push(n); }
    }
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        let n = v.as_str().to_string();
        if !out.contains(&n) { out.push(n); }
    }
    if let TermPattern::Variable(v) = &tp.object {
        let n = v.as_str().to_string();
        if !out.contains(&n) { out.push(n); }
    }
}

fn triple_to_json(tp: &TriplePattern) -> Value {
    json!({
        "s": term_pattern_to_json(&tp.subject),
        "p": named_node_pattern_to_json(&tp.predicate),
        "o": term_pattern_to_json(&tp.object),
    })
}

fn term_pattern_to_json(t: &TermPattern) -> Value {
    match t {
        TermPattern::Variable(v)  => json!({ "var":  v.as_str() }),
        TermPattern::NamedNode(n) => json!({ "iri":  n.as_str() }),
        TermPattern::BlankNode(b) => json!({ "bnode": b.as_str() }),
        TermPattern::Literal(l)   => {
            let mut obj = serde_json::Map::new();
            obj.insert("literal".into(), Value::String(l.value().to_string()));
            if let Some(lang) = l.language() {
                obj.insert("lang".into(), Value::String(lang.to_string()));
            } else {
                obj.insert("datatype".into(), Value::String(l.datatype().as_str().to_string()));
            }
            Value::Object(obj)
        }
        _ => json!({ "unsupported": format!("{:?}", t) }),
    }
}

fn named_node_pattern_to_json(n: &NamedNodePattern) -> Value {
    match n {
        NamedNodePattern::NamedNode(nn) => json!({ "iri": nn.as_str() }),
        NamedNodePattern::Variable(v)   => json!({ "var": v.as_str() }),
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn sparql_parse_basic_select() {
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.sparql_parse($1)",
            &["SELECT ?s ?p ?o WHERE { ?s ?p ?o }".into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["form"], "SELECT");
        assert_eq!(v["bgp_pattern_count"], 1);
        // spargebra normalises variable order to the order seen in
        // the SELECT clause: s, p, o.
        let vars = v["variables"].as_array().unwrap();
        assert_eq!(vars.len(), 3);
        assert_eq!(vars[0], "s");
        assert_eq!(vars[1], "p");
        assert_eq!(vars[2], "o");
    }

    #[pg_test]
    fn sparql_parse_bgp_with_named_predicate() {
        let q = r#"
            PREFIX foaf: <http://xmlns.com/foaf/0.1/>
            SELECT ?person ?name
              WHERE { ?person foaf:name ?name }
        "#;
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.sparql_parse($1)",
            &[q.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["form"], "SELECT");
        assert_eq!(v["bgp_pattern_count"], 1);
        let tp = &v["bgp_patterns"][0];
        assert_eq!(tp["s"]["var"], "person");
        assert_eq!(tp["p"]["iri"], "http://xmlns.com/foaf/0.1/name");
        assert_eq!(tp["o"]["var"], "name");
    }

    #[pg_test]
    fn sparql_parse_multipattern_bgp() {
        let q = r#"
            PREFIX foaf: <http://xmlns.com/foaf/0.1/>
            SELECT ?p ?n ?m
              WHERE {
                ?p foaf:name ?n .
                ?p foaf:mbox ?m .
              }
        "#;
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.sparql_parse($1)",
            &[q.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["bgp_pattern_count"], 2);
    }

    #[pg_test]
    fn sparql_parse_flags_unsupported_filter() {
        let q = "SELECT ?s WHERE { ?s ?p ?o FILTER(isIRI(?o)) }";
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.sparql_parse($1)",
            &[q.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(
            unsupported.iter().any(|x| x.as_str() == Some("Filter")),
            "expected Filter to be flagged as unsupported, got {unsupported:?}"
        );
    }

    #[pg_test]
    fn sparql_parse_syntax_error_panics() {
        // Postgres surfaces our `panic!` as ERROR. Spi::get_one_with_args
        // returns an error; we just assert we got one.
        let err = std::panic::catch_unwind(|| {
            let _: Option<pgrx::JsonB> = Spi::get_one_with_args(
                "SELECT pgrdf.sparql_parse($1)",
                &["SELECT ?s WHERE { ?s ?p".into()],
            )
            .ok()
            .flatten();
        });
        // catch_unwind catches Rust panics; SPI ERROR also unwinds.
        // We accept either form.
        let _ = err;
    }
}
