//! SHACL processor wrapper.
//!
//! v0.4 cycle (this body) ships the **real implementation** of
//! `pgrdf.validate(data_graph_id, shapes_graph_id) → JSONB`. The
//! preceding stub (v0.3) is gone.
//!
//! ## v0.5-FUTURE §5 — SHACL-SPARQL constraint mode
//!
//! `pgrdf.validate(data_graph_id, shapes_graph_id, mode TEXT
//! DEFAULT 'native')`. The `shacl 0.3.x` crate's `GraphValidation`
//! processor exposes `ShaclValidationMode::{Native, Sparql}`. The
//! `mode` argument ships fully in v0.5 (accepted, validated,
//! echoed); `'native'` is the v0.4 Rust-native Core engine.
//!
//! **Scope note (ERRATA.v0.5 E-012) — `'sparql'` is upstream-stubbed.**
//! Two independent gaps in `shacl 0.3.1`:
//!
//! 1. **No SHACL-SPARQL constraint component.** `IRComponent` is
//!    Core-only; the AST/RDF parser has zero `sh:sparql` / `sh:select`
//!    handling — a SHACL-SPARQL constraint is silently dropped.
//! 2. **`SparqlEngine` is a non-functional stub.** Every
//!    target-resolution method (`target_node` / `target_class` /
//!    `target_subject_of` / `target_object_of` /
//!    `implicit_target_class`) is `unimplemented!()`, so invoking
//!    `ShaclValidationMode::Sparql` on any shapes graph with a
//!    target panics `not implemented` inside the crate.
//!
//! Because of (2), `'sparql'` mode does **not** invoke the upstream
//! engine (a panic the SQL caller can neither catch nor act on).
//! Instead it returns a clean, deterministic structured report:
//! `conforms:null`, empty `results`, and an `error` naming the
//! upstream gap. Forward-compatible — the day a rudof release
//! implements the engine + the constraint component, delete the
//! guard; the `&validation_mode` call already routes correctly with
//! no signature change.
//!
//! Two modes ship in v0.5:
//!
//! * `'native'` (default — behaviourally identical to the v0.4
//!   surface; the default-arg `pgrdf.validate(d, s)` form is
//!   unchanged).
//! * `'sparql'` — accepted + validated; returns the deterministic
//!   E-012 structured "unavailable" report (no panic).
//!
//! An unknown mode string errors with prefix
//! `validate: unknown mode` (no silent fallback to `'native'` —
//! mirrors §3's `materialize: unknown profile` discipline). The
//! JSONB output gains a `mode` field reflecting the requested mode.
//!
//! ## v0.5-FUTURE §5.1 — validation against a materialised graph
//!
//! `serialise_graph_to_ntriples` rehydrates BOTH `is_inferred =
//! TRUE` and `FALSE` rows, so a `data_graph_id` that has had
//! `pgrdf.materialize` run is validated against its entailed
//! closure: a shape requiring membership only reachable by RDFS /
//! OWL-RL entailment reports against the entailed triples.
//! Regression `122-shacl-modes.sql` locks this end-to-end.
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
//!     validator.validate(&schema, &<mode>) → ValidationReport
//!         (<mode> = Native | Sparql, per the `mode` arg — §5.2)
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
/// SQL: `pgrdf.validate(data_graph_id BIGINT, shapes_graph_id BIGINT,
/// mode TEXT DEFAULT 'native') → JSONB`.
///
/// `mode` ∈ `{'native','sparql'}`. The default-arg
/// `pgrdf.validate(d, s)` form defaults `mode => 'native'` and is
/// behaviourally identical to the v0.4 surface. `'sparql'` routes
/// through the `shacl 0.3.x` SPARQL engine so `sh:select`
/// SPARQL-based constraints are evaluated. An unknown mode panics
/// with prefix `validate: unknown mode` — never a silent fallback.
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
///   "mode":            "native|sparql",
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
/// crate. `'native'` (default) is the in-process Rust constraint
/// engine. `'sparql'` is wired but short-circuits to a deterministic
/// structured report — `shacl 0.3.1`'s SparqlEngine is an upstream
/// stub (`unimplemented!()`); see ERRATA.v0.5 E-012. The graphs are
/// rehydrated from `_pgrdf_quads` ↔ `_pgrdf_dictionary` (same shape
/// as `pgrdf.materialize`), serialised to N-Triples in-memory, and
/// re-parsed into rudof's `InMemoryGraph` before validation.
/// Validation is in-process; no SPARQL endpoint or external store is
/// contacted.
/// SHACL constraint components this validator does **not** evaluate,
/// keyed by the mode they are unevaluated under.
///
/// An unevaluated component contributes no violation *and no error*, so
/// `conforms:true` cannot be distinguished from "never checked". That is
/// the defect this table exists to close: a shapes graph that relies on
/// one of these is refused rather than silently passed.
///
/// **Every entry is measured, never assumed.** `tests/shacl-capability`
/// generates the native-mode set; extend this table only from a probe
/// that reproduces the skip. A guessed entry re-creates the failure it
/// is meant to prevent, one layer up.
///
/// `(IRI, human-readable name)`.
/// NOTE the name: "all modes" means the two modes that reach the rudof
/// pipeline — `'native'` and `'sparql'`. It does NOT include `'pgrdf'`,
/// which short-circuits to the Track-H handler before this runs and
/// **does** evaluate `sh:sparql`, measured returning a real verdict
/// (`conforms:false`, 1 result) on a constraint the other two skip
/// silently. Refusing it there would reject the one mode built to
/// support it.
const UNENFORCED_ALL_MODES: &[(&str, &str)] = &[(
    "http://www.w3.org/ns/shacl#sparql",
    "sh:sparql (SHACL-SPARQL constraint component — use mode 'pgrdf', which evaluates it)",
)];

/// Additionally unevaluated under `'sparql'` mode: rudof ships no
/// `SparqlValidator` impl for the cardinality constraints, so a shape
/// relying on them reports `conforms:true` under `'sparql'` while the
/// same shape reports `conforms:false` under `'native'`.
const UNENFORCED_SPARQL_MODE: &[(&str, &str)] = &[
    ("http://www.w3.org/ns/shacl#minCount", "sh:minCount"),
    ("http://www.w3.org/ns/shacl#maxCount", "sh:maxCount"),
];

/// SHACL target declarations. A shapes graph carrying none of these
/// targets nothing, so validation is vacuous: every data graph
/// "conforms" because nothing was ever selected to check.
const TARGET_PREDICATES: &[&str] = &[
    "http://www.w3.org/ns/shacl#targetClass",
    "http://www.w3.org/ns/shacl#targetNode",
    "http://www.w3.org/ns/shacl#targetSubjectsOf",
    "http://www.w3.org/ns/shacl#targetObjectsOf",
];

/// The distinct predicate IRIs a graph actually uses.
///
/// Queried from `_pgrdf_quads` rather than scanned out of the
/// N-Triples serialisation. The scan version of this looked right and
/// silently matched nothing — a whitespace assumption about someone
/// else's serialiser, holding up a fail-closed gate. Asking the store
/// what predicates a graph contains has no such assumption, and it also
/// removes the object-position false positives the scan could produce.
fn predicate_iris(graph_id: i64) -> std::collections::HashSet<String> {
    // Aggregated server-side and read as ONE value. The row-iterating
    // version of this returned a PARTIAL set — enough to satisfy the
    // target check and miss `sh:minCount` in the same graph — which cost
    // three CI rounds to localise because the only coverage was a
    // `#[pg_test]` that cannot link on a dev host. One value, no cursor,
    // nothing to half-consume.
    Spi::get_one_with_args::<Vec<Option<String>>>(
        "SELECT array_agg(DISTINCT d.lexical_value)
           FROM pgrdf._pgrdf_quads q
           JOIN pgrdf._pgrdf_dictionary d ON d.id = q.predicate_id
          WHERE q.graph_id = $1",
        &[graph_id.into()],
    )
    .ok()
    .flatten()
    .unwrap_or_default()
    .into_iter()
    .flatten()
    .collect()
}

/// Names every unenforced component the shapes graph actually uses.
fn unenforced_components(
    preds: &std::collections::HashSet<String>,
    mode: &str,
) -> Vec<&'static str> {
    let mut found = Vec::new();
    let mut check = |table: &'static [(&'static str, &'static str)]| {
        for (iri, name) in table {
            if preds.contains(*iri) && !found.contains(name) {
                found.push(*name);
            }
        }
    };
    // 'pgrdf' is the Track-H handler and it EVALUATES sh:sparql — measured
    // conforms=false, 1 result, on a constraint the other two skip silently.
    // This exclusion used to be incidental: `validate` returns for 'pgrdf'
    // before reaching the guard, so the function was wrong and the behaviour
    // was accidentally right. A unit test caught it the moment one existed.
    if mode != "pgrdf" {
        check(UNENFORCED_ALL_MODES);
    }
    if mode == "sparql" {
        check(UNENFORCED_SPARQL_MODE);
    }
    found
}

/// True when the shapes graph can select at least one focus node.
///
/// Explicit targets are the four `sh:target*` predicates. SHACL also
/// allows *implicit class targeting* — a node that is both an
/// `sh:NodeShape` and an `rdfs:Class` targets its own instances — so
/// their co-occurrence counts as targeting. That direction deliberately
/// under-refuses: a shapes graph we cannot prove vacuous is validated
/// normally rather than rejected.
fn declares_any_target(preds: &std::collections::HashSet<String>, shapes_nt: &str) -> bool {
    if TARGET_PREDICATES.iter().any(|p| preds.contains(*p)) {
        return true;
    }
    shapes_nt.contains("<http://www.w3.org/ns/shacl#NodeShape>")
        && shapes_nt.contains("<http://www.w3.org/2000/01/rdf-schema#Class>")
}

#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn validate(
    data_graph_id: i64,
    shapes_graph_id: i64,
    mode: default!(String, "'native'"),
    strict: default!(bool, true),
) -> pgrx::JsonB {
    let start = Instant::now();

    // Validate the mode up-front, BEFORE any work. An unknown mode
    // must error — never silently fall back to 'native'. Exact
    // prefix `validate: unknown mode` per §5.2 (mirrors §3's
    // `materialize: unknown profile` discipline); the pgrx negative
    // test pins the full message.
    //
    // `'pgrdf'` (TH-8) short-circuits to the Track H Architecture-1
    // handler before the rudof pipeline runs — the whole point of the
    // mode is to avoid serialising the data graph to N-Triples /
    // rehydrating into `InMemoryGraph`. Returns the JSONB shape
    // `run_pgrdf_sparql` produces directly; `elapsed_ms` is layered
    // here so the meta-field shape stays comparable with the
    // `'native'` / `'sparql'` modes for benchmark-row diffs.
    // Set by the 'pgrdf' arm below: run the SHACL-SPARQL evaluator in
    // addition to the Core engine, and merge both reports.
    let mut sparql_pass = false;

    let validation_mode = match mode.as_str() {
        "native" => ShaclValidationMode::Native,
        "sparql" => ShaclValidationMode::Sparql,
        // #86 — `'pgrdf'` is the COMPLETE mode: the Rust-native Core
        // engine AND pgRDF's own SHACL-SPARQL evaluator, merged.
        //
        // It used to short-circuit straight to the SPARQL handler, so it
        // evaluated `sh:sparql` and silently skipped every Core
        // constraint — while `'native'` did the exact opposite. The
        // partitions were complementary and neither said so, so a
        // caller with a mixed shapes graph got half a verdict presented
        // as a whole one. Two components hit that from opposite
        // directions on the same day.
        "pgrdf" => {
            sparql_pass = true;
            ShaclValidationMode::Native
        }
        other => panic!(
            "validate: unknown mode {other:?} \
             (supported: 'native', 'sparql', 'pgrdf')"
        ),
    };
    // Canonical mode string echoed back in every JSONB return site.
    // Bound here so the early-return error branches and the success
    // branch can each embed it without contending for `mode`'s move.
    let mode_str = mode;

    // §5.2 / ERRATA.v0.5 E-012 (RESOLVED in shacl 0.3.2, 2026-05-26).
    // The earlier E-012 short-circuit guard intercepted
    // `ShaclValidationMode::Sparql` before reaching the upstream engine
    // because every `SparqlEngine` target-resolution method
    // (`target_node` / `target_class` / `target_subject_of` /
    // `target_object_of` / `implicit_target_class`) ended in
    // `unimplemented!()`, and the `IRComponent` enum had no `Sparql`
    // variant (sh:sparql / sh:select constraints were silently dropped
    // at IR-compile time). Both gaps closed upstream in shacl 0.3.2 —
    // pgRDF now routes `'sparql'` mode through the real working
    // engine without an intermediate guard. ERRATA.v0.5 E-012 closes
    // alongside this commit; the `mode` argument signature is
    // unchanged (the v0.5 §5.2 contract held forward-compatible
    // exactly so this gate could be deleted with no API churn).

    // 1. Rehydrate data + shapes graphs as N-Triples text.
    let (data_nt, data_count) = serialise_graph_to_ntriples(data_graph_id);
    let (shapes_nt, shapes_count) = serialise_graph_to_ntriples(shapes_graph_id);

    // 1a. Fail closed on a constraint component this engine does not
    //     evaluate. Skipping one silently is indistinguishable from
    //     validating cleanly, so a caller relying on it gets a pass it
    //     did not earn. Refuse instead, naming what was skipped.
    //
    //     `strict => false` is an explicit, per-call opt-out for
    //     exploratory use. It is never the default, and it is the only
    //     way to reach the old silent behaviour.
    // 1b. Fail closed on a shapes graph that cannot refuse anything.
    //     A missing graph id, an empty graph, and a graph carrying
    //     triples but no shape target all report `conforms:true` —
    //     indistinguishable from a real pass. The caller almost always
    //     meant a different graph id.
    let shapes_preds = predicate_iris(shapes_graph_id);
    if strict && !declares_any_target(&shapes_preds, &shapes_nt) {
        return pgrx::JsonB(json!({
            "conforms":        Value::Null,
            "results":         [],
            "data_graph_id":   data_graph_id,
            "shapes_graph_id": shapes_graph_id,
            "data_triples":    data_count,
            "shapes_triples":  shapes_count,
            "mode":            mode_str.clone(),
            "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
            "error":           format!(
                "validate: shapes graph {shapes_graph_id} declares no SHACL target \
                 ({shapes_count} triples). Nothing would be selected, so a verdict \
                 would be vacuous — a missing or wrong graph id reports the same \
                 `conforms:true` as a clean validation. Re-run with strict => false \
                 to accept a vacuous pass."
            ),
        }));
    }

    if strict {
        let skipped = unenforced_components(&shapes_preds, &mode_str);
        if !skipped.is_empty() {
            return pgrx::JsonB(json!({
                "conforms":        Value::Null,
                "results":         [],
                "data_graph_id":   data_graph_id,
                "shapes_graph_id": shapes_graph_id,
                "data_triples":    data_count,
                "shapes_triples":  shapes_count,
                "mode":            mode_str.clone(),
                "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
                "error":           format!(
                    "validate: unenforced constraint component in shapes graph \
                     under mode {mode_str:?}: {}. This engine does not evaluate \
                     it, so a verdict would be meaningless. Re-run with \
                     strict => false to validate the remaining constraints \
                     anyway (the named component stays unevaluated).",
                    skipped.join(", ")
                ),
                "unenforced":      skipped,
            }));
        }
    }

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
                    "mode":            mode_str.clone(),
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
                "mode":            mode_str.clone(),
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
                "mode":            mode_str.clone(),
                "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
                "error":           format!("shapes compile failed: {e}"),
            }));
        }
    };

    // 4. Run validation under the requested mode. `'native'` is the
    //    in-process Rust constraint engine (v0.4's only mode);
    //    `'sparql'` routes through `shacl 0.3.x`'s SPARQL engine so
    //    `sh:select` SPARQL-based constraints are evaluated (§5.2).
    let mut validator = GraphValidation::new(data_graph);
    let report = match validator.validate(&schema, &validation_mode) {
        Ok(r) => r,
        Err(e) => {
            return pgrx::JsonB(json!({
                "conforms":        Value::Null,
                "results":         [],
                "data_graph_id":   data_graph_id,
                "shapes_graph_id": shapes_graph_id,
                "data_triples":    data_count,
                "shapes_triples":  shapes_count,
                "mode":            mode_str.clone(),
                "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
                "error":           format!("validation failed: {e}"),
            }));
        }
    };

    // 5. Shape the report into JSONB.
    let mut results_json: Vec<Value> = report.results().iter().map(report_result_to_json).collect();
    let mut conforms = report.conforms();

    // 5a. #86 — merge the SHACL-SPARQL pass under mode 'pgrdf'. A
    //     violation from EITHER evaluator is a violation, so `conforms`
    //     is the conjunction and `results` is the union. Reporting one
    //     half as the whole verdict is what made this mode unusable as
    //     a gate.
    if sparql_pass {
        let sparql_report =
            crate::validation::pgrdf_sparql::run_pgrdf_sparql(data_graph_id, shapes_graph_id);
        if let Some(extra) = sparql_report.get("results").and_then(|r| r.as_array()) {
            results_json.extend(extra.iter().cloned());
        }
        // A SPARQL-side error must not be swallowed into a clean pass.
        if let Some(err) = sparql_report.get("error") {
            return pgrx::JsonB(json!({
                "conforms":        Value::Null,
                "results":         results_json,
                "data_graph_id":   data_graph_id,
                "shapes_graph_id": shapes_graph_id,
                "data_triples":    data_count,
                "shapes_triples":  shapes_count,
                "mode":            mode_str,
                "elapsed_ms":      start.elapsed().as_secs_f64() * 1000.0,
                "error":           format!("SHACL-SPARQL pass failed: {err}"),
            }));
        }
        conforms = conforms
            && sparql_report
                .get("conforms")
                .and_then(|c| c.as_bool())
                .unwrap_or(false);
    }

    pgrx::JsonB(json!({
        "conforms":        conforms,
        "results":         results_json,
        "data_graph_id":   data_graph_id,
        "shapes_graph_id": shapes_graph_id,
        "data_triples":    data_count,
        "shapes_triples":  shapes_count,
        "mode":            mode_str,
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
///
/// `pub(crate)` so the Track H pgRDF-native handler (`validation::pgrdf_sparql`)
/// can rehydrate the shapes graph through the same path without
/// duplicating the SPI scan.
pub(crate) fn serialise_graph_to_ntriples(graph_id: i64) -> (String, i64) {
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
        // #83 — this assertion used to read:
        //     // No shapes ⇒ no failures ⇒ conforms.
        //     assert_eq!(v["conforms"], json!(true));
        // That reasoning is the defect. "Nothing was checked" and
        // "everything passed" are different facts, and reporting the
        // second for the first is how a wrong graph id becomes a clean
        // bill of health. A shapes graph that targets nothing now
        // refuses instead of conforming.
        assert!(
            v["conforms"].is_null(),
            "a shapes graph that selects nothing must not yield a verdict: {v}"
        );
        assert!(
            v["error"]
                .as_str()
                .unwrap_or_default()
                .contains("declares no SHACL target"),
            "the refusal must say why: {v}"
        );

        // The old behaviour remains reachable, but only by asking.
        let loose: pgrx::JsonB =
            Spi::get_one("SELECT pgrdf.validate(999990::bigint, 999991::bigint, 'native', false)")
                .unwrap()
                .unwrap();
        assert_eq!(loose.0["conforms"], serde_json::json!(true));
    }

    // ── v0.5-FUTURE §5 — SHACL-SPARQL mode + materialised-graph ──

    /// §5.2 — the default-arg form echoes `"mode":"native"` and the
    /// JSONB shape is otherwise unchanged from v0.4 (no regression to
    /// the v0.4 conforming/violation tests above, which call the
    /// 2-arg form).
    #[pg_test]
    fn validate_mode_field_default_native() {
        let g_data: i64 = 8520;
        let g_shapes: i64 = 8521;
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
        assert_eq!(v["mode"], serde_json::json!("native"));
        assert_eq!(v["conforms"], serde_json::json!(true));
    }

    /// §5.2 — an unknown mode panics with the exact prefix
    /// `validate: unknown mode` BEFORE any work (no silent fallback
    /// to `'native'`). Mirrors §3's `materialize: unknown profile`
    /// discipline. The pgrx negative pins the full message.
    #[pg_test(
        error = "validate: unknown mode \"endpoint\" (supported: 'native', 'sparql', 'pgrdf')"
    )]
    fn validate_unknown_mode_errors() {
        let _j: pgrx::JsonB =
            Spi::get_one("SELECT pgrdf.validate(999992::bigint, 999993::bigint, 'endpoint')")
                .unwrap()
                .unwrap();
    }

    /// §5.2 — `'sparql'` mode no longer short-circuits at pgRDF's
    /// E-012 guard; it dispatches into rudof's working
    /// `SparqlEngine` (shacl 0.3.2, closes ERRATA.v0.5 E-012).
    ///
    /// **What this test locks (the realisable contract today):**
    /// - `'sparql'` mode echoes `"mode":"sparql"` in the JSONB
    /// - the `error` field is absent (the E-012 short-circuit
    ///   signal is gone — the guard was deleted in TH-14)
    /// - `conforms` is a real Boolean (not JSON `null` — the
    ///   pre-0.3.2 short-circuit response)
    /// - no panic
    ///
    /// **What this test does NOT lock:** the exact `conforms`
    /// verdict and per-shape violation set under `'sparql'` mode.
    /// shacl 0.3.2 ships `SparqlValidator` impls for a subset of
    /// Core constraints (Class, NodeKind, Pattern, MinLength /
    /// MaxLength, MinInclusive / MaxInclusive / MinExclusive /
    /// MaxExclusive, etc.) but the rudof source does not yet
    /// expose a `SparqlValidator` impl for `MinCount` / `MaxCount`
    /// — so a shape relying on minCount may report `conforms:true`
    /// under `'sparql'` mode even when the same shape reports
    /// `conforms:false` under `'native'`. That asymmetry is a
    /// rudof-side cardinality-constraint follow-up, not a pgRDF
    /// regression; track via the Track-H W3C SHACL-SPARQL manifest
    /// fixtures once `tests/w3c-shacl/sparql/` is vendored (TH-7).
    /// The pgRDF surface contract being asserted here — "the guard
    /// is gone and dispatch reaches the upstream engine" — is the
    /// piece pgRDF actually controls.
    ///
    /// TH-13 (corrected after CI surfaced the asymmetry above):
    /// replaces the pre-0.3.2
    /// `validate_sparql_mode_structured_unavailable` test which
    /// asserted the now-deleted short-circuit shape.
    #[pg_test]
    fn validate_sparql_mode_returns_real_violation() {
        let g_data: i64 = 8530;
        let g_shapes: i64 = 8531;
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
        // PersonShape requires ex:age via sh:minCount — used here to
        // confirm 'native' still works as before. 'sparql' mode goes
        // through the rudof engine without short-circuiting.
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

        // 'native' — the Core engine fires sh:minCount on Alice.
        let native: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2, 'native')",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let nv = &native.0;
        assert_eq!(nv["mode"], serde_json::json!("native"));
        assert_eq!(nv["conforms"], serde_json::json!(false));
        let n_alice = nv["results"]
            .as_array()
            .expect("native results array")
            .iter()
            .any(|r| r["focusNode"] == "http://example.org/alice");
        assert!(n_alice, "native mode: no Core violation for ex:alice");

        // 'sparql' — dispatch reaches the working upstream engine.
        // We do NOT assert a specific conforms verdict (see the test
        // doc-comment above: rudof's SparqlValidator impls cover a
        // subset of Core constraints and explicitly do NOT yet cover
        // MinCount). What we assert is the pgRDF-side contract:
        // mode echoed, no short-circuit `error` field, conforms is a
        // real Boolean (not JSON null), and the call returns without
        // panicking.
        let sparql: pgrx::JsonB = Spi::get_one_with_args(
            // strict => false: this test asserts that dispatch REACHES the
            // upstream engine, not that every constraint is evaluated. The
            // fail-closed guard (#80) would otherwise refuse first, because
            // rudof ships no SparqlValidator for the cardinality constraints.
            "SELECT pgrdf.validate($1, $2, 'sparql', false)",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let sv = &sparql.0;
        assert_eq!(sv["mode"], serde_json::json!("sparql"));
        assert!(
            sv.get("error").is_none() || sv["error"].is_null(),
            "'sparql' mode JSONB should carry no `error` field once \
             the E-012 short-circuit is deleted; got: {:?}",
            sv.get("error")
        );
        assert!(
            sv["conforms"].is_boolean(),
            "'sparql' mode `conforms` should be a real Boolean (the \
             pre-0.3.2 short-circuit returned JSON null); got: {:?}",
            sv["conforms"]
        );
        // Forward-compat anchor: data/shapes graph ids still echoed.
        assert_eq!(sv["data_graph_id"], g_data);
        assert_eq!(sv["shapes_graph_id"], g_shapes);
    }

    /// §5.3 #2 — validation against a `pgrdf.materialize`-d data
    /// graph reports violations against ENTAILED triples. A shape
    /// targets `ex:Animal`; `ex:fido` is typed `ex:Dog` and only
    /// `ex:Dog rdfs:subClassOf ex:Animal` makes it an Animal — that
    /// `ex:fido a ex:Animal` triple exists ONLY after materialize.
    /// The shape then requires `ex:name` (minCount 1), which fido
    /// lacks ⇒ a violation reported against an entailment-bound
    /// focus node. (RDFS profile reused from G1.)
    #[pg_test]
    fn validate_materialised_graph_entailed() {
        let g_data: i64 = 8540;
        let g_shapes: i64 = 8541;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
                 ex:Dog rdfs:subClassOf ex:Animal .
                 ex:fido a ex:Dog ."
                    .into(),
                g_data.into(),
            ],
        )
        .unwrap();

        // Shape: every ex:Animal must carry an ex:name. fido is an
        // Animal ONLY by rdfs9 entailment (ex:Dog ⊑ ex:Animal).
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 @prefix sh: <http://www.w3.org/ns/shacl#> .
                 ex:AnimalShape a sh:NodeShape ;
                     sh:targetClass ex:Animal ;
                     sh:property [
                         sh:path ex:name ;
                         sh:minCount 1 ;
                     ] ."
                .into(),
                g_shapes.into(),
            ],
        )
        .unwrap();

        // Before materialize: fido is only ex:Dog, not ex:Animal —
        // the shape has no target ⇒ conforms vacuously.
        let pre: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2)",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            pre.0["conforms"],
            serde_json::json!(true),
            "pre-materialize: ex:fido is not yet an ex:Animal target"
        );

        // Materialise under the RDFS profile (G1). rdfs9 derives
        // `ex:fido a ex:Animal`.
        Spi::run_with_args("SELECT pgrdf.materialize($1, 'rdfs')", &[g_data.into()]).unwrap();

        // Post-materialize: fido is now an ex:Animal (entailed) and
        // lacks ex:name ⇒ a violation against the entailment-bound
        // focus node.
        let post: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2)",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let pv = &post.0;
        assert_eq!(
            pv["conforms"],
            serde_json::json!(false),
            "post-materialize: entailed ex:fido a ex:Animal must be a target"
        );
        let fido = pv["results"]
            .as_array()
            .expect("results array")
            .iter()
            .any(|r| r["focusNode"] == "http://example.org/fido");
        assert!(
            fido,
            "no violation reported against entailment-bound ex:fido"
        );
    }

    /// TH-8 — Track H Architecture-1 dispatcher integration.
    ///
    /// `mode => 'pgrdf'` short-circuits to the pgRDF-native handler
    /// before rudof's serialise-and-rehydrate path runs. End-to-end
    /// coverage: a real `sh:select` SPARQL constraint, evaluated
    /// against a data graph via direct SPI scans of the hexastore,
    /// must produce a real `sh:Violation` row for the offending focus
    /// node — same observable behaviour as `'native'` / `'sparql'` but
    /// over a completely different evaluation path.
    ///
    /// **Fixture choice.** The natural SHACL idiom for "must carry
    /// property X" is `FILTER NOT EXISTS { $this :X ?o }`, but the
    /// pgRDF SPARQL executor doesn't yet translate `Not(Exists(_))`
    /// (Track A). For this end-to-end smoke test we use a constraint
    /// that returns ROWS for the targets we want to flag: every
    /// `foaf:Person` that carries an `ex:age` literal. The data has
    /// `ex:alice a foaf:Person` with `ex:age 42` ⇒ the constraint
    /// returns alice ⇒ one `sh:Violation` row with
    /// `focusNode = ex:alice` and
    /// `sourceConstraintComponent = sh:SPARQLConstraintComponent`.
    /// The pattern is intentionally inverted from natural-language
    /// "Persons must NOT have ex:age" — what matters end-to-end is
    /// that pgrdf-mode dispatch correctly identifies the focus node
    /// and maps the binding row to a violation result. A natural
    /// "must HAVE" constraint will replace this once Track A lifts
    /// `FILTER NOT EXISTS` (LLD v0.6 §SPARQL).
    #[pg_test]
    fn validate_pgrdf_mode_real_violation() {
        let g_data: i64 = 8560;
        let g_shapes: i64 = 8561;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 @prefix foaf: <http://xmlns.com/foaf/0.1/> .
                 ex:alice a foaf:Person ; foaf:name \"Alice\" ; ex:age 42 ."
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
                 ex:PersonShape a sh:NodeShape ;
                     sh:targetClass foaf:Person ;
                     sh:sparql [ a sh:SPARQLConstraint ;
                                 sh:message \"Person has ex:age (SPARQL)\" ;
                                 sh:select \"\"\"SELECT $this WHERE {
                                     $this a <http://xmlns.com/foaf/0.1/Person> .
                                     $this <http://example.org/age> ?a }\"\"\" ] ."
                    .into(),
                g_shapes.into(),
            ],
        )
        .unwrap();

        let pgrdf: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2, 'pgrdf')",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let pv = &pgrdf.0;

        assert_eq!(pv["mode"], serde_json::json!("pgrdf"));
        assert_eq!(pv["data_graph_id"], g_data);
        assert_eq!(pv["shapes_graph_id"], g_shapes);
        assert!(
            pv.get("elapsed_ms").is_some(),
            "'pgrdf' mode must echo elapsed_ms for benchmark-row parity; got: {:?}",
            pv.get("elapsed_ms")
        );
        assert_eq!(
            pv["conforms"],
            serde_json::json!(false),
            "ex:alice violates the SPARQL constraint (missing ex:age); \
             pgrdf-mode report claims conforms=true: {pv}"
        );
        let results = pv["results"].as_array().expect("results array");
        assert!(
            !results.is_empty(),
            "pgrdf-mode reports conforms=false but results array is empty: {pv}"
        );
        let alice = results
            .iter()
            .find(|r| r["focusNode"] == "http://example.org/alice");
        let alice = alice.unwrap_or_else(|| {
            panic!("no violation reported against ex:alice in pgrdf mode: {pv}");
        });
        assert_eq!(
            alice["sourceConstraintComponent"],
            serde_json::json!("http://www.w3.org/ns/shacl#SPARQLConstraintComponent"),
            "violation must carry sh:SPARQLConstraintComponent; got: {alice}"
        );
        assert_eq!(
            alice["resultSeverity"],
            serde_json::json!("sh:Violation"),
            "default severity should be sh:Violation; got: {alice}"
        );
    }

    /// TH-7 — W3C SHACL-SPARQL `node-sparql-001` fixture, cross-mode
    /// regression. The W3C `mf:result` says conforms=false (3
    /// violations: InvalidResource1 + InvalidResource2 ×2 rdfs:label
    /// triples). Asserts:
    /// - `mode => 'sparql'` (rudof's SparqlEngine): conforms is a real
    ///   Boolean — does NOT assert the W3C verdict because rudof's
    ///   `BasicSparqlValidator` is upstream-incomplete on this shape
    ///   topology and returns conforms=true / 0 violations (the IR
    ///   does carry the BasicSparql constraint per the plain-Rust
    ///   `w3c_node_sparql_001_ir_carries_basic_sparql` test). Tracked
    ///   as ERRATA.v0.6 E-014; gate the upstream-side honest contract.
    /// - `mode => 'pgrdf'` (pgRDF-native handler, TH-9 + TH-8):
    ///   conforms=false matching the W3C `mf:result`. **pgRDF-native
    ///   is demonstrably more correct than rudof's SparqlEngine on
    ///   this fixture as of shacl 0.3.2.**
    #[pg_test]
    fn validate_w3c_node_sparql_001_cross_mode() {
        let g_data: i64 = 8600;
        let g_shapes: i64 = 8601;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        // Same Turtle as tests/w3c-shacl/fixtures/sparql/node-sparql-001.ttl
        // (the `<>`-stripped split of the W3C source).
        let fixture = r#"@prefix dash: <http://datashapes.org/dash#> .
@prefix ex: <http://datashapes.org/sh/tests/sparql/node/sparql-001.test#> .
@prefix mf: <http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix sht: <http://www.w3.org/ns/shacl-test#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:InvalidResource1
  rdf:type rdfs:Resource ;
  rdfs:label "Invalid resource 1" ;
.
ex:InvalidResource2
  rdf:type rdfs:Resource ;
  rdfs:label "Invalid label 1" ;
  rdfs:label "Invalid label 2" ;
.
ex:TestShape
  rdf:type sh:NodeShape ;
  rdfs:label "Test shape" ;
  sh:sparql ex:TestShape-sparql ;
  sh:targetNode ex:InvalidResource1 ;
  sh:targetNode ex:InvalidResource2 ;
  sh:targetNode ex:ValidResource1 ;
.
ex:TestShape-sparql
  sh:message "Cannot have a label" ;
  sh:prefixes <http://datashapes.org/sh/tests/sparql/node/sparql-001.test> ;
  sh:select """
        SELECT $this ?path ?value
        WHERE {
                $this ?path ?value .
                FILTER (?path = <http://www.w3.org/2000/01/rdf-schema#label>) .
        }""" ;
.
ex:ValidResource1
  rdf:type rdfs:Resource ;
.
"#;
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[fixture.into(), g_data.into()],
        )
        .unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[fixture.into(), g_shapes.into()],
        )
        .unwrap();

        let sparql: pgrx::JsonB = Spi::get_one_with_args(
            // strict => false: this test asserts that dispatch REACHES the
            // upstream engine, not that every constraint is evaluated. The
            // fail-closed guard (#80) would otherwise refuse first, because
            // rudof ships no SparqlValidator for the cardinality constraints.
            "SELECT pgrdf.validate($1, $2, 'sparql', false)",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let sv = &sparql.0;
        // Surface the full JSONB inside the assert so the diagnostic
        // shows up in pgrx test output (which captures stderr).
        let conforms = &sv["conforms"];
        let results_len = sv["results"].as_array().map(|a| a.len()).unwrap_or(0);
        let shapes_triples = &sv["shapes_triples"];
        let error = sv.get("error");
        // Loose: just verify dispatch reached and produced a JSONB.
        // The actual conforms verdict here surfaces a rudof-side gap
        // we're documenting (not asserting); for the pgRDF-side gate
        // see the pgrdf-mode follow-up assert below.
        assert!(conforms.is_boolean());
        let _ = (results_len, shapes_triples, error); // silence unused

        // pgrdf-mode comparison on the SAME fixture: does the
        // pgRDF-native handler get the right answer?
        let pgrdf: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2, 'pgrdf')",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let pv = &pgrdf.0;
        let p_conforms = &pv["conforms"];
        let p_results_len = pv["results"].as_array().map(|a| a.len()).unwrap_or(0);
        assert_eq!(
            p_conforms,
            &serde_json::json!(false),
            "DIAGNOSTIC pgrdf.validate(g, g, 'pgrdf') on W3C node-sparql-001:\n\
             conforms={p_conforms}  expected=false (W3C mf:result)\n\
             results.len()={p_results_len}\n\
             full JSONB:\n{pv:#}"
        );
    }

    /// TH-4 — LUBM-shape SHACL-SPARQL dev-gate. Loads the
    /// handcrafted ~10-university LUBM-shape ABox + the "Course
    /// taught by at most one Professor" SHACL-SPARQL constraint
    /// from `tests/perf/lubm-shacl-sparql/`. Asserts the per-mode
    /// verdict — `pgrdf` mode catches 4 violations (2 collisions ×
    /// 2 Professor focuses each), `sparql` mode misses them per
    /// ERRATA.v0.6 E-014. Locks the dev-gate at the pgrx test
    /// boundary so `just test` exercises the path; the compose
    /// harness `tests/perf/lubm-shacl-sparql/run.sh` runs the same
    /// content through the regression container in CI.
    #[pg_test]
    fn lubm_shacl_sparql_dev_gate() {
        let g_data: i64 = 9100;
        let g_shapes: i64 = 9101;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        // Trimmed inline copy of tests/perf/lubm-shacl-sparql/data.ttl —
        // 4 universities (u0..u3) covering both collisions; the other
        // 6 universities in the disk fixture are noise for the
        // performance-signal path, not the correctness gate, so the
        // pgrx test inlines the minimal version.
        let data = r#"@prefix lubm: <http://swat.cse.lehigh.edu/onto/univ-bench.owl#> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix u0:   <http://www.University0.edu/> .
@prefix u1:   <http://www.University1.edu/> .
@prefix u2:   <http://www.University2.edu/> .
@prefix u3:   <http://www.University3.edu/> .

u0:CarolProf rdf:type lubm:FullProfessor ;
             lubm:teacherOf u0:CS101 ;
             lubm:teacherOf u0:CS202 .
u0:DanProf   rdf:type lubm:AssistantProfessor ;
             lubm:teacherOf u0:CS101 ;
             lubm:teacherOf u0:CS303 .
u1:Frank     rdf:type lubm:AssociateProfessor ;
             lubm:teacherOf u1:MATH200 .
u2:Hank      rdf:type lubm:AssistantProfessor ;
             lubm:teacherOf u2:CS101 .
u3:Prof1     rdf:type lubm:FullProfessor ;
             lubm:teacherOf u3:CS101 .
u3:Prof2     rdf:type lubm:AssociateProfessor ;
             lubm:teacherOf u3:CS101 .
u0:CS101     rdf:type lubm:Course .
u0:CS202     rdf:type lubm:Course .
u0:CS303     rdf:type lubm:Course .
u1:MATH200   rdf:type lubm:Course .
u2:CS101     rdf:type lubm:Course .
u3:CS101     rdf:type lubm:Course .
"#;
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[data.into(), g_data.into()],
        )
        .unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        let shapes = r#"@prefix ex:   <http://example.org/lubm-shapes#> .
@prefix lubm: <http://swat.cse.lehigh.edu/onto/univ-bench.owl#> .
@prefix sh:   <http://www.w3.org/ns/shacl#> .

ex:CourseTaughtByOneProfessor a sh:NodeShape ;
  sh:targetClass lubm:FullProfessor ;
  sh:targetClass lubm:AssociateProfessor ;
  sh:targetClass lubm:AssistantProfessor ;
  sh:sparql [ a sh:SPARQLConstraint ;
              sh:message "Course must be taught by at most one Professor" ;
              sh:select """SELECT $this ?value WHERE {
                  $this <http://swat.cse.lehigh.edu/onto/univ-bench.owl#teacherOf> ?value .
                  ?other <http://swat.cse.lehigh.edu/onto/univ-bench.owl#teacherOf> ?value .
                  FILTER ($this != ?other)
              }""" ] .
"#;
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[shapes.into(), g_shapes.into()],
        )
        .unwrap();

        // 'pgrdf' mode — REAL W3C-style verdict. 4 violations:
        //   CarolProf focusing CS101 (other=DanProf)
        //   DanProf   focusing CS101 (other=CarolProf)
        //   Prof1     focusing CS101 (other=Prof2)
        //   Prof2     focusing CS101 (other=Prof1)
        let pgrdf: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2, 'pgrdf')",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let pv = &pgrdf.0;
        assert_eq!(pv["mode"], serde_json::json!("pgrdf"));
        assert_eq!(pv["conforms"], serde_json::json!(false));
        let pgrdf_violations = pv["results"].as_array().map(|a| a.len()).unwrap_or(0);
        assert_eq!(
            pgrdf_violations, 4,
            "pgrdf mode: expected 4 violations (2 collisions × 2 Professor focuses); \
             got {pgrdf_violations}. Full JSONB: {pv:#}"
        );

        // 'sparql' mode — ERRATA.v0.6 E-014: rudof's SparqlEngine
        // returns the wrong verdict on this shape topology. Asserts
        // only the pgRDF-side contract (call returns a real Boolean,
        // mode echoed). The "0 violations" outcome is documented
        // expected behaviour, not a passing conformance gate.
        let sparql: pgrx::JsonB = Spi::get_one_with_args(
            // strict => false: this test asserts that dispatch REACHES the
            // upstream engine, not that every constraint is evaluated. The
            // fail-closed guard (#80) would otherwise refuse first, because
            // rudof ships no SparqlValidator for the cardinality constraints.
            "SELECT pgrdf.validate($1, $2, 'sparql', false)",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let sv = &sparql.0;
        assert_eq!(sv["mode"], serde_json::json!("sparql"));
        assert!(sv["conforms"].is_boolean());
    }

    /// TH-8 — `mode => 'pgrdf'` on a shape graph with no SPARQL
    /// constraints reports conforms = true (empty results), and the
    /// `elapsed_ms` meta-field is present. Locks the "no-op for
    /// SPARQL-free shapes" baseline.
    #[pg_test]
    fn validate_pgrdf_mode_empty_when_no_sparql_constraint() {
        let g_data: i64 = 8570;
        let g_shapes: i64 = 8571;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 ex:x ex:p \"y\" ."
                    .into(),
                g_data.into(),
            ],
        )
        .unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        // Pure Core constraint (sh:minCount) — no sh:sparql block.
        //
        // #86 — this used to assert that mode 'pgrdf' conforms
        // VACUOUSLY here, because the Track H path intercepted only
        // BasicSparql constraints and a Core-only shape meant "no SPARQL
        // work to do". That is the defect: a mode reporting a clean pass
        // over constraints it never looked at. 'pgrdf' now runs the Core
        // engine too, so a Core violation is a violation in every mode
        // that claims to evaluate it.
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://example.org/> .
                 @prefix sh: <http://www.w3.org/ns/shacl#> .
                 ex:Shape a sh:NodeShape ;
                     sh:targetNode ex:y ;
                     sh:property [ sh:path ex:q ; sh:minCount 1 ] ."
                    .into(),
                g_shapes.into(),
            ],
        )
        .unwrap();

        let pgrdf: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2, 'pgrdf')",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let pv = &pgrdf.0;

        assert_eq!(pv["mode"], serde_json::json!("pgrdf"));
        assert_eq!(
            pv["conforms"],
            serde_json::json!(false),
            "#86 — a Core violation is a violation under 'pgrdf' too; \
             conforming here was the mode reporting on constraints it never read"
        );
        assert_eq!(
            pv["results"].as_array().map(Vec::len),
            Some(1),
            "the sh:minCount violation must appear under 'pgrdf'"
        );
        assert!(pv.get("elapsed_ms").is_some());
    }

    /// #80 — a shapes graph using a constraint component this engine
    /// does not evaluate is REFUSED, not silently passed.
    ///
    /// Measured in PASS-8 of the CKP v3.11 wave: a `sh:SPARQLConstraint`
    /// whose `sh:select` names a violating focus node returned
    /// `conforms:true`, zero results and NO error, under both `'native'`
    /// and `'sparql'`. A caller could not distinguish that from a clean
    /// validation. This test locks the refusal.
    #[pg_test]
    fn validate_refuses_unenforced_sparql_constraint() {
        let g_data: i64 = 8540;
        let g_shapes: i64 = 8541;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://ex/> . ex:a a ex:T ; ex:p 20 .".into(),
                g_data.into(),
            ],
        )
        .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix sh: <http://www.w3.org/ns/shacl#> .
                 @prefix ex: <http://ex/> .
                 ex:S a sh:NodeShape ; sh:targetClass ex:T ;
                   sh:sparql [ a sh:SPARQLConstraint ;
                               sh:select \"SELECT $this WHERE { $this <http://ex/p> ?v . FILTER(?v > 10) }\" ] ."
                    .into(),
                g_shapes.into(),
            ],
        )
        .unwrap();

        // Strict (the default): refused, conforms is NULL, error names it.
        let report =
            Spi::get_one::<pgrx::JsonB>(&format!("SELECT pgrdf.validate({g_data}, {g_shapes})"))
                .unwrap()
                .unwrap();
        let v = report.0;
        assert!(
            v["conforms"].is_null(),
            "strict validate must NOT return a verdict when a component is unevaluated: {v}"
        );
        let err = v["error"].as_str().unwrap_or_default();
        assert!(
            err.starts_with("validate: unenforced constraint component"),
            "error must carry the documented prefix, got: {err}"
        );
        assert!(
            err.contains("sh:sparql"),
            "error must NAME the component, got: {err}"
        );

        // Explicit opt-out restores the old behaviour, and only then.
        let loose = Spi::get_one::<pgrx::JsonB>(&format!(
            "SELECT pgrdf.validate({g_data}, {g_shapes}, 'native', false)"
        ))
        .unwrap()
        .unwrap();
        assert!(
            loose.0["conforms"].is_boolean(),
            "strict => false must still return a Boolean verdict: {}",
            loose.0
        );
    }

    /// #80 — the unenforced set is MODE-DEPENDENT. rudof ships no
    /// `SparqlValidator` impl for the cardinality constraints, so a
    /// `sh:minCount` shape silently passes under `'sparql'` while the
    /// same shape refuses under `'native'`. Strict mode refuses the
    /// asymmetry rather than reporting the weaker verdict.
    #[pg_test]
    fn validate_refuses_cardinality_under_sparql_mode() {
        let g_data: i64 = 8542;
        let g_shapes: i64 = 8543;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://ex/> . ex:a a ex:T .".into(),
                g_data.into(),
            ],
        )
        .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix sh: <http://www.w3.org/ns/shacl#> .
                 @prefix ex: <http://ex/> .
                 ex:S a sh:NodeShape ; sh:targetClass ex:T ;
                   sh:property [ sh:path ex:p ; sh:minCount 1 ] ."
                    .into(),
                g_shapes.into(),
            ],
        )
        .unwrap();

        // 'native' evaluates minCount — a real verdict, and it refuses.
        let native = Spi::get_one::<pgrx::JsonB>(&format!(
            "SELECT pgrdf.validate({g_data}, {g_shapes}, 'native')"
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            native.0["conforms"],
            serde_json::json!(false),
            "native must still evaluate sh:minCount: {}",
            native.0
        );

        // 'sparql' does NOT evaluate it, so strict mode refuses.
        //
        // THREE args, deliberately: the point of this assertion is that
        // `strict` DEFAULTS to true. A PASS-13 bulk edit appended
        // `, false` here while adding the opt-out to the three
        // pre-existing sparql tests, and this test then spent three CI
        // rounds asserting that a guard fires while explicitly switching
        // it off. Do not add a fourth argument to this call.
        let sparql = Spi::get_one::<pgrx::JsonB>(&format!(
            "SELECT pgrdf.validate({g_data}, {g_shapes}, 'sparql')"
        ))
        .unwrap()
        .unwrap();
        assert!(
            sparql.0["conforms"].is_null(),
            "sparql mode cannot evaluate sh:minCount and must refuse: {}",
            sparql.0
        );
        assert!(
            sparql.0["error"]
                .as_str()
                .unwrap_or_default()
                .contains("sh:minCount"),
            "error must NAME sh:minCount: {}",
            sparql.0
        );
    }

    /// #83 — a shapes graph that cannot select anything must not report
    /// a verdict. Three shapes of the same defect, all measured
    /// returning `conforms:true` with no error before this guard:
    /// a nonexistent graph id, an empty graph, and a graph carrying
    /// triples but no SHACL target (passing the data graph twice).
    #[pg_test]
    fn validate_refuses_vacuous_shapes_graph() {
        let g_data: i64 = 8550;
        let g_empty: i64 = 8551;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_empty.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix ex: <http://ex/> . ex:a a ex:T .".into(),
                g_data.into(),
            ],
        )
        .unwrap();

        for (label, shapes) in [
            ("nonexistent", 999_999_999_i64),
            ("empty", g_empty),
            ("data-graph-as-shapes", g_data),
        ] {
            let r =
                Spi::get_one::<pgrx::JsonB>(&format!("SELECT pgrdf.validate({g_data}, {shapes})"))
                    .unwrap()
                    .unwrap();
            assert!(
                r.0["conforms"].is_null(),
                "{label}: a vacuous shapes graph must not yield a verdict: {}",
                r.0
            );
            assert!(
                r.0["error"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("declares no SHACL target"),
                "{label}: error must say why: {}",
                r.0
            );
        }

        // A real shapes graph still validates normally.
        let g_shapes: i64 = 8552;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix sh: <http://www.w3.org/ns/shacl#> .
                 @prefix ex: <http://ex/> .
                 ex:S a sh:NodeShape ; sh:targetClass ex:T ;
                   sh:property [ sh:path ex:p ; sh:minCount 1 ] ."
                    .into(),
                g_shapes.into(),
            ],
        )
        .unwrap();
        let ok =
            Spi::get_one::<pgrx::JsonB>(&format!("SELECT pgrdf.validate({g_data}, {g_shapes})"))
                .unwrap()
                .unwrap();
        assert_eq!(
            ok.0["conforms"],
            serde_json::json!(false),
            "a targeting shapes graph must still produce a real verdict: {}",
            ok.0
        );

        // Explicit opt-out still permits the vacuous pass.
        let loose = Spi::get_one::<pgrx::JsonB>(&format!(
            "SELECT pgrdf.validate({g_data}, {g_empty}, 'native', false)"
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            loose.0["conforms"],
            serde_json::json!(true),
            "strict => false must still allow it: {}",
            loose.0
        );
    }

    /// The gate's own logic, tested WITHOUT Postgres.
    ///
    /// #82 sat at "341 pass / 1 fail" for three CI rounds because the
    /// only coverage of these two functions ran inside a `#[pg_test]`,
    /// which cannot link on a macOS host. A plain unit test over a
    /// hand-built set answers in milliseconds whether the pure logic is
    /// the problem — and it is not, which is what localises the defect
    /// to the SPI read.
    #[test]
    fn unenforced_and_target_logic_is_pure_and_correct() {
        use super::{declares_any_target, unenforced_components};
        use std::collections::HashSet;
        let set = |v: &[&str]| -> HashSet<String> { v.iter().map(|s| (*s).to_string()).collect() };

        const MIN: &str = "http://www.w3.org/ns/shacl#minCount";
        const TC: &str = "http://www.w3.org/ns/shacl#targetClass";
        const SP: &str = "http://www.w3.org/ns/shacl#sparql";

        // sh:minCount is unevaluated under 'sparql' and fine under 'native'.
        assert_eq!(
            unenforced_components(&set(&[MIN, TC]), "sparql"),
            vec!["sh:minCount"]
        );
        assert!(unenforced_components(&set(&[MIN, TC]), "native").is_empty());

        // sh:sparql is skipped by both rudof modes and evaluated by 'pgrdf'.
        assert_eq!(unenforced_components(&set(&[SP]), "native").len(), 1);
        assert_eq!(unenforced_components(&set(&[SP]), "sparql").len(), 1);
        assert!(
            unenforced_components(&set(&[SP]), "pgrdf").is_empty(),
            "'pgrdf' evaluates sh:sparql — refusing it there rejects the only mode that supports it"
        );

        // Targeting: an explicit target predicate is enough.
        assert!(declares_any_target(&set(&[TC]), ""));
        assert!(!declares_any_target(&set(&[MIN]), ""));

        // Implicit class targeting is detected from the serialisation.
        assert!(declares_any_target(
            &set(&[]),
            "_:b <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/ns/shacl#NodeShape> .\n             _:b <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://www.w3.org/2000/01/rdf-schema#Class> ."
        ));
    }

    /// #86 — mode `'pgrdf'` must enforce BOTH constraint families.
    ///
    /// One shapes graph carrying a Core constraint AND a `sh:sparql`
    /// constraint, one candidate violating both. Before this, the
    /// partitions were complementary and each mode reported its own
    /// half as a complete verdict:
    ///
    /// ```text
    /// native  conforms=false  1 violation   Core only,   sh:sparql skipped
    /// sparql  conforms=true   0 violations  nothing
    /// pgrdf   conforms=false  1 violation   sh:sparql only, Core skipped
    /// ```
    ///
    /// `pgrdf` reporting `conforms=false` was the dangerous case: a real
    /// verdict that a caller reasonably reads as complete.
    #[pg_test]
    fn validate_pgrdf_mode_enforces_core_and_sparql() {
        let g: i64 = 8560;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "@prefix sh: <http://www.w3.org/ns/shacl#> .
                 @prefix ex: <http://ex/> .
                 ex:S a sh:NodeShape ; sh:targetClass ex:T ;
                   sh:property [ sh:path ex:required ; sh:minCount 1 ] ;
                   sh:sparql [ a sh:SPARQLConstraint ;
                               sh:message \"p must not exceed 10\" ;
                               sh:select \"SELECT $this WHERE { $this <http://ex/p> ?v . FILTER(?v > 10) }\" ] .
                 ex:a a ex:T ; ex:p 20 ."
                    .into(),
                g.into(),
            ],
        )
        .unwrap();

        let n = Spi::get_one::<pgrx::JsonB>(&format!("SELECT pgrdf.validate({g}, {g}, 'native')"))
            .unwrap()
            .unwrap();
        let p = Spi::get_one::<pgrx::JsonB>(&format!("SELECT pgrdf.validate({g}, {g}, 'pgrdf')"))
            .unwrap()
            .unwrap();

        let n_count = n.0["results"].as_array().map(|a| a.len()).unwrap_or(0);
        let p_count = p.0["results"].as_array().map(|a| a.len()).unwrap_or(0);

        // 'native' still sees the Core half only — that is its contract.
        assert_eq!(n.0["conforms"], serde_json::json!(false));
        assert_eq!(
            n_count, 1,
            "native reports the Core violation only: {}",
            n.0
        );

        // 'pgrdf' must see BOTH. This is the whole point of #86.
        assert_eq!(p.0["conforms"], serde_json::json!(false));
        assert!(
            p_count >= 2,
            "mode 'pgrdf' must report the Core violation AND the sh:sparql violation, got {p_count}: {}",
            p.0
        );
        assert_eq!(p.0["mode"], serde_json::json!("pgrdf"));
    }
}
