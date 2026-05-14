//! Turtle ingestion.
//!
//! Phase 2.2: per-call in-process dict cache + batched quad INSERTs.
//!   * HashMap<(value, type, datatype_id, language) -> id> keyed
//!     dictionary cache across one ingest call. Common terms
//!     (predicates, repeated subjects, common datatype IRIs) resolve
//!     to a cached id after the first lookup instead of a fresh
//!     scalar-subquery SELECT.
//!   * Quad inserts buffer until BATCH_SIZE then flush as a single
//!     `INSERT ... SELECT FROM unnest($1::bigint[], $2::bigint[],
//!     $3::bigint[])` — one SPI call per BATCH_SIZE tuples instead
//!     of one per triple.
//!
//! Phase 3 step 1: every fall-through to `put_term_full` first checks
//! the cross-backend shmem cache from `super::shmem_cache`. The
//! loader observes the global HITS counter around the call to
//! attribute hits to the current ingest in its verbose stats.
//!
//! The COPY ... FROM STDIN (FORMAT BINARY) fast path from
//! SPEC.pgRDF.LLD.v0.2 §4.3 needs lower-level Postgres integration
//! than pgrx 0.16 exposes cleanly. Tracked for Phase 3 step 3.

use crate::storage::dict::{put_term_full, term_type};
use crate::storage::shmem_cache;
use oxrdf::{NamedOrBlankNode, Term};
use oxttl::TurtleParser;
use pgrx::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::mem;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Quads buffered before each `INSERT ... unnest` flush. 1000 keeps
/// the array parameters comfortably below Postgres' 1 GB datum
/// ceiling while amortising the SPI round-trip cost.
const BATCH_SIZE: usize = 1000;

type DictKey = (i16, String, Option<i64>, Option<String>);

#[derive(Default)]
struct LoaderStats {
    triples: i64,
    /// Term references that resolved out of the per-call HashMap cache.
    dict_cache_hits: i64,
    /// Term references that fell through the per-call HashMap and
    /// were satisfied by the cross-backend shmem cache without
    /// touching `_pgrdf_dictionary` (LLD §4.1).
    shmem_cache_hits: i64,
    /// Term references that fell through to `put_term_full` and hit
    /// the underlying _pgrdf_dictionary (either a select-hit or an
    /// insert).
    dict_db_calls: i64,
    quad_batches: i64,
    elapsed_ms: f64,
}

/// Resolve a term to its dictionary id, caching the result for the
/// remainder of the current ingest call.
fn intern_term(
    cache: &mut HashMap<DictKey, i64>,
    stats: &mut LoaderStats,
    value: &str,
    term_type: i16,
    datatype_id: Option<i64>,
    language: Option<&str>,
) -> i64 {
    let key = (
        term_type,
        value.to_string(),
        datatype_id,
        language.map(str::to_string),
    );
    if let Some(&id) = cache.get(&key) {
        stats.dict_cache_hits += 1;
        return id;
    }
    // Snapshot the global shmem-hit counter so we can attribute
    // hits to this individual put_term_full call. Atomics are
    // cheap; this stays well under the per-lookup µs budget.
    let hits_before = if shmem_cache::is_ready() {
        shmem_cache::HITS.get().load(Ordering::Relaxed)
    } else {
        0
    };
    let id = put_term_full(value, term_type, datatype_id, language);
    let hits_after = if shmem_cache::is_ready() {
        shmem_cache::HITS.get().load(Ordering::Relaxed)
    } else {
        0
    };
    if hits_after > hits_before {
        stats.shmem_cache_hits += 1;
    } else {
        stats.dict_db_calls += 1;
    }
    cache.insert(key, id);
    id
}

fn subject_to_id(
    s: &NamedOrBlankNode,
    cache: &mut HashMap<DictKey, i64>,
    stats: &mut LoaderStats,
) -> i64 {
    match s {
        NamedOrBlankNode::NamedNode(n) => {
            intern_term(cache, stats, n.as_str(), term_type::URI, None, None)
        }
        NamedOrBlankNode::BlankNode(b) => {
            intern_term(cache, stats, b.as_str(), term_type::BLANK_NODE, None, None)
        }
    }
}

fn object_to_id(
    t: &Term,
    cache: &mut HashMap<DictKey, i64>,
    stats: &mut LoaderStats,
) -> i64 {
    match t {
        Term::NamedNode(n) => intern_term(cache, stats, n.as_str(), term_type::URI, None, None),
        Term::BlankNode(b) => {
            intern_term(cache, stats, b.as_str(), term_type::BLANK_NODE, None, None)
        }
        Term::Literal(lit) => {
            let lang = lit.language();
            let datatype_id = if lang.is_some() {
                None
            } else {
                Some(intern_term(
                    cache,
                    stats,
                    lit.datatype().as_str(),
                    term_type::URI,
                    None,
                    None,
                ))
            };
            intern_term(cache, stats, lit.value(), term_type::LITERAL, datatype_id, lang)
        }
        #[allow(unreachable_patterns)]
        _ => panic!("load_turtle: unsupported object term (RDF-star not in v0.2 scope)"),
    }
}

/// Flush a buffered batch of quads to the partitioned hexastore.
/// Moves the buffer Vecs into Postgres-side arrays so we don't pay
/// a clone; callers see empty Vecs after this returns.
fn flush_batch(
    batch_s: &mut Vec<i64>,
    batch_p: &mut Vec<i64>,
    batch_o: &mut Vec<i64>,
    graph_id: i64,
    stats: &mut LoaderStats,
) {
    if batch_s.is_empty() {
        return;
    }
    Spi::run_with_args(
        "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
         SELECT s, p, o, $4
           FROM unnest($1::bigint[], $2::bigint[], $3::bigint[]) AS t(s, p, o)",
        &[
            mem::take(batch_s).into(),
            mem::take(batch_p).into(),
            mem::take(batch_o).into(),
            graph_id.into(),
        ],
    )
    .expect("flush_batch: insert failed");
    stats.quad_batches += 1;
}

/// Core ingest loop. Shared by load_turtle / parse_turtle and their
/// _verbose variants.
fn ingest_turtle_with_stats<R: Read>(
    reader: R,
    graph_id: i64,
    base_iri: Option<&str>,
) -> LoaderStats {
    let mut parser = TurtleParser::new();
    if let Some(base) = base_iri {
        parser = parser
            .with_base_iri(base)
            .unwrap_or_else(|e| panic!("load_turtle: invalid base IRI {base:?}: {e}"));
    }
    let parser = parser.for_reader(reader);

    let start = Instant::now();
    let mut cache: HashMap<DictKey, i64> = HashMap::new();
    let mut stats = LoaderStats::default();
    let mut batch_s: Vec<i64> = Vec::with_capacity(BATCH_SIZE);
    let mut batch_p: Vec<i64> = Vec::with_capacity(BATCH_SIZE);
    let mut batch_o: Vec<i64> = Vec::with_capacity(BATCH_SIZE);

    for triple_result in parser {
        let triple = triple_result.expect("load_turtle: turtle parse error");
        let s = subject_to_id(&triple.subject, &mut cache, &mut stats);
        let p = intern_term(
            &mut cache,
            &mut stats,
            triple.predicate.as_str(),
            term_type::URI,
            None,
            None,
        );
        let o = object_to_id(&triple.object, &mut cache, &mut stats);
        batch_s.push(s);
        batch_p.push(p);
        batch_o.push(o);
        stats.triples += 1;
        if batch_s.len() >= BATCH_SIZE {
            flush_batch(&mut batch_s, &mut batch_p, &mut batch_o, graph_id, &mut stats);
        }
    }
    flush_batch(&mut batch_s, &mut batch_p, &mut batch_o, graph_id, &mut stats);
    stats.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    stats
}

fn stats_to_jsonb(stats: &LoaderStats) -> pgrx::JsonB {
    pgrx::JsonB(json!({
        "triples":          stats.triples,
        "dict_cache_hits":  stats.dict_cache_hits,
        "shmem_cache_hits": stats.shmem_cache_hits,
        "dict_db_calls":    stats.dict_db_calls,
        "quad_batches":     stats.quad_batches,
        "elapsed_ms":       stats.elapsed_ms,
    }))
}

// ─────────────────────────────────────────────────────────────────────
// UDF surface
// ─────────────────────────────────────────────────────────────────────

/// Load a Turtle file from a server-side path into the named graph.
/// Returns the number of triples inserted. `base_iri` resolves
/// relative IRIs; pass NULL or '' for absolute-IRI-only files.
///
/// SQL: `pgrdf.load_turtle(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) -> BIGINT`.
#[pg_extern]
fn load_turtle(
    path: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
) -> i64 {
    let file = File::open(path)
        .unwrap_or_else(|e| panic!("load_turtle: failed to open {path:?}: {e}"));
    let base = base_iri.filter(|s| !s.is_empty());
    ingest_turtle_with_stats(BufReader::new(file), graph_id, base).triples
}

/// Same as `load_turtle` but returns JSONB stats: triples,
/// dict_cache_hits, dict_db_calls, quad_batches, elapsed_ms.
/// Useful for measuring whether the cache + batching paths are firing.
///
/// SQL: `pgrdf.load_turtle_verbose(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) -> JSONB`.
#[pg_extern]
fn load_turtle_verbose(
    path: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
) -> pgrx::JsonB {
    let file = File::open(path)
        .unwrap_or_else(|e| panic!("load_turtle_verbose: failed to open {path:?}: {e}"));
    let base = base_iri.filter(|s| !s.is_empty());
    let stats = ingest_turtle_with_stats(BufReader::new(file), graph_id, base);
    stats_to_jsonb(&stats)
}

/// Parse Turtle from a string. Same semantics as `load_turtle` for
/// dict caching and batched inserts, just with an in-memory source.
///
/// SQL: `pgrdf.parse_turtle(content TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) -> BIGINT`.
#[pg_extern]
fn parse_turtle(
    content: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
) -> i64 {
    let base = base_iri.filter(|s| !s.is_empty());
    ingest_turtle_with_stats(content.as_bytes(), graph_id, base).triples
}

/// Verbose variant of `parse_turtle` returning JSONB stats.
#[pg_extern]
fn parse_turtle_verbose(
    content: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
) -> pgrx::JsonB {
    let base = base_iri.filter(|s| !s.is_empty());
    let stats = ingest_turtle_with_stats(content.as_bytes(), graph_id, base);
    stats_to_jsonb(&stats)
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

        let by_graph: i64 =
            Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[7_001i64.into()])
                .unwrap()
                .unwrap();
        assert_eq!(by_graph, 5);

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

        let dt: Option<i64> = Spi::get_one(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary
                      WHERE term_type = 1
                        AND lexical_value = 'http://www.w3.org/2001/XMLSchema#integer')",
        )
        .unwrap();
        assert!(dt.is_some());
    }

    /// Cache fires on repeated subjects + predicates within a single
    /// ingest call. Three FOAF-shape triples share both subject and
    /// predicate, so after the first triple's three DB calls the
    /// other two should be entirely cached except for distinct objects.
    #[pg_test]
    fn parse_turtle_verbose_cache_fires() {
        let ttl = r#"
            @prefix ex:   <http://example.com/> .
            ex:s ex:p ex:o1 .
            ex:s ex:p ex:o2 .
            ex:s ex:p ex:o3 .
        "#;
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.parse_turtle_verbose($1, $2)",
            &[ttl.into(), 7_003i64.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["triples"], 3);
        // 3 triples × 3 terms each = 9 references; 5 distinct
        // (s, p, o1, o2, o3) -> 5 fall-throughs and 4 hashmap hits.
        // Of the 5 fall-throughs every shmem-vs-db split is allowed
        // (depends on prior tests in this postmaster), so only the
        // sum is invariant.
        assert_eq!(v["dict_cache_hits"], 4);
        let shmem_hits = v["shmem_cache_hits"].as_i64().unwrap();
        let db_calls = v["dict_db_calls"].as_i64().unwrap();
        assert_eq!(shmem_hits + db_calls, 5);
        assert_eq!(v["quad_batches"], 1);
    }
}
