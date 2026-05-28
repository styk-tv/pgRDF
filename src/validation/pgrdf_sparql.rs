//! Track H Architecture-1 — pgRDF-native SHACL-SPARQL execution.
//!
//! TH-12 SCAFFOLD only. The full implementation lands across
//! TH-11 → TH-8 per `_WIP/SPEC.ROADMAP.TRACK.TASKS.v1.0-devel.md §10
//! TH-12 native handler design`.
//!
//! ## Goal
//!
//! Provide a third validation backend (`mode => 'pgrdf'`) alongside
//! `'native'` (rudof in-memory) and `'sparql'` (rudof endpoint-shaped).
//! For shapes that carry `IRComponent::Sparql` constraints
//! (`sh:sparql [ sh:select "…" ]` — the SHACL Part-2 vocabulary that
//! `shacl 0.3.2` now parses), pgRDF intercepts the constraint, walks
//! the focus-node set produced by the shape's targets, substitutes
//! `$this` per focus node, and executes through `pgrdf.sparql` — the
//! same dictionary-indexed hexastore path that already powers
//! `pgrdf.sparql` and `pgrdf.construct`. Core constraints continue to
//! evaluate through rudof's `NativeEngine`; only the `Sparql` variant
//! is intercepted.
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
//! ## Module shape (locked in TH-12)
//!
//! - **Public entry point** (this file): `run_pgrdf_sparql(data_g,
//!   shapes_g) → serde_json::Value`. Returns a ValidationReport in
//!   the same JSON shape as `'native'` / `'sparql'`.
//! - **Mode name**: `'pgrdf'` (locked; alternatives `'pg'`, `'sql'`,
//!   `'native-sql'`, `'fast'` rejected).
//! - **Schema walk** (TH-11/TH-10): `walk_schema_for_sparql(schema)`
//!   returns `Vec<(IRShape, IRComponent::Sparql)>`. Iterates the
//!   compiled `IRSchema`; collects only the SPARQL constraints.
//! - **Per-shape evaluation** (TH-9): for each `(shape, sparql)`,
//!   resolve the shape's target set against the data graph
//!   (`target_node`, `target_class`, `target_subject_of`,
//!   `target_object_of`, `implicit_target_class`); for each focus
//!   node, dict-lookup its lexical, substitute `$this` in the
//!   `sh:select` text, run `pgrdf.sparql`, map result rows to
//!   `sh:ValidationResult` JSONB.
//! - **Dispatcher integration** (TH-8): a third arm in
//!   `validate()`'s `match mode` calls
//!   `pgrdf_sparql::run_pgrdf_sparql(...)`. Until TH-8, this scaffold
//!   is unreachable from SQL — `validate()` continues to accept only
//!   `'native'` / `'sparql'`.
//!
//! ## What this scaffold delivers (TH-12 acceptance)
//!
//! - Module exists at the locked path.
//! - Public function signature matches the dispatcher's eventual
//!   call site.
//! - Body returns a deterministic placeholder so call sites can be
//!   stubbed without spurious test failures.
//! - The `_status` field in the response makes it impossible to
//!   silently ship an unfinished `'pgrdf'` mode — if a future commit
//!   wires this in without removing the `_status` marker, regression
//!   tests will surface the placeholder shape.

use serde_json::{json, Value};
use shacl::ir::components::BasicSparql;
use shacl::ir::{IRComponent, IRSchema, IRShape};

/// TH-12 scaffold. Execute the pgRDF-native SHACL-SPARQL path.
///
/// Returns a deterministic placeholder JSONB until TH-10 → TH-8 land:
///
/// ```text
/// {
///   "conforms": true,
///   "results":  [],
///   "mode":     "pgrdf",
///   "_status":  "scaffold (TH-12); implementation pending TH-10..TH-8"
/// }
/// ```
///
/// **Not yet wired** into `validate()` — the dispatcher still
/// short-circuits to the existing `'native'` / `'sparql'` arms until
/// TH-8 lands.
#[allow(dead_code)]
pub fn run_pgrdf_sparql(_data_graph_id: i64, _shapes_graph_id: i64) -> Value {
    json!({
        "conforms": true,
        "results": [],
        "mode": "pgrdf",
        "_status": "scaffold (TH-12); implementation pending TH-10..TH-8"
    })
}

/// TH-11 — Extract every `IRComponent::BasicSparql` constraint from a
/// compiled `IRSchema`, paired with the shape that owns it.
///
/// The upstream variant the SPEC originally named `IRComponent::Sparql`
/// is actually `IRComponent::BasicSparql` in `shacl 0.3.2`; the wrapped
/// value is a `BasicSparql` struct exposing `.select() -> &String`
/// (the raw SPARQL SELECT text), `.message() -> Option<&MessageMap>`,
/// `.deactivated() -> Option<bool>`, and `.prefixes() -> Option<&PrefixMap>`.
///
/// Walk semantics (matches the v0.5 §5.3 contract a SPARQL-based
/// constraint expects):
/// - Iterates every shape via `IRSchema::iter()` — both node shapes
///   and property shapes (a property shape can carry SPARQL
///   constraints just like a node shape).
/// - Skips deactivated shapes (`shape.deactivated() == true`) — a
///   deactivated shape contributes no constraints per SHACL spec
///   §3.3 / W3C SHACL Recommendation.
/// - Skips deactivated constraints within a live shape
///   (`sparql.deactivated() == Some(true)`) — `sh:deactivated`
///   carried by the `sh:sparql` block itself.
/// - Returns owned values (clones) so the caller does not need to
///   hold the schema borrow across the per-shape SPI loop in
///   `run_pgrdf_sparql` (TH-10 / TH-9 / TH-8 spawn SPI scans per
///   focus node; a held borrow would conflict with the SPI runtime).
///
/// Output ordering matches `IRSchema::iter()` (insertion order of the
/// IR builder), so successive calls against the same schema are
/// deterministic — important once TH-3 / TH-4 lock LUBM benchmark
/// comparison rows.
#[allow(dead_code)]
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

#[cfg(test)]
mod th11_walk_schema_unit_tests {
    use super::walk_schema_for_sparql;
    use prefixmap::PrefixMap;
    use shacl::ir::IRSchema;

    /// An empty `IRSchema` (no shapes, no components) yields an empty
    /// extraction vector. Establishes the function shape + the
    /// "empty in, empty out" baseline. Full-schema extraction with
    /// real `BasicSparql` constraints is covered once TH-9 / TH-8 wire
    /// the end-to-end path through `pgrdf.validate(..., 'pgrdf')` and
    /// the pgrx tests / regression fixtures land per
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
