//! Round-trip ingest for `pgrdf.construct` output rows.
//!
//! Phase D slice 53 — closes the round-trip half of LLD v0.4 §6.3.
//! `pgrdf.construct(q)` emits one structured-term JSONB row per
//! (solution, template-triple) pair; this module is its inverse —
//! `pgrdf.put_construct_row(row JSONB, graph_id BIGINT)` decodes one
//! such row back into the dictionary + hexastore, and the batch helper
//! `pgrdf.put_construct_rows(rows JSONB[], graph_id BIGINT)` ingests
//! a captured rowset while preserving within-batch blank-node label
//! identity.
//!
//! Row shape (per LLD v0.4 §6.1, locked by the construct emitter in
//! `src/query/executor.rs`):
//!
//! ```json
//! {
//!   "subject":   {"type":"iri",     "value":"<iri>"},
//!   "predicate": {"type":"iri",     "value":"<iri>"},
//!   "object":    {"type":"iri"|"literal"|"bnode",
//!                 "value":"<lex>",
//!                 "datatype"?: "<iri>",
//!                 "language"?: "<tag>"}
//! }
//! ```
//!
//! Blank-node round-trip semantics
//! ────────────────────────────────
//! Within one call to `put_construct_rows`, repeated bnode labels (the
//! fresh per-solution labels CONSTRUCT emits like `b0_1`, `b1_1`)
//! resolve to a SINGLE interned dictionary row — so the within-solution
//! joining that the slice-56 / slice-57 emitter preserves on the way
//! out is preserved on the way back in. Distinct labels resolve to
//! distinct rows. Per-call state lives in a `HashMap<String, i64>` so
//! the second call to `put_construct_rows` re-uses the dict ids
//! interned by the first, but does not introduce cross-call joining
//! that the captured row data does not warrant — the labels themselves
//! are unique within one construct call by minter design.
//!
//! Quad insertion routes through the same `WHERE NOT EXISTS` guard as
//! the SPARQL UPDATE path in `src/query/executor.rs::insert_quad`, so
//! re-ingesting the same captured rowset twice is idempotent (set
//! semantics; matches LLD v0.4 §4.2 / §6.3).

use crate::storage::dict::{put_term_full, term_type};
use pgrx::prelude::*;
use serde_json::Value;
use std::collections::HashMap;

/// Stable panic prefix family for diagnostic substring asserts in the
/// regression harness. Format: `pgrdf.put_construct_row: <reason>`.
const PANIC_PREFIX: &str = "pgrdf.put_construct_row";

/// Decode one structured-term JSONB cell `{"type": ..., "value": ...,
/// "datatype"?: ..., "language"?: ...}` into a dict id, routing IRIs
/// and literals through the shmem-aware `put_term_full` and blank
/// nodes through the per-call label map. `position` is a free-form
/// label baked into panic messages (`"subject"`, `"predicate"`,
/// `"object"`) so call-site failures pinpoint the offending cell.
///
/// Blank-node handling: the per-call `bnode_map` is consulted before
/// the dictionary. First sighting of a label interns it via
/// `put_term_full(label, BLANK_NODE, ...)` and stores the result;
/// subsequent sightings of the same label inside one
/// `put_construct_rows` call return the cached id, so two emitted
/// rows referencing the same fresh bnode label re-ingest as
/// references to one stored blank node.
fn decode_term_cell(cell: &Value, position: &str, bnode_map: &mut HashMap<String, i64>) -> i64 {
    let obj = cell.as_object().unwrap_or_else(|| {
        panic!("{PANIC_PREFIX}: {position}: term cell must be a JSON object, got {cell}")
    });

    let ty = obj.get("type").and_then(Value::as_str).unwrap_or_else(|| {
        panic!("{PANIC_PREFIX}: {position}: missing or non-string 'type' field")
    });

    let value = obj.get("value").and_then(Value::as_str).unwrap_or_else(|| {
        panic!("{PANIC_PREFIX}: {position}: missing or non-string 'value' field")
    });

    match ty {
        "iri" => put_term_full(value, term_type::URI, None, None),
        "bnode" => {
            if let Some(&id) = bnode_map.get(value) {
                return id;
            }
            let id = put_term_full(value, term_type::BLANK_NODE, None, None);
            bnode_map.insert(value.to_string(), id);
            id
        }
        "literal" => {
            // Reject literals in subject/predicate position — legal
            // RDF disallows them there. The construct emitter never
            // produces such rows; this guard catches hand-crafted
            // input feeding the UDF directly.
            if position == "subject" || position == "predicate" {
                panic!(
                    "{PANIC_PREFIX}: {position}: literal not allowed in subject/predicate position (RDF)"
                );
            }
            let language = obj.get("language").and_then(Value::as_str);
            // Language-tagged literals MUST NOT carry a separately
            // resolved datatype id (rdf:langString is implicit per
            // RDF 1.1 §3.3). The construct emitter writes the
            // datatype IRI text for callers' convenience, but the
            // ingest side treats `language.is_some()` as the gate.
            let datatype_id = if language.is_some() {
                None
            } else {
                let dt = obj
                    .get("datatype")
                    .and_then(Value::as_str)
                    // Per slice 59's contract the construct emitter
                    // always writes a datatype IRI explicitly (default
                    // xsd:string for plain strings). Tolerate absence
                    // anyway — a missing datatype falls back to
                    // xsd:string per RDF 1.1 §3.3.
                    .unwrap_or("http://www.w3.org/2001/XMLSchema#string");
                Some(put_term_full(dt, term_type::URI, None, None))
            };
            put_term_full(value, term_type::LITERAL, datatype_id, language)
        }
        // Forward-compat — slice 53 covers v0.4 §6.1's three encodings
        // exhaustively. Future "triple" / "quoted-triple" cells from
        // RDF 1.2 (gated on E-009 per v0.5-FUTURE §9) would land here.
        other => panic!("{PANIC_PREFIX}: {position}: unknown term type {other:?}"),
    }
}

/// Insert one resolved quad into `pgrdf._pgrdf_quads`, auto-creating
/// the named partition for non-default `graph_id`. Routed through
/// `WHERE NOT EXISTS` so re-ingest is idempotent (set semantics per
/// LLD v0.4 §4.2 / §6.3). Mirrors `executor::insert_quad` exactly so
/// the SPARQL UPDATE path and the construct-ingest path remain two
/// consumers of one canonical INSERT shape.
fn insert_quad(s_id: i64, p_id: i64, o_id: i64, g_id: i64) -> bool {
    if g_id != 0 {
        Spi::run_with_args("SELECT pgrdf.add_graph($1::bigint)", &[g_id.into()])
            .unwrap_or_else(|e| panic!("{PANIC_PREFIX}: add_graph({g_id}) failed: {e}"));
    }
    // RETURNING gives us the row count so we can report "newly
    // inserted" vs "deduplicated by NOT EXISTS guard" without a
    // second probe.
    let inserted: i64 = Spi::get_one_with_args(
        "WITH ins AS (
            INSERT INTO pgrdf._pgrdf_quads
                  (subject_id, predicate_id, object_id, graph_id, is_inferred)
            SELECT $1, $2, $3, $4, false
             WHERE NOT EXISTS (
                SELECT 1 FROM pgrdf._pgrdf_quads
                 WHERE subject_id = $1
                   AND predicate_id = $2
                   AND object_id = $3
                   AND graph_id = $4)
            RETURNING 1)
         SELECT count(*)::bigint FROM ins",
        &[s_id.into(), p_id.into(), o_id.into(), g_id.into()],
    )
    .unwrap_or_else(|e| panic!("{PANIC_PREFIX}: INSERT quad failed: {e}"))
    .unwrap_or(0);
    inserted > 0
}

/// Core decode + insert step. Shared between the single-row and
/// batch variants. Returns `1` if the row landed a fresh quad, `0` if
/// the `WHERE NOT EXISTS` guard found it already present.
fn ingest_one(row: &Value, graph_id: i64, bnode_map: &mut HashMap<String, i64>) -> i64 {
    let obj = row
        .as_object()
        .unwrap_or_else(|| panic!("{PANIC_PREFIX}: row must be a JSON object, got {row}"));
    let s = obj
        .get("subject")
        .unwrap_or_else(|| panic!("{PANIC_PREFIX}: missing 'subject' field"));
    let p = obj
        .get("predicate")
        .unwrap_or_else(|| panic!("{PANIC_PREFIX}: missing 'predicate' field"));
    let o = obj
        .get("object")
        .unwrap_or_else(|| panic!("{PANIC_PREFIX}: missing 'object' field"));

    let s_id = decode_term_cell(s, "subject", bnode_map);
    let p_id = decode_term_cell(p, "predicate", bnode_map);
    let o_id = decode_term_cell(o, "object", bnode_map);

    if insert_quad(s_id, p_id, o_id, graph_id) {
        1
    } else {
        0
    }
}

/// Re-ingest one `pgrdf.construct(q)` output row into the hexastore.
///
/// Returns `1` if a fresh quad landed, `0` if `WHERE NOT EXISTS`
/// found it already present. Per-call state is independent of other
/// `put_construct_row` calls — so blank-node label sharing across
/// rows requires the batch helper [`put_construct_rows`] or
/// a single-row caller that handles the cross-row identity manually.
///
/// SQL surface:
/// `pgrdf.put_construct_row(row JSONB, graph_id BIGINT DEFAULT 0) → BIGINT`.
#[pg_extern]
fn put_construct_row(row: pgrx::JsonB, graph_id: default!(i64, 0)) -> i64 {
    if graph_id < 0 {
        panic!("{PANIC_PREFIX}: graph_id must be >= 0, got {graph_id}");
    }
    let mut bnode_map: HashMap<String, i64> = HashMap::new();
    ingest_one(&row.0, graph_id, &mut bnode_map)
}

/// Re-ingest a captured `pgrdf.construct(q)` rowset into the
/// hexastore. **Recommended round-trip surface** — call-batch shares
/// a single `HashMap<String, i64>` of blank-node labels, so any two
/// rows referencing the same bnode label resolve to the same stored
/// blank node. Two rows with DIFFERENT labels resolve to different
/// blank nodes. This preserves the within-solution / across-template-
/// triple bnode joining that the construct emitter establishes per
/// W3C SPARQL 1.1 §16.2 and LLD v0.4 §6.3.
///
/// Returns the count of NEWLY-inserted quads (rows that landed past
/// the `WHERE NOT EXISTS` set-semantics guard). Re-running the same
/// call with the same input therefore returns 0 — idempotent.
///
/// NULL input is a no-op (returns 0). This handles the common round-
/// trip pattern `(SELECT array_agg(j) FROM pgrdf.construct(...))`,
/// where `array_agg` returns NULL on empty input — invariant G in
/// `tests/regression/sql/106-construct-round-trip.sql`.
///
/// SQL surface:
/// `pgrdf.put_construct_rows(rows JSONB[], graph_id BIGINT DEFAULT 0) → BIGINT`.
#[pg_extern]
fn put_construct_rows(rows: Option<Vec<Option<pgrx::JsonB>>>, graph_id: default!(i64, 0)) -> i64 {
    if graph_id < 0 {
        panic!("{PANIC_PREFIX}: graph_id must be >= 0, got {graph_id}");
    }
    let Some(rows) = rows else {
        // `array_agg(j)` returns NULL when the construct emits zero
        // rows; mirror Turtle's "load nothing → 0 triples" semantics
        // rather than panicking.
        return 0;
    };
    let mut bnode_map: HashMap<String, i64> = HashMap::new();
    let mut inserted: i64 = 0;
    for (idx, maybe_row) in rows.iter().enumerate() {
        let row = maybe_row.as_ref().unwrap_or_else(|| {
            panic!("{PANIC_PREFIX}: rows[{idx}] is NULL — expected JSONB element")
        });
        inserted += ingest_one(&row.0, graph_id, &mut bnode_map);
    }
    inserted
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// Slice 53 invariant A miniature — round-trip three IRI-only
    /// triples and verify the new graph holds the same `(s,p,o)`
    /// content as the source. Captures via `pgrdf.construct(...)`,
    /// re-ingests via `pgrdf.put_construct_rows(...)`, asserts row
    /// count + content match.
    #[pg_test]
    fn round_trip_iri_triples_preserve_content() {
        // Seed graph 7_100 with three triples sharing a predicate so
        // the construct emits three rows. `add_graph` must run BEFORE
        // `parse_turtle` so the `_pgrdf_graphs` IRI binding exists —
        // the variable-GRAPH form of CONSTRUCT's WHERE pattern needs
        // it to bind `urn:pgrdf:graph:7100` to graph_id 7100 (LLD
        // v0.4 §3.1).
        Spi::run("SELECT pgrdf.add_graph(7100::bigint)").unwrap();
        let ttl = r#"
            @prefix ex: <http://example.com/> .
            ex:a ex:p ex:o1 .
            ex:b ex:p ex:o2 .
            ex:c ex:p ex:o3 .
        "#;
        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), 7_100i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(n, 3, "seed should land 3 triples");

        // Re-ingest into a fresh graph 7_101 via construct(...) +
        // put_construct_rows(...). The CONSTRUCT WHERE shorthand
        // disallows GRAPH inside the pattern (slice 54 contract per
        // W3C SPARQL 1.1 §16.2.4), so use the explicit two-block form.
        Spi::run("SELECT pgrdf.add_graph(7101::bigint)").unwrap();
        let landed: i64 = Spi::get_one(
            "SELECT pgrdf.put_construct_rows(
               (SELECT array_agg(j)
                  FROM pgrdf.construct(
                    'CONSTRUCT { ?s ?p ?o } '
                    'WHERE { GRAPH <urn:pgrdf:graph:7100> { ?s ?p ?o } }')
                    AS t(j)),
               7101::bigint)",
        )
        .unwrap()
        .unwrap();
        assert_eq!(landed, 3, "round-trip should land 3 fresh quads");

        let on_dst: i64 =
            Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[7_101i64.into()])
                .unwrap()
                .unwrap();
        assert_eq!(on_dst, 3);

        // Idempotency — re-ingest yields 0 new rows.
        let again: i64 = Spi::get_one(
            "SELECT pgrdf.put_construct_rows(
               (SELECT array_agg(j)
                  FROM pgrdf.construct(
                    'CONSTRUCT { ?s ?p ?o } '
                    'WHERE { GRAPH <urn:pgrdf:graph:7100> { ?s ?p ?o } }')
                    AS t(j)),
               7101::bigint)",
        )
        .unwrap()
        .unwrap();
        assert_eq!(again, 0, "re-ingest should be a no-op (set semantics)");
    }

    /// Slice 53 invariant B miniature — typed integer literal
    /// round-trips with its `xsd:integer` datatype preserved.
    #[pg_test]
    fn round_trip_typed_literal_preserves_datatype() {
        // Bind `_pgrdf_graphs` IRI before seeding so the variable-form
        // GRAPH WHERE pattern can resolve `urn:pgrdf:graph:7110`.
        Spi::run("SELECT pgrdf.add_graph(7110::bigint)").unwrap();
        let ttl = r#"
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            @prefix ex:  <http://example.com/> .
            ex:n ex:age "42"^^xsd:integer .
        "#;
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), 7_110i64.into()],
        )
        .unwrap()
        .unwrap();

        Spi::run("SELECT pgrdf.add_graph(7111::bigint)").unwrap();
        let landed: i64 = Spi::get_one(
            "SELECT pgrdf.put_construct_rows(
               (SELECT array_agg(j)
                  FROM pgrdf.construct(
                    'CONSTRUCT { ?s ?p ?o } '
                    'WHERE { GRAPH <urn:pgrdf:graph:7110> { ?s ?p ?o } }')
                    AS t(j)),
               7111::bigint)",
        )
        .unwrap()
        .unwrap();
        assert_eq!(landed, 1);

        // Dictionary now carries the same lexical_value + datatype IRI
        // for the re-ingested literal. Verify via JOIN.
        let dt: Option<String> = Spi::get_one(
            "SELECT dt.lexical_value
               FROM pgrdf._pgrdf_quads q
               JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
               JOIN pgrdf._pgrdf_dictionary dt ON dt.id = o.datatype_iri_id
              WHERE q.graph_id = 7111
                AND o.term_type = 3
                AND o.lexical_value = '42'
              LIMIT 1",
        )
        .unwrap();
        assert_eq!(
            dt.as_deref(),
            Some("http://www.w3.org/2001/XMLSchema#integer")
        );
    }

    /// Slice 53 invariant E miniature — bnode within-solution
    /// joining preserved across round-trip. The construct emitter
    /// produces a multi-triple template where `_:r` appears twice in
    /// the same solution; the re-ingest must collapse those two
    /// references onto one stored blank node.
    #[pg_test]
    fn round_trip_bnode_within_solution_joins() {
        Spi::run("SELECT pgrdf.add_graph(7120::bigint)").unwrap();
        let ttl = r#"
            @prefix ex: <http://example.com/> .
            ex:s ex:p ex:o .
        "#;
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), 7_120i64.into()],
        )
        .unwrap()
        .unwrap();

        Spi::run("SELECT pgrdf.add_graph(7121::bigint)").unwrap();
        // Multi-triple template: _:r appears as object of triple 1
        // and as subject of triple 2 within the same solution.
        // Within-solution sameness (slice 56) means both refer to ONE
        // fresh label; the re-ingest must preserve that — both quads
        // in graph 7121 must share the same `subject_id`/`object_id`.
        let landed: i64 = Spi::get_one(
            "SELECT pgrdf.put_construct_rows(
               (SELECT array_agg(j)
                  FROM pgrdf.construct(
                    'CONSTRUCT { <http://example.com/s> <http://example.com/has> _:r .
                                 _:r <http://example.com/about> <http://example.com/o> } '
                    'WHERE { GRAPH <urn:pgrdf:graph:7120> { ?s ?p ?o } }') AS t(j)),
               7121::bigint)",
        )
        .unwrap()
        .unwrap();
        assert_eq!(landed, 2, "two template triples * one solution = 2 quads");

        // Verify the bnode joins: the object_id of the first quad
        // (predicate :has) equals the subject_id of the second
        // (predicate :about). Both reference the same blank node.
        let joined: bool = Spi::get_one(
            "SELECT EXISTS (
                SELECT 1
                  FROM pgrdf._pgrdf_quads q_has
                  JOIN pgrdf._pgrdf_quads q_about
                    ON q_about.subject_id = q_has.object_id
                  JOIN pgrdf._pgrdf_dictionary d_has ON d_has.id = q_has.predicate_id
                  JOIN pgrdf._pgrdf_dictionary d_about ON d_about.id = q_about.predicate_id
                  JOIN pgrdf._pgrdf_dictionary d_bnode ON d_bnode.id = q_has.object_id
                 WHERE q_has.graph_id = 7121
                   AND q_about.graph_id = 7121
                   AND d_has.lexical_value = 'http://example.com/has'
                   AND d_about.lexical_value = 'http://example.com/about'
                   AND d_bnode.term_type = 2)",
        )
        .unwrap()
        .unwrap();
        assert!(
            joined,
            "bnode within-solution joining must survive round-trip"
        );
    }
}
