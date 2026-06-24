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
use std::io::{BufRead, BufReader, Read};
use std::mem;
use std::sync::atomic::Ordering;
use std::time::Instant;

/// Quads buffered before each `INSERT ... unnest` flush. 1000 keeps
/// the array parameters comfortably below Postgres' 1 GB datum
/// ceiling while amortising the SPI round-trip cost.
const BATCH_SIZE: usize = 1000;

/// Quads buffered per flush on the v0.6.2 parallel BULK path (PASS 4).
/// A pure batch load has no per-statement latency concern (unlike the
/// streaming `BATCH_SIZE` path), so a much larger batch amortises the
/// SPI + executor round-trip over ~50× fewer flushes — the handoff
/// flagged the 1k-batch `INSERT … unnest` as a quad-insert bottleneck.
/// 50k × 3 `int8` arrays ≈ 1.2 MB each, far under PG's 1 GB datum
/// ceiling, and the rows insert heap-only when the bulk path has
/// deferred its indexes. (v0.6.6)
const BULK_QUAD_BATCH: usize = 50_000;

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
    /// Lenient bulk parse (v0.7) — triples skipped because oxttl returned a
    /// syntax error (malformed source, e.g. Wikidata's stray control-char
    /// IRIs). oxttl recovers + continues (toolkit `error_recovery_state`), so
    /// these are dropped + counted, not fatal. 0 on a clean load / serial paths.
    parse_skipped: i64,
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
    /// v0.6.2 bulk path only — the PASS-3 triple→id resolve phase
    /// (`resolve` of every s/p/o against the in-memory term→id maps,
    /// run in parallel via rayon). 0.0 on the serial paths, which
    /// resolve terms inline during `dict_ms` rather than as a distinct
    /// phase. Surfaced in the verbose JSONB so the bulk breakdown reads
    /// parse → dict → resolve → insert.
    resolve_ms: f64,
    /// v0.6.3 bulk path only — the index drop+rebuild phase (the
    /// defer-index optimization). 0.0 when defer did not fire.
    index_ms: f64,
    /// v0.6.3 bulk path only — whether the defer-index optimization
    /// fired (hexastore + dict-hash indexes dropped before the load and
    /// rebuilt after). False on the serial paths and on bulk loads below
    /// `pgrdf.bulk_defer_index_min`.
    defer_index: bool,
    /// TA-5 — which `pgrdf.ingest_dict_path` route the dispatcher
    /// selected for this call (`baseline` / `batched` / `shmem_warm`
    /// / `combined`). Set by `ingest_dispatch` / `ingest_quads_dispatch`
    /// and surfaced as the `path` field in the verbose-ingest JSONB
    /// so callers can confirm the route. Empty for ingest functions
    /// invoked outside the dispatch (e.g. the `parse_turtle_dict_batched`
    /// spike UDF, which sets its own `dict_batched` discriminator).
    path: &'static str,
    /// Streaming path only — number of windows processed (each ~`window_triples`
    /// lines streamed → parsed → dict-resolved against the persistent map → inserted).
    windows: i64,
    /// Streaming path only — distinct dictionary terms the persistent in-Rust map
    /// resolved across the whole load (final cross-window `HashMap` size).
    dict_terms: i64,
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
    // Literals with explicit datatype carry a `datatype_iri_id`
    // pointing at the datatype IRI's dict row. Lang-tagged literals
    // have None (the datatype is implicitly rdf:langString and not
    // stored). Per the baseline `intern_term` rule (and the v0.5.37
    // fix mirrored from the combined path), EVERY non-lang literal
    // — including plain `xsd:string` — keeps an explicit datatype
    // IRI so dict rows round-trip through the SPARQL executor's
    // term equality. Gather all such datatype IRIs into a single
    // batch and resolve them first.
    let t_dict = Instant::now();
    let mut datatype_iris: HashSet<String> = HashSet::new();
    for tr in &triples {
        if let Term::Literal(lit) = &tr.object {
            if lit.language().is_none() {
                datatype_iris.insert(lit.datatype().as_str().to_string());
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
    // Match baseline / combined: lang-tagged → datatype_id = None;
    // every other literal → Some(<dict id of the datatype IRI>).
    for tr in &triples {
        if let Term::Literal(lit) = &tr.object {
            let lang = lit.language();
            let datatype_id = if lang.is_some() {
                None
            } else {
                dt_iri_id.get(lit.datatype().as_str()).copied()
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
                let datatype_id = if lang.is_some() {
                    None
                } else {
                    dt_iri_id.get(lit.datatype().as_str()).copied()
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

/// TA-7 production landing — single-pass streaming ingest combining
/// TA-D3 (batched dict resolution) and TA-D2 (cross-backend shmem
/// dict cache hot-path). USER APPROVED via
/// `_WIP/DECISION.TRACK-A.dict-path-and-insert-path.md` (2026-06-01).
///
/// Differences from `ingest_turtle_dict_batched`:
///
/// * **Single pass** over the parser stream. The spike materialised
///   every parsed Triple into `Vec<oxrdf::Triple>` then re-walked
///   it. Here triples land in a bounded `pending` buffer (drained at
///   `BATCH_SIZE` boundaries) so peak memory is O(BATCH_SIZE) instead
///   of O(triples).
/// * **Shmem hot-cache check FIRST** for every term. Hits skip the
///   defer queue entirely; only misses queue. The TA-D2 spike's
///   −54% e2e win was driven by this layer; the spike measured it
///   against the BASELINE path, this lands it in the production path.
/// * **`put_terms_batch` flush** happens when either the defer queue
///   hits `dict_batch_size` OR the pending-triple buffer is about to
///   flush its quads. The latter is the safety net that guarantees
///   every key in a draining pending-triple has a resolved id by the
///   time we look it up.
/// * **`stage_for_commit` after each batch flush** — newly-resolved
///   terms get published into shmem on commit, so subsequent ingests
///   in the same backend see them as hot-cache hits. The spike
///   bypassed shmem on the write side.
///
/// Datatype IRIs are resolved synchronously (cache → shmem →
/// `put_term_full`) because the literal's key depends on the
/// datatype's dict_id. RDF/RDFS/XSD/OWL URIs warm up in the first
/// few SPI roundtrips of any non-trivial ingest and stay hot for the
/// rest of the backend's lifetime, so the synchronous resolve cost
/// is bounded.
fn ingest_turtle_combined<R: Read>(
    reader: R,
    graph_id: i64,
    base_iri: Option<&str>,
    dict_batch_size: usize,
) -> LoaderStats {
    let mut parser = TurtleParser::new();
    if let Some(base) = base_iri {
        // Panic prefix `load_turtle:` matches `ingest_turtle_with_stats`'s
        // baseline error contract — downstream tooling routes on this
        // substring (see `tests/regression/sql/81-error-paths.sql`).
        parser = parser
            .with_base_iri(base)
            .unwrap_or_else(|e| panic!("load_turtle: invalid base IRI {base:?}: {e}"));
    }
    let iter = parser.for_reader(reader);

    let start = Instant::now();
    let mut stats = LoaderStats::default();
    let mut parse_ns: u128 = 0;
    let mut dict_ns: u128 = 0;
    let mut insert_ns: u128 = 0;

    let mut cache: HashMap<DictKey, i64> = HashMap::new();
    let mut defer_queue: Vec<DictKey> = Vec::with_capacity(dict_batch_size);
    let mut defer_set: HashSet<DictKey> = HashSet::with_capacity(dict_batch_size);
    let mut pending_triples: Vec<(DictKey, DictKey, DictKey)> = Vec::with_capacity(BATCH_SIZE);
    let mut batch_s: Vec<i64> = Vec::with_capacity(BATCH_SIZE);
    let mut batch_p: Vec<i64> = Vec::with_capacity(BATCH_SIZE);
    let mut batch_o: Vec<i64> = Vec::with_capacity(BATCH_SIZE);

    // Drive the parser by hand (not `for … in iter`) so the parse
    // timer wraps the actual `next()` call. A `for` loop polls
    // `next()` at the top of each iteration BEFORE any in-body timer
    // could start, which would attribute ~0 ns to parse and leak the
    // real parse time into the unaccounted per-iteration gap (the
    // bug v0.5.43's LUBM-10 measurement surfaced: parse_ms read 29 ms
    // vs the baseline path's honest 1549 ms). Mirrors the
    // `ingest_turtle_with_stats` baseline loop shape.
    let mut iter = iter;
    loop {
        let t_parse = Instant::now();
        let next = iter.next();
        parse_ns += t_parse.elapsed().as_nanos();
        let triple = match next {
            // Panic prefix `load_turtle:` matches baseline contract.
            Some(r) => r.expect("load_turtle: turtle parse error"),
            None => break,
        };

        let t_dict = Instant::now();

        let s_key: DictKey = subject_key(&triple.subject);
        let p_key: DictKey = (
            term_type::URI,
            triple.predicate.as_str().to_string(),
            None,
            None,
        );
        let o_key: DictKey = object_key(
            &triple.object,
            &mut cache,
            &mut stats,
            "ingest_turtle_combined",
        );

        // Try-resolve each of s/p/o: cache → shmem → defer queue.
        for key in [&s_key, &p_key, &o_key] {
            try_resolve_or_defer(
                key,
                &mut cache,
                &mut defer_queue,
                &mut defer_set,
                &mut stats,
            );
        }

        pending_triples.push((s_key, p_key, o_key));
        stats.triples += 1;
        dict_ns += t_dict.elapsed().as_nanos();

        if defer_queue.len() >= dict_batch_size {
            let t = Instant::now();
            flush_defer(&mut defer_queue, &mut defer_set, &mut cache, &mut stats);
            dict_ns += t.elapsed().as_nanos();
        }

        if pending_triples.len() >= BATCH_SIZE {
            // Defer queue MUST be empty before draining pending — every
            // s/p/o key has to be in `cache` for the lookup below.
            if !defer_queue.is_empty() {
                let t = Instant::now();
                flush_defer(&mut defer_queue, &mut defer_set, &mut cache, &mut stats);
                dict_ns += t.elapsed().as_nanos();
            }
            let t = Instant::now();
            drain_pending_into_batch(
                &mut pending_triples,
                &cache,
                &mut batch_s,
                &mut batch_p,
                &mut batch_o,
            );
            flush_batch(
                &mut batch_s,
                &mut batch_p,
                &mut batch_o,
                graph_id,
                &mut stats,
            );
            insert_ns += t.elapsed().as_nanos();
        }
    }

    if !defer_queue.is_empty() {
        let t = Instant::now();
        flush_defer(&mut defer_queue, &mut defer_set, &mut cache, &mut stats);
        dict_ns += t.elapsed().as_nanos();
    }
    if !pending_triples.is_empty() {
        let t = Instant::now();
        drain_pending_into_batch(
            &mut pending_triples,
            &cache,
            &mut batch_s,
            &mut batch_p,
            &mut batch_o,
        );
        flush_batch(
            &mut batch_s,
            &mut batch_p,
            &mut batch_o,
            graph_id,
            &mut stats,
        );
        insert_ns += t.elapsed().as_nanos();
    }

    stats.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    stats.parse_ms = parse_ns as f64 / 1_000_000.0;
    stats.dict_ms = dict_ns as f64 / 1_000_000.0;
    stats.insert_ms = insert_ns as f64 / 1_000_000.0;
    stats
}

/// Build the `DictKey` for a subject term (URI or blank node).
/// Shared by the Turtle and quad combined ingest paths so both
/// produce byte-identical dict rows.
fn subject_key(s: &NamedOrBlankNode) -> DictKey {
    match s {
        NamedOrBlankNode::NamedNode(n) => (term_type::URI, n.as_str().to_string(), None, None),
        NamedOrBlankNode::BlankNode(b) => {
            (term_type::BLANK_NODE, b.as_str().to_string(), None, None)
        }
    }
}

/// Build the `DictKey` for an object term (URI, blank node, or
/// literal). Literals follow the baseline `intern_term` rule:
/// lang-tagged literals carry `datatype_id = None` (rdf:langString
/// is implicit + not stored); every other literal — INCLUDING plain
/// `xsd:string` — carries an explicit datatype IRI dict id resolved
/// synchronously via `resolve_datatype_iri_sync`. Shared by the
/// Turtle and quad combined paths. `prefix` selects the stable
/// panic prefix for the unreachable RDF-star arm.
fn object_key(
    o: &Term,
    cache: &mut HashMap<DictKey, i64>,
    stats: &mut LoaderStats,
    prefix: &str,
) -> DictKey {
    match o {
        Term::NamedNode(n) => (term_type::URI, n.as_str().to_string(), None, None),
        Term::BlankNode(b) => (term_type::BLANK_NODE, b.as_str().to_string(), None, None),
        Term::Literal(lit) => {
            let lang = lit.language();
            let datatype_id = if lang.is_some() {
                None
            } else {
                Some(resolve_datatype_iri_sync(
                    lit.datatype().as_str(),
                    cache,
                    stats,
                ))
            };
            (
                term_type::LITERAL,
                lit.value().to_string(),
                datatype_id,
                lang.map(str::to_string),
            )
        }
        #[allow(unreachable_patterns)]
        _ => panic!("{prefix}: unsupported object term"),
    }
}

/// Try cache → shmem; on miss, queue the key for bulk resolve.
/// Used by the combined ingest path's per-term hot path.
///
/// Accounting note: `dict_cache_hits` counts terms whose resolution
/// is satisfied in-call without re-hitting any layer below it. This
/// includes both terms already resolved into `cache` AND terms
/// queued earlier in this call (i.e. `defer_set.contains`). Treating
/// the defer_set hit as a cache hit keeps the per-term invariant
/// `dict_cache_hits + shmem_cache_hits + dict_db_calls = 3 ×
/// triples` consistent across the baseline / batched / shmem_warm /
/// combined paths — the SPI shape differs but the counter is a
/// term-reference taxonomy that holds for all four.
fn try_resolve_or_defer(
    key: &DictKey,
    cache: &mut HashMap<DictKey, i64>,
    defer_queue: &mut Vec<DictKey>,
    defer_set: &mut HashSet<DictKey>,
    stats: &mut LoaderStats,
) {
    if cache.contains_key(key) || defer_set.contains(key) {
        stats.dict_cache_hits += 1;
        return;
    }
    if let Some(id) = shmem_cache::lookup(key.0, &key.1, key.2, key.3.as_deref()) {
        cache.insert(key.clone(), id);
        stats.shmem_cache_hits += 1;
        return;
    }
    defer_set.insert(key.clone());
    defer_queue.push(key.clone());
}

/// Resolve a literal's datatype IRI to its dict id synchronously
/// (cache → shmem → `put_term_full`). Datatype IRIs are
/// short-tailed (XSD + RDFS + OWL + a handful of user types) and
/// warm up fast; the synchronous cost is bounded by the unique
/// datatype count, not the literal count.
fn resolve_datatype_iri_sync(
    iri: &str,
    cache: &mut HashMap<DictKey, i64>,
    stats: &mut LoaderStats,
) -> i64 {
    let key: DictKey = (term_type::URI, iri.to_string(), None, None);
    if let Some(&id) = cache.get(&key) {
        stats.dict_cache_hits += 1;
        return id;
    }
    if let Some(id) = shmem_cache::lookup(term_type::URI, iri, None, None) {
        cache.insert(key, id);
        stats.shmem_cache_hits += 1;
        return id;
    }
    let hits_before = if shmem_cache::is_ready() {
        shmem_cache::HITS.get().load(Ordering::Relaxed)
    } else {
        0
    };
    let id = put_term_full(iri, term_type::URI, None, None);
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

/// Bulk-resolve the defer queue via `put_terms_batch` (2 SPI calls
/// regardless of size), publish each fresh (key, id) to shmem on
/// commit so future ingests in this backend see hot-cache hits.
fn flush_defer(
    defer_queue: &mut Vec<DictKey>,
    defer_set: &mut HashSet<DictKey>,
    cache: &mut HashMap<DictKey, i64>,
    stats: &mut LoaderStats,
) {
    if defer_queue.is_empty() {
        return;
    }
    let batch: Vec<(i16, String, Option<i64>, Option<String>)> = defer_queue
        .iter()
        .map(|k| (k.0, k.1.clone(), k.2, k.3.clone()))
        .collect();
    let resolved_count = batch.len() as i64;
    let ids = crate::storage::dict::put_terms_batch(&batch);
    for (k, id) in defer_queue.iter().zip(ids.iter()) {
        cache.insert(k.clone(), *id);
        shmem_cache::stage_for_commit(k.0, &k.1, k.2, k.3.as_deref(), *id);
    }
    // `dict_db_calls` is counted per-term (not per-batch) so the
    // taxonomy matches the baseline / batched / shmem_warm paths.
    // The PHYSICAL SPI shape is 2 calls per flush (INSERT ON
    // CONFLICT DO NOTHING + JOIN-back lookup with WITH ORDINALITY);
    // the SHAPE difference shows up in elapsed_ms, not the counter.
    stats.dict_db_calls += resolved_count;
    defer_queue.clear();
    defer_set.clear();
}

/// Walk drained pending triples, lookup s/p/o ids from `cache`
/// (every key was resolved by a prior `flush_defer`), push into the
/// flush_batch input vectors. Used only by the combined path; the
/// baseline + batched paths already drive `batch_*` themselves.
fn drain_pending_into_batch(
    pending: &mut Vec<(DictKey, DictKey, DictKey)>,
    cache: &HashMap<DictKey, i64>,
    batch_s: &mut Vec<i64>,
    batch_p: &mut Vec<i64>,
    batch_o: &mut Vec<i64>,
) {
    for (s, p, o) in pending.drain(..) {
        batch_s.push(
            *cache
                .get(&s)
                .unwrap_or_else(|| panic!("ingest_turtle_combined: cache miss for subject {s:?}")),
        );
        batch_p.push(
            *cache.get(&p).unwrap_or_else(|| {
                panic!("ingest_turtle_combined: cache miss for predicate {p:?}")
            }),
        );
        batch_o.push(
            *cache
                .get(&o)
                .unwrap_or_else(|| panic!("ingest_turtle_combined: cache miss for object {o:?}")),
        );
    }
}

/// TA-7 dispatch — read `pgrdf.ingest_dict_path` + apply
/// `pgrdf.shmem_prewarm_on_init` (with a per-backend latch), then
/// route to one of: baseline (legacy single-term SPI), batched
/// (TA-D3 spike path), shmem_warm (baseline after a forced prewarm),
/// combined (TA-7 production path). All four produce identical
/// `_pgrdf_quads` rows by construction; only the SPI shape differs.
fn ingest_dispatch<R: Read>(reader: R, graph_id: i64, base_iri: Option<&str>) -> LoaderStats {
    use crate::query::guc::{
        dict_batch_size, ingest_dict_path, shmem_prewarm_on_init, IngestDictPath,
    };
    let path = ingest_dict_path();
    if shmem_prewarm_on_init() || path == IngestDictPath::ShmemWarm {
        maybe_prewarm_once();
    }
    let mut stats = match path {
        IngestDictPath::Baseline | IngestDictPath::ShmemWarm => {
            ingest_turtle_with_stats(reader, graph_id, base_iri)
        }
        IngestDictPath::Batched => {
            ingest_turtle_dict_batched(reader, graph_id, base_iri, dict_batch_size())
        }
        IngestDictPath::Combined => {
            ingest_turtle_combined(reader, graph_id, base_iri, dict_batch_size())
        }
    };
    // TA-5 — record the dispatched route so the verbose JSONB can
    // surface it. `as_str()` matches the GUC enum values exactly.
    stats.path = path.as_str();
    stats
}

thread_local! {
    static PREWARM_DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run the shmem dict-cache prewarm exactly once per backend.
/// Idempotent across the per-backend lifetime; cheap on repeat
/// invocations (just an atomic-cell check).
fn maybe_prewarm_once() {
    if PREWARM_DONE.with(|c| c.get()) {
        return;
    }
    let _ = crate::storage::stats::shmem_cache_prewarm_impl(100_000);
    PREWARM_DONE.with(|c| c.set(true));
}

fn stats_to_jsonb(stats: &LoaderStats) -> pgrx::JsonB {
    pgrx::JsonB(json!({
        "triples":          stats.triples,
        "parse_skipped":    stats.parse_skipped,
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
        "resolve_ms":       stats.resolve_ms,
        "index_ms":         stats.index_ms,
        "insert_ms":        stats.insert_ms,
        "defer_index":      stats.defer_index,
        // TA-5 — the `pgrdf.ingest_dict_path` route the dispatcher
        // selected. Empty only when a stats struct is serialized
        // outside the dispatch (e.g. the `parse_turtle_dict_batched`
        // spike UDF overrides `path` with `dict_batched` after this).
        "path":             stats.path,
        "windows":          stats.windows,
        "dict_terms":       stats.dict_terms,
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

/// TA-6 — combined dict path for the quad stream (TriG / N-Quads).
///
/// Mirrors `ingest_turtle_combined`'s single-pass design (shmem
/// hot-cache check first, defer queue for misses, bulk-resolve via
/// `put_terms_batch` at `dict_batch_size` or before draining the
/// pending buffer) but routes each resolved quad into its
/// destination graph's `GraphBatches` partition instead of a single
/// flat batch. The dictionary is global so the defer queue is shared
/// across graphs; only the quad routing is per-graph.
///
/// Strict-mode semantics are preserved: `resolve_graph_id` resolves
/// (and under `strict` may reject) the destination graph BEFORE the
/// quad's terms are queued, and a rejection panics — aborting the
/// surrounding statement and rolling back every flushed dict row +
/// quad batch, so no partial ingest survives.
fn ingest_quads_combined<P, E>(
    parser: P,
    default_graph_id: i64,
    strict: bool,
    prefix: &'static str,
    dict_batch_size: usize,
) -> (LoaderStats, Vec<i64>)
where
    P: Iterator<Item = Result<oxrdf::Quad, E>>,
    E: std::fmt::Display,
{
    let start = Instant::now();
    let mut stats = LoaderStats::default();
    let mut cache: HashMap<DictKey, i64> = HashMap::new();
    let mut graph_id_cache: HashMap<String, i64> = HashMap::new();
    let mut defer_queue: Vec<DictKey> = Vec::with_capacity(dict_batch_size);
    let mut defer_set: HashSet<DictKey> = HashSet::with_capacity(dict_batch_size);
    // Pending quads carry their resolved destination graph_id so the
    // drain step can route each into the right `GraphBatches`
    // partition once all its term keys are resolved.
    let mut pending: Vec<(i64, DictKey, DictKey, DictKey)> = Vec::with_capacity(BATCH_SIZE);
    let mut batches = GraphBatches::default();
    let mut graphs_order: Vec<i64> = Vec::new();
    let mut graphs_seen: HashSet<i64> = HashSet::new();

    for quad_result in parser {
        let quad = quad_result.unwrap_or_else(|e| panic!("{prefix}: quad parse error: {e}"));
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

        let s_key = subject_key(&quad.subject);
        let p_key: DictKey = (
            term_type::URI,
            quad.predicate.as_str().to_string(),
            None,
            None,
        );
        let o_key = object_key(&quad.object, &mut cache, &mut stats, prefix);

        for key in [&s_key, &p_key, &o_key] {
            try_resolve_or_defer(
                key,
                &mut cache,
                &mut defer_queue,
                &mut defer_set,
                &mut stats,
            );
        }

        pending.push((gid, s_key, p_key, o_key));
        stats.triples += 1;

        if defer_queue.len() >= dict_batch_size {
            flush_defer(&mut defer_queue, &mut defer_set, &mut cache, &mut stats);
        }

        if pending.len() >= BATCH_SIZE {
            // Defer queue MUST be empty before draining — every s/p/o
            // key has to be resolved into `cache` for the lookup.
            if !defer_queue.is_empty() {
                flush_defer(&mut defer_queue, &mut defer_set, &mut cache, &mut stats);
            }
            drain_pending_quads_into_batches(&mut pending, &cache, &mut batches, &mut stats);
        }
    }

    if !defer_queue.is_empty() {
        flush_defer(&mut defer_queue, &mut defer_set, &mut cache, &mut stats);
    }
    drain_pending_quads_into_batches(&mut pending, &cache, &mut batches, &mut stats);
    batches.flush_all(&mut stats);

    stats.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    (stats, graphs_order)
}

/// Walk drained pending quads, lookup s/p/o ids from `cache` (every
/// key was resolved by a prior `flush_defer`), route each into its
/// destination graph's batch partition via `GraphBatches::push`
/// (which auto-flushes a partition once it reaches `BATCH_SIZE`).
fn drain_pending_quads_into_batches(
    pending: &mut Vec<(i64, DictKey, DictKey, DictKey)>,
    cache: &HashMap<DictKey, i64>,
    batches: &mut GraphBatches,
    stats: &mut LoaderStats,
) {
    for (gid, s, p, o) in pending.drain(..) {
        let s_id = *cache
            .get(&s)
            .unwrap_or_else(|| panic!("ingest_quads_combined: cache miss for subject {s:?}"));
        let p_id = *cache
            .get(&p)
            .unwrap_or_else(|| panic!("ingest_quads_combined: cache miss for predicate {p:?}"));
        let o_id = *cache
            .get(&o)
            .unwrap_or_else(|| panic!("ingest_quads_combined: cache miss for object {o:?}"));
        batches.push(gid, s_id, p_id, o_id, stats);
    }
}

/// TA-6 dispatch for the quad stream — read `pgrdf.ingest_dict_path`
/// + apply `pgrdf.shmem_prewarm_on_init`, then route to baseline
/// (`ingest_quads_with_stats`, the per-term SPI path) or combined
/// (`ingest_quads_combined`). `batched` and `shmem_warm` map to the
/// same two physical paths the Turtle dispatch uses: `batched`
/// shares the combined defer-queue mechanism (there is no separate
/// 2-pass quad spike — the quad path was added after TA-D3), and
/// `shmem_warm` is the baseline path after a forced prewarm. All
/// routes produce byte-identical `_pgrdf_quads` + dict rows.
fn ingest_quads_dispatch<P, E>(
    parser: P,
    default_graph_id: i64,
    strict: bool,
    prefix: &'static str,
) -> (LoaderStats, Vec<i64>)
where
    P: Iterator<Item = Result<oxrdf::Quad, E>>,
    E: std::fmt::Display,
{
    use crate::query::guc::{
        dict_batch_size, ingest_dict_path, shmem_prewarm_on_init, IngestDictPath,
    };
    let path = ingest_dict_path();
    if shmem_prewarm_on_init() || path == IngestDictPath::ShmemWarm {
        maybe_prewarm_once();
    }
    let (mut stats, graphs) = match path {
        IngestDictPath::Baseline | IngestDictPath::ShmemWarm => {
            ingest_quads_with_stats(parser, default_graph_id, strict, prefix)
        }
        IngestDictPath::Batched | IngestDictPath::Combined => {
            ingest_quads_combined(parser, default_graph_id, strict, prefix, dict_batch_size())
        }
    };
    // TA-5 — record the dispatched route (see ingest_dispatch).
    stats.path = path.as_str();
    (stats, graphs)
}

/// JSONB stats for the quad-ingest UDFs. Mirrors `stats_to_jsonb`
/// (same verbose-stats keys + conventions as `parse_turtle_verbose`)
/// and extends it with a `graphs` array of the resolved destination
/// graph ids, in first-seen order.
fn quad_stats_to_jsonb(stats: &LoaderStats, graphs: &[i64]) -> pgrx::JsonB {
    pgrx::JsonB(json!({
        "triples":          stats.triples,
        "parse_skipped":    stats.parse_skipped,
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
        "resolve_ms":       stats.resolve_ms,
        "index_ms":         stats.index_ms,
        "insert_ms":        stats.insert_ms,
        "defer_index":      stats.defer_index,
        // TA-5 — the `pgrdf.ingest_dict_path` route the quad
        // dispatcher selected for this call.
        "path":             stats.path,
    }))
}

// ─────────────────────────────────────────────────────────────────────
// UDF surface
// ─────────────────────────────────────────────────────────────────────

/// Drop the hexastore (SPO/POS/OSP) + the dict `_pgrdf_dict_val_idx`
/// hash + the dict `unique_term` UNIQUE constraint (v0.6.4) before a
/// fresh bulk load, so the dict and quad bulk-inserts skip per-row index
/// + uniqueness maintenance. Serialised through the same advisory gate
/// as partition creation (`acquire_partition_ddl_gate`, released at txn
/// end). Only called on the empty-dict fast path at/above
/// `pgrdf.bulk_defer_index_min`, where the quads table is empty. Safe to
/// drop `unique_term`: the self-assigned-id path de-duplicates terms in
/// Rust, so the load produces no duplicate tuples; the re-add in
/// `bulk_rebuild_indexes` VALIDATES that (a backstop against a dedup bug).
///
/// `pub(crate)` so the R2.1 staged loader coordinator
/// (`storage::staged::pool::load_turtle_staged_run`) reuses the EXACT same defer-index drop (incl.
/// the partition-DDL gate) before its STAGE phase — the staged INDEX phase then rebuilds via
/// `jobctl::index_ddls()` (byte-identical to `bulk_rebuild_indexes`).
pub(crate) fn bulk_drop_indexes() {
    crate::storage::partition::acquire_partition_ddl_gate();
    Spi::run(
        "DROP INDEX IF EXISTS pgrdf._pgrdf_idx_spo, pgrdf._pgrdf_idx_pos, \
         pgrdf._pgrdf_idx_osp, pgrdf._pgrdf_dict_val_idx",
    )
    .expect("load_turtle: bulk defer-index drop");
    Spi::run("ALTER TABLE pgrdf._pgrdf_dictionary DROP CONSTRAINT IF EXISTS unique_term")
        .expect("load_turtle: bulk defer unique_term drop");
}

/// Rebuild the indexes + the `unique_term` constraint dropped by
/// `bulk_drop_indexes` after the bulk load completes. The DDL mirrors
/// `sql/schema_v0_2_0.sql` exactly and uses `ON pgrdf._pgrdf_quads` (not
/// `ON ONLY`) so the hexastore indexes cascade to the LIST partitions.
/// `CREATE INDEX` is parallel-aware (honours
/// `max_parallel_maintenance_workers`); the final `ADD CONSTRAINT
/// unique_term` re-validates dictionary uniqueness over the loaded data.
fn bulk_rebuild_indexes() {
    for ddl in [
        "CREATE INDEX _pgrdf_idx_spo ON pgrdf._pgrdf_quads (subject_id, predicate_id, object_id) INCLUDE (is_inferred)",
        "CREATE INDEX _pgrdf_idx_pos ON pgrdf._pgrdf_quads (predicate_id, object_id, subject_id) INCLUDE (is_inferred)",
        "CREATE INDEX _pgrdf_idx_osp ON pgrdf._pgrdf_quads (object_id, subject_id, predicate_id) INCLUDE (is_inferred)",
        "CREATE INDEX _pgrdf_dict_val_idx ON pgrdf._pgrdf_dictionary USING HASH (lexical_value)",
        "ALTER TABLE pgrdf._pgrdf_dictionary ADD CONSTRAINT unique_term \
         UNIQUE (term_type, lexical_md5, datatype_iri_id, language_tag)",
    ] {
        Spi::run(ddl).expect("load_turtle: bulk defer-index rebuild");
    }
}

/// v0.6.2 — parallel bulk-ingest fast path (`load_turtle(..., bulk_load
/// => TRUE)` on a FRESH dictionary). Four passes; SPI touches ONLY the
/// main backend thread and the rayon regions are pure-CPU (no SPI /
/// palloc inside any closure), so it is safe on a single PG backend.
///
///   PASS 1 (rayon) — parse the whole line-oriented `.nt` on all cores,
///                    split at newline boundaries → per-chunk raw triples.
///   PASS 2 (main)  — dedup unique terms in-memory and dictionary-load
///                    with ids RESERVED from the identity sequence
///                    (`nextval` per term, race-free against a concurrent
///                    loader — #20), inserted via `OVERRIDING SYSTEM
///                    VALUE`. No per-term anti-join — that anti-join +
///                    lookup JOIN is the measured 66–74 % of serial
///                    ingest, unnecessary on a fresh dict where the
///                    in-Rust dedup already guarantees one row per term.
///   PASS 3 (rayon) — resolve every triple → (s,p,o) id tuple against the
///                    now-complete read-only term→id maps.
///   PASS 4 (main)  — bulk-insert the quads via the existing prepared
///                    `flush_batch`.
///
/// Dict rows are byte-identical to the serial combined path (same
/// interning rules), so `count_quads` + the LUBM query counts are the
/// correctness gate. The caller guarantees the dict is empty (see
/// `bulk_load_guarded`). `.nt` blank-node labels are document-scoped, so
/// merging by label across chunks is correct for N-Triples; arbitrary
/// multi-line Turtle / anonymous `[]` blanks are out of scope for the
/// newline-split parse (use the default serial path for those).
fn ingest_turtle_parallel_bulk(path: &str, graph_id: i64) -> LoaderStats {
    use rayon::prelude::*;
    // Raw term as parsed; a literal's datatype is carried as its IRI
    // STRING here and resolved to a dict id in PASS 2.
    type RawKey = (i16, String, Option<String>, Option<String>);

    let t_all = Instant::now();
    let mut stats = LoaderStats {
        path: "parallel_bulk",
        ..Default::default()
    };

    let mut bytes = Vec::new();
    File::open(path)
        .unwrap_or_else(|e| panic!("load_turtle: failed to open {path:?}: {e}"))
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("load_turtle: failed to read {path:?}: {e}"));

    let nthreads = rayon::current_num_threads().max(1);
    let total = bytes.len();
    let mut bounds = vec![0usize];
    for k in 1..nthreads {
        let mut i = (total * k) / nthreads;
        while i < total && bytes[i] != b'\n' {
            i += 1;
        }
        if i < total {
            i += 1;
        }
        bounds.push(i.min(total));
    }
    bounds.push(total);
    let chunks: Vec<(usize, usize)> = bounds
        .windows(2)
        .map(|w| (w[0], w[1]))
        .filter(|(a, b)| a < b)
        .collect();

    // ── PASS 1: parallel parse → per-chunk raw triples ──────────────
    let t_parse = Instant::now();
    // Lenient: oxttl recovers after a syntax error (toolkit `error_recovery_state`) and the
    // iterator continues, so a malformed triple (e.g. Wikidata's stray control-char IRIs) is
    // SKIPPED + counted rather than aborting the whole multi-hundred-million-triple load. Full
    // streaming parse speed is retained; the count surfaces as `parse_skipped` in the stats.
    type RawTriple = (RawKey, RawKey, RawKey);
    type ParsedChunk = (Vec<RawTriple>, i64);
    let parsed: Vec<ParsedChunk> = chunks
        .par_iter()
        .map(|&(a, b)| {
            let mut out = Vec::new();
            let mut skipped: i64 = 0;
            for r in TurtleParser::new().for_reader(&bytes[a..b]) {
                let t = match r {
                    Ok(t) => t,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let s: RawKey = match &t.subject {
                    NamedOrBlankNode::NamedNode(n) => {
                        (term_type::URI, n.as_str().to_string(), None, None)
                    }
                    NamedOrBlankNode::BlankNode(bn) => {
                        (term_type::BLANK_NODE, bn.as_str().to_string(), None, None)
                    }
                };
                let p: RawKey = (term_type::URI, t.predicate.as_str().to_string(), None, None);
                let o: RawKey = match &t.object {
                    Term::NamedNode(n) => (term_type::URI, n.as_str().to_string(), None, None),
                    Term::BlankNode(bn) => {
                        (term_type::BLANK_NODE, bn.as_str().to_string(), None, None)
                    }
                    Term::Literal(lit) => match lit.language() {
                        Some(l) => (
                            term_type::LITERAL,
                            lit.value().to_string(),
                            None,
                            Some(l.to_string()),
                        ),
                        None => (
                            term_type::LITERAL,
                            lit.value().to_string(),
                            Some(lit.datatype().as_str().to_string()),
                            None,
                        ),
                    },
                    #[allow(unreachable_patterns)]
                    _ => panic!("load_turtle: unsupported object term"),
                };
                out.push((s, p, o));
            }
            (out, skipped)
        })
        .collect();
    stats.parse_ms = t_parse.elapsed().as_secs_f64() * 1000.0;
    stats.parse_skipped = parsed.iter().map(|(_, s)| *s).sum();
    let per_chunk: Vec<Vec<(RawKey, RawKey, RawKey)>> =
        parsed.into_iter().map(|(c, _)| c).collect();
    let triples: usize = per_chunk.iter().map(|c| c.len()).sum();
    stats.triples = triples as i64;

    // Defer-index (v0.6.3): on a large fresh load, drop the hexastore +
    // dict-hash indexes now and rebuild them after PASS 4, so the dict
    // and quad bulk-inserts don't pay per-row index maintenance. Gated
    // above `pgrdf.bulk_defer_index_min` so tiny loads (the parallel test
    // suite) never take the global ACCESS-EXCLUSIVE index DDL. The
    // empty-dict precondition (see `bulk_load_guarded`) means the quads
    // table is empty, so the rebuild is over just the freshly-loaded data.
    let defer = (triples as i64) >= crate::query::guc::bulk_defer_index_min() as i64;
    stats.defer_index = defer;
    if defer {
        let t_idx = Instant::now();
        bulk_drop_indexes();
        stats.index_ms += t_idx.elapsed().as_secs_f64() * 1000.0;
    }

    // ── PASS 2: dict resolve via sequence-RESERVED ids ──────────────
    let t_dict = Instant::now();
    let chunk_sz = 500_000usize;
    // Reserve `n` ids atomically from the dictionary's IDENTITY sequence
    // (the same allocator GENERATED ALWAYS uses), so a concurrent loader
    // is never handed an overlapping range (#20). Replaces the old
    // `max(id)` snapshot + trailing `setval`, which raced two writers
    // onto the same ids. Ids may be non-contiguous under concurrency
    // (they are opaque, so it is harmless); a lone single-connection
    // load gets a contiguous block, identical to the prior behaviour.
    let reserve = |n: usize| -> Vec<i64> {
        if n == 0 {
            return Vec::new();
        }
        Spi::connect_mut(|client| {
            client
                .update(
                    "SELECT nextval(pg_get_serial_sequence('pgrdf._pgrdf_dictionary','id')) \
                     FROM generate_series(1, $1)",
                    None,
                    &[(n as i64).into()],
                )
                .expect("load_turtle: dict id reservation")
                .into_iter()
                .map(|row| {
                    row.get::<i64>(1)
                        .expect("reserved id")
                        .expect("reserved id NULL")
                })
                .collect::<Vec<i64>>()
        })
    };
    let ins = |ids: Vec<i64>,
               tt: Vec<i16>,
               lv: Vec<String>,
               di: Vec<Option<i64>>,
               lt: Vec<Option<String>>| {
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_dictionary \
             (id, term_type, lexical_value, datatype_iri_id, language_tag) \
             OVERRIDING SYSTEM VALUE \
             SELECT * FROM unnest($1::int8[], $2::int2[], $3::text[], $4::int8[], $5::text[])",
            &[ids.into(), tt.into(), lv.into(), di.into(), lt.into()],
        )
        .expect("load_turtle: dict bulk insert");
    };
    // tier 1 — URI / BLANK terms + literal datatype IRIs (ids base+1..).
    // Dedup in parallel: each chunk builds a local HashSet, then the sets
    // are union-reduced (extend the larger with the smaller to minimise
    // rehashing). The serial single-HashSet build over ~100M term-refs was
    // a measured ~22 s at LUBM-250 (v0.6.5).
    let uri_set: HashSet<(i16, String)> = per_chunk
        .par_iter()
        .map(|chunk| {
            let mut local: HashSet<(i16, String)> = HashSet::new();
            for (s, p, o) in chunk {
                if s.0 != term_type::LITERAL {
                    local.insert((s.0, s.1.clone()));
                }
                local.insert((p.0, p.1.clone()));
                if o.0 != term_type::LITERAL {
                    local.insert((o.0, o.1.clone()));
                } else if let Some(dt) = &o.2 {
                    local.insert((term_type::URI, dt.clone()));
                }
            }
            local
        })
        .reduce(HashSet::new, |mut a, mut b| {
            if a.len() < b.len() {
                std::mem::swap(&mut a, &mut b);
            }
            a.extend(b);
            a
        });
    // Reserve + insert one chunk at a time so neither the reserved-id
    // vector nor the SPI result set ever exceeds `chunk_sz` — bounds
    // memory at billion-term scale (a single reserve(N) would materialise
    // N int8s twice over). `uri_map` is the unavoidable O(N) term→id table.
    let uri_keys: Vec<(i16, String)> = uri_set.into_iter().collect();
    let mut uri_map: HashMap<(i16, String), i64> = HashMap::with_capacity(uri_keys.len());
    for ch in uri_keys.chunks(chunk_sz) {
        let ch_ids = reserve(ch.len());
        for ((tt, lv), id) in ch.iter().zip(&ch_ids) {
            uri_map.insert((*tt, lv.clone()), *id);
        }
        ins(
            ch_ids,
            ch.iter().map(|(tt, _)| *tt).collect(),
            ch.iter().map(|(_, lv)| lv.clone()).collect(),
            vec![None; ch.len()],
            vec![None; ch.len()],
        );
    }
    // tier 2 — literals keyed (lexical, datatype_id, lang); ids base+U+1..
    // Same parallel dedup; `uri_map` is read-only here, safe to share
    // across the rayon closures.
    let lit_set: HashSet<(String, Option<i64>, Option<String>)> = per_chunk
        .par_iter()
        .map(|chunk| {
            let mut local: HashSet<(String, Option<i64>, Option<String>)> = HashSet::new();
            for (_, _, o) in chunk {
                if o.0 == term_type::LITERAL {
                    let dt_id =
                        o.2.as_ref()
                            .map(|dt| uri_map[&(term_type::URI, dt.clone())]);
                    local.insert((o.1.clone(), dt_id, o.3.clone()));
                }
            }
            local
        })
        .reduce(HashSet::new, |mut a, mut b| {
            if a.len() < b.len() {
                std::mem::swap(&mut a, &mut b);
            }
            a.extend(b);
            a
        });
    let lit_keys: Vec<(String, Option<i64>, Option<String>)> = lit_set.into_iter().collect();
    let mut lit_map: HashMap<(String, Option<i64>, Option<String>), i64> =
        HashMap::with_capacity(lit_keys.len());
    for ch in lit_keys.chunks(chunk_sz) {
        let ch_ids = reserve(ch.len());
        for (k, id) in ch.iter().zip(&ch_ids) {
            lit_map.insert(k.clone(), *id);
        }
        ins(
            ch_ids,
            vec![term_type::LITERAL; ch.len()],
            ch.iter().map(|(lv, _, _)| lv.clone()).collect(),
            ch.iter().map(|(_, di, _)| *di).collect(),
            ch.iter().map(|(_, _, lt)| lt.clone()).collect(),
        );
    }
    // No `setval` needed: nextval already advanced the sequence past the
    // reserved block, so a later GENERATED-ALWAYS insert can't collide.
    stats.dict_ms = t_dict.elapsed().as_secs_f64() * 1000.0;

    // ── PASS 3: resolve every triple → id tuple (parallel) ──────────
    let t_resolve = Instant::now();
    let resolve = |k: &RawKey| -> i64 {
        if k.0 == term_type::LITERAL {
            let dt_id =
                k.2.as_ref()
                    .map(|dt| uri_map[&(term_type::URI, dt.clone())]);
            lit_map[&(k.1.clone(), dt_id, k.3.clone())]
        } else {
            uri_map[&(k.0, k.1.clone())]
        }
    };
    let ids: Vec<(i64, i64, i64)> = per_chunk
        .par_iter()
        .flat_map_iter(|chunk| {
            chunk
                .iter()
                .map(|(s, p, o)| (resolve(s), resolve(p), resolve(o)))
        })
        .collect();
    stats.resolve_ms = t_resolve.elapsed().as_secs_f64() * 1000.0;

    // ── PASS 4: bulk INSERT quads via the existing prepared flush_batch ─
    let t_insert = Instant::now();
    let mut bs: Vec<i64> = Vec::with_capacity(BULK_QUAD_BATCH);
    let mut bp: Vec<i64> = Vec::with_capacity(BULK_QUAD_BATCH);
    let mut bo: Vec<i64> = Vec::with_capacity(BULK_QUAD_BATCH);
    for (s, p, o) in &ids {
        bs.push(*s);
        bp.push(*p);
        bo.push(*o);
        if bs.len() >= BULK_QUAD_BATCH {
            flush_batch(&mut bs, &mut bp, &mut bo, graph_id, &mut stats);
            bs.reserve(BULK_QUAD_BATCH);
            bp.reserve(BULK_QUAD_BATCH);
            bo.reserve(BULK_QUAD_BATCH);
        }
    }
    if !bs.is_empty() {
        flush_batch(&mut bs, &mut bp, &mut bo, graph_id, &mut stats);
    }
    stats.insert_ms = t_insert.elapsed().as_secs_f64() * 1000.0;

    // Rebuild the deferred indexes over the freshly-loaded data.
    if defer {
        let t_idx = Instant::now();
        bulk_rebuild_indexes();
        stats.index_ms += t_idx.elapsed().as_secs_f64() * 1000.0;
    }
    stats.elapsed_ms = t_all.elapsed().as_secs_f64() * 1000.0;
    stats
}

/// v0.7 — **streaming / windowed** parallel bulk-ingest. Breaks the whole-file-in-RAM
/// ceiling of `ingest_turtle_parallel_bulk` AND avoids the slow SQL anti-join past
/// window 1, while staying parallel on parse. The `.nt` is streamed through a
/// `BufReader` in WINDOWS of ~`window_triples` lines (never `read_to_end`); a
/// **persistent** in-Rust `HashMap<DictKey,i64>` lives across ALL windows, so dict
/// resolution is an in-memory lookup — never a SQL anti-join. Peak RAM ≈ one window's
/// triples + the persistent map (unique-terms, sub-linear in triples), so it's linear
/// where the whole-file path went super-linear (a 32 GB load was killed at ~2 h).
/// Defer-index ONCE across all windows. Empty-dict by construction (the guard enforces).
/// Single-loader id reservation: contiguous block via `min(nextval … generate_series)`.
fn ingest_turtle_streaming(
    path: &str,
    graph_id: i64,
    window_triples: usize,
    id_reserve_block: usize,
) -> LoaderStats {
    use rayon::prelude::*;
    type RawKey = (i16, String, Option<String>, Option<String>);

    let t_all = Instant::now();
    let mut stats = LoaderStats {
        path: "streaming",
        ..Default::default()
    };

    // Persistent across ALL windows: the anti-join replacement.
    let mut dict: HashMap<DictKey, i64> = HashMap::new();
    let mut next_id: i64 = 0;
    let mut id_hi: i64 = 0;
    let reserve_block_sz = id_reserve_block.max(1) as i64;

    // Reserve a contiguous [lo, lo+n) id block from the dict IDENTITY sequence.
    // (Single-loader: generate_series pulls nextval n times with nothing interleaved,
    // so the block is contiguous; ids are opaque, gaps under concurrency are harmless.)
    let reserve = |n: i64| -> i64 {
        Spi::get_one_with_args::<i64>(
            "SELECT min(v) FROM (SELECT nextval(\
             pg_get_serial_sequence('pgrdf._pgrdf_dictionary','id')) AS v \
             FROM generate_series(1, $1)) s",
            &[n.into()],
        )
        .expect("load_turtle_streaming: dict id block reservation")
        .expect("load_turtle_streaming: id block reservation NULL")
    };
    // Same dict-insert SQL as the bulk path's `ins`.
    let ins = |ids: Vec<i64>,
               tt: Vec<i16>,
               lv: Vec<String>,
               di: Vec<Option<i64>>,
               lt: Vec<Option<String>>| {
        if ids.is_empty() {
            return;
        }
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_dictionary \
             (id, term_type, lexical_value, datatype_iri_id, language_tag) \
             OVERRIDING SYSTEM VALUE \
             SELECT * FROM unnest($1::int8[], $2::int2[], $3::text[], $4::int8[], $5::text[])",
            &[ids.into(), tt.into(), lv.into(), di.into(), lt.into()],
        )
        .expect("load_turtle_streaming: dict bulk insert");
    };

    // Defer-index ONCE across the whole multi-window load.
    let t_idx0 = Instant::now();
    bulk_drop_indexes();
    stats.index_ms += t_idx0.elapsed().as_secs_f64() * 1000.0;
    stats.defer_index = true;

    let file = File::open(path)
        .unwrap_or_else(|e| panic!("load_turtle_streaming: failed to open {path:?}: {e}"));
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut window_lines: Vec<String> = Vec::with_capacity(window_triples);
    let mut bs: Vec<i64> = Vec::with_capacity(BULK_QUAD_BATCH);
    let mut bp: Vec<i64> = Vec::with_capacity(BULK_QUAD_BATCH);
    let mut bo: Vec<i64> = Vec::with_capacity(BULK_QUAD_BATCH);

    loop {
        window_lines.clear();
        let mut eof = false;
        while window_lines.len() < window_triples {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .unwrap_or_else(|e| panic!("load_turtle_streaming: read error on {path:?}: {e}"));
            if n == 0 {
                eof = true;
                break;
            }
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            window_lines.push(line);
        }
        if window_lines.is_empty() {
            break;
        }

        // PASS 1 (rayon): parallel parse this window, lenient (skip+count parse errors).
        let t_parse = Instant::now();
        type RawTriple = (RawKey, RawKey, RawKey);
        type ParsedChunk = (Vec<RawTriple>, i64);
        let parsed: Vec<ParsedChunk> = window_lines
            .par_chunks(4096)
            .map(|lines| {
                let mut out: Vec<(RawKey, RawKey, RawKey)> = Vec::with_capacity(lines.len());
                let mut skipped: i64 = 0;
                for line in lines {
                    for r in TurtleParser::new().for_reader(line.as_bytes()) {
                        let t = match r {
                            Ok(t) => t,
                            Err(_) => {
                                skipped += 1;
                                continue;
                            }
                        };
                        let s: RawKey = match &t.subject {
                            NamedOrBlankNode::NamedNode(n) => {
                                (term_type::URI, n.as_str().to_string(), None, None)
                            }
                            NamedOrBlankNode::BlankNode(bn) => {
                                (term_type::BLANK_NODE, bn.as_str().to_string(), None, None)
                            }
                        };
                        let p: RawKey =
                            (term_type::URI, t.predicate.as_str().to_string(), None, None);
                        let o: RawKey = match &t.object {
                            Term::NamedNode(n) => {
                                (term_type::URI, n.as_str().to_string(), None, None)
                            }
                            Term::BlankNode(bn) => {
                                (term_type::BLANK_NODE, bn.as_str().to_string(), None, None)
                            }
                            Term::Literal(lit) => match lit.language() {
                                Some(l) => (
                                    term_type::LITERAL,
                                    lit.value().to_string(),
                                    None,
                                    Some(l.to_string()),
                                ),
                                None => (
                                    term_type::LITERAL,
                                    lit.value().to_string(),
                                    Some(lit.datatype().as_str().to_string()),
                                    None,
                                ),
                            },
                            #[allow(unreachable_patterns)]
                            _ => panic!("load_turtle_streaming: unsupported object term"),
                        };
                        out.push((s, p, o));
                    }
                }
                (out, skipped)
            })
            .collect();
        stats.parse_ms += t_parse.elapsed().as_secs_f64() * 1000.0;
        stats.parse_skipped += parsed.iter().map(|(_, s)| *s).sum::<i64>();
        let window: Vec<(RawKey, RawKey, RawKey)> =
            parsed.into_iter().flat_map(|(c, _)| c).collect();
        stats.triples += window.len() as i64;

        // PASS 2 (main): intern NEW terms into the persistent map. Macros (not closures)
        // so `n_*` are not borrow-captured and `mem::take` is free after each tier.
        let t_dict = Instant::now();
        let mut n_ids: Vec<i64> = Vec::new();
        let mut n_tt: Vec<i16> = Vec::new();
        let mut n_lv: Vec<String> = Vec::new();
        let mut n_di: Vec<Option<i64>> = Vec::new();
        let mut n_lt: Vec<Option<String>> = Vec::new();
        macro_rules! alloc_id {
            () => {{
                if next_id >= id_hi {
                    let lo = reserve(reserve_block_sz);
                    next_id = lo;
                    id_hi = lo + reserve_block_sz;
                }
                let id = next_id;
                next_id += 1;
                id
            }};
        }
        macro_rules! intern1 {
            ($tt:expr, $lv:expr) => {{
                let key: DictKey = ($tt, $lv.to_string(), None, None);
                if !dict.contains_key(&key) {
                    let id = alloc_id!();
                    n_ids.push(id);
                    n_tt.push($tt);
                    n_lv.push($lv.to_string());
                    n_di.push(None);
                    n_lt.push(None);
                    dict.insert(key, id);
                }
            }};
        }
        // tier-1: URI/blank + datatype IRIs first (so a literal's datatype id exists).
        for (s, p, o) in &window {
            if s.0 != term_type::LITERAL {
                intern1!(s.0, &s.1);
            }
            intern1!(p.0, &p.1);
            if o.0 != term_type::LITERAL {
                intern1!(o.0, &o.1);
            } else if let Some(dt) = &o.2 {
                intern1!(term_type::URI, dt);
            }
        }
        ins(
            mem::take(&mut n_ids),
            mem::take(&mut n_tt),
            mem::take(&mut n_lv),
            mem::take(&mut n_di),
            mem::take(&mut n_lt),
        );
        // tier-2: literals, keyed (LITERAL, lexical, datatype_id, lang).
        macro_rules! intern2 {
            ($lv:expr, $di:expr, $lt:expr) => {{
                let key: DictKey = (term_type::LITERAL, $lv.to_string(), $di, $lt.clone());
                if !dict.contains_key(&key) {
                    let id = alloc_id!();
                    n_ids.push(id);
                    n_tt.push(term_type::LITERAL);
                    n_lv.push($lv.to_string());
                    n_di.push($di);
                    n_lt.push($lt.clone());
                    dict.insert(key, id);
                }
            }};
        }
        for (_, _, o) in &window {
            if o.0 == term_type::LITERAL {
                let di =
                    o.2.as_ref()
                        .map(|dt| dict[&(term_type::URI, dt.clone(), None, None)]);
                intern2!(&o.1, di, &o.3);
            }
        }
        ins(
            mem::take(&mut n_ids),
            mem::take(&mut n_tt),
            mem::take(&mut n_lv),
            mem::take(&mut n_di),
            mem::take(&mut n_lt),
        );
        stats.dict_ms += t_dict.elapsed().as_secs_f64() * 1000.0;

        // PASS 3+4: resolve (pure lookups) + bulk-insert quads via flush_batch.
        let t_ins = Instant::now();
        for (s, p, o) in &window {
            let rs = dict[&(s.0, s.1.clone(), None, None)];
            let rp = dict[&(p.0, p.1.clone(), None, None)];
            let ro = if o.0 == term_type::LITERAL {
                let di =
                    o.2.as_ref()
                        .map(|dt| dict[&(term_type::URI, dt.clone(), None, None)]);
                dict[&(term_type::LITERAL, o.1.clone(), di, o.3.clone())]
            } else {
                dict[&(o.0, o.1.clone(), None, None)]
            };
            bs.push(rs);
            bp.push(rp);
            bo.push(ro);
            if bs.len() >= BULK_QUAD_BATCH {
                flush_batch(&mut bs, &mut bp, &mut bo, graph_id, &mut stats);
                bs.reserve(BULK_QUAD_BATCH);
                bp.reserve(BULK_QUAD_BATCH);
                bo.reserve(BULK_QUAD_BATCH);
            }
        }
        stats.insert_ms += t_ins.elapsed().as_secs_f64() * 1000.0;
        stats.windows += 1;
        if eof {
            break;
        }
    }
    if !bs.is_empty() {
        let t_ins = Instant::now();
        flush_batch(&mut bs, &mut bp, &mut bo, graph_id, &mut stats);
        stats.insert_ms += t_ins.elapsed().as_secs_f64() * 1000.0;
    }

    let t_idx1 = Instant::now();
    bulk_rebuild_indexes();
    stats.index_ms += t_idx1.elapsed().as_secs_f64() * 1000.0;

    stats.dict_terms = dict.len() as i64;
    stats.elapsed_ms = t_all.elapsed().as_secs_f64() * 1000.0;
    stats
}

/// Guard for the streaming path: like `bulk_load_guarded`, correct ONLY on an empty
/// dict (it dedups via its persistent map, not against existing rows). Empty → stream;
/// populated → fall back to the always-correct combined `ingest_dispatch`.
fn streaming_load_guarded(
    path: &str,
    graph_id: i64,
    window_triples: usize,
    id_reserve_block: usize,
    base_iri: Option<&str>,
) -> LoaderStats {
    let empty = Spi::get_one::<bool>("SELECT NOT EXISTS (SELECT 1 FROM pgrdf._pgrdf_dictionary)")
        .expect("load_turtle_streaming: empty-dict probe")
        .unwrap_or(false);
    if empty {
        ingest_turtle_streaming(path, graph_id, window_triples, id_reserve_block)
    } else {
        let file = File::open(path)
            .unwrap_or_else(|e| panic!("load_turtle_streaming: failed to open {path:?}: {e}"));
        ingest_dispatch(BufReader::new(file), graph_id, base_iri)
    }
}

/// v0.7 — streaming/windowed parallel bulk ingest of a line-oriented N-Triples file.
/// Bounded RAM (one window + the persistent dict) and no SQL anti-join — the fix for
/// the whole-file `bulk_load => TRUE` ceiling + super-linearity. Requires an EMPTY dict.
///
/// SQL: `pgrdf.load_turtle_streaming(path TEXT, graph_id BIGINT,
///       window_triples INT DEFAULT 20000000, id_reserve_block INT DEFAULT 1000000,
///       base_iri TEXT DEFAULT NULL) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn load_turtle_streaming(
    path: &str,
    graph_id: i64,
    window_triples: default!(i32, 20_000_000),
    id_reserve_block: default!(i32, 1_000_000),
    base_iri: default!(Option<&str>, "NULL"),
) -> pgrx::JsonB {
    let base = base_iri.filter(|s| !s.is_empty());
    let stats = streaming_load_guarded(
        path,
        graph_id,
        window_triples.max(1) as usize,
        id_reserve_block.max(1) as usize,
        base,
    );
    stats_to_jsonb(&stats)
}

// ─────────────────────────────────────────────────────────────────────
// §6 — v0.6.14 format sniff (N-Triples vs Turtle) for the staged dispatch
// ─────────────────────────────────────────────────────────────────────
//
// The staged loader's STAGE phase parses with `oxttl::NTriplesParser` —
// strictly line-oriented N-Triples. Routing a full Turtle file (one with
// `@prefix` / `@base` directives, `pfx:name` prefixed names, multi-line
// `;`/`,` predicate-object lists) through that parser would make it
// lenient-SKIP every line it can't read as a complete bare-term statement
// = SILENT DATA LOSS. So `load_turtle` only takes the staged path when it
// is CONFIDENT the input is N-Triples; everything else falls back to the
// full `TurtleParser`. The sniff below is the confidence gate, and it is
// deliberately conservative: any doubt returns `false` (⇒ Turtle, safe).

/// Bytes of the input sampled by [`file_sniffs_as_ntriples`]. 64 KiB is
/// plenty to classify a line-oriented format from its first records while
/// staying a single cheap read; capped further by [`SNIFF_MAX_LINES`].
const SNIFF_SAMPLE_BYTES: usize = 64 * 1024;

/// Maximum non-blank, non-comment lines inspected by [`sniff_is_ntriples`].
/// Bounds the work on pathological inputs (e.g. one enormous line) and is
/// the "~first 200 lines" half of the sample budget.
const SNIFF_MAX_LINES: usize = 200;

/// Per-line phase of [`sniff_is_ntriples`]: classify ONE physical line as
/// "looks like a complete bare-term N-Triples statement" or not.
///
/// Walks the line token-by-token, consuming each TERM as a unit so that a
/// `:` / `;` / `,` / `@` living INSIDE an `<IRI>` or a `"…"`/`'…'` literal
/// is never mistaken for Turtle syntax. The terms it accepts are exactly
/// the bare-term N-Triples vocabulary:
///
/// * **IRI** `<…>` — consumed to the first unescaped `>` (`\>` stays
///   inside; an unterminated `<…` ⇒ not a clean line).
/// * **blank node** `_:label` — consumed to the next whitespace / term
///   boundary. A bare `_` not followed by `:` disqualifies.
/// * **literal** `"…"` / `'…'` — consumed honouring `\` escapes, optionally
///   decorated by a `@lang` tag (`[A-Za-z0-9-]+`) OR a `^^<datatype>` whose
///   datatype is itself an absolute `<IRI>`. A long-string opener
///   (`"""` / `'''`) is Turtle-only ⇒ disqualifies. A `@`/`^` NOT
///   decorating a just-closed literal disqualifies (catches `@prefix` /
///   `@base` and a `^^pfx:type` prefixed datatype).
/// * **terminator** `.` — outside any term; marks the statement complete.
///
/// ANY other token ⇒ NOT N-Triples (⇒ Turtle): a bare `:` (a `pfx:name`
/// prefixed name), `;` / `,` (predicate-object lists), `[` `]` `(` `)`
/// `{` `}` (blank-node property lists / collections / TriG graph blocks),
/// or any bare alphanumeric token (the `a` keyword, `true`, a number,
/// `PREFIX` / `BASE`). A complete statement must also END (after optional
/// trailing whitespace + a trailing `# comment`) with that `.`; an
/// unterminated or continued line (the body of a `;` / `,` list, or two
/// statements on one physical line) fails. Conservative throughout — a
/// malformed decoration like `"x" @en` (space before `@`) is treated as
/// Turtle and routed to the full parser, which is the safe direction.
fn line_is_bare_ntriples(line: &str) -> bool {
    let bytes = line.as_bytes();
    // Did we see a statement-terminating `.` outside any IRI/string?
    let mut terminated = false;
    // True for exactly one TERM-position after a literal closes, so the
    // optional `@lang` / `^^<datatype>` that decorates it is accepted (and
    // consumed) rather than treated as a stray token.
    let mut after_literal = false;
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\r' | b'\n' => {
                i += 1;
                continue;
            }
            // A `#` outside a term begins a trailing comment — the
            // statement (if any) is complete; stop scanning.
            b'#' => break,
            // Anything below is a term/punctuation token at statement
            // level: nothing legal follows the terminating `.`.
            _ if terminated => return false,
            // ── IRI: <…> ───────────────────────────────────────────
            b'<' => {
                // Consume to the first unescaped `>`. N-Triples forbids a
                // raw `<`/`>` inside; `\>` (uchar/echar) stays inside.
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return false, // unterminated IRI ⇒ not clean
                        Some(b'\\') => i += 2,
                        Some(b'>') => {
                            i += 1;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
                after_literal = false;
            }
            // ── Blank node: _:label ────────────────────────────────
            b'_' if bytes.get(i + 1) == Some(&b':') => {
                i += 2;
                // Consume the blank-node label (no whitespace / term
                // punctuation). A bare `_` not followed by `:` falls to the
                // catch-all and disqualifies (illegal bare token).
                while let Some(&b) = bytes.get(i) {
                    if b.is_ascii_whitespace() || matches!(b, b'.' | b'<' | b'"' | b'\'' | b'#') {
                        break;
                    }
                    i += 1;
                }
                after_literal = false;
            }
            // ── Literal: "…" or '…' ────────────────────────────────
            b'"' | b'\'' => {
                let quote = c;
                // A long-string opener (""" / ''') is Turtle-only.
                if bytes.get(i + 1) == Some(&quote) && bytes.get(i + 2) == Some(&quote) {
                    return false;
                }
                i += 1;
                loop {
                    match bytes.get(i) {
                        None => return false, // unterminated string ⇒ not clean
                        Some(b'\\') => i += 2,
                        Some(&b) if b == quote => {
                            i += 1;
                            break;
                        }
                        Some(_) => i += 1,
                    }
                }
                after_literal = true;
            }
            // ── @lang tag — only valid decorating a just-closed literal ─
            b'@' if after_literal => {
                i += 1;
                let start = i;
                while let Some(&b) = bytes.get(i) {
                    if b.is_ascii_alphanumeric() || b == b'-' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                if i == start {
                    return false; // bare `@` with no tag body
                }
                after_literal = false;
            }
            // ── ^^<datatype> — only valid decorating a just-closed literal ─
            b'^' if after_literal && bytes.get(i + 1) == Some(&b'^') => {
                i += 2;
                // The datatype MUST be an absolute `<IRI>` in N-Triples
                // (a `pfx:type` prefixed datatype is Turtle). Skip optional
                // whitespace, then require `<`; the loop's `<` arm consumes
                // the IRI on the next iteration.
                while matches!(bytes.get(i), Some(b' ') | Some(b'\t')) {
                    i += 1;
                }
                if bytes.get(i) != Some(&b'<') {
                    return false;
                }
                after_literal = false;
                // Fall through to the next iteration to consume the IRI.
            }
            // ── Statement terminator ───────────────────────────────
            b'.' => {
                terminated = true;
                after_literal = false;
                i += 1;
            }
            // ── Everything else is Turtle (or malformed N-Triples) ──
            // A bare `:` (a `pfx:name` prefixed name), `;`/`,` (predicate-
            // object lists), `[]`/`()`/`{}` (blank-node / collection / TriG
            // graph blocks), a stray `@`/`^` not decorating a literal, or
            // any bare alphanumeric token (`a` keyword, `true`, a number,
            // `PREFIX`/`BASE`) — none legal in a bare-term N-Triples line.
            _ => return false,
        }
    }
    // Must have cleanly terminated and not be mid-term.
    terminated && !after_literal
}

/// Pure format classifier: does `sample` (the head of a file, as bytes)
/// look like **N-Triples**? Returns `true` ONLY when confident — every
/// sampled non-blank, non-comment line is a complete bare-term statement
/// (see [`line_is_bare_ntriples`]) and the sample held at least one such
/// line. Anything else — a `@prefix`/`@base` directive, a `pfx:name`
/// prefixed term, a `;`/`,` predicate-object list, a multi-line /
/// unterminated statement, an `a`/`PREFIX`/`BASE` keyword, or simply an
/// empty/comment-only sample — returns `false` (⇒ Turtle, the safe
/// default that always uses the full parser).
///
/// Pure over `&[u8]` so it unit-tests as a plain `#[test]` without the
/// pgrx harness. A final partial line (the sample boundary cut a line in
/// half) is dropped rather than judged, so a truncated last line can't
/// cause a false negative on an otherwise-clean N-Triples head.
fn sniff_is_ntriples(sample: &[u8]) -> bool {
    let text = String::from_utf8_lossy(sample);
    // Drop a trailing partial line only when the sample didn't end on a
    // newline (i.e. the read boundary split a line). A sample that ends
    // exactly on `\n` has no partial tail to drop.
    let ends_clean = text.ends_with('\n');
    let mut lines: Vec<&str> = text.lines().collect();
    if !ends_clean && lines.len() > 1 {
        lines.pop();
    }

    let mut saw_statement = false;
    let mut inspected = 0usize;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        inspected += 1;
        if inspected > SNIFF_MAX_LINES {
            break;
        }
        if !line_is_bare_ntriples(trimmed) {
            return false;
        }
        saw_statement = true;
    }
    // Confident N-Triples requires at least one clean statement; an empty
    // or comment-only sample is NOT confidently N-Triples ⇒ Turtle.
    saw_statement
}

/// Read the head of `path` (≤ [`SNIFF_SAMPLE_BYTES`]) and classify it via
/// [`sniff_is_ntriples`]. Any I/O error ⇒ `false` (⇒ Turtle / the standard
/// path), which then opens the file itself and surfaces the real open
/// error with the loader's existing message — the sniff never owns the
/// failure path.
fn file_sniffs_as_ntriples(path: &str) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut buf = vec![0u8; SNIFF_SAMPLE_BYTES];
    let mut filled = 0usize;
    let mut reader = BufReader::new(file);
    // Fill up to SNIFF_SAMPLE_BYTES across however many short reads it
    // takes (a single `read` may return fewer bytes than requested).
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return false,
        }
    }
    sniff_is_ntriples(&buf[..filled])
}

/// v0.6.14 — drive the native STAGED loader (`load_turtle_staged_run`) and
/// return its triple count, so `load_turtle` can route a confidently
/// N-Triples file to the fast staged path when pgRDF is preloaded. Invoked
/// via SPI (`SELECT pgrdf.load_turtle_staged_run($1,$2,$3)`) rather than a
/// direct Rust call: the coordinator lives in `storage::staged::pool`
/// (owned by another module, not re-exported), and the SPI route reuses its
/// exact spawn/wait/gate behaviour unchanged.
///
/// `n_workers = 0` ⇒ auto, matching `load_turtle_staged_run`'s own default.
/// The staged coordinator returns `{ok, triples, …}` on success, or — on a
/// phase failure — `{ok:false, failed_phase, error, …}` with the staging
/// table left as the resume point (it does NOT panic). We mirror that: on
/// `ok:false` raise the staged error so the caller sees the failure (the
/// resume point persists); on success return the `triples` count.
fn staged_load_default(path: &str, graph_id: i64) -> i64 {
    let j: pgrx::JsonB = Spi::get_one_with_args(
        "SELECT pgrdf.load_turtle_staged_run($1, $2, $3)",
        &[path.into(), graph_id.into(), 0i32.into()],
    )
    .expect("load_turtle: staged coordinator SPI call")
    .expect("load_turtle: staged coordinator returned NULL");

    let v = &j.0;
    if v.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        let phase = v
            .get("failed_phase")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let reason = v
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unspecified");
        error!(
            "load_turtle: staged loader aborted in the {phase} phase: {reason} \
             (staging table left in place as the resume point)"
        );
    }
    v.get("triples")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0)
}

/// Guard for `load_turtle(..., bulk_load => TRUE)`. The self-assigned-id
/// fast path is correct ONLY on an empty dictionary: it dedups within
/// the input (not against existing rows) and `unique_term` is
/// NULLS-DISTINCT, so a clash with an existing term would silently
/// insert a duplicate. An empty dict also implies empty quads, which is
/// what makes the (future) defer-index step safe. On a populated dict we
/// fall back to the standard `ingest_dispatch` (combined) path, which
/// anti-joins every term — slower, always correct. Callers see the
/// `path` discriminator (`parallel_bulk` vs the GUC route) to confirm
/// which fired. Recommended order for the win: bulk-load the large file
/// FIRST into a fresh database, then load smaller files normally.
fn bulk_load_guarded(path: &str, graph_id: i64, base_iri: Option<&str>) -> LoaderStats {
    let empty = Spi::get_one::<bool>("SELECT NOT EXISTS (SELECT 1 FROM pgrdf._pgrdf_dictionary)")
        .expect("bulk_load: empty-dict probe")
        .unwrap_or(false);
    if empty {
        ingest_turtle_parallel_bulk(path, graph_id)
    } else {
        let file = File::open(path)
            .unwrap_or_else(|e| panic!("load_turtle: failed to open {path:?}: {e}"));
        ingest_dispatch(BufReader::new(file), graph_id, base_iri)
    }
}

/// Load a Turtle file from a server-side path into the named graph.
/// Returns the number of triples inserted. `base_iri` resolves
/// relative IRIs; pass NULL or '' for absolute-IRI-only files.
///
/// **v0.6.14 — format-aware staged dispatch.** When ALL of three hold —
/// (1) pgRDF is in `shared_preload_libraries` (so the staged worker pool /
/// jobctl shmem segment is initialised, [`jobctl::is_ready`]), (2) the file
/// SNIFFS as N-Triples ([`file_sniffs_as_ntriples`]), and (3) no `base_iri`
/// is given — the default path dispatches to the native STAGED loader
/// (`load_turtle_staged_run`, `n_workers = 0` auto), which is materially
/// faster on large files. Otherwise the call uses the standard full-Turtle
/// `ingest_dispatch` path UNCHANGED.
///
/// The sniff is the safety gate: the staged STAGE phase parses with
/// `oxttl::NTriplesParser` (line-oriented N-Triples only), so a full Turtle
/// file routed there would be lenient-SKIPPED line-by-line = silent data
/// loss. We therefore route to staged ONLY when confident the input is
/// N-Triples; **Turtle ALWAYS uses the full parser** (no silent loss).
/// A `base_iri` likewise forces the standard path — the N-Triples staged
/// STAGE has no relative-IRI base. When the input is Turtle (and pgRDF is
/// preloaded) a `NOTICE` recommends N-Triples + preload for the fast path.
///
/// At-scale stability of the staged path depends on the RESOLVE temp
/// routing / memory reduction — for billion-scale loads point
/// `pgrdf.staged_temp_tablespaces` at a roomy mount.
///
/// `bulk_load => TRUE` selects the v0.6.2 parallel bulk fast path on a
/// fresh database (line-oriented N-Triples input; see
/// `ingest_turtle_parallel_bulk`); it transparently falls back to the
/// default path when the dictionary is already populated. It is independent
/// of the staged dispatch — an explicit opt-in to the older in-backend bulk
/// path — and is unchanged by v0.6.14.
///
/// SQL: `pgrdf.load_turtle(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, bulk_load BOOLEAN DEFAULT FALSE) -> BIGINT`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn load_turtle(
    path: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
    bulk_load: default!(bool, false),
) -> i64 {
    let base = base_iri.filter(|s| !s.is_empty());
    // Explicit opt-in to the v0.6.2 in-backend parallel bulk path is honoured
    // verbatim — it predates the staged loader and stays the caller's choice.
    if bulk_load {
        return bulk_load_guarded(path, graph_id, base).triples;
    }
    // v0.6.14 format-aware dispatch: prefer the native staged loader, but
    // ONLY when its worker pool is available (pgRDF preloaded), the file is
    // confidently N-Triples (the staged STAGE phase is N-Triples-only — a
    // Turtle file would be silently skipped there), AND no base_iri is set
    // (staged has no relative-IRI base). Any miss ⇒ the standard full-Turtle
    // path, unchanged.
    if crate::storage::staged::jobctl::is_ready() && base.is_none() {
        if file_sniffs_as_ntriples(path) {
            return staged_load_default(path, graph_id);
        }
        // Preloaded but the input is Turtle — the safe full parser runs.
        // Nudge toward the fast staged path (N-Triples + preload).
        notice!(
            "pgrdf.load_turtle: input is Turtle (prefixed/multi-line); using the full \
             parser. For the faster staged loader, supply N-Triples (one bare-term \
             statement per line) with pgrdf preloaded"
        );
    }
    let file =
        File::open(path).unwrap_or_else(|e| panic!("load_turtle: failed to open {path:?}: {e}"));
    ingest_dispatch(BufReader::new(file), graph_id, base).triples
}

/// Same as `load_turtle` but returns JSONB stats: triples,
/// dict_cache_hits, shmem_cache_hits, dict_db_calls, quad_batches,
/// elapsed_ms, and the parse/dict/resolve/insert phase breakdown.
/// Useful for measuring whether the cache + batching paths are firing.
/// `bulk_load => TRUE` measures the v0.6.2 parallel bulk path (the
/// `resolve_ms` phase is non-zero only there).
///
/// SQL: `pgrdf.load_turtle_verbose(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, bulk_load BOOLEAN DEFAULT FALSE) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn load_turtle_verbose(
    path: &str,
    graph_id: i64,
    base_iri: default!(Option<&str>, "NULL"),
    bulk_load: default!(bool, false),
) -> pgrx::JsonB {
    let base = base_iri.filter(|s| !s.is_empty());
    let stats = if bulk_load {
        bulk_load_guarded(path, graph_id, base)
    } else {
        let file = File::open(path)
            .unwrap_or_else(|e| panic!("load_turtle_verbose: failed to open {path:?}: {e}"));
        ingest_dispatch(BufReader::new(file), graph_id, base)
    };
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
    ingest_dispatch(content.as_bytes(), graph_id, base).triples
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
    let stats = ingest_dispatch(content.as_bytes(), graph_id, base);
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
    let (stats, graphs) = ingest_quads_dispatch(parser, default_graph_id, strict, "parse_trig");
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
    let (stats, graphs) = ingest_quads_dispatch(parser, default_graph_id, strict, "parse_nquads");
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

    /// R1: a literal longer than PostgreSQL's 2704-byte btree-key limit loads.
    /// `unique_term` now keys on `lexical_md5` (a 16-byte hash of the value), so a
    /// long Wikidata-style literal no longer overflows the btree. Before R1 the
    /// raw-`lexical_value` unique btree aborted such inserts — at Wikidata scale
    /// (a measured 3312-byte literal) this rolled back the entire 8.2B-triple
    /// full-truthy load at the final index rebuild.
    #[pg_test]
    fn parse_turtle_long_literal_over_btree_limit() {
        let long = "x".repeat(3000); // > 2704
        let ttl = format!("@prefix ex: <http://example.com/> .\nex:s ex:p \"{long}\" .\n");
        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), 7_133i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(n, 1);
        let got: Option<i64> = Spi::get_one_with_args(
            "SELECT id FROM pgrdf._pgrdf_dictionary WHERE term_type = 3 AND length(lexical_value) = $1",
            &[3000i32.into()],
        )
        .unwrap();
        assert!(got.is_some(), "3000-byte literal must be in the dictionary");
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

    // ─── TA-4: dict-path matrix parity (pgrx level) ────────────────
    //
    // The SQL regression gates 130 (Turtle) + 132 (N-Quads/TriG)
    // already lock dict-path parity against the compose Postgres.
    // These pgrx tests lock the SAME invariant against the freshly
    // built `.so` inside CI's `test (17)` job — a distinct execution
    // path from the pg_regress-style runner — so a path divergence
    // is caught at the unit level too. The fixture deliberately
    // carries no blank nodes (whose labels are parser-assigned and
    // need not byte-match across two parser invocations), so every
    // triple can be compared by decoded lexical value with no caveat.

    /// Assert that the same content ingested under all four
    /// `pgrdf.ingest_dict_path` values into `gids` produced
    /// byte-identical decoded-lexical triple sets — i.e. exactly
    /// `expected_triples` distinct (s,p,o,o_type,o_has_dt,o_lang)
    /// rows shared across all four graphs, and each graph holds
    /// exactly that many quads.
    fn assert_path_matrix_parity(gids: [i64; 4], expected_triples: i64) {
        let glist = format!("{},{},{},{}", gids[0], gids[1], gids[2], gids[3]);
        let sql = format!(
            "WITH lex AS (
               SELECT q.graph_id,
                      s.lexical_value sl, p.lexical_value pl, o.lexical_value ol,
                      o.term_type ot, (o.datatype_iri_id IS NOT NULL) od,
                      o.language_tag og
                 FROM pgrdf._pgrdf_quads q
                 JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
                 JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
                 JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
                WHERE q.graph_id IN ({glist})
             )
             SELECT
               -- all four paths agree on the exact lexical-triple set:
               (SELECT count(DISTINCT (sl,pl,ol,ot,od,og)) FROM lex) = {expected_triples}
               -- all four graphs are present:
               AND (SELECT count(DISTINCT graph_id) FROM lex) = 4
               -- each graph holds exactly the expected quad count:
               AND (SELECT bool_and(c = {expected_triples})
                      FROM (SELECT count(*) c FROM lex GROUP BY graph_id) z)"
        );
        let ok: bool = Spi::get_one(&sql).unwrap().unwrap_or(false);
        assert!(
            ok,
            "dict-path matrix parity failed for graphs {glist} (expected {expected_triples} shared triples)"
        );
    }

    /// Turtle ingested under baseline / batched / shmem_warm /
    /// combined yields identical decoded-lexical triple sets.
    #[pg_test]
    fn dict_path_matrix_turtle() {
        let ttl = r#"
            @prefix ex:  <http://example.com/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:s1 ex:name  "Alice" .
            ex:s2 ex:age   "30"^^xsd:integer .
            ex:s3 ex:greet "Hello"@en .
            ex:s4 ex:ref   ex:target .
            ex:s5 ex:zero  "0030"^^xsd:integer .
        "#;
        let paths = ["baseline", "batched", "shmem_warm", "combined"];
        let gids = [7_400i64, 7_401, 7_402, 7_403];
        for (p, gid) in paths.iter().zip(gids.iter()) {
            Spi::run(&format!("SET pgrdf.ingest_dict_path = '{p}'")).unwrap();
            Spi::run_with_args(
                "SELECT pgrdf.parse_turtle($1, $2)",
                &[ttl.into(), (*gid).into()],
            )
            .unwrap();
        }
        Spi::run("SET pgrdf.ingest_dict_path = 'combined'").unwrap();
        assert_path_matrix_parity(gids, 5);
    }

    /// N-Quads (all 3-position → default graph) ingested under all
    /// four paths into per-path default graphs yields identical sets.
    #[pg_test]
    fn dict_path_matrix_nquads() {
        let nq = concat!(
            "<http://example.com/s1> <http://example.com/name> \"Alice\" .\n",
            "<http://example.com/s2> <http://example.com/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://example.com/s3> <http://example.com/greet> \"Hello\"@en .\n",
            "<http://example.com/s4> <http://example.com/ref> <http://example.com/target> .\n",
            "<http://example.com/s5> <http://example.com/zero> \"0030\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
        );
        let paths = ["baseline", "batched", "shmem_warm", "combined"];
        let gids = [7_410i64, 7_411, 7_412, 7_413];
        for (p, gid) in paths.iter().zip(gids.iter()) {
            Spi::run(&format!("SET pgrdf.ingest_dict_path = '{p}'")).unwrap();
            Spi::run_with_args(
                "SELECT pgrdf.parse_nquads($1, $2)",
                &[nq.into(), (*gid).into()],
            )
            .unwrap();
        }
        Spi::run("SET pgrdf.ingest_dict_path = 'combined'").unwrap();
        assert_path_matrix_parity(gids, 5);
    }

    /// TriG (default-graph triples → per-path default graph) ingested
    /// under all four paths yields identical decoded-lexical sets.
    #[pg_test]
    fn dict_path_matrix_trig() {
        let trig = r#"
            @prefix ex:  <http://example.com/> .
            @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
            ex:s1 ex:name  "Alice" .
            ex:s2 ex:age   "30"^^xsd:integer .
            ex:s3 ex:greet "Hello"@en .
            ex:s4 ex:ref   ex:target .
            ex:s5 ex:zero  "0030"^^xsd:integer .
        "#;
        let paths = ["baseline", "batched", "shmem_warm", "combined"];
        let gids = [7_420i64, 7_421, 7_422, 7_423];
        for (p, gid) in paths.iter().zip(gids.iter()) {
            Spi::run(&format!("SET pgrdf.ingest_dict_path = '{p}'")).unwrap();
            Spi::run_with_args(
                "SELECT pgrdf.parse_trig($1, $2)",
                &[trig.into(), (*gid).into()],
            )
            .unwrap();
        }
        Spi::run("SET pgrdf.ingest_dict_path = 'combined'").unwrap();
        assert_path_matrix_parity(gids, 5);
    }

    // ─── v0.6.2 — parallel bulk-ingest fast path ────────────────────
    //
    // `load_turtle(path, graph_id, base_iri, bulk_load => TRUE)` is a
    // FRESH-LOAD fast path: rayon-parse the line-oriented .nt on all
    // cores, dedup terms in-memory, dictionary-load with SELF-ASSIGNED
    // ids (no per-term anti-join), resolve triples → id tuples in
    // parallel, bulk-insert quads. Valid only on an EMPTY dictionary;
    // on a populated dict it falls back to the (correct) `combined`
    // path. These tests lock both behaviours + the verbose breakdown.
    //
    // Each `#[pg_test]` runs in a rolled-back txn, so the dictionary
    // starts EMPTY — the fast path fires. Fixtures carry no blank nodes
    // (parser-assigned labels needn't byte-match across invocations).

    /// Fast path fires on an empty dict and is byte-correct: exact
    /// triple/quad counts, the right decoded-lexical set, and — the
    /// self-assigned-id invariant — every logical term interned EXACTLY
    /// once (no duplicate dict rows; GROUP BY treats the NULL datatype/
    /// lang columns as equal, unlike the NULLS-DISTINCT `unique_term`).
    #[pg_test]
    fn load_turtle_bulk_basic_parity() {
        let nt = concat!(
            "<http://ex/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .\n",
            "<http://ex/alice> <http://xmlns.com/foaf/0.1/name> \"Alice\" .\n",
            "<http://ex/alice> <http://ex/age> \"30\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
            "<http://ex/alice> <http://ex/greet> \"Hi\"@en .\n",
            "<http://ex/alice> <http://xmlns.com/foaf/0.1/knows> <http://ex/bob> .\n",
        );
        let path = "/tmp/pgrdf_bulk_basic.nt";
        std::fs::write(path, nt).expect("write temp .nt");

        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.load_turtle($1, $2, NULL, TRUE)",
            &[path.into(), 7_500i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(n, 5, "bulk load returns the triple count");

        let cq: i64 = Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[7_500i64.into()])
            .unwrap()
            .unwrap();
        assert_eq!(cq, 5, "all 5 quads landed in the graph");

        let dupes: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM (
               SELECT 1 FROM pgrdf._pgrdf_dictionary
               GROUP BY term_type, lexical_value, datatype_iri_id, language_tag
               HAVING count(*) > 1) z",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            dupes, 0,
            "no duplicate dictionary rows (self-assigned-id correctness)"
        );

        let set_ok: bool = Spi::get_one(
            "WITH lex AS (
               SELECT s.lexical_value sl, p.lexical_value pl, o.lexical_value ol,
                      o.term_type ot, o.language_tag og
                 FROM pgrdf._pgrdf_quads q
                 JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
                 JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
                 JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
                WHERE q.graph_id = 7500)
             SELECT count(*) = 5
                AND count(*) FILTER (WHERE ol = 'Alice' AND ot = 3) = 1
                AND count(*) FILTER (WHERE ol = '30' AND ot = 3) = 1
                AND count(*) FILTER (WHERE ol = 'Hi' AND og = 'en') = 1
                AND count(*) FILTER (WHERE ol = 'http://ex/bob' AND ot = 1) = 1
                AND count(DISTINCT sl) = 1
               FROM lex",
        )
        .unwrap()
        .unwrap_or(false);
        assert!(set_ok, "decoded-lexical triple set matches expected");

        let _ = std::fs::remove_file(path);
    }

    /// Verbose variant keeps the four-phase breakdown on the bulk path,
    /// including the new `resolve_ms` (triple→id) timer.
    #[pg_test]
    fn load_turtle_bulk_verbose_phase_breakdown() {
        let nt = "<http://ex/s> <http://ex/p> <http://ex/o> .\n";
        let path = "/tmp/pgrdf_bulk_verbose.nt";
        std::fs::write(path, nt).expect("write temp .nt");

        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.load_turtle_verbose($1, $2, NULL, TRUE)",
            &[path.into(), 7_510i64.into()],
        )
        .unwrap()
        .unwrap();
        let v = &j.0;
        assert_eq!(v["triples"], 1);
        for k in ["parse_ms", "dict_ms", "resolve_ms", "insert_ms", "index_ms"] {
            assert!(v[k].is_number(), "verbose bulk output carries {k}");
        }
        // A tiny load stays below `pgrdf.bulk_defer_index_min`, so the
        // defer-index optimization does not fire (no global index DDL).
        assert_eq!(
            v["defer_index"], false,
            "a tiny load stays below the defer-index threshold"
        );

        let _ = std::fs::remove_file(path);
    }

    /// On a NON-empty dictionary the fast path's empty-dict guard routes
    /// to the correct `combined` fallback: a term shared with the prior
    /// load stays a SINGLE dict row (the fast path would have inserted a
    /// duplicate with a fresh self-assigned id).
    #[pg_test]
    fn load_turtle_bulk_falls_back_on_populated_dict() {
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "<http://ex/s> <http://ex/p> <http://ex/shared> .".into(),
                7_520i64.into(),
            ],
        )
        .unwrap();

        let nt = concat!(
            "<http://ex/s2> <http://ex/p> <http://ex/shared> .\n",
            "<http://ex/s3> <http://ex/p> <http://ex/new> .\n",
        );
        let path = "/tmp/pgrdf_bulk_fallback.nt";
        std::fs::write(path, nt).expect("write temp .nt");

        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.load_turtle($1, $2, NULL, TRUE)",
            &[path.into(), 7_521i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(n, 2);

        let shared_rows: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary
              WHERE term_type = 1 AND lexical_value = 'http://ex/shared'",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            shared_rows, 1,
            "shared term deduped via fallback (no duplicate row)"
        );

        let _ = std::fs::remove_file(path);
    }

    /// Concurrency-safety (issue #20): the bulk fast path must allocate
    /// dictionary ids from the table's IDENTITY sequence (the shared,
    /// race-free allocator), NOT from a `max(id)` snapshot. The sequence
    /// is non-transactional, so a prior allocation can leave it ahead of
    /// `max(id)` on an empty dict — exactly the divergence the old
    /// `max(id)`-based reservation raced on. Pre-advancing the sequence
    /// and asserting the assigned ids respect it pins the fix and fails on
    /// the old `max(id)` + `setval` reservation (which assigns ids from 1,
    /// ignoring the sequence).
    #[pg_test]
    fn load_turtle_bulk_reserves_from_identity_sequence() {
        // Empty dict (rolled-back txn) but a sequence already advanced
        // well past max(id): the classic max(id) != nextval divergence.
        Spi::run(
            "SELECT setval(pg_get_serial_sequence('pgrdf._pgrdf_dictionary','id'), 100000, true)",
        )
        .unwrap();

        let nt = concat!(
            "<http://ex/s1> <http://ex/p> <http://ex/o1> .\n",
            "<http://ex/s2> <http://ex/p> <http://ex/o2> .\n",
        );
        let path = "/tmp/pgrdf_bulk_seq.nt";
        std::fs::write(path, nt).expect("write temp .nt");

        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.load_turtle($1, $2, NULL, TRUE)",
            &[path.into(), 7_530i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(n, 2, "bulk load returns the triple count");

        // Every assigned id must come from the sequence (> 100000), not
        // from max(id)+1 (the old reservation → ids from 1).
        let min_id: i64 = Spi::get_one("SELECT min(id) FROM pgrdf._pgrdf_dictionary")
            .unwrap()
            .unwrap();
        assert!(
            min_id > 100_000,
            "bulk path allocates from the IDENTITY sequence (min id {min_id} must exceed the pre-advanced sequence value)"
        );

        // The sequence is left at/above the highest assigned id, so a
        // following IDENTITY insert can't collide with a bulk-assigned id.
        let max_id: i64 = Spi::get_one("SELECT max(id) FROM pgrdf._pgrdf_dictionary")
            .unwrap()
            .unwrap();
        let next_after: i64 =
            Spi::get_one("SELECT nextval(pg_get_serial_sequence('pgrdf._pgrdf_dictionary','id'))")
                .unwrap()
                .unwrap();
        assert!(
            next_after > max_id,
            "sequence advanced past the highest assigned id ({next_after} > {max_id})"
        );

        let _ = std::fs::remove_file(path);
    }

    /// Force the defer-index path (`bulk_defer_index_min = 0`) and confirm
    /// the load is correct and the four indexes are rebuilt + usable. This
    /// fires global ACCESS-EXCLUSIVE index DDL, which is unsafe to run
    /// concurrently with the rest of the suite, so it is a no-op unless
    /// `PGRDF_RUN_DEFER_TEST` is set. Run it in isolation:
    ///   PGRDF_RUN_DEFER_TEST=1 cargo pgrx test pg17 load_turtle_bulk_defer_index_rebuilds
    #[pg_test]
    fn load_turtle_bulk_defer_index_rebuilds() {
        if std::env::var("PGRDF_RUN_DEFER_TEST").is_err() {
            return;
        }
        Spi::run("SET pgrdf.bulk_defer_index_min = 0").unwrap();

        let nt = concat!(
            "<http://ex/a> <http://ex/p> <http://ex/b> .\n",
            "<http://ex/a> <http://ex/q> \"v\" .\n",
            "<http://ex/c> <http://ex/p> <http://ex/b> .\n",
        );
        let path = "/tmp/pgrdf_bulk_defer.nt";
        std::fs::write(path, nt).expect("write temp .nt");

        let j: pgrx::JsonB = Spi::get_one_with_args(
            "SELECT pgrdf.load_turtle_verbose($1, $2, NULL, TRUE)",
            &[path.into(), 7_531i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(j.0["triples"], 3);
        assert_eq!(j.0["defer_index"], true, "forced defer-index fired");

        let idx: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pg_indexes
              WHERE schemaname = 'pgrdf'
                AND indexname IN ('_pgrdf_idx_spo', '_pgrdf_idx_pos',
                                  '_pgrdf_idx_osp', '_pgrdf_dict_val_idx')",
        )
        .unwrap()
        .unwrap();
        assert_eq!(idx, 4, "all four indexes rebuilt after the defer load");

        // v0.6.4: the deferred `unique_term` constraint is re-added (and
        // validated) over the loaded data.
        let con: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pg_constraint
              WHERE conname = 'unique_term'
                AND conrelid = 'pgrdf._pgrdf_dictionary'::regclass",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            con, 1,
            "unique_term constraint re-added after the defer load"
        );

        let cq: i64 = Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[7_531i64.into()])
            .unwrap()
            .unwrap();
        assert_eq!(cq, 3, "quads queryable via the rebuilt indexes");

        let _ = std::fs::remove_file(path);
    }

    // ─── v0.6.14 — format-aware staged dispatch: Turtle always uses the full parser ───
    //
    // `load_turtle` routes to the staged loader only when pgRDF is in
    // `shared_preload_libraries` AND the file sniffs as N-Triples AND no
    // base_iri is set. A Turtle fixture (`@prefix` / prefixed names / a `;`-list)
    // sniffs as NOT N-Triples, so it ALWAYS takes the full parser — whether or
    // not pgRDF is preloaded (so this test is robust to either harness config).
    // This pins that the format-aware dispatch never silently drops Turtle
    // (exactly the input the N-Triples staged path would skip); the preloaded
    // N-Triples staged route is exercised at scale.

    /// A small Turtle fixture (`@prefix`, prefixed names, a `;`-list — none
    /// of which the N-Triples staged STAGE phase could read) loads
    /// correctly through `load_turtle`: the returned triple count, the landed
    /// quads, and the decoded-lexical set all match. Proves the format-aware
    /// dispatch never silently drops Turtle (it always uses the full parser
    /// unless the file is confidently N-Triples — independent of preload).
    #[pg_test]
    fn load_turtle_turtle_fixture_uses_full_parser() {
        // A Turtle file sniffs as NOT N-Triples, so `load_turtle` uses the full
        // parser regardless of whether the staged pool is available — that is
        // the no-silent-Turtle-loss guarantee under test, and it holds whether
        // or not the harness preloads pgRDF.

        // Full Turtle: a @prefix directive, prefixed names, and a
        // predicate-object `;`-list — all silently skipped by the
        // N-Triples staged parser, all correct via the full parser.
        let ttl = concat!(
            "@prefix ex:   <http://example.com/> .\n",
            "@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n",
            "ex:alice a foaf:Person ;\n",
            "         foaf:name \"Alice\" ;\n",
            "         foaf:knows ex:bob .\n",
            "ex:bob a foaf:Person .\n",
        );
        let path = "/tmp/pgrdf_0614_turtle_fallback.ttl";
        std::fs::write(path, ttl).expect("write temp .ttl");

        // Default arity: no base_iri, no bulk_load → the v0.6.14 dispatch,
        // which falls back to the full parser when not preloaded.
        let n: i64 = Spi::get_one_with_args(
            "SELECT pgrdf.load_turtle($1, $2)",
            &[path.into(), 7_540i64.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            n, 5,
            "full Turtle fixture (4 ex:alice + 1 ex:bob) loads via the fallback"
        );

        let cq: i64 = Spi::get_one_with_args("SELECT pgrdf.count_quads($1)", &[7_540i64.into()])
            .unwrap()
            .unwrap();
        assert_eq!(cq, 5, "all 5 quads landed via the full-parser fallback");

        let set_ok: bool = Spi::get_one(
            "WITH lex AS (
               SELECT s.lexical_value sl, p.lexical_value pl,
                      o.lexical_value ol, o.term_type ot
                 FROM pgrdf._pgrdf_quads q
                 JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
                 JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
                 JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
                WHERE q.graph_id = 7540)
             SELECT count(*) = 5
                -- prefixed names expanded to absolute IRIs by the parser
                AND count(*) FILTER (
                      WHERE sl = 'http://example.com/alice'
                        AND ol = 'Alice' AND ot = 3) = 1
                AND count(*) FILTER (
                      WHERE ol = 'http://example.com/bob' AND ot = 1) = 1
                AND count(*) FILTER (
                      WHERE pl = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#type') = 2
               FROM lex",
        )
        .unwrap()
        .unwrap_or(false);
        assert!(
            set_ok,
            "decoded-lexical set matches: prefixed names expanded, ;-list + `a` honoured"
        );

        let _ = std::fs::remove_file(path);
    }
}

// ─────────────────────────────────────────────────────────────────────
// v0.6.14 — pure format-sniff unit tests (no pgrx harness)
// ─────────────────────────────────────────────────────────────────────
//
// `sniff_is_ntriples` is a pure fn over `&[u8]`, so these are plain
// `#[test]`s in a NON-`#[pg_schema]` module: they run under `cargo test`
// without a running Postgres. They pin the conservative boundary — bare
// absolute-term N-Triples classifies as N-Triples; ANY Turtle feature
// (`@prefix`/`@base`, a `pfx:name` prefixed term, a `;`/`,` list, a
// multi-line statement) classifies as Turtle (the safe default).
#[cfg(test)]
mod sniff_tests {
    use super::sniff_is_ntriples;

    /// Real N-Triples — every line a complete statement of bare absolute
    /// `<IRI>` terms (and a literal with a language tag + a typed literal,
    /// both legal N-Triples) terminated by ` .` — classifies as N-Triples.
    #[test]
    fn real_ntriples_is_ntriples() {
        let nt = concat!(
            "<http://ex/alice> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .\n",
            "<http://ex/alice> <http://xmlns.com/foaf/0.1/name> \"Alice\" .\n",
            "<http://ex/alice> <http://xmlns.com/foaf/0.1/knows> _:bob .\n",
            "<http://ex/alice> <http://ex/label> \"Alice\"@en .\n",
            "<http://ex/alice> <http://ex/age> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
        );
        assert!(
            sniff_is_ntriples(nt.as_bytes()),
            "bare-term N-Triples must classify as N-Triples"
        );
    }

    /// A `@prefix`-led Turtle snippet classifies as Turtle (the directive
    /// is illegal in N-Triples).
    #[test]
    fn prefix_directive_is_turtle() {
        let ttl = concat!(
            "@prefix ex: <http://example.com/> .\n",
            "<http://ex/s> <http://ex/p> <http://ex/o> .\n",
        );
        assert!(
            !sniff_is_ntriples(ttl.as_bytes()),
            "@prefix directive must classify as Turtle"
        );
    }

    /// A `@base` directive likewise classifies as Turtle.
    #[test]
    fn base_directive_is_turtle() {
        let ttl = "@base <http://example.com/> .\n<http://ex/s> <http://ex/p> <http://ex/o> .\n";
        assert!(
            !sniff_is_ntriples(ttl.as_bytes()),
            "@base directive must classify as Turtle"
        );
    }

    /// SPARQL-style `PREFIX`/`BASE` (no leading `@`) also classify as
    /// Turtle — a bare keyword token is illegal in N-Triples.
    #[test]
    fn sparql_style_prefix_is_turtle() {
        let ttl = concat!(
            "PREFIX ex: <http://example.com/>\n",
            "<http://ex/s> <http://ex/p> <http://ex/o> .\n",
        );
        assert!(
            !sniff_is_ntriples(ttl.as_bytes()),
            "SPARQL-style PREFIX must classify as Turtle"
        );
    }

    /// A prefixed-name snippet with NO `@prefix` (e.g. `wd:Q42 rdfs:label
    /// …`) still classifies as Turtle — the bare `:` of a prefixed name is
    /// the tell, and the N-Triples parser would silently skip these.
    #[test]
    fn prefixed_names_without_directive_is_turtle() {
        let ttl = "wd:Q42 rdfs:label \"Douglas Adams\"@en .\nwd:Q42 wdt:P31 wd:Q5 .\n";
        assert!(
            !sniff_is_ntriples(ttl.as_bytes()),
            "prefixed names (pfx:name) must classify as Turtle even without @prefix"
        );
    }

    /// A multi-line `;`-list Turtle statement (predicate-object list across
    /// lines, comma object list) classifies as Turtle.
    #[test]
    fn multiline_predicate_object_list_is_turtle() {
        let ttl = concat!(
            "<http://ex/alice>\n",
            "    <http://ex/name> \"Alice\" ;\n",
            "    <http://ex/knows> <http://ex/bob> , <http://ex/carol> .\n",
        );
        assert!(
            !sniff_is_ntriples(ttl.as_bytes()),
            "multi-line ;/, predicate-object list must classify as Turtle"
        );
    }

    /// The `a` (rdf:type) keyword is Turtle-only; a line using it is Turtle.
    #[test]
    fn a_keyword_is_turtle() {
        let ttl = "<http://ex/alice> a <http://xmlns.com/foaf/0.1/Person> .\n";
        assert!(
            !sniff_is_ntriples(ttl.as_bytes()),
            "the `a` keyword must classify as Turtle"
        );
    }

    /// A `:` that lives INSIDE an IRI or a string literal must NOT trip the
    /// prefixed-name detector — these stay confident N-Triples.
    #[test]
    fn colon_inside_iri_or_literal_stays_ntriples() {
        let nt = concat!(
            "<http://ex/s> <http://ex/p> \"a: b; c, d\" .\n",
            "<http://ex/s2> <http://ex/p2> <urn:isbn:0451450523> .\n",
        );
        assert!(
            sniff_is_ntriples(nt.as_bytes()),
            "`:`/`;`/`,` inside an IRI or string literal must not force Turtle"
        );
    }

    /// An empty or comment-only sample is NOT confidently N-Triples ⇒
    /// Turtle (the safe default).
    #[test]
    fn empty_or_comment_only_is_turtle() {
        assert!(!sniff_is_ntriples(b""), "empty sample ⇒ Turtle");
        assert!(
            !sniff_is_ntriples(b"# just a comment\n# and another\n"),
            "comment-only sample ⇒ Turtle"
        );
    }

    /// Two statements on one physical line is not clean N-Triples ⇒ Turtle.
    #[test]
    fn two_statements_one_line_is_turtle() {
        let s = "<http://ex/s> <http://ex/p> <http://ex/o> . <http://ex/s2> <http://ex/p2> <http://ex/o2> .\n";
        assert!(
            !sniff_is_ntriples(s.as_bytes()),
            "two statements on one line ⇒ Turtle (not a single bare statement)"
        );
    }
}
