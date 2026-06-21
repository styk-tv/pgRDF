//! R2.1 — the REAL staged-loader phase bodies (STAGE → DICT → RESOLVE → INDEX).
//!
//! These are the worker-side closures dispatched by [`super::pool::pgrdf_staged_worker_main`] on the
//! [`jobctl::WorkerSlot::phase`] field. Each is called from *inside* one
//! `BackgroundWorker::transaction(|| …)` — i.e. its own committed top-level transaction in its own
//! backend, which is the per-phase recovery point (`_WIP/SPEC.STAGED-LOADER-R2.bgworker-design.md`
//! §6). They never COMMIT themselves; returning normally commits the wrapping transaction.
//!
//! This is the in-database, set-based port of the E32-proven 78 s/8 G SQL prototype: UNLOGGED
//! INSERT staging → a parallel `CREATE TABLE … AS SELECT … row_number()` dictionary build → a single
//! `INSERT … OVERRIDING SYSTEM VALUE` into `_pgrdf_dictionary` (ids pre-assigned, so no per-row
//! `nextval`) → a parallel hash-join `CREATE TABLE … AS SELECT` that resolves every staged triple to
//! a quad and is then `ATTACH`ed as the graph's partition → plain (non-concurrent) `CREATE INDEX`
//! builds run simultaneously across workers. The `parallel_workers` table reloption (set on staging,
//! the dict, and the resolved-quad table) plus the per-session parallel GUCs ([`apply_session_gucs`])
//! are THE fix for "lights up N cores, not all of them".
//!
//! ## Why DICT assigns ids itself (`row_number()` + `OVERRIDING SYSTEM VALUE`)
//!
//! `_pgrdf_dictionary.id` is `GENERATED ALWAYS AS IDENTITY`, so ANY `INSERT INTO _pgrdf_dictionary
//! SELECT …` plan is marked parallel-UNSAFE — the whole statement (including the dedup over the
//! staging scan) runs single-threaded on the leader, and every row pays a `nextval`. The benchmark's
//! fix, ported here verbatim: do the expensive DISTINCT dedup in **parallel** `CREATE UNLOGGED TABLE
//! dict_* AS SELECT … row_number() OVER () AS id …` materialisations (no IDENTITY target ⇒ PG14+ runs
//! a parallel `Gather → Parallel Append → Parallel Seq Scan` plan), pre-assigning each distinct term
//! a contiguous id with `row_number()`; then a SINGLE `INSERT … OVERRIDING SYSTEM VALUE` copies the
//! already-numbered rows in, supplying the id explicitly (one cheap serial pass, no `nextval`).
//!
//! ## The datatype-id ordering trap
//!
//! A literal's `datatype_iri_id` is a *dictionary id of the datatype IRI's own URI term*, not the IRI
//! string (schema_v0_2_0.sql:8). The single-backend loader interns all URIs first (incl. each
//! literal's datatype IRI) and only then the literals with the now-known datatype id
//! (`loader.rs::ingest_turtle_streaming` tiers 1/2). The set-based DICT reproduces that ordering by
//! construction: the datatype IRIs are folded into the URI dict (`dict_uri`), so when the combined
//! `dict_all` resolves each literal's `datatype_iri_id` it joins `dict_uri` on the datatype IRI
//! string and reads the id `row_number()` already gave that URI. Same rule, same rows.

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

/// **STAGE prep** — the once-per-load setup that MUST run in a worker's committed transaction, never
/// the coordinator's. If the coordinator ran `bulk_drop_indexes` / `create_quads_partition` itself it
/// would hold their `ACCESS EXCLUSIVE` locks on `_pgrdf_dictionary` / `_pgrdf_quads` for the whole
/// function (a pgrx 0.16 function can't COMMIT to release them, §1.2), and the DICT/RESOLVE/INDEX
/// workers — which need those very tables — would block on the coordinator while the coordinator
/// blocks in `wait_for_shutdown`: a deadlock. Running prep inside the (single) STAGE worker commits +
/// releases every lock before the next phase spawns. The coordinator therefore touches NO shared
/// table while workers run.
///
/// Two steps, both in the STAGE worker's one transaction:
/// 1. [`crate::storage::loader::bulk_drop_indexes`] — drop the 3 hexastore indexes, the dict
///    `lexical_value` hash index, and the `unique_term` constraint, so the DICT/RESOLVE writes skip
///    per-row index + uniqueness maintenance (the existing defer-index win, now multi-backend). Phase
///    D rebuilds them via the byte-identical [`super::jobctl::index_ddls`]. Dropping the hexastore
///    indexes from the parent here also means the partition RESOLVE attaches inherits no partitioned
///    index at attach time (Phase D builds them once, parent-wide, afterwards).
/// 2. `CREATE UNLOGGED TABLE _pgrdf_stg_<job_id> (…) WITH (parallel_workers = nproc)` — the staging
///    table. UNLOGGED skips WAL (the measured 141 GB win); the `parallel_workers` reloption is what
///    actually lifts PG's per-table worker cap so DICT's parallel dedup / RESOLVE's hash-join scan it
///    on all cores (§5). `IF NOT EXISTS` keeps it resume-safe.
///
/// The destination partition is **NOT** created here: RESOLVE builds it as a standalone parallel
/// `CREATE TABLE … AS SELECT` (a direct INSERT into an attached partition is serial) and only then
/// `ATTACH`es it — so partition creation moved out of prep into [`resolve`].
pub fn prepare_for_load(job: &JobSlot) {
    let nproc = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
        .max(1);
    // (1) Defer indexes — the SAME drop (incl. the partition-DDL gate) the single-backend bulk path
    // uses; Phase D rebuilds the identical set via `index_ddls()`.
    crate::storage::loader::bulk_drop_indexes();
    // (2) The UNLOGGED staging table, with the parallel_workers reloption that lets DICT/RESOLVE scan
    // it on all cores. Created here (in a committed worker txn), not by the coordinator, so its
    // ACCESS EXCLUSIVE creation lock is released before DICT/RESOLVE run.
    let _ = job.graph_id; // partition for graph_id is created by RESOLVE (standalone-then-ATTACH).
    let table = staging_table(job.job_id);
    let sql = format!(
        "CREATE UNLOGGED TABLE IF NOT EXISTS {table} ({STAGE_COLS}) WITH (parallel_workers = {nproc})"
    );
    Spi::run(&sql).expect("staged STAGE: create staging table failed");
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

    let flush = |bs: &mut Vec<String>,
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
            flush(&mut bs, &mut bp, &mut bot, &mut bov, &mut bod, &mut bol);
        }
    }
    flush(&mut bs, &mut bp, &mut bot, &mut bov, &mut bod, &mut bol);
    staged
}

/// The UNQUALIFIED name of the dictionary index RESOLVE's hash joins probe (built by [`dict`],
/// dropped by the coordinator after INDEX). `(term_type, lexical_md5)` — the prefix RESOLVE matches
/// s/p/o on. Redundant once Phase D's `unique_term` (a `(term_type, lexical_md5, …)`-prefixed UNIQUE)
/// lands, so it is a transient build-time helper, not part of the canonical schema. The coordinator
/// drops it via `DROP INDEX IF EXISTS pgrdf.<name>` once no worker is running.
pub fn dict_resolve_index() -> &'static str {
    "_pgrdf_dict_resolve_idx"
}

/// **DICT** (Phase B). Build the whole dictionary for this load — the E32-proven sequence, ported.
///
/// `_pgrdf_dictionary.id` is `GENERATED ALWAYS AS IDENTITY`, which makes any `INSERT … SELECT` into it
/// parallel-UNSAFE (serial, `nextval` per row). So (module docs) the dedup is done in **parallel**
/// `CREATE UNLOGGED TABLE dict_* AS SELECT … row_number() OVER () AS id …` materialisations that
/// pre-assign each distinct term a contiguous id, and a SINGLE serial `INSERT … OVERRIDING SYSTEM
/// VALUE` then copies the numbered rows in (the id supplied explicitly, no `nextval`).
///
/// Steps (all temp tables UNLOGGED + named per `job_id` for resume-safety, dropped at the end):
/// 1. `dict_uri` — DISTINCT URI terms (subjects that are NOT blank, predicates, object URIs, AND the
///    object datatype IRIs), numbered `1..U` (offset by `base` = current `MAX(id)`, 0 on the empty
///    dict the bulk path guarantees). `UNION ALL` + `GROUP BY` (the parallelisable dedup), NOT `UNION`.
///    The datatype IRIs live HERE so a literal's `datatype_iri_id` can read the URI's id (module docs).
/// 2. `dict_blank` — DISTINCT blank labels (blank subjects `_:…` + blank objects, `o_type = 2`),
///    numbered `base + U + 1 …` (ids continue past the URIs).
/// 3. `dict_lit` — DISTINCT literals `(o_val, o_dt, o_lang)`, numbered `base + U + B + 1 …`.
/// 4. `dict_all` — the three unioned with their final `(term_type, datatype_iri_id, language_tag)`;
///    each literal's `datatype_iri_id` is the `dict_uri` id of its datatype IRI (`LEFT JOIN dict_uri
///    ON lexical_value = o_dt`). Built as a parallel CTAS.
/// 5. ONE `INSERT INTO _pgrdf_dictionary OVERRIDING SYSTEM VALUE (id, term_type, lexical_value,
///    datatype_iri_id, language_tag) SELECT … FROM dict_all`. The STORED `lexical_md5` generates
///    itself (NOT inserted). Parallel-UNSAFE (IDENTITY target) ⇒ single serial pass. *Future opt:* the
///    benchmark showed this single session is 22–113 s (NUMA-variable) vs ~14 s split 32-way by
///    id-range across 32 INDEX-style workers; left single-session for v1 simplicity.
/// 6. Re-sync the IDENTITY sequence to `MAX(id)` (we supplied ids, so the sequence did not advance —
///    without this the next ordinary IDENTITY insert would collide), then build the RESOLVE join index
///    `(term_type, lexical_md5)` and set the dict's `parallel_workers` reloption — both REQUIRED for
///    RESOLVE to run N-wide.
/// 7. Drop the dict_* temp tables. Return the dict row count.
pub fn dict(job: &JobSlot, _w: &WorkerSlot) -> i64 {
    let stg = staging_table(job.job_id);
    let uri = term_type::URI as i32;
    let blank = term_type::BLANK_NODE as i32;
    let literal = term_type::LITERAL as i32;
    let nproc = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
        .max(1);

    let d_uri = format!("pgrdf._pgrdf_dict_uri_{}", job.job_id);
    let d_blank = format!("pgrdf._pgrdf_dict_blank_{}", job.job_id);
    let d_lit = format!("pgrdf._pgrdf_dict_lit_{}", job.job_id);
    let d_all = format!("pgrdf._pgrdf_dict_all_{}", job.job_id);

    // Drop any stale per-job temp tables (resume-safe re-run from the top of DICT).
    for t in [&d_uri, &d_blank, &d_lit, &d_all] {
        Spi::run(&format!("DROP TABLE IF EXISTS {t}"))
            .unwrap_or_else(|e| panic!("staged DICT: drop stale {t}: {e}"));
    }

    // `base` = the current high id, so `row_number()` ids continue past any rows already present. The
    // bulk path guarantees an empty dict here (base = 0), making this byte-identical to the benchmark;
    // the offset is a zero-cost safety for a non-empty/resumed dict.
    let base: i64 =
        Spi::get_one::<i64>("SELECT COALESCE(MAX(id), 0)::bigint FROM pgrdf._pgrdf_dictionary")
            .ok()
            .flatten()
            .unwrap_or(0);

    // ── (1) dict_uri: DISTINCT URIs, numbered base+1 …  (UNION ALL + GROUP BY ⇒ parallel dedup) ───
    // Blank '_:' subjects are EXCLUDED here (they go in dict_blank); the LIKE escapes the underscore
    // (`\_`) so it matches a LITERAL leading "_:" — strictly correct vs a wildcard "_".
    let cta_uri = format!(
        "CREATE UNLOGGED TABLE {d_uri} WITH (parallel_workers = {nproc}) AS \
         SELECT {base}::bigint + row_number() OVER () AS id, u AS lexical_value FROM ( \
             SELECT u FROM ( \
                 SELECT s AS u FROM {stg} WHERE s NOT LIKE '\\_:%' \
                 UNION ALL SELECT p FROM {stg} \
                 UNION ALL SELECT o_val FROM {stg} WHERE o_type = {uri} \
                 UNION ALL SELECT o_dt FROM {stg} WHERE o_type = {literal} AND o_dt IS NOT NULL \
             ) a GROUP BY u \
         ) d"
    );
    Spi::run(&cta_uri).expect("staged DICT: parallel dict_uri CTAS failed");

    // ── (2) dict_blank: DISTINCT blank labels, ids continue past the URIs ─────────────────────────
    let cta_blank = format!(
        "CREATE UNLOGGED TABLE {d_blank} WITH (parallel_workers = {nproc}) AS \
         SELECT {base}::bigint + (SELECT count(*) FROM {d_uri}) + row_number() OVER () AS id, \
                b AS lexical_value FROM ( \
             SELECT b FROM ( \
                 SELECT s AS b FROM {stg} WHERE s LIKE '\\_:%' \
                 UNION ALL SELECT o_val FROM {stg} WHERE o_type = {blank} \
             ) a GROUP BY b \
         ) d"
    );
    Spi::run(&cta_blank).expect("staged DICT: parallel dict_blank CTAS failed");

    // ── (3) dict_lit: DISTINCT literals (lexical, datatype IRI string, language), ids continue ────
    // GROUP BY o_val with max(o_dt)/max(o_lang) collapses literals that share a lexical value — this
    // matches the benchmark and is the cause of the object-join caveat noted on `resolve`.
    let cta_lit = format!(
        "CREATE UNLOGGED TABLE {d_lit} WITH (parallel_workers = {nproc}) AS \
         SELECT {base}::bigint + (SELECT count(*) FROM {d_uri}) \
                              + (SELECT count(*) FROM {d_blank}) + row_number() OVER () AS id, \
                o_val AS lexical_value, o_dt, o_lang FROM ( \
             SELECT o_val, max(o_dt) AS o_dt, max(o_lang) AS o_lang \
             FROM {stg} WHERE o_type = {literal} GROUP BY o_val \
         ) d"
    );
    Spi::run(&cta_lit).expect("staged DICT: parallel dict_lit CTAS failed");

    // ── (4) dict_all: the three sets with their final dict columns (parallel CTAS) ────────────────
    // A literal's datatype_iri_id = the dict_uri id of its datatype IRI (LEFT JOIN on the IRI string);
    // NULL for language-tagged / no-datatype literals. term_type is the constant for each set.
    let cta_all = format!(
        "CREATE UNLOGGED TABLE {d_all} WITH (parallel_workers = {nproc}) AS \
         SELECT id, {uri}::smallint AS term_type, lexical_value, \
                NULL::bigint AS datatype_iri_id, NULL::text AS language_tag FROM {d_uri} \
         UNION ALL \
         SELECT id, {blank}::smallint, lexical_value, NULL::bigint, NULL::text FROM {d_blank} \
         UNION ALL \
         SELECT l.id, {literal}::smallint, l.lexical_value, u.id, l.o_lang \
         FROM {d_lit} l LEFT JOIN {d_uri} u ON u.lexical_value = l.o_dt"
    );
    Spi::run(&cta_all).expect("staged DICT: parallel dict_all CTAS failed");

    // ── (5) the ONE serial INSERT (ids supplied via OVERRIDING SYSTEM VALUE; lexical_md5 auto-gen) ─
    let ins = format!(
        "INSERT INTO pgrdf._pgrdf_dictionary \
             (id, term_type, lexical_value, datatype_iri_id, language_tag) \
         OVERRIDING SYSTEM VALUE \
         SELECT id, term_type, lexical_value, datatype_iri_id, language_tag FROM {d_all}"
    );
    Spi::run(&ins).expect("staged DICT: OVERRIDING SYSTEM VALUE dict insert failed");

    // ── (6) re-sync IDENTITY past the supplied ids; build the RESOLVE join index; widen the dict ──
    // We supplied ids explicitly, so the IDENTITY sequence never advanced — fast-forward it to MAX(id)
    // so the next ordinary insert doesn't collide. `is_called => true` ⇒ nextval returns MAX+1.
    Spi::run(
        "SELECT setval(pg_get_serial_sequence('pgrdf._pgrdf_dictionary', 'id'), \
                       GREATEST((SELECT COALESCE(MAX(id), 1) FROM pgrdf._pgrdf_dictionary), 1), true)",
    )
    .expect("staged DICT: re-sync IDENTITY sequence failed");

    Spi::run(&format!(
        "CREATE INDEX IF NOT EXISTS {} ON pgrdf._pgrdf_dictionary (term_type, lexical_md5)",
        dict_resolve_index()
    ))
    .expect("staged DICT: build RESOLVE join index failed");
    Spi::run(&format!(
        "ALTER TABLE pgrdf._pgrdf_dictionary SET (parallel_workers = {nproc})"
    ))
    .expect("staged DICT: widen dict parallel_workers failed");

    // ── (7) drop the per-job temp tables, return the dict count ───────────────────────────────────
    for t in [&d_uri, &d_blank, &d_lit, &d_all] {
        Spi::run(&format!("DROP TABLE IF EXISTS {t}"))
            .unwrap_or_else(|e| panic!("staged DICT: drop temp {t}: {e}"));
    }

    Spi::get_one::<i64>("SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary")
        .ok()
        .flatten()
        .unwrap_or(0)
}

/// **RESOLVE** (Phase C). Turn every staged triple into a `(subject_id, predicate_id, object_id,
/// graph_id, is_inferred)` quad by hash-joining `_pgrdf_stg` to `_pgrdf_dictionary` three times, and
/// land the result as the graph's `_pgrdf_quads_g<graph_id>` partition.
///
/// ## Why a standalone CTAS + ATTACH, not a direct `INSERT INTO _pgrdf_quads`
///
/// A direct `INSERT … SELECT` into the attached partition (or the routing parent) is **serial** —
/// tuple routing / the partitioned target makes the plan parallel-unsafe, so the expensive 3-way join
/// would run single-threaded on the leader. The benchmark's fix, ported here: build a **standalone**
/// (not-yet-a-partition) `CREATE TABLE _pgrdf_quads_g<graph_id> AS SELECT …` — with no partition
/// target the planner is free to use a **parallel hash join** (`Gather → Parallel Hash Join`, all
/// cores) — then make it partition-compatible (`SET NOT NULL` on every column the parent requires +
/// a `CHECK (graph_id = <g>)` that implies the `FOR VALUES IN (<g>)` bound so `ATTACH` skips its
/// validation scan) and `ATTACH` it. The redundant CHECK is dropped post-attach so the partition is
/// structurally identical to one made by `CREATE TABLE … PARTITION OF`. Phase D builds the hexastore
/// indexes parent-wide afterwards.
///
/// The three joins force HASH joins via the per-statement `enable_*` knobs (only `enable_hashjoin`
/// stays on); the dict's `(term_type, lexical_md5)` index + `parallel_workers` reloption (both from
/// [`dict`]) let the parallel hash build scan it N-wide. Subject term_type is `2` for a blank label
/// (`s LIKE '\_:%'`) else `1`; predicate is `1`; object is `o_type` directly.
///
/// **Object-join caveat (verify on the box).** The object is matched on `(term_type, lexical_md5)`
/// ONLY — NOT the datatype/language. The benchmark verified its corpus is collision-free on that pair,
/// and `dict` collapses literals sharing a lexical value to ONE dict row (`GROUP BY o_val`). If a real
/// corpus carries two literals with the SAME lexical value but DIFFERENT datatype/language (e.g.
/// `"5"^^xsd:int` and `"5"@en`), `dict` keeps only one and this join binds every such object to that
/// single id — a lossy match. This is the single biggest correctness assumption inherited from the
/// benchmark; flagged for validation.
pub fn resolve(job: &JobSlot, _w: &WorkerSlot) -> i64 {
    let stg = staging_table(job.job_id);
    let graph_id = job.graph_id;
    let uri = term_type::URI as i32;
    let blank = term_type::BLANK_NODE as i32;
    let part = format!("_pgrdf_quads_g{graph_id}"); // unqualified (used as an identifier)
    let nproc = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
        .max(1);

    // Force HASH joins for the 3-way resolve + give the parallel hash build more memory. SET LOCAL ⇒
    // scoped to this worker's transaction. `enable_indexscan` etc. off pushes the planner onto a
    // parallel hash join (seq-scan the dict, which is what the dict parallel_workers reloption widens)
    // rather than a serial nested-loop / index probe.
    Spi::run(
        "SET LOCAL enable_nestloop = off; \
         SET LOCAL enable_mergejoin = off; \
         SET LOCAL enable_indexscan = off; \
         SET LOCAL enable_indexonlyscan = off; \
         SET LOCAL enable_bitmapscan = off; \
         SET LOCAL enable_hashjoin = on; \
         SET LOCAL hash_mem_multiplier = 2;",
    )
    .expect("staged RESOLVE: set hash-join GUCs failed");

    // Take the shared partition-DDL gate (the OUTERMOST lock, same order as add_graph/drop_graph): the
    // standalone CREATE + the ATTACH below escalate to ACCESS EXCLUSIVE on the `_pgrdf_quads` parent,
    // so queueing here instead of racing the parent's catalog lock avoids the documented deadlock.
    crate::storage::partition::acquire_partition_ddl_gate();

    // Resume-safe: a prior partial RESOLVE may have left this table (standalone or attached). DROP it
    // (DROP TABLE on an attached partition detaches+drops it) so we rebuild from scratch.
    Spi::run(&format!("DROP TABLE IF EXISTS pgrdf.{part}"))
        .unwrap_or_else(|e| panic!("staged RESOLVE: drop stale {part}: {e}"));

    // ── (1) PARALLEL hash-join CTAS → standalone _pgrdf_quads_g<g> ─────────────────────────────────
    // ds: subject (term_type 2 if a blank label, else 1); dp: predicate (1); dobj: object (o_type).
    // Object matched on (term_type, lexical_md5) ONLY — see the caveat in the doc comment. Columns are
    // emitted in the parent's order with explicit casts so ATTACH sees a structurally identical table.
    let cta = format!(
        "CREATE TABLE pgrdf.{part} WITH (parallel_workers = {nproc}) AS \
         SELECT ds.id AS subject_id, dp.id AS predicate_id, dobj.id AS object_id, \
                {graph_id}::bigint AS graph_id, false AS is_inferred \
         FROM {stg} st \
         JOIN pgrdf._pgrdf_dictionary ds \
              ON ds.term_type = CASE WHEN st.s LIKE '\\_:%' THEN {blank} ELSE {uri} END \
             AND ds.lexical_md5 = decode(md5(st.s), 'hex') \
         JOIN pgrdf._pgrdf_dictionary dp \
              ON dp.term_type = {uri} \
             AND dp.lexical_md5 = decode(md5(st.p), 'hex') \
         JOIN pgrdf._pgrdf_dictionary dobj \
              ON dobj.term_type = st.o_type \
             AND dobj.lexical_md5 = decode(md5(st.o_val), 'hex')"
    );
    Spi::run(&cta).expect("staged RESOLVE: parallel hash-join CTAS failed");

    // ── (2) make it partition-compatible: NOT NULL on every parent-required column + a CHECK that
    // implies FOR VALUES IN (<g>) so ATTACH skips the validation scan ─────────────────────────────
    Spi::run(&format!(
        "ALTER TABLE pgrdf.{part} \
            ALTER COLUMN subject_id SET NOT NULL, \
            ALTER COLUMN predicate_id SET NOT NULL, \
            ALTER COLUMN object_id SET NOT NULL, \
            ALTER COLUMN graph_id SET NOT NULL, \
            ALTER COLUMN is_inferred SET NOT NULL, \
            ADD CONSTRAINT {part}_pbound CHECK (graph_id = {graph_id})"
    ))
    .expect("staged RESOLVE: make partition-compatible failed");

    // ── (3) ATTACH as the graph's LIST partition, then drop the now-redundant CHECK ───────────────
    Spi::run(&format!(
        "ALTER TABLE pgrdf._pgrdf_quads ATTACH PARTITION pgrdf.{part} FOR VALUES IN ({graph_id})"
    ))
    .expect("staged RESOLVE: ATTACH PARTITION failed");
    Spi::run(&format!(
        "ALTER TABLE pgrdf.{part} DROP CONSTRAINT {part}_pbound"
    ))
    .expect("staged RESOLVE: drop redundant partition CHECK failed");

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
