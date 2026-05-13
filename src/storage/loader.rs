//! Turtle ingestion.
//!
//! Phase 2.1: per-triple SPI inserts via `put_term_full` + a direct
//! INSERT into `_pgrdf_quads`. Sufficient for medium-sized ontologies
//! (~100K triples) at single-digit-thousands-per-second. The
//! `COPY … FROM STDIN (FORMAT BINARY)` fast path lands in Phase 2.2
//! per [`docs/02-storage.md`] and SPEC.pgRDF.LLD.v0.2 §4.3.

use crate::storage::dict::{put_term_full, term_type};
use oxrdf::{NamedOrBlankNode, Term};
use oxttl::TurtleParser;
use pgrx::prelude::*;
use std::fs::File;
use std::io::{BufReader, Read};

/// Resolve a Turtle subject (NamedNode | BlankNode) to a dictionary ID.
/// In oxrdf 0.2 a `Triple.subject` is a `NamedOrBlankNode`, not the
/// broader `Subject` (which also admits RDF-star quoted triples).
fn subject_to_id(s: &NamedOrBlankNode) -> i64 {
    match s {
        NamedOrBlankNode::NamedNode(n) => put_term_full(n.as_str(), term_type::URI, None, None),
        NamedOrBlankNode::BlankNode(b) => {
            put_term_full(b.as_str(), term_type::BLANK_NODE, None, None)
        }
    }
}

/// Resolve an object (any Term: NamedNode | BlankNode | Literal) to a
/// dictionary ID. Literals carry datatype and language through to
/// `put_term_full`. RDF-star quoted-triple objects are out of scope
/// for v0.2 (LLD §2) and we panic if they appear.
fn object_to_id(t: &Term) -> i64 {
    match t {
        Term::NamedNode(n) => put_term_full(n.as_str(), term_type::URI, None, None),
        Term::BlankNode(b) => put_term_full(b.as_str(), term_type::BLANK_NODE, None, None),
        Term::Literal(lit) => {
            let lang = lit.language();
            // For lang-tagged literals, datatype is rdf:langString and
            // we keep it implicit (None in our schema). For everything
            // else, intern the datatype IRI itself and store its ID.
            let datatype_id = if lang.is_some() {
                None
            } else {
                Some(put_term_full(lit.datatype().as_str(), term_type::URI, None, None))
            };
            put_term_full(lit.value(), term_type::LITERAL, datatype_id, lang)
        }
        #[allow(unreachable_patterns)]
        _ => panic!("load_turtle: unsupported object term (RDF-star not in v0.2 scope)"),
    }
}

/// Ingest Turtle from any `Read`er into the named graph. Returns the
/// number of triples inserted. Shared between `load_turtle` (file) and
/// `parse_turtle` (string).
///
/// `base_iri` resolves relative IRIs like `<#>` and `<../foo>` that
/// appear in many published vocabularies (notably W3C PROV's prov.ttl).
/// Pass `None` when the document only uses absolute IRIs.
fn ingest_turtle<R: Read>(reader: R, graph_id: i64, base_iri: Option<&str>) -> i64 {
    let mut parser = TurtleParser::new();
    if let Some(base) = base_iri {
        parser = parser
            .with_base_iri(base)
            .unwrap_or_else(|e| panic!("load_turtle: invalid base IRI {base:?}: {e}"));
    }
    let parser = parser.for_reader(reader);
    let mut count: i64 = 0;
    for triple_result in parser {
        let triple = triple_result.expect("load_turtle: turtle parse error");
        let s = subject_to_id(&triple.subject);
        let p = put_term_full(triple.predicate.as_str(), term_type::URI, None, None);
        let o = object_to_id(&triple.object);
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_quads
                 (subject_id, predicate_id, object_id, graph_id)
             VALUES ($1, $2, $3, $4)",
            &[s.into(), p.into(), o.into(), graph_id.into()],
        )
        .expect("load_turtle: quad insert failed");
        count += 1;
    }
    count
}

/// Load a Turtle file from a server-side path into the named graph.
/// Returns the number of triples inserted. `base_iri` is the document
/// URL used to resolve relative IRIs (`<#>`, `<../foo>`); pass `''` or
/// NULL when the file uses absolute IRIs only.
///
/// SQL surface:
/// `pgrdf.load_turtle(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) → BIGINT`.
///
/// Note: the path is server-side. With the compose runtime this means
/// `/fixtures/...` (see `compose/compose.yml` mount).
#[pg_extern]
fn load_turtle(
    path: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
) -> i64 {
    let file = File::open(path)
        .unwrap_or_else(|e| panic!("load_turtle: failed to open {path:?}: {e}"));
    let base = base_iri.filter(|s| !s.is_empty());
    ingest_turtle(BufReader::new(file), graph_id, base)
}

/// Parse a Turtle string and ingest its triples into the named graph.
/// Useful for tests and small inline TTL — for large files prefer
/// `load_turtle` to avoid copying the whole document through SQL.
///
/// SQL surface:
/// `pgrdf.parse_turtle(content TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) → BIGINT`.
#[pg_extern]
fn parse_turtle(
    content: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
) -> i64 {
    let base = base_iri.filter(|s| !s.is_empty());
    ingest_turtle(content.as_bytes(), graph_id, base)
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// parse_turtle on a tiny FOAF graph reports the expected triple
    /// count and the dictionary contains the well-known IRIs.
    #[pg_test]
    fn parse_turtle_basic() {
        // Five triples:
        //   ex:alice rdf:type   foaf:Person
        //   ex:alice foaf:name  "Alice"
        //   ex:alice foaf:mbox  <mailto:alice@example.com>
        //   ex:alice foaf:knows ex:bob
        //   ex:bob   rdf:type   foaf:Person
        let ttl = r#"
            @prefix ex:   <http://example.com/> .
            @prefix foaf: <http://xmlns.com/foaf/0.1/> .
            ex:alice a foaf:Person ;
                     foaf:name "Alice" ;
                     foaf:mbox <mailto:alice@example.com> ;
                     foaf:knows ex:bob .
            ex:bob   a foaf:Person .
        "#;

        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), 7_001i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(n, 5);

        let by_graph: i64 = Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[7_001i64.into()])
            .unwrap()
            .unwrap();
        assert_eq!(by_graph, 5);

        // foaf:Person ended up in the dictionary as a URI.
        let person: Option<i64> = Spi::get_one(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary
                      WHERE term_type = 1
                        AND lexical_value = 'http://xmlns.com/foaf/0.1/Person')",
        )
        .unwrap();
        assert!(person.is_some());
    }

    /// Datatypes round-trip into the dictionary.
    #[pg_test]
    fn parse_turtle_typed_literal() {
        let ttl = r#"
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            @prefix ex:  <http://example.com/> .
            ex:n ex:age "42"^^xsd:integer .
        "#;
        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), 7_002i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(n, 1);

        // The integer datatype IRI was interned too.
        let dt: Option<i64> = Spi::get_one(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary
                      WHERE term_type = 1
                        AND lexical_value = 'http://www.w3.org/2001/XMLSchema#integer')",
        )
        .unwrap();
        assert!(dt.is_some());
    }
}
