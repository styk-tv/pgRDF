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

use crate::query::plan_cache;
use crate::storage::dict::{put_term_full, term_type};
use crate::storage::shmem_cache;
use oxrdf::{GraphName, NamedOrBlankNode, Term};
use oxttl::{NQuadsParser, TriGParser, TurtleParser};
use pgrx::datum::DatumWithOid;
use pgrx::pg_sys::{Oid, PgBuiltInOids};
use pgrx::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::mem;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Quads buffered before each `INSERT ... unnest` flush. 1000 keeps
/// the array parameters comfortably below Postgres' 1 GB datum
/// ceiling while amortising the SPI round-trip cost.
const BATCH_SIZE: usize = 1000;

/// Static SQL for the batched quad flush. Phase 3 step 3 (LLD §4.3,
/// phase A): the string is constant, so a single prepared statement
/// — stashed in the per-backend `plan_cache` from Phase 3 step 2 —
/// is reused for every flush, in every load call, for the rest of
/// the backend's lifetime. Saves one parse+plan per batch.
const QUAD_INSERT_SQL: &str = "INSERT INTO pgrdf._pgrdf_quads \
    (subject_id, predicate_id, object_id, graph_id) \
    SELECT s, p, o, $4 \
      FROM unnest($1::bigint[], $2::bigint[], $3::bigint[]) AS t(s, p, o)";

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
    /// Phase-0 instrumentation supporting the TA-11 (heap_multi_insert)
    /// + TA-10 (COPY BINARY) spike scope decision. The three phase
    /// timers below sum to roughly `elapsed_ms` (small per-iteration
    /// overhead lives outside any phase — Instant calls themselves
    /// plus the loop bookkeeping). Each measures the cumulative
    /// time spent in its phase across the entire ingest call:
    ///
    /// - `parse_ms` — rio parser `next()` calls (Turtle/TriG/N-Quads
    ///   lexer + grammar; the time to read the next triple from the
    ///   input stream).
    /// - `dict_ms` — every `intern_term` call: HashMap lookup +
    ///   `put_term_full` cross-shmem-cache check + dictionary SPI when
    ///   the term wasn't cached.
    /// - `insert_ms` — every `flush_batch` call: the prepared
    ///   `INSERT ... unnest($1,$2,$3)` plan execution against
    ///   `_pgrdf_quads`.
    ///
    /// If `insert_ms` is a small fraction of `elapsed_ms`, replacing
    /// the unnest path with `heap_multi_insert` or `COPY BINARY`
    /// (TA-11 / TA-10) has low ROI and the spikes get re-scoped to
    /// whichever phase IS dominant.
    parse_ms: f64,
    dict_ms: f64,
    insert_ms: f64,
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

fn object_to_id(t: &Term, cache: &mut HashMap<DictKey, i64>, stats: &mut LoaderStats) -> i64 {
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
            intern_term(
                cache,
                stats,
                lit.value(),
                term_type::LITERAL,
                datatype_id,
                lang,
            )
        }
        #[allow(unreachable_patterns)]
        _ => panic!("load_turtle: unsupported object term (RDF-star not in v0.2 scope)"),
    }
}

/// Flush a buffered batch of quads to the partitioned hexastore via
/// the cached prepared `INSERT ... unnest` statement.
///
/// On first call in a backend the SQL is prepared and `keep()`-ed
/// into `plan_cache`; every subsequent call (in this load and in
/// future loads) is a pure execute. Moves the buffer Vecs into
/// Postgres-side arrays so we don't pay a clone; callers see empty
/// Vecs after this returns.
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
    let s_arr = mem::take(batch_s);
    let p_arr = mem::take(batch_p);
    let o_arr = mem::take(batch_o);
    let int8_oid: Oid = PgBuiltInOids::INT8OID.into();
    let int8_array_oid: Oid = PgBuiltInOids::INT8ARRAYOID.into();

    Spi::connect_mut(|client| {
        // Prepare-once / reuse-many via the per-backend plan cache.
        // Same mechanism as the SPARQL executor (Phase 3 step 2);
        // keyed on the SQL string which is `QUAD_INSERT_SQL`
        // verbatim.
        if !plan_cache::contains(QUAD_INSERT_SQL) {
            let arg_oids = vec![
                PgOid::BuiltIn(PgBuiltInOids::INT8ARRAYOID),
                PgOid::BuiltIn(PgBuiltInOids::INT8ARRAYOID),
                PgOid::BuiltIn(PgBuiltInOids::INT8ARRAYOID),
                PgOid::BuiltIn(PgBuiltInOids::INT8OID),
            ];
            let prepared = client
                .prepare_mut(QUAD_INSERT_SQL, &arg_oids)
                .expect("flush_batch: prepare failed")
                .keep();
            plan_cache::insert(QUAD_INSERT_SQL.to_string(), prepared);
            plan_cache::record_miss();
        } else {
            plan_cache::record_hit();
        }

        // Build Datums for the cached plan. SAFETY: the (value, oid)
        // pairs match by construction (Vec<i64>/INT8ARRAYOID,
        // i64/INT8OID).
        let datums: Vec<DatumWithOid<'_>> = unsafe {
            vec![
                DatumWithOid::new(s_arr, int8_array_oid),
                DatumWithOid::new(p_arr, int8_array_oid),
                DatumWithOid::new(o_arr, int8_array_oid),
                DatumWithOid::new(graph_id, int8_oid),
            ]
        };

        plan_cache::with_plan(QUAD_INSERT_SQL, |maybe_owned| {
            let owned = maybe_owned.expect("load_turtle: plan must be in cache after insert");
            client
                .update(owned, None, &datums)
                .expect("flush_batch: prepared insert failed");
        });
    });
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
    // Phase-0 timers: nanosecond accumulators converted to ms at
    // the end. Per-iteration Instant::now() pairs add ~1ns each on
    // modern CPUs; across 100k triples that's well under a ms of
    // overhead, dwarfed by the work being measured.
    let mut parse_ns: u128 = 0;
    let mut dict_ns: u128 = 0;
    let mut insert_ns: u128 = 0;

    let mut iter = parser;
    loop {
        let t0 = Instant::now();
        let next = iter.next();
        parse_ns += t0.elapsed().as_nanos();
        let triple = match next {
            Some(r) => r.expect("load_turtle: turtle parse error"),
            None => break,
        };

        let t1 = Instant::now();
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
        dict_ns += t1.elapsed().as_nanos();

        batch_s.push(s);
        batch_p.push(p);
        batch_o.push(o);
        stats.triples += 1;
        if batch_s.len() >= BATCH_SIZE {
            let t2 = Instant::now();
            flush_batch(
                &mut batch_s,
                &mut batch_p,
                &mut batch_o,
                graph_id,
                &mut stats,
            );
            insert_ns += t2.elapsed().as_nanos();
        }
    }
    let t3 = Instant::now();
    flush_batch(
        &mut batch_s,
        &mut batch_p,
        &mut batch_o,
        graph_id,
        &mut stats,
    );
    insert_ns += t3.elapsed().as_nanos();

    stats.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    stats.parse_ms = parse_ns as f64 / 1_000_000.0;
    stats.dict_ms = dict_ns as f64 / 1_000_000.0;
    stats.insert_ms = insert_ns as f64 / 1_000_000.0;
    stats
}

/// TA-D3 spike: 2-pass ingest with batched dict resolution.
///
/// Phase-0 baseline (LUBM-1): dict_ms is 73% of total ingest time
/// (1,114 ms of 1,518 ms), driven by 26,473 individual SPI calls
/// from `put_term_full`. This spike replaces those N independent
/// calls with **2 SPI calls per dict-batch** via `put_terms_batch`
/// (insert-with-ON-CONFLICT + bulk-join lookup).
///
/// Algorithm:
///
/// 1. **Parse pass**: iterate the parser once, materializing every
///    Triple into an owned `Vec<oxrdf::Triple>` and noting each
///    unique `(term_type, lexical, datatype_id, language)` tuple in
///    a HashSet so we can resolve them in batch.
/// 2. **Bulk resolve**: chunk the unique terms by `dict_batch_size`
///    and call `put_terms_batch` per chunk; fill a HashMap with the
///    resulting (key → id) bindings.
/// 3. **Insert pass**: walk the materialized triples again, looking
///    up s / p / o ids from the HashMap (every term now guaranteed
///    present), build the s/p/o arrays, and `flush_batch` per
///    QUAD_BATCH_SIZE chunk — same prepared INSERT plan as the
///    baseline path. (TA-11/TA-10 spikes target THIS insert phase
///    separately.)
///
/// Memory cost: O(triples) for the materialized Vec — LUBM-1 is
/// ~100k triples × ~3 strings each ≈ ~10MB. Acceptable for the
/// LUBM-1/10 tiers; LUBM-100 (~13M triples, ~1GB owned) would need
/// a streaming variant. The SPIKE is LUBM-1-scope only.
fn ingest_turtle_dict_batched<R: Read>(
    reader: R,
    graph_id: i64,
    base_iri: Option<&str>,
    dict_batch_size: usize,
) -> LoaderStats {
    use std::collections::HashSet;
    let mut parser = TurtleParser::new();
    if let Some(base) = base_iri {
        parser = parser
            .with_base_iri(base)
            .unwrap_or_else(|e| panic!("load_turtle_dict_batched: invalid base IRI {base:?}: {e}"));
    }
    let iter = parser.for_reader(reader);

    let start = Instant::now();
    let mut stats = LoaderStats::default();
    let mut parse_ns: u128 = 0;
    let mut dict_ns: u128 = 0;
    let mut insert_ns: u128 = 0;

    // ── Phase 1: parse + collect ─────────────────────────────────
    let t_parse = Instant::now();
    let mut triples: Vec<oxrdf::Triple> = Vec::new();
    let mut unique_terms: HashSet<DictKey> = HashSet::new();
    let term_key = |tt: i16, v: &str, dt: Option<i64>, lang: Option<&str>| -> DictKey {
        (tt, v.to_string(), dt, lang.map(str::to_string))
    };
    for triple_result in iter {
        let triple = triple_result.expect("load_turtle_dict_batched: turtle parse error");
        // Collect unique term tuples for s, p, o into the set.
        match &triple.subject {
            NamedOrBlankNode::NamedNode(n) => {
                unique_terms.insert(term_key(term_type::URI, n.as_str(), None, None));
            }
            NamedOrBlankNode::BlankNode(b) => {
                unique_terms.insert(term_key(term_type::BLANK_NODE, b.as_str(), None, None));
            }
        }
        unique_terms.insert(term_key(
            term_type::URI,
            triple.predicate.as_str(),
            None,
            None,
        ));
        match &triple.object {
            Term::NamedNode(n) => {
                unique_terms.insert(term_key(term_type::URI, n.as_str(), None, None));
            }
            Term::BlankNode(b) => {
                unique_terms.insert(term_key(term_type::BLANK_NODE, b.as_str(), None, None));
            }
            Term::Literal(lit) => {
                // Phase 1 places literals with placeholder None
                // datatype_id. Custom-datatype literals need the
                // datatype IRI's own dict id; Phase 1.5 + Phase 2
                // resolve datatype IRIs first then re-key the
                // literal with the proper datatype_id.
                let lang = lit.language();
                unique_terms.insert(term_key(term_type::LITERAL, lit.value(), None, lang));
            }
            #[allow(unreachable_patterns)]
            _ => {}
        }
        triples.push(triple);
    }
    parse_ns += t_parse.elapsed().as_nanos();
    stats.triples = triples.len() as i64;

    // ── Phase 1.5: resolve datatype IRIs first ──────────────────
    // Literals with custom datatypes need an `datatype_iri_id` that
    // references a dict row. Gather all unique datatype IRIs into a
    // separate batch + resolve THEM first, then we know the ids
    // when keying the literals.
    let t_dict = Instant::now();
    let mut datatype_iris: HashSet<String> = HashSet::new();
    for tr in &triples {
        if let Term::Literal(lit) = &tr.object {
            let dt = lit.datatype().as_str();
            if dt != "http://www.w3.org/2001/XMLSchema#string"
                && dt != "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"
            {
                datatype_iris.insert(dt.to_string());
            }
        }
    }
    let mut dt_iri_id: HashMap<String, i64> = HashMap::new();
    let dt_terms: Vec<(i16, String, Option<i64>, Option<String>)> = datatype_iris
        .iter()
        .map(|iri| (term_type::URI, iri.clone(), None, None))
        .collect();
    for chunk in dt_terms.chunks(dict_batch_size) {
        let ids = crate::storage::dict::put_terms_batch(chunk);
        for (term, id) in chunk.iter().zip(ids.iter()) {
            dt_iri_id.insert(term.1.clone(), *id);
        }
    }

    // ── Phase 2: re-key triples with proper datatype_ids + collect
    //               the full set of unique non-datatype-IRI terms ──
    let mut full_terms: HashSet<DictKey> = HashSet::new();
    // Re-add subject/predicate URIs (these are subset of unique_terms).
    full_terms.extend(
        unique_terms
            .iter()
            .filter(|k| {
                // Drop literals from the partially-built set; they'll be
                // re-added with proper datatype_id below.
                k.0 != term_type::LITERAL
            })
            .cloned(),
    );
    // Datatype IRIs are also URI terms — already in unique_terms via
    // the predicate-position scan; but datatypes that appear ONLY as
    // literal datatypes also need to be in the dict. Re-include.
    for iri in &datatype_iris {
        full_terms.insert((term_type::URI, iri.clone(), None, None));
    }
    // Walk triples, build the proper literal keys, add to full set.
    for tr in &triples {
        if let Term::Literal(lit) = &tr.object {
            let lang = lit.language();
            let dt = lit.datatype().as_str();
            // Plain `xsd:string` and lang-tagged `rdf:langString`
            // literals have None datatype_id per pgRDF convention;
            // all other typed literals reference the resolved dict
            // id for their datatype IRI (from Phase 1.5).
            let datatype_id = if dt == "http://www.w3.org/2001/XMLSchema#string"
                || (dt == "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString" && lang.is_some())
            {
                None
            } else {
                dt_iri_id.get(dt).copied()
            };
            full_terms.insert((
                term_type::LITERAL,
                lit.value().to_string(),
                datatype_id,
                lang.map(str::to_string),
            ));
        }
    }

    // Bulk resolve the rest in batches.
    let mut cache: HashMap<DictKey, i64> = HashMap::new();
    // Pre-seed cache with datatype IRI ids (already resolved).
    for (iri, id) in &dt_iri_id {
        cache.insert((term_type::URI, iri.clone(), None, None), *id);
    }
    let pending: Vec<DictKey> = full_terms
        .iter()
        .filter(|k| !cache.contains_key(*k))
        .cloned()
        .collect();
    let mut total_calls = 0i64;
    for chunk in pending.chunks(dict_batch_size) {
        let chunk_v: Vec<(i16, String, Option<i64>, Option<String>)> = chunk
            .iter()
            .map(|k| (k.0, k.1.clone(), k.2, k.3.clone()))
            .collect();
        let ids = crate::storage::dict::put_terms_batch(&chunk_v);
        for (k, id) in chunk.iter().zip(ids.iter()) {
            cache.insert(k.clone(), *id);
        }
        total_calls += 2; // insert + lookup
    }
    stats.dict_db_calls = total_calls; // repurposed: SPI call count
    stats.dict_cache_hits = (stats.triples * 3) - pending.len() as i64;
    dict_ns += t_dict.elapsed().as_nanos();

    // ── Phase 3: build s/p/o arrays and flush_batch as usual ────
    let t_insert = Instant::now();
    let mut batch_s: Vec<i64> = Vec::with_capacity(BATCH_SIZE);
    let mut batch_p: Vec<i64> = Vec::with_capacity(BATCH_SIZE);
    let mut batch_o: Vec<i64> = Vec::with_capacity(BATCH_SIZE);
    let lookup = |k: &DictKey, cache: &HashMap<DictKey, i64>| -> i64 {
        *cache
            .get(k)
            .unwrap_or_else(|| panic!("load_turtle_dict_batched: cache miss for {:?}", k))
    };
    for triple in triples {
        let s_key = match &triple.subject {
            NamedOrBlankNode::NamedNode(n) => (term_type::URI, n.as_str().to_string(), None, None),
            NamedOrBlankNode::BlankNode(b) => {
                (term_type::BLANK_NODE, b.as_str().to_string(), None, None)
            }
        };
        let p_key = (
            term_type::URI,
            triple.predicate.as_str().to_string(),
            None,
            None,
        );
        let o_key = match &triple.object {
            Term::NamedNode(n) => (term_type::URI, n.as_str().to_string(), None, None),
            Term::BlankNode(b) => (term_type::BLANK_NODE, b.as_str().to_string(), None, None),
            Term::Literal(lit) => {
                let lang = lit.language();
                let dt = lit.datatype().as_str();
                let datatype_id = if dt == "http://www.w3.org/2001/XMLSchema#string"
                    || (dt == "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString"
                        && lang.is_some())
                {
                    None
                } else {
                    dt_iri_id.get(dt).copied()
                };
                (
                    term_type::LITERAL,
                    lit.value().to_string(),
                    datatype_id,
                    lang.map(str::to_string),
                )
            }
            #[allow(unreachable_patterns)]
            _ => panic!("load_turtle_dict_batched: unexpected term shape"),
        };
        batch_s.push(lookup(&s_key, &cache));
        batch_p.push(lookup(&p_key, &cache));
        batch_o.push(lookup(&o_key, &cache));
        if batch_s.len() >= BATCH_SIZE {
            flush_batch(
                &mut batch_s,
                &mut batch_p,
                &mut batch_o,
                graph_id,
                &mut stats,
            );
        }
    }
    flush_batch(
        &mut batch_s,
        &mut batch_p,
        &mut batch_o,
        graph_id,
        &mut stats,
    );
    insert_ns += t_insert.elapsed().as_nanos();

    stats.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    stats.parse_ms = parse_ns as f64 / 1_000_000.0;
    stats.dict_ms = dict_ns as f64 / 1_000_000.0;
    stats.insert_ms = insert_ns as f64 / 1_000_000.0;
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
        // Phase-0 breakdown (TA-11 / TA-10 scope decision). Sum
        // approximates `elapsed_ms` minus a small per-iteration
        // overhead. See `LoaderStats` doc-comment for what each
        // measures.
        "parse_ms":         stats.parse_ms,
        "dict_ms":          stats.dict_ms,
        "insert_ms":        stats.insert_ms,
    }))
}

// ─────────────────────────────────────────────────────────────────────
// §4 — TriG / N-Quads quad ingest (graph-routed)
// ─────────────────────────────────────────────────────────────────────

/// Per-graph quad batch buffers. The v0.3 `flush_batch` prepared plan
/// is single-graph (`graph_id` is `$4`); a quad stream interleaves
/// graphs, so we partition the buffers by resolved `graph_id` and
/// flush each partition through the SAME `QUAD_INSERT_SQL` plan. Net
/// effect: identical batched-insert path as `parse_turtle`, just
/// keyed per destination graph.
/// One graph's pending (subject, predicate, object) id columns
/// awaiting a `flush_batch`.
type QuadCols = (Vec<i64>, Vec<i64>, Vec<i64>);

#[derive(Default)]
struct GraphBatches {
    /// graph_id → pending (s_ids, p_ids, o_ids) flush buffers.
    pending: HashMap<i64, QuadCols>,
}

impl GraphBatches {
    fn push(&mut self, graph_id: i64, s: i64, p: i64, o: i64, stats: &mut LoaderStats) {
        let entry = self
            .pending
            .entry(graph_id)
            .or_insert_with(|| (Vec::new(), Vec::new(), Vec::new()));
        entry.0.push(s);
        entry.1.push(p);
        entry.2.push(o);
        if entry.0.len() >= BATCH_SIZE {
            let (mut bs, mut bp, mut bo) = mem::take(entry);
            flush_batch(&mut bs, &mut bp, &mut bo, graph_id, stats);
        }
    }

    fn flush_all(&mut self, stats: &mut LoaderStats) {
        // Deterministic flush order (sorted graph ids) keeps the
        // quad_batches accounting + any error surface stable across
        // runs regardless of HashMap iteration order.
        let mut gids: Vec<i64> = self.pending.keys().copied().collect();
        gids.sort_unstable();
        for gid in gids {
            if let Some((mut bs, mut bp, mut bo)) = self.pending.remove(&gid) {
                flush_batch(&mut bs, &mut bp, &mut bo, gid, stats);
            }
        }
    }
}

/// Resolve a parsed `GraphName` to a pgRDF `graph_id`.
///
/// * `DefaultGraph` (TriG triples outside any GRAPH block / N-Quads
///   3-position lines) → `default_graph_id` verbatim.
/// * A named-node graph IRI → `pgrdf.graph_id(iri)` if already bound;
///   otherwise, under the default (`strict == false`),
///   `pgrdf.add_graph(iri)` auto-allocates the next id and creates the
///   LIST partition. Under `strict == true` an unbound IRI raises with
///   the stable `{prefix}: unknown graph iri <iri>` error — no
///   allocation, no partial ingest (the raise aborts the surrounding
///   statement; nothing has been flushed for an unknown IRI because
///   resolution happens BEFORE the quad is buffered).
/// * A blank-node graph label is not a legal pgRDF graph key
///   (graphs are IRI- or id-addressed); raise with the stable prefix.
///
/// Resolved ids are cached for the rest of the call so a repeated
/// graph IRI costs one lookup, not one per quad.
fn resolve_graph_id(
    g: &GraphName,
    default_graph_id: i64,
    strict: bool,
    prefix: &str,
    cache: &mut HashMap<String, i64>,
) -> i64 {
    match g {
        GraphName::DefaultGraph => default_graph_id,
        GraphName::NamedNode(n) => {
            let iri = n.as_str();
            if let Some(&id) = cache.get(iri) {
                return id;
            }
            // Already bound? (read-only lookup, no side effect)
            let existing: Option<i64> = Spi::get_one_with_args(
                "SELECT (SELECT graph_id FROM pgrdf._pgrdf_graphs WHERE iri = $1 LIMIT 1)",
                &[iri.into()],
            )
            .unwrap_or_else(|e| panic!("{prefix}: graph iri lookup failed: {e}"));
            let id = match existing {
                Some(id) => id,
                None if strict => {
                    panic!("{prefix}: unknown graph iri {iri}");
                }
                None => {
                    // Auto-allocate + create the LIST partition through
                    // the v0.4 §3.2 IRI-keyed add_graph overload (the
                    // partition DDL is single-sourced there).
                    Spi::get_one_with_args::<i64>("SELECT pgrdf.add_graph($1)", &[iri.into()])
                        .unwrap_or_else(|e| panic!("{prefix}: add_graph({iri}) failed: {e}"))
                        .unwrap_or_else(|| panic!("{prefix}: add_graph({iri}) returned NULL"))
                }
            };
            cache.insert(iri.to_string(), id);
            id
        }
        GraphName::BlankNode(b) => {
            panic!(
                "{prefix}: blank-node graph label _:{} is not a valid pgRDF graph key \
                 (use an IRI-named graph)",
                b.as_str()
            );
        }
    }
}

/// Quad-stream ingest core shared by `parse_trig` / `parse_nquads`.
/// Reuses the term-interning dict cache + the `flush_batch` prepared
/// plan exactly like `ingest_turtle_with_stats`, partition-routed by
/// the per-quad resolved `graph_id`. `prefix` selects the stable
/// error prefix (`parse_trig` / `parse_nquads`). The `Iterator` is
/// the oxttl quad parser (TriG or N-Quads); both yield the same
/// `Result<Quad, _>` item shape.
fn ingest_quads_with_stats<P, E>(
    parser: P,
    default_graph_id: i64,
    strict: bool,
    prefix: &'static str,
) -> (LoaderStats, Vec<i64>)
where
    P: Iterator<Item = Result<oxrdf::Quad, E>>,
    E: std::fmt::Display,
{
    let start = Instant::now();
    let mut cache: HashMap<DictKey, i64> = HashMap::new();
    let mut graph_id_cache: HashMap<String, i64> = HashMap::new();
    let mut stats = LoaderStats::default();
    let mut batches = GraphBatches::default();
    // Insertion-ordered set of graph ids touched (for the JSONB
    // `graphs` array — first-seen order is stable + useful).
    let mut graphs_order: Vec<i64> = Vec::new();
    let mut graphs_seen: HashSet<i64> = HashSet::new();

    for quad_result in parser {
        let quad = quad_result.unwrap_or_else(|e| panic!("{prefix}: quad parse error: {e}"));
        // Resolve the destination graph FIRST. Under `strict` an
        // unknown IRI raises here, before any term interning or quad
        // buffering for it — so a rejected IRI never leaves a partial
        // row (the raise rolls back the enclosing statement).
        let gid = resolve_graph_id(
            &quad.graph_name,
            default_graph_id,
            strict,
            prefix,
            &mut graph_id_cache,
        );
        if graphs_seen.insert(gid) {
            graphs_order.push(gid);
        }
        let s = subject_to_id(&quad.subject, &mut cache, &mut stats);
        let p = intern_term(
            &mut cache,
            &mut stats,
            quad.predicate.as_str(),
            term_type::URI,
            None,
            None,
        );
        let o = object_to_id(&quad.object, &mut cache, &mut stats);
        batches.push(gid, s, p, o, &mut stats);
        stats.triples += 1;
    }
    batches.flush_all(&mut stats);
    stats.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    (stats, graphs_order)
}

/// JSONB stats for the quad-ingest UDFs. Mirrors `stats_to_jsonb`
/// (same verbose-stats keys + conventions as `parse_turtle_verbose`)
/// and extends it with a `graphs` array of the resolved destination
/// graph ids, in first-seen order.
fn quad_stats_to_jsonb(stats: &LoaderStats, graphs: &[i64]) -> pgrx::JsonB {
    pgrx::JsonB(json!({
        "triples":          stats.triples,
        "dict_cache_hits":  stats.dict_cache_hits,
        "shmem_cache_hits": stats.shmem_cache_hits,
        "dict_db_calls":    stats.dict_db_calls,
        "quad_batches":     stats.quad_batches,
        "graphs":           graphs,
        "elapsed_ms":       stats.elapsed_ms,
        // Phase-0 breakdown (TA-11 / TA-10 scope decision); zero on
        // the quad path until `ingest_quads_with_stats` gets its own
        // phase timers (Phase-0 first focused on the Turtle path —
        // LUBM-1 baseline is Turtle). The fields stay present in the
        // JSON shape so downstream consumers see a stable schema.
        "parse_ms":         stats.parse_ms,
        "dict_ms":          stats.dict_ms,
        "insert_ms":        stats.insert_ms,
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
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn load_turtle(path: &str, graph_id: i64, base_iri: default!(Option<&str>, "NULL")) -> i64 {
    let file =
        File::open(path).unwrap_or_else(|e| panic!("load_turtle: failed to open {path:?}: {e}"));
    let base = base_iri.filter(|s| !s.is_empty());
    ingest_turtle_with_stats(BufReader::new(file), graph_id, base).triples
}

/// Same as `load_turtle` but returns JSONB stats: triples,
/// dict_cache_hits, shmem_cache_hits, dict_db_calls, quad_batches,
/// elapsed_ms. Useful for measuring whether the cache + batching
/// paths are firing.
///
/// SQL: `pgrdf.load_turtle_verbose(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
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
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn parse_turtle(content: &str, graph_id: i64, base_iri: default!(Option<&str>, "NULL")) -> i64 {
    let base = base_iri.filter(|s| !s.is_empty());
    ingest_turtle_with_stats(content.as_bytes(), graph_id, base).triples
}

/// Verbose variant of `parse_turtle` returning JSONB stats.
#[search_path(pgrdf, pg_temp)]
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

/// TA-D3 spike — 2-pass ingest with batched dict resolution.
///
/// Same JSONB output shape as `parse_turtle_verbose`; an extra
/// `path` discriminator field is added so consumers can confirm
/// which path the measurement came from.
///
/// Defaults: `dict_batch_size = 500` (heuristic — chunks ≥ ~200
/// amortize SPI roundtrip cost; ≤ 2000 stays under PG's stack-
/// alloc'd unnest sizing).
///
/// SQL: `pgrdf.parse_turtle_dict_batched(content TEXT, graph_id BIGINT,
///       base_iri TEXT DEFAULT NULL, dict_batch_size INT DEFAULT 500) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn parse_turtle_dict_batched(
    content: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
    dict_batch_size: default!(i32, 500),
) -> pgrx::JsonB {
    let base = base_iri.filter(|s| !s.is_empty());
    let stats = ingest_turtle_dict_batched(
        content.as_bytes(),
        graph_id,
        base,
        dict_batch_size.max(1) as usize,
    );
    let mut j = stats_to_jsonb(&stats);
    if let serde_json::Value::Object(ref mut m) = j.0 {
        m.insert(
            "path".to_string(),
            serde_json::Value::String("dict_batched".to_string()),
        );
        m.insert(
            "dict_batch_size".to_string(),
            serde_json::Value::Number(dict_batch_size.into()),
        );
    }
    j
}

/// File-source variant of `parse_turtle_dict_batched`. Useful when
/// the LUBM-N data is on a server-side path rather than passed in as
/// a TEXT literal.
///
/// SQL: `pgrdf.load_turtle_dict_batched(path TEXT, graph_id BIGINT,
///       base_iri TEXT DEFAULT NULL, dict_batch_size INT DEFAULT 500) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn load_turtle_dict_batched(
    path: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
    dict_batch_size: default!(i32, 500),
) -> pgrx::JsonB {
    let file = File::open(path)
        .unwrap_or_else(|e| panic!("load_turtle_dict_batched: failed to open {path:?}: {e}"));
    let base = base_iri.filter(|s| !s.is_empty());
    let stats = ingest_turtle_dict_batched(
        BufReader::new(file),
        graph_id,
        base,
        dict_batch_size.max(1) as usize,
    );
    let mut j = stats_to_jsonb(&stats);
    if let serde_json::Value::Object(ref mut m) = j.0 {
        m.insert(
            "path".to_string(),
            serde_json::Value::String("dict_batched".to_string()),
        );
        m.insert(
            "dict_batch_size".to_string(),
            serde_json::Value::Number(dict_batch_size.into()),
        );
    }
    j
}

/// Parse a TriG document from a string and ingest it into pgRDF,
/// honouring inline `GRAPH <iri> { … }` blocks. Default-graph triples
/// (outside any GRAPH block) land in `default_graph_id`. Each named
/// graph's `<iri>` resolves via the v0.4 §3.2 IRI mapping:
/// `pgrdf.graph_id(iri)` if already bound, else `pgrdf.add_graph(iri)`
/// auto-allocates a fresh id + LIST partition. Under `strict => TRUE`
/// an unknown graph IRI is rejected (`parse_trig: unknown graph iri
/// <iri>`) instead of auto-allocating — resolution happens before any
/// quad for that IRI is buffered, so a rejection leaves no partial
/// rows. Reuses the v0.3 batched-insert path, partition-routed per
/// resolved graph_id.
///
/// Returns verbose JSONB stats (same shape as
/// `pgrdf.parse_turtle_verbose` plus a `graphs` array of the resolved
/// destination graph ids in first-seen order).
///
/// SQL: `pgrdf.parse_trig(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn parse_trig(
    content: &str,
    default_graph_id: default!(i64, 0),
    strict: default!(bool, false),
) -> pgrx::JsonB {
    let parser = TriGParser::new().for_slice(content.as_bytes());
    let (stats, graphs) = ingest_quads_with_stats(parser, default_graph_id, strict, "parse_trig");
    quad_stats_to_jsonb(&stats, &graphs)
}

/// Parse an N-Quads document from a string and ingest it into pgRDF.
/// Each line is a 4-position quad; the fourth-position graph IRI
/// resolves via the v0.4 §3.2 IRI mapping (bound → its id, unbound →
/// `pgrdf.add_graph(iri)` auto-allocate by default). 3-position lines
/// (no fourth term) fall to `default_graph_id`. Under `strict => TRUE`
/// an unknown graph IRI is rejected (`parse_nquads: unknown graph iri
/// <iri>`) with no partial ingest. Reuses the v0.3 batched-insert
/// path, partition-routed per resolved graph_id.
///
/// Returns verbose JSONB stats (same shape as
/// `pgrdf.parse_turtle_verbose` plus a `graphs` array of the resolved
/// destination graph ids in first-seen order).
///
/// SQL: `pgrdf.parse_nquads(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn parse_nquads(
    content: &str,
    default_graph_id: default!(i64, 0),
    strict: default!(bool, false),
) -> pgrx::JsonB {
    let parser = NQuadsParser::new().for_slice(content.as_bytes());
    let (stats, graphs) = ingest_quads_with_stats(parser, default_graph_id, strict, "parse_nquads");
    quad_stats_to_jsonb(&stats, &graphs)
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

    // ─── §4 (Phase G group G2, slice 17) — N-Quads ingest ───────────

    /// parse_nquads: a 4-position line lands in the named graph
    /// (auto-allocated by default); a 3-position line falls to the
    /// default_graph_id. Verbose stats carry the batched-insert shape
    /// + the `graphs` array of touched ids.
    #[pg_test]
    fn parse_nquads_basic() {
        let nq = r#"<http://ex/a> <http://ex/p> "x" <http://ex/g1> .
<http://ex/b> <http://ex/p> "y" <http://ex/g1> .
<http://ex/c> <http://ex/p> "z" ."#;
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.parse_nquads($1, $2)",
            &[nq.into(), 7_201i64.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["triples"], 3, "3 quads parsed");
        assert_eq!(v["quad_batches"], 2, "two graphs → two flush batches");
        let g1: i64 = Spi::get_one("SELECT pgrdf.graph_id('http://ex/g1')")
            .unwrap()
            .unwrap();
        let in_g1: i64 = Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[g1.into()])
            .unwrap()
            .unwrap();
        assert_eq!(in_g1, 2, "two quads routed to g1");
        let in_def: i64 =
            Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[7_201i64.into()])
                .unwrap()
                .unwrap();
        assert_eq!(in_def, 1, "the 3-position line fell to default_graph_id");
        let graphs = v["graphs"].as_array().unwrap();
        assert!(graphs.iter().any(|x| x.as_i64() == Some(7_201)));
        assert!(graphs.iter().any(|x| x.as_i64() == Some(g1)));
    }

    /// parse_nquads: typed + language-tagged literals round-trip into
    /// the dictionary (4th-position graph routes them all to g1).
    #[pg_test]
    fn parse_nquads_typed_and_lang_literals() {
        let nq = r#"<http://nq2/n> <http://nq2/age> "42"^^<http://www.w3.org/2001/XMLSchema#integer> <http://nq2/g> .
<http://nq2/n> <http://nq2/label> "hi"@en <http://nq2/g> ."#;
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.parse_nquads($1, $2)",
            &[nq.into(), 0i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(j.0["triples"], 2);
        let xsd_int: Option<i64> = Spi::get_one(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary
                      WHERE term_type = 1
                        AND lexical_value = 'http://www.w3.org/2001/XMLSchema#integer')",
        )
        .unwrap();
        assert!(xsd_int.is_some(), "xsd:integer datatype IRI interned");
        let lang_lit: Option<i64> = Spi::get_one(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary
                      WHERE term_type = 3 AND lexical_value = 'hi'
                        AND language_tag = 'en')",
        )
        .unwrap();
        assert!(lang_lit.is_some(), "lang-tagged literal interned with @en");
    }

    /// parse_nquads strict-mode: an unknown 4th-position IRI rejects
    /// with the EXACT stable prefix (no auto-allocation).
    #[pg_test(error = "parse_nquads: unknown graph iri http://ex/never")]
    fn parse_nquads_strict_rejects_unknown() {
        let nq = "<http://ex/s> <http://ex/p> \"v\" <http://ex/never> .";
        Spi::run_with_args("SELECT pgrdf.parse_nquads($1, 0, TRUE)", &[nq.into()]).unwrap();
    }

    // ─── §4 (Phase G group G2, slice 16) — TriG ingest ──────────────

    /// parse_trig acceptance #1: a TriG document declaring three
    /// inline named graphs loads into three pgRDF graphs in a single
    /// call; each graph's quad count + binding is asserted.
    #[pg_test]
    fn parse_trig_three_graphs_one_call() {
        let trig = r#"@prefix ex: <http://example.com/> .
            ex:default0 ex:p "d" .
            GRAPH <http://example.com/g/1> { ex:a ex:p "1" . ex:a2 ex:p "1b" }
            GRAPH <http://example.com/g/2> { ex:b ex:p "2" }
            GRAPH <http://example.com/g/3> { ex:c ex:p "3" . ex:c2 ex:p "3b" . ex:c3 ex:p "3c" }"#;
        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.parse_trig($1, $2)",
            &[trig.into(), 7_210i64.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["triples"], 7, "1 default + 2 + 1 + 3 = 7 quads");

        for (iri, want) in [
            ("http://example.com/g/1", 2i64),
            ("http://example.com/g/2", 1),
            ("http://example.com/g/3", 3),
        ] {
            let gid: i64 = Spi::get_one_with_args("SELECT pgrdf.graph_id($1)", &[iri.into()])
                .unwrap()
                .expect("named graph IRI was bound");
            let n: i64 = Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[gid.into()])
                .unwrap()
                .unwrap();
            assert_eq!(n, want, "graph {iri} quad count");
        }
        // The default-graph triple landed in default_graph_id.
        let in_def: i64 =
            Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[7_210i64.into()])
                .unwrap()
                .unwrap();
        assert_eq!(in_def, 1, "the GRAPH-less triple → default_graph_id");
        // Acceptance #3 realisation (quad-set isomorphism per graph):
        // CONSTRUCT each graph back out and compare the triple set.
        let g1_triples: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pgrdf.construct(
               'PREFIX ex: <http://example.com/>
                CONSTRUCT { ?s ?p ?o } WHERE { GRAPH <http://example.com/g/1> { ?s ?p ?o } }')",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            g1_triples, 2,
            "round-trip: CONSTRUCT-of-g1 yields the same 2 triples (quad-set isomorphism)"
        );
    }

    /// parse_trig strict-mode rejects an unknown inline GRAPH IRI
    /// with the EXACT stable prefix.
    #[pg_test(error = "parse_trig: unknown graph iri http://example.com/unbound")]
    fn parse_trig_strict_rejects_unknown() {
        let trig = "GRAPH <http://example.com/unbound> { <http://ex/s> <http://ex/p> \"v\" }";
        Spi::run_with_args("SELECT pgrdf.parse_trig($1, 0, TRUE)", &[trig.into()]).unwrap();
    }
}
