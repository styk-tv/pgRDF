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
use oxttl::NTriplesParser;
use pgrx::prelude::*;
use rayon::prelude::*;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Read, Write};

/// Staging columns: the raw triple as parsed, with the object split into its
/// (type, lexical value, datatype IRI string, language tag) so the set-based DICT/RESOLVE phases can
/// reproduce the dictionary's `(term_type, lexical_value, datatype_iri_id, language_tag)` key.
/// `o_type` is the `term_type` SMALLINT; `o_dt` is the datatype IRI **string** (resolved to an id in
/// DICT); `o_lang` is the language tag. NULL `o_dt`/`o_lang` mirror the NULLS-DISTINCT dict key.
const STAGE_COLS: &str = "s text, p text, o_type smallint, o_val text, o_dt text, o_lang text";

/// Lines pulled from the worker's byte range into one STAGE window before it is parsed (on all cores)
/// and `COPY`-flushed. The whole file is ~1.2 TB, so STAGE must never hold more than one window: peak
/// RAM is bounded by `STAGE_WINDOW_LINES` × (the parsed row's owned strings + the serialized TSV
/// buffer for that window), NOT by the worker's byte range. At ~120 B/line for truthy N-Triples, 4 M
/// lines ≈ ~0.5 GB of raw line bytes plus a similar order for the parsed `String`s and the TSV
/// buffer — call it ~1.5–2 GB resident per window. That comfortably fits the 251 GB E32 box (with
/// 32 cores all parsing one window) and is dwarfed on the 1.26 TB E160. Larger windows amortise the
/// per-window COPY round-trip further but raise the floor; 4 M is the balance the E160 benchmark hit.
const STAGE_WINDOW_LINES: usize = 4_000_000;

/// Lines per rayon `par_chunks` slice when parsing a window. Each chunk is parsed by its own
/// `NTriplesParser`; 4096 matches the streaming loader's `par_chunks(4096)` — small enough to balance
/// work across all cores on the last (short) window, large enough that the per-chunk parser
/// construction cost is amortised over thousands of lines.
const STAGE_PARSE_CHUNK: usize = 4096;

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

/// One parsed staging row, owned: exactly the six staging columns. The object is pre-split into
/// `(o_type, o_val, o_dt, o_lang)` by the verbatim oxttl term→column mapping (see [`stage`]). Built
/// in the rayon parse closures (pure-CPU, no SPI), then serialized to the COPY TSV on the worker
/// thread.
type StagedRow = (String, String, i16, String, Option<String>, Option<String>);

/// Append one text value to a COPY-TEXT-format buffer, escaping the four bytes PostgreSQL's COPY TEXT
/// reader treats specially: backslash, newline, carriage-return, and tab (the field delimiter). Every
/// other byte (including the UTF-8 continuation bytes of multi-byte chars) is passed through
/// untouched — COPY TEXT is byte-transparent outside these escapes, so a value round-trips exactly.
/// This is the inverse of the server-side `COPY … FROM '<file>' WITH (FORMAT text)` parse, so a value
/// written here is read back byte-identical to the oxttl-parsed string. NULLs are handled by the
/// caller (it writes the literal `\N` instead of calling this).
fn copy_escape_into(out: &mut String, v: &str) {
    for ch in v.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
}

/// Serialize one [`StagedRow`] as a single COPY-TEXT line (tab-delimited, `\n`-terminated) into
/// `out`. Column order matches the `COPY {stg} (s,p,o_type,o_val,o_dt,o_lang)` target. `o_type` is an
/// integer (no escaping needed); `o_dt`/`o_lang` emit the literal `\N` when `None` (the COPY-TEXT NULL
/// sentinel), mirroring the staging table's NULL `o_dt`/`o_lang` and the dict's NULLS-DISTINCT key.
fn write_copy_row(out: &mut String, row: &StagedRow) {
    let (s, p, o_type, o_val, o_dt, o_lang) = row;
    copy_escape_into(out, s);
    out.push('\t');
    copy_escape_into(out, p);
    out.push('\t');
    // o_type is a smallint 1/2/3 — write it directly, no escaping.
    out.push_str(itoa_smallint(*o_type));
    out.push('\t');
    copy_escape_into(out, o_val);
    out.push('\t');
    match o_dt {
        Some(dt) => copy_escape_into(out, dt),
        None => out.push_str("\\N"),
    }
    out.push('\t');
    match o_lang {
        Some(l) => copy_escape_into(out, l),
        None => out.push_str("\\N"),
    }
    out.push('\n');
}

/// The `o_type` smallint is always one of `term_type::{URI,BLANK_NODE,LITERAL}` (1/2/3), so map it to
/// a static `&str` instead of allocating via `format!`/`to_string` on the hot path (one call per
/// staged row, billions of rows). Any other value is a contract violation (the term→column mapping
/// only emits 1/2/3) and panics rather than silently mis-staging.
fn itoa_smallint(v: i16) -> &'static str {
    match v {
        x if x == term_type::URI => "1",
        x if x == term_type::BLANK_NODE => "2",
        x if x == term_type::LITERAL => "3",
        _ => panic!("staged STAGE: unexpected o_type {v}"),
    }
}

/// Parse one window's lines ACROSS ALL CORES into owned [`StagedRow`]s, leniently (a malformed line
/// is skipped and counted, not fatal — the same Wikidata-control-byte robustness as
/// `loader.rs::load_turtle_streaming`). N-Triples is line-oriented, so the window buffer splits
/// cleanly on `\n` and each `par_chunks` slice is parsed by its own `NTriplesParser` — embarrassingly
/// parallel, no shared state. The term→column extraction is byte-for-byte the same match the serial
/// `stage()` used (URI/blank → `as_str()` sans brackets; literal → lexical value sans quotes, with
/// datatype IRI string XOR language tag), so the staged rows are identical regardless of core count.
/// Returns `(rows, skipped)`.
fn parse_window_par(lines: &[String]) -> (Vec<StagedRow>, i64) {
    let parsed: Vec<(Vec<StagedRow>, i64)> = lines
        .par_chunks(STAGE_PARSE_CHUNK)
        .map(|chunk| {
            let mut out: Vec<StagedRow> = Vec::with_capacity(chunk.len());
            let mut skipped: i64 = 0;
            for line in chunk {
                for r in NTriplesParser::new().for_reader(line.as_bytes()) {
                    let t = match r {
                        Ok(t) => t,
                        Err(_) => {
                            skipped += 1;
                            continue; // lenient: skip malformed triples (Wikidata control bytes)
                        }
                    };
                    let s = match &t.subject {
                        NamedOrBlankNode::NamedNode(n) => n.as_str().to_string(),
                        NamedOrBlankNode::BlankNode(b) => b.as_str().to_string(),
                    };
                    let p = t.predicate.as_str().to_string();
                    let (o_type, o_val, o_dt, o_lang): (
                        i16,
                        String,
                        Option<String>,
                        Option<String>,
                    ) = match &t.object {
                        Term::NamedNode(n) => (term_type::URI, n.as_str().to_string(), None, None),
                        Term::BlankNode(b) => {
                            (term_type::BLANK_NODE, b.as_str().to_string(), None, None)
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
                        _ => panic!("staged STAGE: unsupported object term"),
                    };
                    out.push((s, p, o_type, o_val, o_dt, o_lang));
                }
            }
            (out, skipped)
        })
        .collect();
    let skipped: i64 = parsed.iter().map(|(_, s)| *s).sum();
    let rows: Vec<StagedRow> = parsed.into_iter().flat_map(|(c, _)| c).collect();
    (rows, skipped)
}

/// Load one parsed window into `{table}` via server-side **COPY** (the fast path the E160 benchmark
/// measured at 71.5 M rows in 31 s, vs minutes for `unnest` INSERT). pgrx/SPI does not expose COPY
/// FROM STDIN, so the robust route is: serialize the window's rows to a server-side temp file in COPY
/// TEXT format, `COPY … FROM '<file>'`, then delete the file.
///
/// The temp file lives in `/tmp` (world-writable + sticky on every supported platform). The pgRDF
/// backend process — which is BOTH the writer here (it is an ordinary OS process) and the reader (the
/// COPY runs in this same backend's transaction) — owns the file, so the server-side COPY can always
/// read it back; no cross-user permission question arises because there is no second user. The file
/// name carries `job_id` + a per-window counter so concurrent jobs / windows never collide, and it is
/// removed on the success path. The COPY runs inside the STAGE worker's transaction (this function is
/// called from within it), so the staged rows are part of the same per-phase recovery unit.
fn copy_window(table: &str, job_id: i64, window_idx: u64, rows: &[StagedRow]) {
    if rows.is_empty() {
        return;
    }
    let path = format!("/tmp/pgrdf_stg_{job_id}_{window_idx}.tsv");

    // Serialize to the TSV. Pre-size generously (~96 B/row is a low-ball for truthy) so the buffer
    // rarely reallocates; a BufWriter keeps the actual disk writes block-sized.
    {
        let f = File::create(&path)
            .unwrap_or_else(|e| panic!("staged STAGE: create temp {path:?} failed: {e}"));
        let mut w = BufWriter::new(f);
        let mut buf = String::with_capacity(rows.len().saturating_mul(96).min(256 << 20));
        for row in rows {
            write_copy_row(&mut buf, row);
            // Drain the in-memory buffer to the writer in bounded chunks so peak extra RAM is the
            // chunk, not the whole window's TSV on top of the parsed rows.
            if buf.len() >= (64 << 20) {
                w.write_all(buf.as_bytes())
                    .unwrap_or_else(|e| panic!("staged STAGE: write temp {path:?} failed: {e}"));
                buf.clear();
            }
        }
        w.write_all(buf.as_bytes())
            .unwrap_or_else(|e| panic!("staged STAGE: write temp {path:?} failed: {e}"));
        w.flush()
            .unwrap_or_else(|e| panic!("staged STAGE: flush temp {path:?} failed: {e}"));
    }

    // COPY it in. FORMAT text + the default tab delimiter + `\N` NULL match `write_copy_row`. The
    // path is a backend-local string literal we control (job_id + counter, no user input), so the
    // single-quote interpolation is safe; still, COPY only accepts a string literal here.
    let sql = format!(
        "COPY {table} (s, p, o_type, o_val, o_dt, o_lang) FROM '{path}' WITH (FORMAT text)"
    );
    let copy_res = Spi::run(&sql);
    // Always remove the temp file, success or failure, so a resumed run does not accrete TSVs.
    let _ = std::fs::remove_file(&path);
    copy_res.unwrap_or_else(|e| panic!("staged STAGE: COPY from {path:?} failed: {e}"));
}

/// **STAGE** (Phase A). Stream this worker's byte range of the `.nt` file in bounded WINDOWS, parse
/// each window across ALL cores with rayon, and load it into the pre-created UNLOGGED staging table
/// via server-side **COPY** (the fast path).
///
/// The coordinator snapped `[range_lo, range_hi)` to newline boundaries and recorded them in the
/// [`WorkerSlot`]; today it spawns ONE STAGE worker over the whole file, so the across-cores
/// parallelism lives INSIDE this worker (rayon over each window) rather than across N table-writing
/// backends — which deliberately avoids the extension-lock contention that N bgworkers writing one
/// table caused. The flow per window:
///
/// 1. **Stream** up to [`STAGE_WINDOW_LINES`] lines from `[lo, hi)` via `File::take(hi-lo)` + a
///    `BufReader` (never `read_to_end` — the file is ~1.2 TB; peak RAM is one window, not the range).
///    Blank/comment lines are dropped here, matching `load_turtle_streaming`.
/// 2. **Parse** the window across all cores ([`parse_window_par`]): split on `\n`, `par_chunks` the
///    lines, each parsed by its own `NTriplesParser` (truthy is N-Triples — line-oriented + perfectly
///    parallel). Lenient: malformed lines are skipped + counted, same as today.
/// 3. **COPY** the parsed rows into the staging table ([`copy_window`]) via a server-side temp TSV,
///    not `unnest` INSERT — the measured order-of-magnitude win at scale.
///
/// Returns the total count of triples staged by THIS worker (summed across windows), for the tally.
pub fn stage(job: &JobSlot, w: &WorkerSlot) -> i64 {
    let path = job.path();
    let lo = w.range_lo;
    let hi = w.range_hi;
    let table = staging_table(job.job_id);

    let mut file =
        File::open(&path).unwrap_or_else(|e| panic!("staged STAGE: open {path:?} failed: {e}"));
    // Stream exactly the byte slice [lo, hi): seek to lo, then `.take(hi-lo)` so the BufReader's reads
    // naturally stop at the range end without ever materialising the whole (~1.2 TB) file. The
    // coordinator snapped both bounds to newline boundaries, so the slice is a whole number of lines.
    use std::io::Seek;
    file.seek(std::io::SeekFrom::Start(lo))
        .unwrap_or_else(|e| panic!("staged STAGE: seek to {lo} failed: {e}"));
    let mut reader = BufReader::with_capacity(1 << 20, file.take(hi.saturating_sub(lo)));

    let mut window_lines: Vec<String> = Vec::with_capacity(STAGE_WINDOW_LINES);
    let mut staged: i64 = 0;
    let mut window_idx: u64 = 0;

    loop {
        // ── Fill one window from the stream (bounded by line count AND the byte range via `take`).
        window_lines.clear();
        let mut eof = false;
        while window_lines.len() < STAGE_WINDOW_LINES {
            let mut line = String::new();
            let n = reader
                .read_line(&mut line)
                .unwrap_or_else(|e| panic!("staged STAGE: read [{lo},{hi}) failed: {e}"));
            if n == 0 {
                eof = true;
                break;
            }
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue; // skip blank / comment lines (match load_turtle_streaming)
            }
            window_lines.push(line);
        }
        if window_lines.is_empty() {
            break;
        }

        // ── Parse this window across all cores, then COPY it in.
        let (rows, _skipped) = parse_window_par(&window_lines);
        staged += rows.len() as i64;
        copy_window(&table, job.job_id, window_idx, &rows);
        window_idx += 1;

        if eof {
            break;
        }
    }
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
