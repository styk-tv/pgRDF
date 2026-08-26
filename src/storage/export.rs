//! E3 + E7 (SPEC.pgRDF.LIB.v0.6.34) — canonical export and the portable
//! graph manifest.
//!
//! **E3 `pgrdf.export_graph(graph_id)`** (closes #36): a graph's ASSERTED
//! triples as canonical N-Triples — full N-Triples escaping, one triple
//! per line, byte-sorted — so every consumer stops hand-rolling its own
//! serializer over the private term store (measured: one client shipped
//! an SQL serializer marked INTERIM in every manifest it wrote, and the
//! engine carried the same grammar a second time in Rust).
//!
//! **E7 `pgrdf.graph_manifest(graph_id)`**: everything needed to decide,
//! away from this database, whether a copy of the graph is faithful —
//! served by the engine so no client reimplements the format. The name
//! is deliberately plain pgRDF vocabulary (a manifest of a graph
//! capture); ecosystems layer their own terms on top.
//!
//! Three digests, three questions, never interchangeable (LIB K5):
//!   bytes     sha256 over the canonical export — "same bytes"; the
//!             admission witness; does NOT survive reload (blank-node
//!             labels are minted per parse)
//!   identity  rdfc-1.0-sha256 — "same graph"; conclusive both ways
//!   structure pgrdf-fd1-sha256 — the cross-implementation pin;
//!             unequal conclusive, equal is evidence only
//! Every value carries its method; comparing across methods is a
//! category error, not a shortcut.
//!
//! `not_carried` is mandatory and non-empty (K8): a package that names
//! what it does not contain is a transcription that says so, instead of
//! a record-shaped object relying on a reader who already knows.

use crate::storage::canon::{CTerm, read_asserted_triples};
use pgrx::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};

/// Canonical N-Triples escaping — the full grammar (backslash, quote,
/// newline, carriage return, tab), shared with the RDFC serializer's
/// conventions. NOTE: deliberately NOT the fd1 fleet-minimal escaping;
/// the two serializations serve different digests and are never mixed.
fn nt_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

fn nt_term(t: &CTerm) -> String {
    match t {
        CTerm::Iri(i) => format!("<{i}>"),
        CTerm::BNode(b) => format!("_:{b}"),
        CTerm::Lit { val, dt, lang } => {
            let esc = nt_escape(val);
            match (lang, dt) {
                (Some(l), _) => format!("\"{esc}\"@{l}"),
                (None, Some(d)) if d != "http://www.w3.org/2001/XMLSchema#string" => {
                    format!("\"{esc}\"^^<{d}>")
                }
                _ => format!("\"{esc}\""),
            }
        }
    }
}

/// Refuse absent graphs with 42704 — the shared existence discipline of
/// the digest/export surface (an EMPTY graph answers; absence refuses).
fn require_graph(graph_id: i64, verb: &str) {
    if graph_id == 0 {
        return; // the default graph always exists
    }
    let registered: bool = Spi::get_one_with_args(
        "SELECT EXISTS(SELECT 1 FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)",
        &[graph_id.into()],
    )
    .unwrap_or_else(|e| panic!("{verb}: registry existence check failed: {e}"))
    .unwrap_or(false);
    if !registered {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
            format!(
                "{verb}: graph {graph_id} does not exist — an absent graph has no \
                 export (an EMPTY graph does: pgrdf.add_graph it first)"
            ),
        );
    }
}

/// The canonical export body: sorted lines, no trailing newline in the
/// vector form. Shared by `export_graph` and the manifest's bytes digest
/// so the digest is BY CONSTRUCTION over exactly what the export emits.
fn export_lines(graph_id: i64) -> Vec<String> {
    let mut lines: Vec<String> = read_asserted_triples(graph_id)
        .iter()
        .map(|(s, p, o)| format!("{} {} {} .", nt_term(s), nt_term(p), nt_term(o)))
        .collect();
    lines.sort();
    lines
}

/// A graph's ASSERTED triples as canonical N-Triples, one line per
/// triple, byte-sorted. Deterministic for a given stored copy; blank
/// node labels are the stored ones, so two loads of the same source
/// export different bytes — that is what `graph_digest` and
/// `structural_digest` exist to see through. Inferred rows are never
/// exported: they re-derive (`pgrdf.materialize`), and exporting them
/// would re-import as asserted what was only derived.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn export_graph(graph_id: i64) -> SetOfIterator<'static, String> {
    require_graph(graph_id, "export_graph");
    SetOfIterator::new(export_lines(graph_id).into_iter())
}

/// The portable manifest of one graph: digests (each with its method),
/// counts, engine identity, capture time, and what is NOT carried.
/// Pair with `export_graph` (the content) to build a redistributable
/// package; verify a copy offline by recomputing `digests.bytes` over
/// the content file.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn graph_manifest(graph_id: i64) -> pgrx::JsonB {
    require_graph(graph_id, "graph_manifest");

    let lines = export_lines(graph_id);
    // The bytes digest is over the export FILE form: every line
    // newline-terminated. An empty graph is an empty file — e3b0c44…,
    // the sha256 of zero bytes, which is an ANSWER here (the graph
    // exists and is empty; absence refused above).
    let mut h = Sha256::new();
    for l in &lines {
        h.update(l.as_bytes());
        h.update(b"\n");
    }
    let bytes_sha: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();

    let (iri, asserted, inferred, captured_at, extversion) = Spi::get_one_with_args::<pgrx::JsonB>(
        "SELECT to_jsonb(x) FROM (
            SELECT (SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = $1) AS iri,
                   (SELECT count(*) FROM pgrdf._pgrdf_quads
                     WHERE graph_id = $1 AND NOT is_inferred) AS asserted,
                   (SELECT count(*) FROM pgrdf._pgrdf_quads
                     WHERE graph_id = $1 AND is_inferred) AS inferred,
                   now()::text AS captured_at,
                   (SELECT extversion FROM pg_extension WHERE extname = 'pgrdf') AS extversion
         ) x",
        &[graph_id.into()],
    )
    .unwrap_or_else(|e| panic!("graph_manifest: metadata read failed: {e}"))
    .map(|j| {
        let v = j.0;
        (
            v["iri"].as_str().map(|s| s.to_string()),
            v["asserted"].as_i64().unwrap_or(0),
            v["inferred"].as_i64().unwrap_or(0),
            v["captured_at"].as_str().unwrap_or_default().to_string(),
            v["extversion"].as_str().unwrap_or_default().to_string(),
        )
    })
    .expect("graph_manifest: metadata row");

    let identity: String =
        Spi::get_one_with_args("SELECT pgrdf.graph_digest($1)", &[graph_id.into()])
            .expect("graph_manifest: graph_digest failed")
            .expect("graph_manifest: graph_digest NULL");
    let structure: String =
        Spi::get_one_with_args("SELECT pgrdf.structural_digest($1)", &[graph_id.into()])
            .expect("graph_manifest: structural_digest failed")
            .expect("graph_manifest: structural_digest NULL");

    pgrx::JsonB(json!({
        "graph_id": graph_id,
        "iri": iri,
        "captured_at": captured_at,
        "engine": {
            "version": crate::version(),
            "build_id": crate::build_id(),
            "extversion": extversion,
        },
        "counts": {
            "asserted": asserted,
            // A CHECK VALUE, never content: inferred triples re-derive.
            "inferred": inferred,
        },
        "digests": {
            "bytes":     { "value": bytes_sha, "method": "sha256-of-canonical-ntriples" },
            "identity":  { "value": identity,  "method": "rdfc-1.0-sha256" },
            "structure": { "value": structure, "method": "pgrdf-fd1-sha256" },
        },
        // K8: mandatory and non-empty. A copy of this graph is a
        // transcription of its triples, not a record of anything more.
        "not_carried": [
            "inferred triples (count above is a check value; re-derive with pgrdf.materialize)",
            "lifecycle state and locks (a restored copy starts unlocked and unmaterialized)",
            "attestation and provenance chains (provenance-shaped triples travel as plain triples; the proof that made them true does not)",
        ],
    }))
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;
    use sha2::{Digest, Sha256};

    /// E3 round trip: export → parse into a fresh graph → identical
    /// rdfc-1.0 identity. Byte digests MAY differ across the round trip
    /// (bnode labels re-mint); asserting byte-equality would be the
    /// exact category error the three-digest design exists to prevent —
    /// so this asserts identity, not bytes.
    #[pg_test]
    fn export_round_trips_by_identity() {
        Spi::run("SELECT pgrdf.add_graph(984001)").unwrap();
        Spi::run("SELECT pgrdf.add_graph(984002)").unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix e: <http://e/> . e:s e:p \"v\\\\nwith\\\\tescapes\" . _:m e:q e:o . e:s e:r _:m .',
                984001)",
        )
        .unwrap();
        let nt: String =
            Spi::get_one("SELECT string_agg(l, E'\\n') FROM pgrdf.export_graph(984001) l")
                .unwrap()
                .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, 984002)",
            &[nt.as_str().into()],
        )
        .unwrap();
        let (a, b) = Spi::get_two::<String, String>(
            "SELECT pgrdf.graph_digest(984001), pgrdf.graph_digest(984002)",
        )
        .unwrap();
        assert_eq!(
            a, b,
            "export must round-trip to the identical graph identity"
        );
    }

    /// Export is deterministic (sorted) and asserted-only.
    #[pg_test]
    fn export_is_sorted_and_asserted_only() {
        Spi::run("SELECT pgrdf.add_graph(984003)").unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix e: <http://e/> . e:b a e:T . e:a a e:T . e:T <http://www.w3.org/2000/01/rdf-schema#subClassOf> e:S .',
                984003)",
        )
        .unwrap();
        Spi::run("SELECT pgrdf.materialize(984003)").unwrap();
        let (n_lines, n_asserted) = Spi::get_two::<i64, i64>(
            "SELECT (SELECT count(*) FROM pgrdf.export_graph(984003)),
                    (SELECT count(*) FROM pgrdf._pgrdf_quads
                      WHERE graph_id = 984003 AND NOT is_inferred)",
        )
        .unwrap();
        assert_eq!(n_lines, n_asserted, "inferred rows must never be exported");
        let sorted: bool = Spi::get_one(
            "SELECT bool_and(l <= next_l) FROM (
                 SELECT l, lead(l) OVER () AS next_l FROM pgrdf.export_graph(984003) l
             ) x WHERE next_l IS NOT NULL",
        )
        .unwrap()
        .unwrap();
        assert!(sorted, "export lines are byte-sorted");
    }

    /// E7: the manifest's bytes digest recomputes from the export
    /// content — the offline-verification contract — and K8's
    /// not_carried is present and non-empty.
    #[pg_test]
    fn manifest_bytes_digest_recomputes_and_not_carried_nonempty() {
        Spi::run("SELECT pgrdf.add_graph(984004)").unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle('<urn:m:s> <urn:m:p> \"v\" . _:x <urn:m:q> \"w\" .', 984004)",
        )
        .unwrap();
        let m: pgrx::JsonB = Spi::get_one("SELECT pgrdf.graph_manifest(984004)")
            .unwrap()
            .unwrap();
        let v = m.0;
        // recompute bytes digest from the export, line + \n each
        let nt: String = Spi::get_one(
            "SELECT coalesce(string_agg(l || E'\\n', ''), '') FROM pgrdf.export_graph(984004) l",
        )
        .unwrap()
        .unwrap();
        let mut h = Sha256::new();
        h.update(nt.as_bytes());
        let recomputed: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            v["digests"]["bytes"]["value"].as_str().unwrap(),
            recomputed,
            "bytes digest must recompute from the export content"
        );
        for d in ["bytes", "identity", "structure"] {
            assert!(
                v["digests"][d]["method"].as_str().unwrap_or("").len() > 0,
                "every digest carries its method (K5)"
            );
        }
        assert!(
            v["not_carried"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
            "not_carried is mandatory and non-empty (K8)"
        );
        assert_eq!(v["counts"]["asserted"].as_i64(), Some(2));
    }
}
