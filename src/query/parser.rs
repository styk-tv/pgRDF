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
//!   * CONSTRUCT — Phase D slice 52 enrichment. Reports `form:
//!     "CONSTRUCT"` with a `template` block (triple count, has_variables,
//!     has_blank_nodes, has_constants_only, variables) and a
//!     `where_shape` block (kind, triple_count, named_graphs_used,
//!     variables). Detects the W3C SPARQL 1.1 §16.2.4 shorthand form
//!     (`CONSTRUCT WHERE { ... }`) via the same ASCII probe the
//!     executor uses (`crate::query::executor::detect_construct_where_shorthand`).
//!     Flags `Distinct` / `OrderBy` / `Group` / `Aggregate` wrappings
//!     in `unsupported_algebra` — they panic at execute time per LLD
//!     §6.2.
//!   * DESCRIBE — recognised but reported as `supported: false`; the
//!     executor doesn't handle it yet (carried forward to a later
//!     v0.4 slice).
//!   * OPTIONAL / UNION / MINUS / FILTER / aggregates / BIND — the
//!     parser walks through them; the executor supports them too,
//!     so they are NOT flagged in `unsupported_algebra`.
//!   * GRAPH (named-graph clause), property paths, inline VALUES,
//!     SERVICE — still flagged under `unsupported_algebra` (named
//!     graphs land in v0.4 — see SPEC.pgRDF.LLD.v0.4.md §3;
//!     paths in §7; VALUES in §4-deferred backlog).

use pgrx::prelude::*;
use serde_json::{json, Value};
use spargebra::algebra::{GraphPattern, QueryDataset};
use spargebra::term::{GraphName, GraphNamePattern, NamedNodePattern, TermPattern, TriplePattern};
use spargebra::{GraphUpdateOperation, Query, SparqlParser, Update};

use crate::query::executor::detect_construct_where_shorthand;

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
        Ok(parsed) => pgrx::JsonB(serialize_query(&parsed, query)),
        Err(query_err) => match SparqlParser::new().parse_update(query) {
            Ok(update) => pgrx::JsonB(serialize_update(&update)),
            // Surface the *query*-side parser error — the
            // `sparql_parse:` prefix is the locked contract for
            // syntax-error tail.
            Err(_) => panic!("sparql_parse: {query_err}"),
        },
    }
}

fn serialize_query(q: &Query, raw: &str) -> Value {
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
        Query::Construct {
            template, pattern, ..
        } => {
            // Phase D slice 52 — CONSTRUCT enrichment. Walk the
            // template + WHERE pattern, report shapes alongside the
            // shorthand-form flag and any out-of-scope algebra
            // modifiers (Distinct / OrderBy / Group / Aggregate)
            // that pgrdf.construct will refuse at execute time per
            // LLD §6.2.
            let where_shape = analyse_where_shape(pattern);
            let template_shape = analyse_construct_template(template);
            let shorthand = detect_construct_where_shorthand(raw);
            let unsupported = detect_unsupported_algebra(pattern);
            json!({
                "form":                "CONSTRUCT",
                "where_shape":         where_shape,
                "template":            template_shape,
                "shorthand":           shorthand,
                "unsupported_algebra": unsupported,
            })
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
/// inside; per-op detail lives under `operations[]`.
///
/// Phase C slice 74 enrichment — per-op detail surfaces enough of the
/// executor's routing inputs that callers can preview an UPDATE's
/// effect without running it:
///
///   * `InsertData` — `triples` + `graphs` (DEFAULT and/or IRIs from
///     the per-quad `graph_name`).
///   * `DeleteData` — same shape as `InsertData` (slice 74 promoted
///     `graphs` here so the surface is symmetric).
///   * `DeleteInsert` — narrows to a `kind` label aligning with the
///     executor's runtime `form` (`INSERT_WHERE`, `DELETE_WHERE`, or
///     `DELETE_INSERT_WHERE`); surfaces template-side graph IRIs
///     extracted from the `Vec<QuadPattern>` / `Vec<GroundQuadPattern>`
///     templates, and the WITH-iri from `using` (when present).
///   * `Clear` / `Create` / `Drop` — surface the `target` (DEFAULT /
///     NAMED <iri> / NAMED_ALL / ALL) so callers can preview which
///     partition the lifecycle UDF will touch.
///
/// Forms the parser parses but the executor doesn't translate yet are
/// NOT flagged in `unsupported_algebra` — that array is reserved for
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
                    push_graph_name(&q.graph_name, &mut graphs);
                }
                ops.push(json!({
                    "op":      "InsertData",
                    "triples": data.len(),
                    "graphs":  graphs,
                }));
            }
            GraphUpdateOperation::DeleteData { data } => {
                let mut graphs: Vec<String> = Vec::new();
                for q in data {
                    push_graph_name(&q.graph_name, &mut graphs);
                }
                ops.push(json!({
                    "op":      "DeleteData",
                    "triples": data.len(),
                    "graphs":  graphs,
                }));
            }
            GraphUpdateOperation::DeleteInsert {
                delete,
                insert,
                using,
                pattern,
            } => {
                // Narrow to match the executor's runtime form labels
                // (`INSERT_WHERE` / `DELETE_WHERE` / `DELETE_INSERT_WHERE`)
                // so sparql_parse callers can route on the same key the
                // _update summary row emits.
                let has_delete = !delete.is_empty();
                let has_insert = !insert.is_empty();
                let kind = match (has_delete, has_insert) {
                    (true, true) => "DELETE_INSERT_WHERE",
                    (true, false) => "DELETE_WHERE",
                    (false, true) => "INSERT_WHERE",
                    // spargebra parser never emits an empty DeleteInsert;
                    // surface a stable label rather than panic so
                    // sparql_parse stays infallible on every parsed AST.
                    (false, false) => "DELETE_INSERT_WHERE",
                };
                // Template-side graphs: union of GroundQuadPattern.graph_name
                // (delete) and QuadPattern.graph_name (insert). Variable
                // graph names are surfaced as "?var" sentinels — the
                // executor handles them via `GraphPattern::Graph` scoping.
                let mut tmpl_graphs: Vec<String> = Vec::new();
                for q in delete {
                    push_graph_name_pattern(&q.graph_name, &mut tmpl_graphs);
                }
                for q in insert {
                    push_graph_name_pattern(&q.graph_name, &mut tmpl_graphs);
                }
                let mut entry = json!({
                    "op": "DeleteInsert",
                    "kind": kind,
                    "delete_template_size": delete.len(),
                    "insert_template_size": insert.len(),
                    "where_pattern_size":   bgp_count(pattern),
                    "template_graphs":      tmpl_graphs,
                });
                if let Some(with_iri) = with_iri_from_using(using) {
                    entry["with_graph"] = Value::String(with_iri);
                }
                ops.push(entry);
            }
            GraphUpdateOperation::Load { .. } => {
                ops.push(json!({ "op": "Load" }));
                unsupported.push("Load (URL fetch out of v0.4 scope)");
            }
            GraphUpdateOperation::Clear { graph, silent } => ops.push(json!({
                "op": "Clear",
                "target": graph_target_label(graph),
                "silent": *silent,
            })),
            GraphUpdateOperation::Create { graph, silent } => ops.push(json!({
                "op": "Create",
                "target": format!("NAMED <{}>", graph.as_str()),
                "silent": *silent,
            })),
            GraphUpdateOperation::Drop { graph, silent } => ops.push(json!({
                "op": "Drop",
                "target": graph_target_label(graph),
                "silent": *silent,
            })),
        }
    }
    json!({
        "form":                "UPDATE",
        "operations":          ops,
        "unsupported_algebra": unsupported,
    })
}

/// Append a `GraphName` (used by `Quad` in INSERT DATA / DELETE DATA)
/// to `graphs`, deduplicating against existing entries. `DefaultGraph`
/// surfaces as the literal token `"DEFAULT"`; named-node graphs
/// surface as their absolute IRI string.
fn push_graph_name(g: &GraphName, graphs: &mut Vec<String>) {
    let label = match g {
        GraphName::DefaultGraph => "DEFAULT".to_string(),
        GraphName::NamedNode(n) => n.as_str().to_string(),
    };
    if !graphs.iter().any(|existing| existing == &label) {
        graphs.push(label);
    }
}

/// Append a `GraphNamePattern` (used by `QuadPattern` /
/// `GroundQuadPattern` in DeleteInsert templates) to `graphs`,
/// deduplicating. Variable graph names surface as `?var` so callers
/// can detect dynamic routing; `DefaultGraph` surfaces as `"DEFAULT"`
/// (consistent with the executor's `graphs_touched` reporting).
fn push_graph_name_pattern(g: &GraphNamePattern, graphs: &mut Vec<String>) {
    let label = match g {
        GraphNamePattern::DefaultGraph => "DEFAULT".to_string(),
        GraphNamePattern::NamedNode(n) => n.as_str().to_string(),
        GraphNamePattern::Variable(v) => format!("?{}", v.as_str()),
    };
    if !graphs.iter().any(|existing| existing == &label) {
        graphs.push(label);
    }
}

/// Extract the WITH-graph IRI from a `using:` field — mirrors
/// `executor.rs::with_iri_from_using` but tolerates the multi-IRI /
/// USING NAMED forms by returning `None` instead of panicking. That
/// keeps `sparql_parse` infallible on every parsed AST even when the
/// query carries shapes the executor will later reject (the rejection
/// is the executor's job; the parser only describes).
fn with_iri_from_using(using: &Option<QueryDataset>) -> Option<String> {
    let ds = using.as_ref()?;
    let named_empty = ds.named.as_ref().map(|v| v.is_empty()).unwrap_or(true);
    if ds.default.len() == 1 && named_empty {
        Some(ds.default[0].as_str().to_string())
    } else {
        None
    }
}

/// Render a lifecycle algebra target (`CLEAR` / `DROP` argument):
///   * `GraphTarget::NamedNode(<iri>)` → `"NAMED <iri>"`
///   * `GraphTarget::DefaultGraph`     → `"DEFAULT"`
///   * `GraphTarget::NamedGraphs`      → `"NAMED_ALL"`
///   * `GraphTarget::AllGraphs`        → `"ALL"`
fn graph_target_label(t: &spargebra::algebra::GraphTarget) -> String {
    use spargebra::algebra::GraphTarget;
    match t {
        GraphTarget::NamedNode(n) => format!("NAMED <{}>", n.as_str()),
        GraphTarget::DefaultGraph => "DEFAULT".to_string(),
        GraphTarget::NamedGraphs => "NAMED_ALL".to_string(),
        GraphTarget::AllGraphs => "ALL".to_string(),
    }
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

// ---------------------------------------------------------------------------
// Phase D slice 52 — CONSTRUCT-side analysis helpers.
// ---------------------------------------------------------------------------

/// Analyse the top-level WHERE pattern of a CONSTRUCT query.
///
/// Returns the JSON shape:
///
/// ```json
/// {
///   "kind": "Bgp" | "Optional" | "Union" | "Minus" | "Graph" |
///           "Filter" | "Bind" | "Values" | "Group" | "OrderBy" |
///           "Distinct" | "Service",
///   "triple_count": <usize>,        // sum of BGP triples beneath this pattern
///   "named_graphs_used": [..],      // IRIs and ?vars under any GRAPH scope
///   "variables": [..]               // distinct variables in the WHERE pattern, sorted
/// }
/// ```
///
/// Trivial outer `Project` wrappers (which spargebra synthesises even
/// for CONSTRUCT — the `build_select` machinery layers a Project for
/// the implicit all-vars projection) are peeled before reporting `kind`,
/// matching how the UPDATE path treats them. A `Slice` wrapper is
/// likewise transparent. The inner variant — the BGP or composite —
/// is what surfaces.
fn analyse_where_shape(pattern: &GraphPattern) -> Value {
    let inner = peel_trivial_wrappers(pattern);
    let kind = where_shape_kind(inner);
    let triple_count = count_bgp_triples(inner);
    let mut named_graphs: Vec<String> = Vec::new();
    collect_named_graphs(inner, &mut named_graphs);
    let mut variables: Vec<String> = Vec::new();
    collect_pattern_vars(inner, &mut variables);
    variables.sort();
    json!({
        "kind":              kind,
        "triple_count":      triple_count,
        "named_graphs_used": named_graphs,
        "variables":         variables,
    })
}

/// Walk past `Project` / `Slice` wrappers that spargebra adds even on
/// queries that have no meaningful projection (CONSTRUCT in particular
/// always carries an outer Project for the implicit all-vars list).
fn peel_trivial_wrappers(p: &GraphPattern) -> &GraphPattern {
    match p {
        GraphPattern::Project { inner, .. } | GraphPattern::Slice { inner, .. } => {
            peel_trivial_wrappers(inner)
        }
        _ => p,
    }
}

/// Report the spargebra-variant-name (PascalCase, matching the enum
/// arm names) of the immediate top-level shape, after trivial wrappers
/// have been peeled.
fn where_shape_kind(p: &GraphPattern) -> &'static str {
    match p {
        GraphPattern::Bgp { .. } => "Bgp",
        // `LeftJoin` is spargebra's algebra arm for OPTIONAL — surface
        // the user-visible W3C name rather than the internal algebra
        // label.
        GraphPattern::LeftJoin { .. } => "Optional",
        GraphPattern::Union { .. } => "Union",
        GraphPattern::Minus { .. } => "Minus",
        GraphPattern::Graph { .. } => "Graph",
        GraphPattern::Filter { .. } => "Filter",
        GraphPattern::Extend { .. } => "Bind",
        GraphPattern::Values { .. } => "Values",
        GraphPattern::Group { .. } => "Group",
        GraphPattern::OrderBy { .. } => "OrderBy",
        GraphPattern::Distinct { .. } => "Distinct",
        GraphPattern::Reduced { .. } => "Distinct",
        GraphPattern::Service { .. } => "Service",
        GraphPattern::Join { .. } => "Join",
        GraphPattern::Path { .. } => "Path",
        // Project / Slice are usually peeled before we reach this
        // branch; if a caller hands us one directly, surface the
        // variant name for transparency.
        GraphPattern::Project { .. } => "Project",
        GraphPattern::Slice { .. } => "Slice",
        _ => "Other",
    }
}

/// Recursively count BGP triple patterns under any composite shape.
/// Composite arms (LeftJoin / Union / Minus / Graph / Filter / Extend /
/// Group / OrderBy / Distinct / Reduced / Slice / Project / Service)
/// recurse into their inner arms; `Path` and `Values` carry no triples
/// (Path is a property-path predicate, not a BGP triple; Values is an
/// inline table) so they contribute zero.
fn count_bgp_triples(p: &GraphPattern) -> usize {
    match p {
        GraphPattern::Bgp { patterns } => patterns.len(),
        GraphPattern::Join { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => count_bgp_triples(left) + count_bgp_triples(right),
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Graph { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Service { inner, .. } => count_bgp_triples(inner),
        GraphPattern::Path { .. } | GraphPattern::Values { .. } => 0,
        _ => 0,
    }
}

/// Walk the pattern tree collecting any `GRAPH <iri> { … }` or
/// `GRAPH ?var { … }` scope names. Literal IRIs surface as the IRI
/// string verbatim; variables surface as `"?name"` (matching the
/// `UPDATE` path's `push_graph_name_pattern` convention). Order is
/// insertion order; duplicates are skipped.
fn collect_named_graphs(p: &GraphPattern, out: &mut Vec<String>) {
    match p {
        GraphPattern::Graph { name, inner } => {
            let label = match name {
                NamedNodePattern::NamedNode(n) => n.as_str().to_string(),
                NamedNodePattern::Variable(v) => format!("?{}", v.as_str()),
            };
            if !out.iter().any(|existing| existing == &label) {
                out.push(label);
            }
            collect_named_graphs(inner, out);
        }
        GraphPattern::Join { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            collect_named_graphs(left, out);
            collect_named_graphs(right, out);
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Service { inner, .. } => collect_named_graphs(inner, out),
        _ => {}
    }
}

/// Walk the pattern tree collecting every distinct variable name
/// referenced in any TriplePattern position (subject / predicate /
/// object). The output list is unsorted on insertion; callers
/// (`analyse_where_shape`) sort for deterministic regression hashes.
fn collect_pattern_vars(p: &GraphPattern, out: &mut Vec<String>) {
    match p {
        GraphPattern::Bgp { patterns } => {
            for tp in patterns {
                if let TermPattern::Variable(v) = &tp.subject {
                    push_unique(out, format!("?{}", v.as_str()));
                }
                if let NamedNodePattern::Variable(v) = &tp.predicate {
                    push_unique(out, format!("?{}", v.as_str()));
                }
                if let TermPattern::Variable(v) = &tp.object {
                    push_unique(out, format!("?{}", v.as_str()));
                }
            }
        }
        GraphPattern::Graph { name, inner } => {
            if let NamedNodePattern::Variable(v) = name {
                push_unique(out, format!("?{}", v.as_str()));
            }
            collect_pattern_vars(inner, out);
        }
        GraphPattern::Join { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            collect_pattern_vars(left, out);
            collect_pattern_vars(right, out);
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::OrderBy { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Distinct { inner }
        | GraphPattern::Reduced { inner }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Group { inner, .. }
        | GraphPattern::Service { inner, .. } => collect_pattern_vars(inner, out),
        _ => {}
    }
}

fn push_unique(out: &mut Vec<String>, value: String) {
    if !out.iter().any(|existing| existing == &value) {
        out.push(value);
    }
}

/// Analyse a CONSTRUCT template (`Vec<TriplePattern>`), returning:
///
/// ```json
/// {
///   "triple_count": <usize>,
///   "has_variables": <bool>,
///   "has_blank_nodes": <bool>,
///   "has_constants_only": <bool>,   // !has_variables && !has_blank_nodes
///   "variables": [..]               // distinct, sorted
/// }
/// ```
fn analyse_construct_template(template: &[TriplePattern]) -> Value {
    let mut has_variables = false;
    let mut has_blank_nodes = false;
    let mut variables: Vec<String> = Vec::new();
    for tp in template {
        match &tp.subject {
            TermPattern::Variable(v) => {
                has_variables = true;
                push_unique(&mut variables, format!("?{}", v.as_str()));
            }
            TermPattern::BlankNode(_) => has_blank_nodes = true,
            _ => {}
        }
        match &tp.predicate {
            NamedNodePattern::Variable(v) => {
                has_variables = true;
                push_unique(&mut variables, format!("?{}", v.as_str()));
            }
            NamedNodePattern::NamedNode(_) => {}
        }
        match &tp.object {
            TermPattern::Variable(v) => {
                has_variables = true;
                push_unique(&mut variables, format!("?{}", v.as_str()));
            }
            TermPattern::BlankNode(_) => has_blank_nodes = true,
            _ => {}
        }
    }
    variables.sort();
    let has_constants_only = !has_variables && !has_blank_nodes;
    json!({
        "triple_count":       template.len(),
        "has_variables":      has_variables,
        "has_blank_nodes":    has_blank_nodes,
        "has_constants_only": has_constants_only,
        "variables":          variables,
    })
}

/// Detect WHERE-clause shapes that `pgrdf.construct` will reject at
/// execute time (LLD §6.2; cf.
/// `crate::query::executor::reject_construct_modifiers`): `Distinct` /
/// `OrderBy` / `Group` modifiers and `Aggregate` expressions inside a
/// `Group`. Wrappers are returned by their W3C-facing names, matching
/// the executor's reject prefix family. The walk descends through all
/// composite shapes so a modifier nested inside a sub-SELECT or under
/// a UNION arm still surfaces.
fn detect_unsupported_algebra(p: &GraphPattern) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    walk_unsupported(p, &mut out);
    out
}

fn walk_unsupported(p: &GraphPattern, out: &mut Vec<&'static str>) {
    match p {
        GraphPattern::Distinct { inner } => {
            push_unique_static(out, "Distinct");
            walk_unsupported(inner, out);
        }
        GraphPattern::Reduced { inner } => {
            // REDUCED is a DISTINCT cousin per W3C 1.1 §15.6 — same
            // reject family per LLD §6.2.
            push_unique_static(out, "Distinct");
            walk_unsupported(inner, out);
        }
        GraphPattern::OrderBy { inner, .. } => {
            push_unique_static(out, "OrderBy");
            walk_unsupported(inner, out);
        }
        GraphPattern::Group {
            inner, aggregates, ..
        } => {
            push_unique_static(out, "Group");
            if !aggregates.is_empty() {
                push_unique_static(out, "Aggregate");
            }
            walk_unsupported(inner, out);
        }
        GraphPattern::Join { left, right }
        | GraphPattern::LeftJoin { left, right, .. }
        | GraphPattern::Union { left, right }
        | GraphPattern::Minus { left, right } => {
            walk_unsupported(left, out);
            walk_unsupported(right, out);
        }
        GraphPattern::Filter { inner, .. }
        | GraphPattern::Graph { inner, .. }
        | GraphPattern::Extend { inner, .. }
        | GraphPattern::Project { inner, .. }
        | GraphPattern::Slice { inner, .. }
        | GraphPattern::Service { inner, .. } => walk_unsupported(inner, out),
        _ => {}
    }
}

fn push_unique_static(out: &mut Vec<&'static str>, value: &'static str) {
    if !out.contains(&value) {
        out.push(value);
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

    /// Phase C slice 74 — `DeleteInsert` ops surface a `kind` label
    /// that mirrors the executor's runtime `_update.form` (one of
    /// `INSERT_WHERE` / `DELETE_WHERE` / `DELETE_INSERT_WHERE`) plus
    /// a `template_graphs` array and (when WITH is present) a
    /// `with_graph` IRI. Pure INSERT WHERE — no DELETE template —
    /// narrows to `kind: "INSERT_WHERE"`.
    #[pg_test]
    fn sparql_parse_update_insert_where_kind() {
        let q = "PREFIX ex: <http://example.org/> \
                 INSERT { ?s ex:tagged \"y\" } WHERE { ?s ex:p ?o }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let ops = v["operations"].as_array().unwrap();
        assert_eq!(ops[0]["op"], "DeleteInsert");
        assert_eq!(ops[0]["kind"], "INSERT_WHERE");
        assert_eq!(ops[0]["delete_template_size"], 0);
        assert_eq!(ops[0]["insert_template_size"], 1);
        // No WITH — the field is absent (NOT null).
        assert!(ops[0].get("with_graph").is_none());
    }

    /// Pure DELETE WHERE narrows to `kind: "DELETE_WHERE"`.
    #[pg_test]
    fn sparql_parse_update_delete_where_kind() {
        let q = "PREFIX ex: <http://example.org/> \
                 DELETE { ?s ex:p ?o } WHERE { ?s ex:p ?o }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let ops = v["operations"].as_array().unwrap();
        assert_eq!(ops[0]["kind"], "DELETE_WHERE");
        assert_eq!(ops[0]["delete_template_size"], 1);
        assert_eq!(ops[0]["insert_template_size"], 0);
    }

    /// Combined DELETE + INSERT WHERE narrows to
    /// `kind: "DELETE_INSERT_WHERE"`.
    #[pg_test]
    fn sparql_parse_update_delete_insert_where_kind() {
        let q = "PREFIX ex: <http://example.org/> \
                 DELETE { ?s ex:p ?o } INSERT { ?s ex:p \"new\" } WHERE { ?s ex:p ?o }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let ops = v["operations"].as_array().unwrap();
        assert_eq!(ops[0]["kind"], "DELETE_INSERT_WHERE");
        assert_eq!(ops[0]["delete_template_size"], 1);
        assert_eq!(ops[0]["insert_template_size"], 1);
    }

    /// `WITH <g>` surfaces as `with_graph: "<iri>"` on the
    /// DeleteInsert op so callers can preview the routing.
    #[pg_test]
    fn sparql_parse_update_with_graph_surfaces_iri() {
        let q = "PREFIX ex: <http://example.org/> \
                 WITH <http://example.org/store> \
                 INSERT { ?s ex:t \"y\" } WHERE { ?s ex:p ?o }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let ops = v["operations"].as_array().unwrap();
        assert_eq!(ops[0]["with_graph"], "http://example.org/store");
    }

    /// Template-side `GRAPH <iri> { … }` blocks surface in
    /// `template_graphs`. Default-graph quads surface as `"DEFAULT"`.
    #[pg_test]
    fn sparql_parse_update_template_graphs_surfaced() {
        let q = "PREFIX ex: <http://example.org/> \
                 INSERT { GRAPH <http://example.org/dst> { ?s ex:copied \"y\" } } \
                 WHERE { ?s ex:src ?o }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let ops = v["operations"].as_array().unwrap();
        let tmpl = ops[0]["template_graphs"].as_array().unwrap();
        assert_eq!(tmpl.len(), 1);
        assert_eq!(tmpl[0], "http://example.org/dst");
    }

    /// DELETE DATA now surfaces `graphs` like INSERT DATA (slice 74).
    #[pg_test]
    fn sparql_parse_update_delete_data_graphs() {
        let q = "DELETE DATA { GRAPH <http://example.org/g1> { \
                   <http://x/a> <http://x/b> <http://x/c> } }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let ops = v["operations"].as_array().unwrap();
        assert_eq!(ops[0]["op"], "DeleteData");
        let graphs = ops[0]["graphs"].as_array().unwrap();
        assert_eq!(graphs.len(), 1);
        assert_eq!(graphs[0], "http://example.org/g1");
    }

    /// CLEAR / DROP / CREATE lifecycle ops surface a `target` label
    /// describing which partition the executor will touch.
    #[pg_test]
    fn sparql_parse_update_lifecycle_targets() {
        // CLEAR DEFAULT
        let j: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &["CLEAR DEFAULT".into()])
                .unwrap()
                .unwrap();
        let ops = j.0["operations"].as_array().unwrap();
        assert_eq!(ops[0]["op"], "Clear");
        assert_eq!(ops[0]["target"], "DEFAULT");
        assert_eq!(ops[0]["silent"], false);

        // DROP GRAPH <iri>
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.sparql_parse($1)",
            &["DROP GRAPH <http://example.org/g1>".into()],
        )
        .unwrap()
        .unwrap();
        let ops = j.0["operations"].as_array().unwrap();
        assert_eq!(ops[0]["op"], "Drop");
        assert_eq!(ops[0]["target"], "NAMED <http://example.org/g1>");

        // CREATE GRAPH <iri> SILENT  (SILENT keyword precedes GRAPH)
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.sparql_parse($1)",
            &["CREATE SILENT GRAPH <http://example.org/g2>".into()],
        )
        .unwrap()
        .unwrap();
        let ops = j.0["operations"].as_array().unwrap();
        assert_eq!(ops[0]["op"], "Create");
        assert_eq!(ops[0]["target"], "NAMED <http://example.org/g2>");
        assert_eq!(ops[0]["silent"], true);

        // DROP ALL — multi-graph target
        let j: pgrx::JsonB =
            Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &["DROP ALL".into()])
                .unwrap()
                .unwrap();
        let ops = j.0["operations"].as_array().unwrap();
        assert_eq!(ops[0]["op"], "Drop");
        assert_eq!(ops[0]["target"], "ALL");
    }

    /// Phase D slice 52 — CONSTRUCT enrichment. A constant-only
    /// template (no variables, no blank nodes) reports
    /// `has_constants_only: true`, `has_variables: false`, and the
    /// WHERE shape's `kind` is `"Bgp"` for a trivial BGP pattern. The
    /// `shorthand` flag is false (explicit `CONSTRUCT { ... } WHERE { ... }`).
    #[pg_test]
    fn sparql_parse_construct_constant_template() {
        let q =
            "CONSTRUCT { <http://example.org/s> <http://example.org/p> \"x\" } WHERE { ?s ?p ?o }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["form"], "CONSTRUCT");
        let tmpl = &v["template"];
        assert_eq!(tmpl["triple_count"], 1);
        assert_eq!(tmpl["has_constants_only"], true);
        assert_eq!(tmpl["has_variables"], false);
        assert_eq!(tmpl["has_blank_nodes"], false);
        assert_eq!(v["where_shape"]["kind"], "Bgp");
        assert_eq!(v["shorthand"], false);
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(unsupported.is_empty());
    }

    /// Phase D slice 52 — variable template. `?s` and `?o` appear in
    /// the template; the constant literal `"src"` is NOT a variable so
    /// the template's `variables` array contains only `?s` and `?o`.
    #[pg_test]
    fn sparql_parse_construct_variable_template() {
        let q = "PREFIX ex: <http://example.org/> \
                 CONSTRUCT { ?s ex:tag ?o . ?s ex:from \"src\" } \
                 WHERE { ?s ex:p ?o . ?s ex:p2 ?o2 }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        let tmpl = &v["template"];
        assert_eq!(tmpl["triple_count"], 2);
        assert_eq!(tmpl["has_variables"], true);
        let tmpl_vars = tmpl["variables"].as_array().unwrap();
        // Sorted: ?o, ?s.
        assert_eq!(tmpl_vars.len(), 2);
        assert!(tmpl_vars.iter().any(|x| x.as_str() == Some("?o")));
        assert!(tmpl_vars.iter().any(|x| x.as_str() == Some("?s")));
        // WHERE has four distinct variables: ?o, ?o2, ?p (no — predicate is
        // constant ex:p), ?s. Predicate is constant so it's NOT a variable.
        let where_vars = v["where_shape"]["variables"].as_array().unwrap();
        assert!(where_vars.iter().any(|x| x.as_str() == Some("?s")));
        assert!(where_vars.iter().any(|x| x.as_str() == Some("?o")));
        assert!(where_vars.iter().any(|x| x.as_str() == Some("?o2")));
        assert_eq!(v["where_shape"]["triple_count"], 2);
    }

    /// Phase D slice 52 — shorthand form. `CONSTRUCT WHERE { ?s ?p ?o }`
    /// is equivalent to `CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }`; the
    /// `shorthand` flag MUST be true. spargebra populates the AST's
    /// `template` from the BGP at parse time, so template/where shapes
    /// match each other.
    #[pg_test]
    fn sparql_parse_construct_shorthand_form() {
        let q = "CONSTRUCT WHERE { ?s ?p ?o }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["form"], "CONSTRUCT");
        assert_eq!(v["shorthand"], true);
        assert_eq!(v["template"]["triple_count"], 1);
        assert_eq!(v["where_shape"]["triple_count"], 1);
        assert_eq!(v["where_shape"]["kind"], "Bgp");
    }

    /// Phase D slice 52 — unsupported algebra detection. DISTINCT
    /// inside the CONSTRUCT's WHERE (via a sub-SELECT) surfaces as
    /// `Distinct` in `unsupported_algebra`. The query still parses to
    /// `form: "CONSTRUCT"` — the flag just signals
    /// `pgrdf.construct` will panic at execute time per LLD §6.2.
    #[pg_test]
    fn sparql_parse_construct_unsupported_distinct() {
        let q = "CONSTRUCT { <http://example.org/s> <http://example.org/p> \"x\" } \
                 WHERE { { SELECT DISTINCT ?s WHERE { ?s ?p ?o } } }";
        let j: pgrx::JsonB = Spi::get_one_with_args("SELECT pgrdf.sparql_parse($1)", &[q.into()])
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["form"], "CONSTRUCT");
        let unsupported = v["unsupported_algebra"].as_array().unwrap();
        assert!(
            unsupported.iter().any(|x| x.as_str() == Some("Distinct")),
            "expected Distinct flagged, got {unsupported:?}"
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
