//! Track H Architecture-1 — pgRDF-native SHACL-SPARQL execution.
//!
//! TH-9 (this file as of v0.5.7): focus-node iteration + `$this`
//! substitution + SPI dispatch to `pgrdf.sparql` + result-row mapping.
//!
//! ## Goal
//!
//! Provide a third validation backend (`mode => 'pgrdf'`) alongside
//! `'native'` (rudof in-memory) and `'sparql'` (rudof endpoint-shaped).
//! For shapes that carry `IRComponent::BasicSparql` constraints
//! (`sh:sparql [ sh:select "…" ]` — the SHACL Part-2 vocabulary that
//! `shacl 0.3.2` parses), pgRDF intercepts the constraint, walks the
//! focus-node set produced by the shape's targets, substitutes `$this`
//! per focus node, executes through `pgrdf.sparql` — the same
//! dictionary-indexed hexastore path that already powers
//! `pgrdf.sparql` and `pgrdf.construct` — and maps every binding row
//! to a `sh:ValidationResult`. Core constraints continue to evaluate
//! through rudof's `NativeEngine`; only the `BasicSparql` variant is
//! intercepted.
//!
//! ## Why
//!
//! Today, `mode => 'sparql'` rehydrates the entire data graph as
//! N-Triples text and parses it into rudof's `InMemoryGraph`. For a
//! 10⁷-triple data graph that's hundreds of MB of text + a parallel
//! in-memory copy of every triple — the rudof path scales with
//! `InMemoryGraph`, not with PostgreSQL. The pgRDF-native path runs
//! every SHACL-SPARQL constraint through the hexastore directly:
//! O(1) per-focus-node lookup via dictionary, indexes used by the
//! planner, prepared-plan cache reuse across the focus iteration.
//!
//! ## Module shape
//!
//! - **Public entry point**: `run_pgrdf_sparql(data_g, shapes_g) →
//!   serde_json::Value`. Returns a ValidationReport in the same JSON
//!   shape as `'native'` / `'sparql'`.
//! - **Mode name**: `'pgrdf'`.
//! - **Schema walk** (TH-11/TH-10): `walk_schema_for_sparql(schema)`
//!   returns `Vec<(IRShape, BasicSparql)>`.
//! - **Per-shape evaluation** (TH-9, this commit): for each
//!   `(shape, sparql)`, resolve the shape's target set against the
//!   data graph (Class, ImplicitClass, Node, SubjectsOf, ObjectsOf);
//!   for each focus node, lexical-substitute `$this` in the
//!   `sh:select` text, run `pgrdf.sparql`, map each binding row to a
//!   `sh:ValidationResult` JSONB.
//! - **Dispatcher integration** (TH-8, next): a third arm in
//!   `validate()`'s `match mode` calls
//!   `pgrdf_sparql::run_pgrdf_sparql(...)`. Until TH-8, this module
//!   is unreachable from SQL — `validate()` continues to accept only
//!   `'native'` / `'sparql'`.
//!
//! ## What TH-9 delivers
//!
//! - Target resolution for the five well-formed `Target` variants
//!   (Node, Class, ImplicitClass, SubjectsOf, ObjectsOf).
//! - `$this` lexical substitution.
//! - `pgrdf.sparql` SPI dispatch per focus node.
//! - Result-row → `sh:ValidationResult` mapping (focusNode,
//!   resultPath, sourceShape, resultMessage, resultSeverity, value,
//!   sourceConstraintComponent = `sh:SPARQLConstraintComponent`).
//!
//! TH-8 wires this module into the SQL dispatcher; the live
//! integration tests + the W3C SHACL-SPARQL manifest sub-run land
//! with TH-7 / TH-6.

use crate::validation::shacl::serialise_graph_to_ntriples;
use pgrx::prelude::*;
use rudof_rdf::rdf_core::term::Object;
use rudof_rdf::rdf_core::RDFFormat;
use rudof_rdf::rdf_core::SHACLPath;
use serde_json::{json, Value};
use shacl::ir::components::BasicSparql;
use shacl::ir::{IRComponent, IRSchema, IRShape};
use shacl::types::{Severity, Target};
use shacl::validator::store::ShaclDataManager;
use std::io::Cursor;

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const SH_SPARQL_CONSTRAINT_COMPONENT: &str = "http://www.w3.org/ns/shacl#SPARQLConstraintComponent";

/// Public entry point — Track H Architecture-1 (pgRDF-native)
/// SHACL-SPARQL execution.
///
/// Rehydrates the shapes graph, compiles it to an `IRSchema`, walks
/// every `IRComponent::BasicSparql` constraint, resolves each owning
/// shape's targets against the data graph, lexical-substitutes `$this`
/// per focus node, and dispatches the rewritten query through
/// `pgrdf.sparql` (the dictionary-indexed hexastore path).
///
/// Output shape — parity with the `'native'` / `'sparql'` modes:
///
/// ```json
/// {
///   "conforms":        true | false,
///   "results":         [ ValidationResult, ... ],
///   "data_graph_id":   <i64>,
///   "shapes_graph_id": <i64>,
///   "shapes_triples":  <i64>,
///   "mode":            "pgrdf",
///   "error":           "<message-if-shapes-compile-failed>"
/// }
/// ```
///
/// `data_triples` is omitted: the pgRDF-native path doesn't rehydrate
/// the data graph (that's the whole point — we hit the hexastore
/// directly), so the count would require an extra SPI scan that
/// serves no purpose here.
pub fn run_pgrdf_sparql(data_graph_id: i64, shapes_graph_id: i64) -> Value {
    let (shapes_nt, shapes_count) = serialise_graph_to_ntriples(shapes_graph_id);
    let schema = match ShaclDataManager::load(
        &mut Cursor::new(shapes_nt.as_bytes()),
        "pgrdf-shapes",
        &RDFFormat::NTriples,
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            return json!({
                "conforms":        Value::Null,
                "results":         [],
                "data_graph_id":   data_graph_id,
                "shapes_graph_id": shapes_graph_id,
                "shapes_triples":  shapes_count,
                "mode":            "pgrdf",
                "error":           format!("shapes compile failed: {e}"),
            });
        }
    };

    let extracted = walk_schema_for_sparql(&schema);
    let mut violations: Vec<Value> = Vec::new();
    for (shape, sparql) in extracted.iter() {
        let focus_iris = resolve_focus_nodes(shape.targets(), data_graph_id);
        for focus_iri in &focus_iris {
            let substituted = substitute_this(sparql.select(), focus_iri);
            for row in call_pgrdf_sparql_spi(&substituted) {
                violations.push(build_violation(shape, sparql, focus_iri, &row));
            }
        }
    }

    json!({
        "conforms":        violations.is_empty(),
        "results":         violations,
        "data_graph_id":   data_graph_id,
        "shapes_graph_id": shapes_graph_id,
        "shapes_triples":  shapes_count,
        "mode":            "pgrdf",
    })
}

/// TH-11 — Extract every `IRComponent::BasicSparql` constraint from a
/// compiled `IRSchema`, paired with the shape that owns it.
///
/// Walk semantics:
/// - Iterates every shape via `IRSchema::iter()` — both node shapes
///   and property shapes.
/// - Skips deactivated shapes (`shape.deactivated() == true`) per
///   SHACL §3.3.
/// - Skips deactivated constraints within a live shape
///   (`sparql.deactivated() == Some(true)`).
/// - Returns owned values (clones) so the caller does not need to
///   hold the schema borrow across the per-shape SPI loop — holding
///   a borrow across SPI would conflict with the SPI runtime.
///
/// Output ordering matches `IRSchema::iter()` (insertion order of
/// the IR builder), so successive calls against the same schema are
/// deterministic.
pub fn walk_schema_for_sparql(schema: &IRSchema) -> Vec<(IRShape, BasicSparql)> {
    let mut out = Vec::new();
    for (_id, shape) in schema.iter() {
        if shape.deactivated() {
            continue;
        }
        for component in shape.components() {
            if let IRComponent::BasicSparql(sparql) = component {
                if sparql.deactivated() == Some(true) {
                    continue;
                }
                out.push((shape.clone(), sparql.clone()));
            }
        }
    }
    out
}

/// TH-9 — Resolve a shape's target declarations to a set of focus-node
/// IRIs.
///
/// SHACL §5.1 defines five well-formed target forms:
/// - `sh:targetNode` (Node): the focus node is the named term itself.
/// - `sh:targetClass` (Class) / implicit class (ImplicitClass): focus
///   nodes are every subject of `?s rdf:type <class>` in the data
///   graph (no rdfs:subClassOf transitive closure here — SHACL Core
///   spec §1.5 says target-class membership is direct typing; users
///   who want transitivity run `pgrdf.materialize` first).
/// - `sh:targetSubjectsOf` (SubjectsOf): every distinct subject of
///   `?s <pred> ?o`.
/// - `sh:targetObjectsOf` (ObjectsOf): every distinct object of
///   `?s <pred> ?o` that is itself an IRI (literals and blank nodes
///   are skipped — SHACL §5.5).
///
/// `Wrong*` variants flag malformed targets per spec §5.6; pgRDF
/// treats them as no-focus for now (the dual-path comparison
/// TH-7/TH-3 will exercise edge cases against the W3C manifest).
///
/// Output is sorted + deduplicated so successive calls against the
/// same (targets, data_graph) pair return a byte-identical IRI list —
/// required by the benchmark-row determinism contract TH-3 / TH-4
/// will land.
pub fn resolve_focus_nodes(targets: &[Target], data_graph_id: i64) -> Vec<String> {
    let mut focus: Vec<String> = Vec::new();
    for target in targets {
        match target {
            Target::Node(Object::Iri(iri)) => {
                focus.push(iri.as_str().to_string());
            }
            Target::Class(Object::Iri(class_iri))
            | Target::ImplicitClass(Object::Iri(class_iri)) => {
                focus.extend(spi_class_targets(class_iri.as_str(), data_graph_id));
            }
            Target::SubjectsOf(pred_iri) => {
                focus.extend(spi_subjects_of(pred_iri.as_str(), data_graph_id));
            }
            Target::ObjectsOf(pred_iri) => {
                focus.extend(spi_objects_of(pred_iri.as_str(), data_graph_id));
            }
            // Wrong* — SHACL §5.6: ill-formed targets. Skip for MVP.
            _ => {}
        }
    }
    focus.sort();
    focus.dedup();
    focus
}

/// TH-9 — Rewrite a `$this`-bearing SHACL-SPARQL constraint into a
/// self-contained SPARQL SELECT pre-bound to one focus node.
///
/// SHACL Part 2 §5.2 defines `$this` as a **pre-bound variable** that
/// every SHACL-SPARQL constraint receives. The naive "text-replace
/// `$this` with `<iri>`" fails the SPARQL 1.1 grammar — the SELECT
/// projection clause accepts variables only, never IRI terms. So a
/// constraint like `SELECT $this WHERE { $this a foaf:Person ... }`
/// must be rewritten so `$this` continues to be a variable while
/// receiving its initial binding.
///
/// Strategy:
/// 1. Replace every `$this` with the synthetic variable
///    `?_pgrdf_this` (a name unlikely to collide with user-authored
///    variables — SHACL constraints typically use short names like
///    `?value`, `?p`, `?o`).
/// 2. Inject a `VALUES ?_pgrdf_this { <focus_iri> }` block at the
///    head of the first `WHERE` clause's brace. SPARQL 1.1 §10.1
///    explicitly allows inline data via `VALUES` at the top of a
///    group graph pattern; this is the standard mechanism for
///    pre-binding values that the SHACL Part-2 evaluation semantics
///    describes.
///
/// Example — `SELECT $this WHERE { $this a foaf:Person . FILTER NOT
/// EXISTS { $this :age ?a } }` with focus `<ex:alice>` becomes:
///
/// ```sparql
/// SELECT ?_pgrdf_this WHERE {
///   VALUES ?_pgrdf_this { <http://example.org/alice> }
///   ?_pgrdf_this a foaf:Person .
///   FILTER NOT EXISTS { ?_pgrdf_this :age ?a }
/// }
/// ```
///
/// If no `WHERE` keyword is found (malformed input), pass the
/// variable-rewritten text through unmodified — the downstream
/// `pgrdf.sparql` parser will surface the real grammar error.
///
/// Naive (lexical) replacement of `$this` is sufficient for the MVP
/// and the W3C SHACL-SPARQL manifest fixtures TH-7 vendors. A future
/// refinement (TH-7 once edge cases surface) would tokenise to avoid
/// matching `$this` inside a string literal — SPARQL strings can
/// carry `$` followed by an identifier without it being a variable
/// reference.
pub fn substitute_this(sparql: &str, focus_iri: &str) -> String {
    const FOCUS_VAR: &str = "?_pgrdf_this";
    let with_var = sparql.replace("$this", FOCUS_VAR);
    inject_values_at_first_where(&with_var, focus_iri, FOCUS_VAR)
}

/// Scan for the first case-insensitive `WHERE` keyword and inject
/// `VALUES <var> { <iri> }` immediately after its opening `{`.
fn inject_values_at_first_where(sparql: &str, focus_iri: &str, focus_var: &str) -> String {
    let upper = sparql.to_uppercase();
    let where_pos = match upper.find("WHERE") {
        Some(p) => p,
        None => return sparql.to_string(),
    };
    let after_where = &sparql[where_pos + "WHERE".len()..];
    let brace_offset = match after_where.find('{') {
        Some(p) => p,
        None => return sparql.to_string(),
    };
    let split_at = where_pos + "WHERE".len() + brace_offset + 1;
    let mut out = String::with_capacity(sparql.len() + 64);
    out.push_str(&sparql[..split_at]);
    out.push_str(" VALUES ");
    out.push_str(focus_var);
    out.push_str(" { <");
    out.push_str(focus_iri);
    out.push_str("> } ");
    out.push_str(&sparql[split_at..]);
    out
}

// ───────────────────── SPI helpers (TH-9) ────────────────────────

/// Single-quote escape for inline SQL string literals. IRIs from a
/// compiled `IRSchema` are already validated as well-formed URIs by
/// the `ShaclDataManager::load` pass; this exists strictly as
/// belt-and-braces hygiene so a corrupt-but-loadable IRI cannot
/// inject SQL through the JOIN predicates below.
fn esc(s: &str) -> String {
    s.replace('\'', "''")
}

/// Resolve `sh:targetClass <class>` (and `sh:targetImplicitClass`)
/// against the data graph — every subject with `?s rdf:type <class>`.
fn spi_class_targets(class_iri: &str, data_graph_id: i64) -> Vec<String> {
    let sql = format!(
        "SELECT DISTINCT s.lexical_value
         FROM pgrdf._pgrdf_quads q
         JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
         JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
         JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
         WHERE q.graph_id = $1
           AND p.term_type = 1 AND p.lexical_value = '{}'
           AND o.term_type = 1 AND o.lexical_value = '{}'
           AND s.term_type = 1",
        esc(RDF_TYPE),
        esc(class_iri),
    );
    spi_collect_lexicals(&sql, data_graph_id)
}

/// Resolve `sh:targetSubjectsOf <pred>` — every distinct subject of
/// `?s <pred> ?o` in the data graph.
fn spi_subjects_of(pred_iri: &str, data_graph_id: i64) -> Vec<String> {
    let sql = format!(
        "SELECT DISTINCT s.lexical_value
         FROM pgrdf._pgrdf_quads q
         JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
         JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
         WHERE q.graph_id = $1
           AND p.term_type = 1 AND p.lexical_value = '{}'
           AND s.term_type = 1",
        esc(pred_iri),
    );
    spi_collect_lexicals(&sql, data_graph_id)
}

/// Resolve `sh:targetObjectsOf <pred>` — every distinct object of
/// `?s <pred> ?o` whose object is itself an IRI (literals / blanks
/// skipped per SHACL §5.5).
fn spi_objects_of(pred_iri: &str, data_graph_id: i64) -> Vec<String> {
    let sql = format!(
        "SELECT DISTINCT o.lexical_value
         FROM pgrdf._pgrdf_quads q
         JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
         JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
         WHERE q.graph_id = $1
           AND p.term_type = 1 AND p.lexical_value = '{}'
           AND o.term_type = 1",
        esc(pred_iri),
    );
    spi_collect_lexicals(&sql, data_graph_id)
}

/// One-column SPI scan — column #1 is a lexical IRI string.
fn spi_collect_lexicals(sql: &str, data_graph_id: i64) -> Vec<String> {
    let mut out = Vec::new();
    Spi::connect(|client| {
        let table = client
            .select(
                sql,
                None,
                &[unsafe {
                    pgrx::datum::DatumWithOid::new(
                        data_graph_id,
                        pgrx::pg_sys::PgBuiltInOids::INT8OID.into(),
                    )
                }],
            )
            .expect("pgrdf_sparql: target-resolution SPI failed");
        for row in table {
            if let Some(iri) = row.get::<String>(1).ok().flatten() {
                out.push(iri);
            }
        }
    });
    out
}

/// Dispatch the rewritten SPARQL through `pgrdf.sparql` and collect
/// every binding row as JSONB. Empty result-set ⇒ no violations.
///
/// SPARQL parse errors are recorded as a single synthetic row with
/// an `_error` key so the surrounding violation builder can surface
/// the failure without crashing the whole `validate()` call — a
/// SHACL-SPARQL constraint that fails to parse is itself a
/// data-quality signal worth reporting.
fn call_pgrdf_sparql_spi(query: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let escaped = query.replace('\'', "''");
    let sql = format!("SELECT * FROM pgrdf.sparql('{escaped}')");
    Spi::connect(|client| match client.select(&sql, None, &[]) {
        Ok(table) => {
            for row in table {
                if let Some(json) = row.get::<pgrx::JsonB>(1).ok().flatten() {
                    out.push(json.0);
                }
            }
        }
        Err(e) => {
            out.push(json!({ "_error": format!("{e}") }));
        }
    });
    out
}

/// Shape one SPARQL binding row into a `sh:ValidationResult` JSONB.
///
/// SHACL §4.6 defines the result fields. The pgRDF-native path
/// supplies the focus node from the substitution context (it's
/// already in the URL we ran the query for), and lifts an optional
/// `?value` binding into the `value` slot. `resultPath` mirrors the
/// owning shape's path if it has one (property shape), else null.
/// `resultMessage` prefers the SPARQL constraint's `sh:message`
/// over any per-shape message; this matches rudof's `NativeEngine`
/// shape.
fn build_violation(shape: &IRShape, sparql: &BasicSparql, focus_iri: &str, row: &Value) -> Value {
    let source_shape = match shape.id() {
        Object::Iri(iri) => Value::String(iri.as_str().to_string()),
        Object::BlankNode(label) => Value::String(format!("_:{label}")),
        _ => Value::Null,
    };
    let result_path = match shape.path() {
        Some(SHACLPath::Predicate { pred }) => Value::String(pred.as_str().to_string()),
        Some(other) => Value::String(format!("{other}")),
        None => Value::Null,
    };
    let result_message = sparql
        .message()
        .and_then(|m| m.iter().next().map(|(_, msg)| Value::String(msg.clone())))
        .unwrap_or(Value::Null);
    let result_severity = match shape.severity() {
        Severity::Trace => Value::String("sh:Trace".to_string()),
        Severity::Debug => Value::String("sh:Debug".to_string()),
        Severity::Info => Value::String("sh:Info".to_string()),
        Severity::Warning => Value::String("sh:Warning".to_string()),
        Severity::Violation => Value::String("sh:Violation".to_string()),
        Severity::Generic(iri) => Value::String(iri.as_str().to_string()),
    };
    // ?value binding lifted into `value`, if present. The shape from
    // `pgrdf.sparql` is `{"varname": {"type":"iri|literal|bnode",
    // "value": "..."}}`; pull whatever's under `value` opaquely.
    let value = row.get("value").cloned().unwrap_or(Value::Null);

    json!({
        "focusNode":      focus_iri,
        "resultPath":     result_path,
        "sourceShape":    source_shape,
        "resultMessage":  result_message,
        "resultSeverity": result_severity,
        "value":          value,
        "sourceConstraintComponent": SH_SPARQL_CONSTRAINT_COMPONENT,
    })
}

#[cfg(test)]
mod th11_walk_schema_unit_tests {
    use super::walk_schema_for_sparql;
    use prefixmap::PrefixMap;
    use shacl::ir::IRSchema;

    /// An empty `IRSchema` (no shapes, no components) yields an empty
    /// extraction vector. Establishes the function shape + the
    /// "empty in, empty out" baseline. Full-schema extraction with
    /// real `BasicSparql` constraints is covered once TH-8 wires the
    /// end-to-end path through `pgrdf.validate(..., 'pgrdf')` and the
    /// pgrx tests / regression fixtures land per
    /// SPEC.ROADMAP.TRACK.TASKS §8 TH-9 / TH-7.
    #[test]
    fn empty_schema_yields_empty_vec() {
        let schema = IRSchema::new(PrefixMap::new());
        let extracted = walk_schema_for_sparql(&schema);
        assert!(
            extracted.is_empty(),
            "empty IRSchema must yield zero (shape, sparql) pairs; got {} pair(s)",
            extracted.len()
        );
    }
}

#[cfg(test)]
mod th9_substitute_this_unit_tests {
    use super::substitute_this;

    /// `$this` is rewritten to the synthetic variable `?_pgrdf_this`
    /// and a `VALUES ?_pgrdf_this { <iri> }` pre-binding is injected
    /// at the head of the WHERE clause. The result is well-formed
    /// SPARQL 1.1 that the pgRDF SPARQL parser accepts (a
    /// `SELECT <iri>` projection — what naive text-substitution
    /// would produce — is rejected by the SPARQL grammar).
    #[test]
    fn single_substitution_uses_values_prebinding() {
        let q = "SELECT $this WHERE { $this a ?c }";
        let out = substitute_this(q, "http://example.org/alice");
        // Note: double space after `} ` is harmless whitespace — the
        // SPARQL grammar is whitespace-insensitive. The test pins the
        // exact bytes so future regressions surface immediately.
        assert_eq!(
            out,
            "SELECT ?_pgrdf_this WHERE { VALUES ?_pgrdf_this \
             { <http://example.org/alice> }  ?_pgrdf_this a ?c }"
        );
    }

    /// Zero `$this` occurrences: `WHERE { ... }` still gets the
    /// VALUES pre-binding (harmlessly unused) so the rewrite is
    /// uniform; a SHACL-SPARQL constraint that doesn't reference
    /// `$this` is unusual but legal — must not be mangled.
    #[test]
    fn zero_substitutions_still_injects_values() {
        let q = "SELECT ?s ?p ?o WHERE { ?s ?p ?o }";
        let out = substitute_this(q, "http://example.org/x");
        assert_eq!(
            out,
            "SELECT ?s ?p ?o WHERE { VALUES ?_pgrdf_this \
             { <http://example.org/x> }  ?s ?p ?o }"
        );
    }

    /// IRI with query string + fragment characters round-trips into
    /// the VALUES block intact (no escaping needed inside SPARQL
    /// `<…>` IRI delimiters — the SPARQL grammar allows almost any
    /// character there).
    #[test]
    fn iri_with_query_string_round_trips_into_values() {
        let q = "SELECT $this WHERE { $this ?p ?o }";
        let out = substitute_this(q, "http://example.org/x?q=1&r=2#frag");
        assert_eq!(
            out,
            "SELECT ?_pgrdf_this WHERE { VALUES ?_pgrdf_this \
             { <http://example.org/x?q=1&r=2#frag> }  ?_pgrdf_this ?p ?o }"
        );
    }

    /// Case-insensitive `WHERE` (lowercase `where`) — SPARQL keywords
    /// are case-insensitive per W3C §4.2.
    #[test]
    fn lowercase_where_keyword_is_recognised() {
        let q = "SELECT $this where { $this a ?c }";
        let out = substitute_this(q, "http://example.org/x");
        assert!(
            out.contains("VALUES ?_pgrdf_this { <http://example.org/x> }"),
            "lowercase where: VALUES block missing; got: {out}"
        );
    }

    /// No `WHERE` keyword in input: pass through with `$this` →
    /// `?_pgrdf_this` rewrite only. Downstream parser surfaces the
    /// real grammar error.
    #[test]
    fn missing_where_passes_through_with_var_rewrite() {
        let q = "SELECT $this { $this a ?c }";
        let out = substitute_this(q, "http://example.org/x");
        assert_eq!(out, "SELECT ?_pgrdf_this { ?_pgrdf_this a ?c }");
    }
}
