//! SHACL processor wrapper.
//!
//! Phase 5 v0.3 ships as a **STUB**. The `pgrdf.validate(data_graph_id,
//! shapes_graph_id) → JSONB` UDF is exposed at the SQL boundary so
//! downstream tooling (CloudNativePG operators, CI jobs, client
//! libraries) can wire against it today; the body returns a
//! `{"status": "stub", …}` payload that explains the upstream-dep
//! block. The dep-resolution issue is captured in
//! [`specs/ERRATA.v0.2.md`](../../specs/ERRATA.v0.2.md) E-009:
//!
//! * `shacl_validation 0.2.x` ships an unfinished `iri_s` →
//!   `rudof_iri` migration; `shacl_ast 0.2.9` no longer compiles
//!   against the resolved dependency tree (`expected
//!   rudof_iri::IriS, found iri_s::IriS`).
//! * `shacl_validation 0.1.149` does compile in isolation, but its
//!   transitives enable `oxrdf`'s `rdf-12` feature. That feature
//!   adds `TermRef::Triple(_)`, a variant `reasonable 0.4.1`'s
//!   pattern match does not handle — so feature unification
//!   between our two upstream crates breaks the build.
//!
//! Until either upstream lands a fix, validation through the real
//! processor is deferred to v0.4 (or whenever shacl_validation and
//! reasonable agree on a triple-term-handling baseline).
//!
//! The stub still:
//!   * Validates the surface (graph_id args, JSONB output shape).
//!   * Reports whether the named graphs exist, plus their triple
//!     counts, so the caller can sanity-check inputs.
//!   * Documents the path forward in the JSONB payload itself.

use pgrx::prelude::*;
use serde_json::json;

/// SHACL validation report — currently a stub.
///
/// SQL: `pgrdf.validate(data_graph_id BIGINT, shapes_graph_id BIGINT) → JSONB`.
///
/// Returns a JSONB payload of the form:
/// ```json
/// {
///   "status":            "stub",
///   "reason":            "ERRATA E-009 — shacl_validation upstream dep block",
///   "data_graph_id":     <i64>,
///   "shapes_graph_id":   <i64>,
///   "data_triples":      <i64>,
///   "shapes_triples":    <i64>,
///   "data_graph_exists":   <bool>,
///   "shapes_graph_exists": <bool>,
///   "conforms":          null,
///   "results":           []
/// }
/// ```
///
/// `conforms` is `null` because no validation is actually performed
/// yet; once Phase 5 lands the real integration the field will be a
/// boolean per `sh:ValidationReport`.
#[pg_extern]
fn validate(data_graph_id: i64, shapes_graph_id: i64) -> pgrx::JsonB {
    let data_triples = count_quads(data_graph_id);
    let shapes_triples = count_quads(shapes_graph_id);
    pgrx::JsonB(json!({
        "status":           "stub",
        "reason":           "ERRATA E-009 — shacl_validation upstream dep block (iri_s/rudof_iri split + rdf-12 feature unification vs reasonable)",
        "data_graph_id":    data_graph_id,
        "shapes_graph_id":  shapes_graph_id,
        "data_triples":     data_triples,
        "shapes_triples":   shapes_triples,
        "conforms":          serde_json::Value::Null,
        "results":          [],
    }))
}

fn count_quads(graph_id: i64) -> i64 {
    Spi::get_one_with_args::<i64>(
        "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = $1",
        &[graph_id.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0)
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// The stub UDF returns a well-formed JSONB report announcing
    /// `status: "stub"`. Surface lives so downstream tooling can
    /// wire against it.
    #[pg_test]
    fn validate_stub_shape() {
        let g_data: i64 = 8500;
        let g_shapes: i64 = 8501;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_data.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle('@prefix ex:<http://example.com/> . ex:a ex:p ex:b .', $1)",
            &[g_data.into()],
        )
        .unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g_shapes.into()]).unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle('@prefix sh:<http://www.w3.org/ns/shacl#> . sh:NodeShape sh:targetClass sh:NodeShape .', $1)",
            &[g_shapes.into()],
        )
        .unwrap();

        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.validate($1, $2)",
            &[g_data.into(), g_shapes.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["status"], "stub");
        assert_eq!(v["data_graph_id"], g_data);
        assert_eq!(v["shapes_graph_id"], g_shapes);
        assert_eq!(v["data_triples"], 1);
        assert!(v["conforms"].is_null());
        assert!(v["results"].is_array());
    }

    /// Calling with unknown graphs returns zero triple counts but
    /// does NOT panic.
    #[pg_test]
    fn validate_stub_unknown_graphs() {
        let j: pgrx::JsonB = Spi::get_one("SELECT pgrdf.validate(999990::bigint, 999991::bigint)")
            .unwrap()
            .unwrap();
        let v = &j.0;
        assert_eq!(v["data_triples"], 0);
        assert_eq!(v["shapes_triples"], 0);
        assert_eq!(v["status"], "stub");
    }
}
