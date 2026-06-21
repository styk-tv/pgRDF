//! R2.1 — the REAL staged-loader phase bodies (STAGE → DICT → RESOLVE → INDEX).
//!
//! These are the worker-side closures dispatched by [`super::pool::pgrdf_staged_worker_main`] on the
//! [`jobctl::WorkerSlot::phase`] field. Each is called from *inside* one
//! `BackgroundWorker::transaction(|| …)` — i.e. its own committed top-level transaction in its own
//! backend, which is the per-phase recovery point (`_WIP/SPEC.STAGED-LOADER-R2.bgworker-design.md`
//! §6). They never COMMIT themselves; returning normally commits the wrapping transaction.
//!
//! This is the in-database, set-based port of the E32-proven SQL prototype
//! (`082.staged-truthy.sh`): UNLOGGED COPY/INSERT staging → `INSERT … SELECT DISTINCT` dedup driven
//! by PG's parallel hash-aggregate → `INSERT … SELECT … JOIN dict ×3` resolve driven by PG's
//! parallel hash-join → plain (non-concurrent) `CREATE INDEX` builds run simultaneously across
//! workers. The `parallel_workers` table reloption (set at staging `CREATE`) plus the per-session
//! parallel GUCs ([`apply_session_gucs`]) are THE fix for "lights up N cores, not all of them".
//!
//! ## The datatype-id ordering trap (why DICT is two statements, not one)
//!
//! A literal's `datatype_iri_id` column is a *dictionary id of the datatype IRI's own URI term*, not
//! the IRI string (schema_v0_2_0.sql:8). The single-backend loader handles this by interning all
//! URIs first (tier-1, including each literal's datatype IRI) and only then interning the literals
//! with the now-known datatype id (`loader.rs::ingest_turtle_streaming` tiers 1/2). The set-based
//! DICT phase reproduces that ordering: it inserts every URI/blank term first (subjects, predicates,
//! object URIs/blanks, AND the distinct object datatype IRIs), then inserts the literals, resolving
//! each literal's `datatype_iri_id` by joining the just-populated dictionary. Same rule, same rows.

use crate::storage::dict::term_type;
use crate::storage::staged::jobctl::{self, JobSlot, WorkerSlot};
use oxrdf::{NamedOrBlankNode, Term};
use oxttl::TurtleParser;
use pgrx::prelude::*;
use std::fs::File;
use std::io::Read;

/// Staging columns: the raw triple as parsed, with the object split into its
/// (type, lexical value, datatype IRI string, language tag) so the set-based DICT/RESOLVE phases can
/// reproduce the dictionary's `(term_type, lexical_value, datatype_iri_id, language_tag)` key.
/// `o_type` is the `term_type` SMALLINT; `o_dt` is the datatype IRI **string** (resolved to an id in
/// DICT); `o_lang` is the language tag. NULL `o_dt`/`o_lang` mirror the NULLS-DISTINCT dict key.
const STAGE_COLS: &str = "s text, p text, o_type smallint, o_val text, o_dt text, o_lang text";

/// Rows inserted per `INSERT … SELECT * FROM unnest(...)` batch in STAGE. 50k matches the loader's
/// `BULK_QUAD_BATCH`; large enough to amortise SPI round-trips, small enough to bound the per-batch
/// array memory.
const STAGE_BATCH: usize = 50_000;

/// The deterministic staging table name for `job_id` (UNLOGGED, dropped on success). Resumable runs
/// re-find it by this name (§7). Qualified into the `pgrdf` schema.
pub fn staging_table(job_id: i64) -> String {
    format!("pgrdf._pgrdf_stg_{job_id}")
}

/// Re-apply the per-session parallel levers (§5) inside the worker's transaction. GUCs are
/// per-session and a dynamic worker starts with server defaults, so each phase that wants PG's
/// intra-query parallelism (DICT hash-agg, RESOLVE hash-join, INDEX maintenance) must `SET LOCAL`
/// them itself. `nproc` is the worker's own `num_cpus`; the staging/dict `parallel_workers`
/// reloption (set elsewhere) is what actually lifts the per-table worker cap, these GUCs raise the
/// session ceilings to match. `SET LOCAL` scopes them to this transaction only.
pub fn apply_session_gucs() {
    let nproc = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
        .max(1);
    let maint = (nproc / 2).max(1);
    // One statement, semicolon-separated: cheaper than N SPI round-trips. All are GUCs that exist on
    // PG17; `SET LOCAL` confines them to this transaction (auto-reset on commit).
    let sql = format!(
        "SET LOCAL max_parallel_workers = {nproc}; \
         SET LOCAL max_parallel_workers_per_gather = {nproc}; \
         SET LOCAL max_parallel_maintenance_workers = {maint}; \
         SET LOCAL enable_parallel_hash = on; \
         SET LOCAL parallel_setup_cost = 0; \
         SET LOCAL parallel_tuple_cost = 0; \
         SET LOCAL min_parallel_table_scan_size = 0; \
         SET LOCAL min_parallel_index_scan_size = 0; \
         SET LOCAL work_mem = '2GB'; \
         SET LOCAL maintenance_work_mem = '16GB';"
    );
    Spi::run(&sql).expect("staged phase: apply_session_gucs failed");
}

/// **STAGE** (Phase A). Parse this worker's byte range of the `.nt` file leniently and bulk-insert
/// its rows into the pre-created UNLOGGED staging table.
///
/// The coordinator snapped `[range_lo, range_hi)` to newline boundaries and recorded them in the
/// [`WorkerSlot`]; the worker opens the file, reads exactly that slice, and feeds it to oxttl. The
/// lenient parse (skip + count on a malformed triple) is the same Wikidata-control-byte robustness
/// the streaming loader uses (`loader.rs` ~1948). Rows go in via `INSERT … SELECT * FROM unnest(...)`
/// batches — the multi-backend parallelism (N workers each owning a disjoint slice, each committing
/// its own transaction) is the win that breaks the single-COPY wall; binary COPY-from-Rust-buffer is
/// a later micro-opt (issue #23). Returns the count of triples staged by THIS worker (for the tally).
pub fn stage(job: &JobSlot, w: &WorkerSlot) -> i64 {
    let path = job.path();
    let lo = w.range_lo as usize;
    let hi = w.range_hi as usize;

    let mut file =
        File::open(&path).unwrap_or_else(|e| panic!("staged STAGE: open {path:?} failed: {e}"));
    // Read exactly the byte slice [lo, hi). seek+take avoids loading the whole (40 GB) file.
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(lo as u64))
        .unwrap_or_else(|e| panic!("staged STAGE: seek to {lo} failed: {e}"));
    let mut buf = vec![0u8; hi.saturating_sub(lo)];
    file.read_exact(&mut buf)
        .unwrap_or_else(|e| panic!("staged STAGE: read [{lo},{hi}) failed: {e}"));

    let table = staging_table(job.job_id);

    // Column batch buffers for the unnest insert.
    let mut bs: Vec<String> = Vec::with_capacity(STAGE_BATCH);
    let mut bp: Vec<String> = Vec::with_capacity(STAGE_BATCH);
    let mut bot: Vec<i16> = Vec::with_capacity(STAGE_BATCH);
    let mut bov: Vec<String> = Vec::with_capacity(STAGE_BATCH);
    let mut bod: Vec<Option<String>> = Vec::with_capacity(STAGE_BATCH);
    let mut bol: Vec<Option<String>> = Vec::with_capacity(STAGE_BATCH);
    let mut staged: i64 = 0;

    let mut flush = |bs: &mut Vec<String>,
                     bp: &mut Vec<String>,
                     bot: &mut Vec<i16>,
                     bov: &mut Vec<String>,
                     bod: &mut Vec<Option<String>>,
                     bol: &mut Vec<Option<String>>| {
        if bs.is_empty() {
            return;
        }
        let sql = format!(
            "INSERT INTO {table} (s, p, o_type, o_val, o_dt, o_lang) \
             SELECT * FROM unnest($1::text[], $2::text[], $3::smallint[], \
                                  $4::text[], $5::text[], $6::text[])"
        );
        Spi::run_with_args(
            &sql,
            &[
                std::mem::take(bs).into(),
                std::mem::take(bp).into(),
                std::mem::take(bot).into(),
                std::mem::take(bov).into(),
                std::mem::take(bod).into(),
                std::mem::take(bol).into(),
            ],
        )
        .expect("staged STAGE: batch insert failed");
    };

    for r in TurtleParser::new().for_reader(buf.as_slice()) {
        let t = match r {
            Ok(t) => t,
            Err(_) => continue, // lenient: skip malformed triples (Wikidata control bytes)
        };
        let s = match &t.subject {
            NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
            NamedOrBlankNode::BlankNode(b) => b.as_str().to_string(),
        };
        let p = t.predicate.as_str().to_string();
        let (o_type, o_val, o_dt, o_lang): (i16, String, Option<String>, Option<String>) =
            match &t.object {
                Term::NamedNode(n) => (term_type::URI, n.as_str().to_string(), None, None),
                Term::BlankNode(b) => (term_type::BLANK_NODE, b.as_str().to_string(), None, None),
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
                _ => panic!("staged STAGE: unsupported object term"),
            };
        bs.push(s);
        bp.push(p);
        bot.push(o_type);
        bov.push(o_val);
        bod.push(o_dt);
        bol.push(o_lang);
        staged += 1;
        if bs.len() >= STAGE_BATCH {
            flush(
                &mut bs, &mut bp, &mut bot, &mut bov, &mut bod, &mut bol,
            );
        }
    }
    flush(
        &mut bs, &mut bp, &mut bot, &mut bov, &mut bod, &mut bol,
    );
    staged
}

/// **DICT** (Phase B). Set-based dedup of every staged term into `_pgrdf_dictionary`, ids drawn from
/// the IDENTITY sequence (never supplied — #20). Two statements to honour the datatype-id ordering
/// (see module docs): URIs/blanks first (so a literal's datatype IRI already has a dict id), then the
/// literals with `datatype_iri_id` resolved by joining the just-inserted dictionary.
///
/// `WHERE NOT EXISTS` against the existing dictionary makes this safe on a non-empty dict too (the
/// staged-loader fast path expects an empty dict, but the anti-join costs little when it IS empty and
/// keeps the statement correct if it isn't). PG's **parallel hash-aggregate** does the `DISTINCT`
/// (spills past `work_mem`, RAM-bounded — the whole point). Returns the dict row count afterwards.
pub fn dict(job: &JobSlot, _w: &WorkerSlot) -> i64 {
    let stg = staging_table(job.job_id);

    // Statement 1 — all URI + blank-node terms in ONE pass: subjects (s, always URI/blank),
    // predicates (p, always URI), object URIs/blanks (o_type in {URI,BLANK}), and the DISTINCT
    // object datatype IRIs (o_dt, a URI). datatype_iri_id + language_tag are NULL for these. The
    // sub-select UNIONs the four term sources; the outer DISTINCT + WHERE NOT EXISTS dedups against
    // both the batch itself and any pre-existing rows. ids come from IDENTITY (4 explicit cols only).
    let uri = term_type::URI as i32;
    let blank = term_type::BLANK_NODE as i32;
    let literal = term_type::LITERAL as i32;
    let sql1 = format!(
        "INSERT INTO pgrdf._pgrdf_dictionary (term_type, lexical_value, datatype_iri_id, language_tag) \
         SELECT DISTINCT tt, lv, NULL::bigint, NULL::text \
         FROM ( \
             SELECT {blank}::smallint AS tt, s AS lv FROM {stg} WHERE s LIKE '_:%' \
             UNION ALL SELECT {uri}::smallint, s FROM {stg} WHERE s NOT LIKE '_:%' \
             UNION ALL SELECT {uri}::smallint, p FROM {stg} \
             UNION ALL SELECT o_type::smallint, o_val FROM {stg} WHERE o_type IN ({uri}, {blank}) \
             UNION ALL SELECT {uri}::smallint, o_dt FROM {stg} WHERE o_type = {literal} AND o_dt IS NOT NULL \
         ) u \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM pgrdf._pgrdf_dictionary d \
             WHERE d.term_type = u.tt AND d.lexical_md5 = decode(md5(u.lv), 'hex') \
               AND d.datatype_iri_id IS NULL AND d.language_tag IS NULL \
         )"
    );
    Spi::run(&sql1).expect("staged DICT: URI/blank dedup insert failed");

    // Statement 2 — literals, keyed (LITERAL, lexical, datatype_iri_id, language_tag). The datatype
    // id is resolved by LEFT JOIN to the dictionary on the datatype IRI's URI term (inserted in
    // statement 1). A language-tagged literal has datatype_iri_id NULL (matching the loader). The
    // anti-join is the matching NULLS-DISTINCT 4-tuple. DISTINCT over the resolved tuple dedups.
    let sql2 = format!(
        "INSERT INTO pgrdf._pgrdf_dictionary (term_type, lexical_value, datatype_iri_id, language_tag) \
         SELECT DISTINCT {literal}::smallint, st.o_val, dt.id, st.o_lang \
         FROM {stg} st \
         LEFT JOIN pgrdf._pgrdf_dictionary dt \
                ON st.o_dt IS NOT NULL AND dt.term_type = {uri} \
               AND dt.lexical_md5 = decode(md5(st.o_dt), 'hex') \
               AND dt.datatype_iri_id IS NULL AND dt.language_tag IS NULL \
         WHERE st.o_type = {literal} \
           AND NOT EXISTS ( \
               SELECT 1 FROM pgrdf._pgrdf_dictionary d \
               WHERE d.term_type = {literal} \
                 AND d.lexical_md5 = decode(md5(st.o_val), 'hex') \
                 AND d.datatype_iri_id IS NOT DISTINCT FROM dt.id \
                 AND d.language_tag IS NOT DISTINCT FROM st.o_lang \
           )"
    );
    Spi::run(&sql2).expect("staged DICT: literal dedup insert failed");

    Spi::get_one::<i64>("SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary")
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// **RESOLVE** (Phase C). Join the staging table against the now-complete dictionary three times
/// (subject, predicate, object) to turn each staged triple into a `(subject_id, predicate_id,
/// object_id, graph_id)` quad, inserted into `_pgrdf_quads`. Driven by PG's **parallel hash join**
/// (`enable_parallel_hash`, the `parallel_workers` reloption on both staging and dict). The hexastore
/// indexes are still dropped at this point (dropped in the coordinator's pre-STAGE prep), so the quad
/// insert skips per-row index maintenance. Returns the quad count inserted by this statement.
///
/// The object join is the subtle one: it must match a URI/blank object by the `(type, lexical)` pair
/// with NULL datatype/lang, and a literal object by the full `(LITERAL, lexical, datatype_iri_id,
/// language_tag)` tuple — the same key the dictionary is unique on. A single join expression handles
/// both by matching `term_type = o_type`, `lexical_md5 = md5(o_val)`, and the datatype/lang columns
/// `IS NOT DISTINCT FROM` the (resolved) staged values. The datatype id is resolved by a fourth join
/// to the dictionary on `o_dt` (NULL for non-literals / language-tagged literals).
pub fn resolve(job: &JobSlot, _w: &WorkerSlot) -> i64 {
    let stg = staging_table(job.job_id);
    let graph_id = job.graph_id;
    let uri = term_type::URI as i32;
    let blank = term_type::BLANK_NODE as i32;
    let literal = term_type::LITERAL as i32;

    // ds: subject (URI unless it is a blank label '_:…'); dp: predicate (URI); dobj: object by the
    // full 4-tuple; dt: the object's datatype-IRI term (so dobj's datatype_iri_id can be matched).
    let sql = format!(
        "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id) \
         SELECT ds.id, dp.id, dobj.id, {graph_id}::bigint \
         FROM {stg} st \
         JOIN pgrdf._pgrdf_dictionary ds \
              ON ds.term_type = CASE WHEN st.s LIKE '_:%' THEN {blank} ELSE {uri} END \
             AND ds.lexical_md5 = decode(md5(st.s), 'hex') \
             AND ds.datatype_iri_id IS NULL AND ds.language_tag IS NULL \
         JOIN pgrdf._pgrdf_dictionary dp \
              ON dp.term_type = {uri} \
             AND dp.lexical_md5 = decode(md5(st.p), 'hex') \
             AND dp.datatype_iri_id IS NULL AND dp.language_tag IS NULL \
         LEFT JOIN pgrdf._pgrdf_dictionary dt \
              ON st.o_type = {literal} AND st.o_dt IS NOT NULL AND dt.term_type = {uri} \
             AND dt.lexical_md5 = decode(md5(st.o_dt), 'hex') \
             AND dt.datatype_iri_id IS NULL AND dt.language_tag IS NULL \
         JOIN pgrdf._pgrdf_dictionary dobj \
              ON dobj.term_type = st.o_type \
             AND dobj.lexical_md5 = decode(md5(st.o_val), 'hex') \
             AND dobj.datatype_iri_id IS NOT DISTINCT FROM dt.id \
             AND dobj.language_tag IS NOT DISTINCT FROM st.o_lang"
    );
    Spi::run(&sql).expect("staged RESOLVE: quad resolve insert failed");

    Spi::get_one_with_args::<i64>(
        "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = $1",
        &[graph_id.into()],
    )
    .ok()
    .flatten()
    .unwrap_or(0)
}

/// **INDEX** (Phase D). Each INDEX worker runs exactly ONE of the [`super::index_ddls`] statements
/// (the 3 hexastore indexes + the dict hash index + the `unique_term` constraint re-add), so the 5
/// build/validate steps run SIMULTANEOUSLY across 5 backends. Plain (non-concurrent) `CREATE INDEX`
/// is correct here because the freshly bulk-loaded quads table is not yet visible/queried during the
/// load — the same situation `bulk_rebuild_indexes` already exploits; "concurrent" parallelism comes
/// from running the separate builds at once, not from `CONCURRENTLY` (§6.D). The worker's `shard`
/// field selects which DDL it runs.
pub fn build_index(_job: &JobSlot, w: &WorkerSlot) {
    let ddls = jobctl::index_ddls();
    let i = w.shard as usize;
    let ddl = ddls
        .get(i)
        .unwrap_or_else(|| panic!("staged INDEX: ddl index {i} out of range"));
    Spi::run(ddl).unwrap_or_else(|e| panic!("staged INDEX: DDL #{i} failed: {e}"));
}
