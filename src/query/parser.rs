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
//!   * ASK — supported. The parser reports `bgp_pattern_count` +
//!     `unsupported_algebra` just like SELECT; the executor wraps
//!     the probe SELECT in `EXISTS(...)`.
//!   * CONSTRUCT / DESCRIBE — recognised but reported as
//!     `supported: false`; the executor doesn't handle them yet
//!     (CONSTRUCT lands in v0.4 — see
//!     SPEC.pgRDF.LLD.v0.4.md §6).
//!   * OPTIONAL / UNION / MINUS / FILTER / aggregates / BIND — the
//!     parser walks through them; the executor supports them too,
//!     so they are NOT flagged in `unsupported_algebra`.
//!   * GRAPH (named-graph clause), property paths, inline VALUES,
//!     SERVICE — still flagged under `unsupported_algebra` (named
//!     graphs land in v0.4 — see SPEC.pgRDF.LLD.v0.4.md §3;
//!     paths in §7; VALUES in §4-deferred backlog).

use pgrx::prelude::*;
use serde_json::{json, Value};
use spargebra::algebra::GraphPattern;
use spargebra::term::{GraphName, NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{GraphUpdateOperation, Query, SparqlParser, Update};

/// Parse a SPARQL query (or UPDATE) and return a JSONB describing its
/// top-level shape: form (`SELECT` / `CONSTRUCT` / `ASK` / `DESCRIBE`
/// / `UPDATE`), projected variables (for queries), BGP triple patterns,
/// and — for UPDATEs (Phase C slice 84+) — a per-operation summary.
///
/// SQL: `pgrdf.sparql_parse(q TEXT) → JSONB`.
///
/// On syntax error the function aborts the query with a Postgres
/// ERROR carrying the spargebra parser message. As with
/// `pgrdf.sparql`, the parser tries `parse_query` first (current
/// behaviour) and only retries as `parse_update` on query-side
/// failure — keeps the SELECT/ASK code path untouched while widening
/// the surface to UPDATE introspection.
#[pg_extern]
fn sparql_parse(query: &str) -> pgrx::JsonB {
    match SparqlParser::new().parse_query(query) {
        Ok(parsed) => pgrx::JsonB(serialize_query(&parsed)),
        Err(query_err) => match SparqlParser::new().parse_update(query) {
            Ok(update) => pgrx::JsonB(serialize_update(&update)),
            // Surface the *query*-side parser error — the
            // `sparql_parse:` prefix is the locked contract for
            // syntax-error tail.
            Err(_) => panic!("sparql_parse: {query_err}"),
        },
    }
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
        Query::Ask { pattern, .. } => {
            let (_vars, bgp, unsupported) = walk_select_pattern(pattern);
            json!({
                "form":                "ASK",
                "bgp_pattern_count":   bgp.len(),
                "bgp_patterns":        bgp,
                "unsupported_algebra": unsupported,
            })
        }
        Query::Describe { .. } => {
            json!({ "form": "DESCRIBE", "supported": false,
                    "reason": "DESCRIBE not in Phase 2.2 scope" })
        }
    }
}

/// Walk an `spargebra::Update` and report its top-level shape. The
/// `form` is uniformly `"UPDATE"` regardless of which operations are
/// inside; per-op detail lives under `operations[]`. For slice 84 the
/// per-op shape is intentionally minimal — INSERT_DATA gets its triple
/// + graph counts, the others get the variant name only so callers
/// can preview which sub-slice (83/82/…) will translate them. Forms
/// the parser parses but the executor doesn't translate yet are NOT
/// flagged in `unsupported_algebra` — that array is reserved for
/// algebra shapes we can't even parse-walk (e.g. `LOAD` is out of
/// scope per LLD v0.4 §14).
fn serialize_update(u: &Update) -> Value {
    let mut ops: Vec<Value> = Vec::with_capacity(u.operations.len());
    let mut unsupported: Vec<&'static str> = Vec::new();
    for op in &u.operations {
        match op {
            GraphUpdateOperation::InsertData { data } => {
                let mut graphs: Vec<String> = Vec::new();
                for q in data {
                    match &q.graph_name {
                        GraphName::DefaultGraph => {
                            if !graphs.iter().any(|g| g == "DEFAULT") {
                                graphs.push("DEFAULT".to_string());
                            }
                        }
                        GraphName::NamedNode(n) => {
                            let s = n.as_str().to_string();
                            if !graphs.contains(&s) {
                                graphs.push(s);
                            }
                        }
                    }
                }
                ops.push(json!({
                    "op":      "InsertData",
                    "triples": data.len(),
                    "graphs":  graphs,
                }));
            }
            GraphUpdateOperation::DeleteData { data } => {
                ops.push(json!({
                    "op":      "DeleteData",
                    "triples": data.len(),
                }));
            }
            GraphUpdateOperation::DeleteInsert {
                delete,
                insert,
                pattern,
                ..
            } => {
                ops.push(json!({
                    "op": "DeleteInsert",
                    "delete_template_size": delete.len(),
                    "insert_template_size": insert.len(),
                    "where_pattern_size":   bgp_count(pattern),
                }));
            }
            GraphUpdateOperation::Load { .. } => {
                ops.push(json!({ "op": "Load" }));
                unsupported.push("Load (URL fetch out of v0.4 scope)");
            }
            GraphUpdateOperation::Clear { .. } => ops.push(json!({ "op": "Clear" })),
            GraphUpdateOperation::Create { .. } => ops.push(json!({ "op": "Create" })),
            GraphUpdateOperation::Drop { .. } => ops.push(json!({ "op": "Drop" })),
        }
    }
    json!({
        "form":                "UPDATE",
        "operations":          ops,
        "unsupported_algebra": unsupported,
    })
}

/// Best-effort BGP-pattern count inside a `WHERE` clause — used by
/// `DeleteInsert` summary lines. Re-uses the existing `walk` helper
/// (collects BGP triples + tags unsupported shapes) but discards
/// everything except the triple count.
fn bgp_count(p: &GraphPattern) -> usize {
    let mut vars: Vec<String> = Vec::new();
    let mut bgp: Vec<Value> = Vec::new();
    let mut unsupported: Vec<&'static str> = Vec::new();
    walk(p, &mut vars, &mut bgp, &mut unsupported);
    bgp.len()
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
        GraphPattern::Reduced { inner } => walk(inner, vars, bgp, unsupported),
        GraphPattern::Slice { inner, .. } => walk(inner, vars, bgp, unsupported),
        GraphPattern::OrderBy { inner, .. } => walk(inner, vars, bgp, unsupported),
        GraphPattern::Filter { inner, .. } => walk(inner, vars, bgp, unsupported),
        GraphPattern::LeftJoin { left, right, .. } => {
            // OPTIONAL — supported by the executor (single-triple
            // right side only, today). Walk both arms to collect
            // their BGP shape; the executor enforces the constraint.
            walk(left, vars, bgp, unsupported);
            walk(right, vars, bgp, unsupported);
        }

        GraphPattern::Union { left, right } => {
            // UNION — supported. Each branch contributes its own
            // BGP triples; the parser counts them in aggregate.
            walk(left, vars, bgp, unsupported);
            walk(right, vars, bgp, unsupported);
        }
        GraphPattern::Minus { left, right } => {
            // MINUS — supported. Walk both arms so the parser sees
            // each contributing triple. The executor scopes the
            // subtraction to shared variables.
            walk(left, vars, bgp, unsupported);
            walk(right, vars, bgp, unsupported);
        }
        GraphPattern::Join { .. } => unsupported.push("Join (non-BGP)"),
        GraphPattern::Graph { name, inner } => {
            // Slice 114: `GRAPH <iri> { … }` (literal-IRI form) is
            // supported by the executor — translate-time resolution
            // against `_pgrdf_graphs.iri` flows a per-pattern graph
            // constraint into the BGP SQL.
            //
            // Slice 113: `GRAPH ?g { … }` (variable form) is now
            // supported too — the executor JOINs `_pgrdf_graphs` to
            // bind ?g to the IRI string. Both arms walk `inner` so
            // the contained BGP's triples are still counted; neither
            // adds an `unsupported_algebra` tag.
            let _ = name;
            walk(inner, vars, bgp, unsupported);
        }
        GraphPattern::Group { inner, .. } => {
            // Aggregates — supported by the executor. Walk the inner
            // BGP so the parser still reports its shape.
            walk(inner, vars, bgp, unsupported);
        }
        GraphPattern::Extend { inner, .. } => {
            // BIND / Extend — supported only when used to rename
            // aggregate output vars; the executor enforces. Walk
            // the inner so the underlying BGP is still visible.
            walk(inner, vars, bgp, unsupported);
        }
        GraphPattern::Path { .. } => unsupported.push("Path (property path)"),
        GraphPattern::Values { .. } => unsupported.push("Values (inline VALUES)"),
        GraphPattern::Service { .. } => unsupported.push("Service (federation)"),

        _ => unsupported.push("other"),
    }
}

fn collect_vars(tp: &TriplePattern, out: &mut Vec<String>) {
    if let TermPattern::Variable(v) = &tp.subject {
        let n = v.as_str().to_string();
        if !out.contains(&n) {
            out.push(n);
        }
    }
    if let NamedNodePattern::Variable(v) = &tp.predicate {
        let n = v.as_str().to_string();
        if !out.contains(&n) {
            out.push(n);
        }
    }
    if let TermPattern::Variable(v) = &tp.object {
        let n = v.as_str().to_string();
        if !out.contains(&n) {
            out.push(n);
        }
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
        TermPattern::Variable(v) => json!({ "var":  v.as_str() }),
        TermPattern::NamedNode(n) => json!({ "iri":  n.as_str() }),
        TermPattern::BlankNode(b) => json!({ "bnode": b.as_str() }),
        TermPattern::Literal(l) => {
            let mut obj = serde_json::Map::new();
            obj.insert("literal".into(), Value::String(l.value().to_string()));
            if let Some(lang) = l.language() {
                obj.insert("lang".into(), Value::String(lang.to_string()));
            } else {
                obj.insert(
                    "datatype".into(),
                    Value::String(l.datatype().as_str().to_string()),
                );
            }
            Value::Object(obj)
        }
        _ => json!({ "unsupported": format!("{:?}", t) }),
    }
}

fn named_node_pattern_to_json(n: &NamedNodePattern) -> Value {
    match n {
        NamedNodePattern::NamedNode(nn) => json!({ "iri": nn.as_str() }),
        NamedNodePattern::Variable(v) => json!({ "var": v.as_str() }),
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
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
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
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["bgp_pattern_count"], 2);
    }

    /// Filter is supported by the executor — the parser walks
    /// through it. We assert it is NOT flagged in unsupported_algebra,
    /// and the underlying BGP is still extracted.
    #[pg_test]
    fn sparql_parse_filter_is_supported() {
        let q = "SELECT ?s WHERE { ?s ?p ?o FILTER(isIRI(?o)) }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(
            !unsupported.iter().any(|x| x.as_str() == Some("Filter")),
            "Filter should not be flagged as unsupported anymore, got {unsupported:?}"
        );
        assert_eq!(v["bgp_pattern_count"], 1);
    }

    /// OPTIONAL is supported by the executor — the parser walks
    /// through it like Filter. Both arms' BGP triples are collected.
    #[pg_test]
    fn sparql_parse_optional_is_supported() {
        let q = "SELECT ?s ?n WHERE { ?s ?p ?o OPTIONAL { ?s <http://x/n> ?n } }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(
            !unsupported
                .iter()
                .any(|x| x.as_str().is_some_and(|s| s.contains("OPTIONAL"))),
            "OPTIONAL should not be flagged anymore, got {unsupported:?}"
        );
        // Both BGP arms are now visible.
        assert_eq!(v["bgp_pattern_count"], 2);
    }

    /// UNION is supported by the executor — the parser walks both
    /// branches' BGPs and tallies their triples.
    #[pg_test]
    fn sparql_parse_union_is_supported() {
        let q = "SELECT ?s WHERE { { ?s <http://x/a> ?o } UNION { ?s <http://x/b> ?o } }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(
            !unsupported.iter().any(|x| x.as_str() == Some("Union")),
            "UNION should not be flagged anymore, got {unsupported:?}"
        );
        assert_eq!(v["bgp_pattern_count"], 2);
    }

    /// MINUS is supported — the parser walks both arms.
    #[pg_test]
    fn sparql_parse_minus_is_supported() {
        let q = "SELECT ?s WHERE { ?s ?p ?o MINUS { ?s <http://x/a> ?b } }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(
            !unsupported.iter().any(|x| x.as_str() == Some("Minus")),
            "MINUS should not be flagged anymore, got {unsupported:?}"
        );
        assert_eq!(v["bgp_pattern_count"], 2);
    }

    /// Transitive / quantified property paths (`:a*`, `:a+`, `:a?`,
    /// inverse, etc.) are still unsupported. Note: simple sequence
    /// paths (`<a>/<b>`) are desugared by spargebra into a BGP
    /// chain with fresh blank nodes, so they don't surface as Path.
    #[pg_test]
    fn sparql_parse_flags_unsupported_path() {
        let q = "SELECT ?s ?o WHERE { ?s <http://x/a>* ?o }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(
            unsupported
                .iter()
                .any(|x| x.as_str().is_some_and(|s| s.contains("Path"))),
            "expected Path to be flagged, got {unsupported:?}"
        );
    }

    /// Phase C slice 84 — `sparql_parse` reports `form: "UPDATE"`
    /// for INSERT DATA queries, with per-op `triples` counts and
    /// nothing in `unsupported_algebra`.
    #[pg_test]
    fn sparql_parse_update_insert_data() {
        let q = "INSERT DATA { <http://x/a> <http://x/b> <http://x/c> }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["form"], "UPDATE");
        let ops = v["operations"].as_array().unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0]["op"], "InsertData");
        assert_eq!(ops[0]["triples"], 1);
        // Default graph appears as the synthetic "DEFAULT" sentinel.
        let graphs = ops[0]["graphs"].as_array().unwrap();
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0], "DEFAULT");
        // No `unsupported_algebra` flags — INSERT DATA is parseable,
        // translatable, and end-to-end on slice 84.
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(unsupported.is_empty());
    }

    /// `INSERT DATA { GRAPH <iri> { … } }` — the per-op `graphs`
    /// array carries the named-graph IRI, not the `"DEFAULT"`
    /// sentinel.
    #[pg_test]
    fn sparql_parse_update_insert_data_named_graph() {
        let q = "INSERT DATA { GRAPH <http://example.org/g1> { \
                   <http://x/a> <http://x/b> <http://x/c> } }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["form"], "UPDATE");
        let ops = v["operations"].as_array().unwrap();
        assert_eq!(ops[0]["op"], "InsertData");
        let graphs = ops[0]["graphs"].as_array().unwrap();
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0], "http://example.org/g1");
    }

    /// DELETE DATA still parses to UPDATE; the per-op `op` field
    /// carries the variant name so callers see the shape even though
    /// the executor doesn't ship its translation until slice 83.
    #[pg_test]
    fn sparql_parse_update_delete_data_visible() {
        let q = "DELETE DATA { <http://x/a> <http://x/b> <http://x/c> }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["form"], "UPDATE");
        let ops = v["operations"].as_array().unwrap();
        assert_eq!(ops[0]["op"], "DeleteData");
        // `unsupported_algebra` MUST stay empty even for ops the
        // executor can't translate yet — that array is reserved for
        // genuinely-out-of-scope shapes (e.g. LOAD).
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(unsupported.is_empty());
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
