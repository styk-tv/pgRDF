//! `pgrdf.graph_integrity(graph_id)` — type-illegal terms, per position (#104).
//!
//! The store places no constraint between a quad's id columns and what
//! those ids denote in `_pgrdf_dictionary`, so a writer that resolves a
//! term to a wrong id stores a structurally illegal quad — a literal in
//! predicate position, a bnode as a predicate — and nothing anywhere
//! says so. SPARQL just returns wrong or empty answers.
//!
//! The witness that forced this function: a graph carried 592
//! literal-predicates and 138 bnode-predicates for weeks (written under
//! a since-fixed cache defect), and the diagnosis took a raw forensic
//! session — xmin bands, dictionary self-consistency, per-position
//! term_type joins. This function IS that session, as one read-only
//! call. A monitoring loop can alarm on corruption instead of
//! discovering it through a query that quietly returns nothing.
//!
//! RDF term-position legality (the rules being checked):
//!   subject:   IRI or BlankNode — a Literal subject is illegal
//!   predicate: IRI only
//!   object:    anything
//! Plus referential integrity: every id a quad carries must have a
//! dictionary row (an id with no row is unreadable, not merely wrong).
//!
//! Runs with the CALLER's privileges — anyone who can read the graph
//! can audit it; nobody who cannot, can.

use pgrx::prelude::*;
use serde_json::json;

/// Term-type constants mirror `storage::dict::term_type`:
/// 1 = URI, 2 = BlankNode, 3 = Literal.
#[pg_extern]
fn graph_integrity(graph_id: i64) -> pgrx::JsonB {
    // Refuse an unknown graph rather than reporting zeros for it — a
    // typo'd id must not read as a clean audit.
    let known = Spi::get_one_with_args::<bool>(
        "SELECT EXISTS(SELECT 1 FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)",
        &[graph_id.into()],
    )
    .expect("graph_integrity: graph lookup failed")
    .unwrap_or(false);
    if !known {
        pgrx::error!("graph_integrity: no graph with id {graph_id} (see pgrdf._pgrdf_graphs)");
    }

    // One pass over the graph's quads, all positions joined at once.
    // LEFT JOIN so a dangling id (no dictionary row) is countable
    // rather than silently dropped by an inner join.
    let row = Spi::get_one_with_args::<pgrx::JsonB>(
        r#"
        SELECT to_jsonb(t) FROM (
          SELECT
            count(*)                                            AS quads,
            count(*) FILTER (WHERE dp.term_type = 3)            AS predicate_literal,
            count(*) FILTER (WHERE dp.term_type = 2)            AS predicate_bnode,
            count(*) FILTER (WHERE ds.term_type = 3)            AS subject_literal,
            count(*) FILTER (WHERE ds.id IS NULL)               AS subject_dangling,
            count(*) FILTER (WHERE dp.id IS NULL)               AS predicate_dangling,
            count(*) FILTER (WHERE dobj.id IS NULL)             AS object_dangling
          FROM pgrdf._pgrdf_quads q
          LEFT JOIN pgrdf._pgrdf_dictionary ds   ON ds.id   = q.subject_id
          LEFT JOIN pgrdf._pgrdf_dictionary dp   ON dp.id   = q.predicate_id
          LEFT JOIN pgrdf._pgrdf_dictionary dobj ON dobj.id = q.object_id
          WHERE q.graph_id = $1
        ) t
        "#,
        &[graph_id.into()],
    )
    .expect("graph_integrity: audit scan failed")
    .expect("graph_integrity: audit scan returned no row");

    let counts = row.0;
    let illegal = ["predicate_literal", "predicate_bnode", "subject_literal"]
        .iter()
        .map(|k| counts.get(k).and_then(|v| v.as_i64()).unwrap_or(0))
        .sum::<i64>();
    let dangling = ["subject_dangling", "predicate_dangling", "object_dangling"]
        .iter()
        .map(|k| counts.get(k).and_then(|v| v.as_i64()).unwrap_or(0))
        .sum::<i64>();

    pgrx::JsonB(json!({
        "graph_id": graph_id,
        "counts":   counts,
        "illegal_terms": illegal,
        "dangling_refs": dangling,
        // The one-field answer a monitor alarms on. `clean` is a
        // statement about structure only — it does not claim the data
        // is right, only that every term sits in a position its kind
        // is allowed to occupy and resolves to a dictionary row.
        "clean": illegal == 0 && dangling == 0,
    }))
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// A clean graph reports clean — and through the sanctioned write
    /// path it is not possible to make it report otherwise, which is
    /// exactly why the corrupt fixture below writes raw.
    #[pg_test]
    fn integrity_clean_graph_is_clean() {
        Spi::run("SELECT pgrdf.add_graph(970001)").unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle(
               '<urn:it:s> <urn:it:p> \"a literal object is legal\" .', 970001)",
        )
        .unwrap();
        let r = Spi::get_one::<pgrx::JsonB>("SELECT pgrdf.graph_integrity(970001)")
            .unwrap()
            .unwrap();
        assert_eq!(r.0["clean"], serde_json::json!(true));
        assert_eq!(r.0["illegal_terms"], serde_json::json!(0));
        assert_eq!(r.0["counts"]["quads"], serde_json::json!(1));
    }

    /// Seed the exact illness the fleet forensics found — a literal id
    /// in predicate position, a bnode id as predicate, a literal
    /// subject — by writing quads raw, then confirm one call reports
    /// every one of them.
    #[pg_test]
    fn integrity_reports_type_illegal_terms() {
        Spi::run("SELECT pgrdf.add_graph(970002)").unwrap();
        // Intern one term of each kind through the sanctioned path.
        let iri = Spi::get_one::<i64>("SELECT pgrdf.put_term('urn:it:iri', 1::smallint)")
            .unwrap()
            .unwrap();
        let bnode = Spi::get_one::<i64>("SELECT pgrdf.put_term('it-bnode-1', 2::smallint)")
            .unwrap()
            .unwrap();
        let lit = Spi::get_one::<i64>("SELECT pgrdf.put_term('just a literal', 3::smallint)")
            .unwrap()
            .unwrap();

        // put_quad is id-typed and does not (yet) type-check positions
        // — the opt-in strict write mode is #104's second half. That
        // gap is what makes this fixture buildable, and the probe is
        // what makes it visible.
        Spi::run_with_args(
            "SELECT pgrdf.put_quad($1, $2, $3, 970002)",
            &[iri.into(), lit.into(), iri.into()],
        )
        .unwrap(); // literal as predicate
        Spi::run_with_args(
            "SELECT pgrdf.put_quad($1, $2, $3, 970002)",
            &[iri.into(), bnode.into(), iri.into()],
        )
        .unwrap(); // bnode as predicate
        Spi::run_with_args(
            "SELECT pgrdf.put_quad($1, $2, $3, 970002)",
            &[lit.into(), iri.into(), iri.into()],
        )
        .unwrap(); // literal as subject
        Spi::run_with_args(
            "SELECT pgrdf.put_quad($1, $2, $3, 970002)",
            &[iri.into(), iri.into(), lit.into()],
        )
        .unwrap(); // literal as OBJECT — legal, must NOT be flagged

        let r = Spi::get_one::<pgrx::JsonB>("SELECT pgrdf.graph_integrity(970002)")
            .unwrap()
            .unwrap();
        assert_eq!(r.0["clean"], serde_json::json!(false));
        assert_eq!(r.0["counts"]["predicate_literal"], serde_json::json!(1));
        assert_eq!(r.0["counts"]["predicate_bnode"], serde_json::json!(1));
        assert_eq!(r.0["counts"]["subject_literal"], serde_json::json!(1));
        assert_eq!(r.0["illegal_terms"], serde_json::json!(3));
        assert_eq!(r.0["counts"]["quads"], serde_json::json!(4));
        // The legal literal-object row contributes to quads but to no
        // illegal counter — the probe flags positions, not literals.
        assert_eq!(r.0["dangling_refs"], serde_json::json!(0));
    }

    /// An id with no dictionary row is counted as dangling, not
    /// silently dropped by the join.
    #[pg_test]
    fn integrity_reports_dangling_refs() {
        Spi::run("SELECT pgrdf.add_graph(970003)").unwrap();
        let iri = Spi::get_one::<i64>("SELECT pgrdf.put_term('urn:it:iri2', 1::smallint)")
            .unwrap()
            .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.put_quad($1, $2, $3, 970003)",
            &[iri.into(), iri.into(), 999_999_999_i64.into()],
        )
        .unwrap();
        let r = Spi::get_one::<pgrx::JsonB>("SELECT pgrdf.graph_integrity(970003)")
            .unwrap()
            .unwrap();
        assert_eq!(r.0["clean"], serde_json::json!(false));
        assert_eq!(r.0["counts"]["object_dangling"], serde_json::json!(1));
        assert_eq!(r.0["dangling_refs"], serde_json::json!(1));
    }

    /// A typo'd graph id must refuse, not report a clean audit of
    /// nothing.
    #[pg_test(error = "graph_integrity: no graph with id 970999 (see pgrdf._pgrdf_graphs)")]
    fn integrity_unknown_graph_refuses() {
        Spi::get_one::<pgrx::JsonB>("SELECT pgrdf.graph_integrity(970999)")
            .unwrap()
            .unwrap();
    }
}
