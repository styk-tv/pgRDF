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

/// Read total host RAM in bytes from Linux `/proc/meminfo` (`MemTotal: <kB> kB`). Returns `None` on
/// any platform without a readable `/proc/meminfo` (e.g. a macOS dev box) or if the line can't be
/// parsed — callers then fall back to the previous fixed work-mem behaviour so nothing changes where
/// we can't measure. Pure I/O + parse, no SPI, so it is cheap to call once per phase.
fn host_mem_total_bytes() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            // Format is `MemTotal:   263456789 kB` — value is in kibibytes.
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb.saturating_mul(1024));
        }
    }
    None
}

/// Derive `(work_mem_kb, maintenance_work_mem_kb)` for the staged phases as memory HARDENING against
/// OOM on small-RAM hosts. RETURNS KIBIBYTES (the unit `SET work_mem = '<n>kB'` takes).
///
/// **This is hardening, not the definitive fix.** The risk it targets is RESOLVE's 3-way parallel
/// hash join: its peak hash-table budget is roughly
///   `work_mem × hash_mem_multiplier(2) × max_parallel_workers_per_gather × 3 joins`
/// (see [`resolve`], which sets `hash_mem_multiplier = 2` and `max_parallel_workers_per_gather =
/// nproc`). We size `work_mem` so that product stays ≤ ~50 % of `MemTotal`, then clamp to a sane
/// range `[64 MB, 2 GB]`. When RAM-per-core is high enough (`MemTotal ≳ 24 GB × nproc`) this pins at
/// the 2 GB cap — IDENTICAL to the prior fixed value; below that it scales DOWN with the core count
/// (e.g. ~669 MB on a 251 GiB / 32-core E32, where the OLD fixed 2 GB would have implied a 384 GB
/// 3-way hash budget — larger than the box). On a small-RAM host it floors at 64 MB and the hash
/// SPILLS to temp files (batched hash joins spill automatically — we keep `enable_hashjoin = on` and
/// add nothing that prevents spilling) instead of risking an OOM kill. The *definitive* 8.2 B-scale
/// RESOLVE fix is DEFERRED pending a real at-scale diagnostic (now obtainable: the staged-worker
/// panic reporting surfaces the actual `ereport` message — see `pool::panic_message`). Don't read
/// this as "RESOLVE at 8.2 B is fixed"; it only lowers the OOM risk where RAM is tight.
///
/// `mem_total`: host RAM in bytes, or `None` when unmeasured (→ the previous fixed 2 GB / 16 GB, so
/// behaviour is unchanged where we can't measure). `nproc`: the worker's own parallelism (the
/// `max_parallel_workers_per_gather` RESOLVE will use). Pure arithmetic, unit-tested below.
fn derive_work_mem_kb(mem_total: Option<u64>, nproc: usize) -> (u64, u64) {
    // Clamp bounds, in kibibytes.
    // work_mem ∈ [64 MB, 2 GB]: the cap equals the prior fixed value (so a high-RAM-per-core host is
    // unchanged); the floor keeps a tight host workable and from there the hash SPILLS (PG default ~4 MB).
    const WORK_MEM_MIN_KB: u64 = 64 * 1024; // 64 MB
    const WORK_MEM_MAX_KB: u64 = 2 * 1024 * 1024; // 2 GB
                                                  // maintenance_work_mem ∈ [256 MB, 16 GB]: the cap equals the prior fixed value.
    const MAINT_MIN_KB: u64 = 256 * 1024; // 256 MB
    const MAINT_MAX_KB: u64 = 16 * 1024 * 1024; // 16 GB

    let mem_total = match mem_total {
        // Unmeasured (e.g. macOS dev box, no /proc/meminfo): keep the EXACT prior fixed values
        // (work_mem = 2 GB, maintenance_work_mem = 16 GB) — no behaviour change where we can't measure.
        None => return (WORK_MEM_MAX_KB, MAINT_MAX_KB),
        Some(b) => b,
    };
    let nproc = nproc.max(1) as u64;

    // Budget for the 3-way parallel hash join must stay ≤ 50 % of RAM:
    //   work_mem × 2 (hash_mem_multiplier) × nproc (workers/gather) × 3 (joins) ≤ MemTotal / 2
    // ⇒ work_mem ≤ MemTotal / (2 × 2 × nproc × 3) = MemTotal / (12 × nproc).
    let mem_total_kb = mem_total / 1024;
    let work_mem_kb = (mem_total_kb / (12 * nproc)).clamp(WORK_MEM_MIN_KB, WORK_MEM_MAX_KB);

    // maintenance_work_mem scales with the same 50 %-of-RAM headroom but is used by a single
    // CREATE INDEX build at a time (per INDEX worker), so it is bounded by ~12 % of RAM (≈ half of
    // RAM / a handful of concurrent builds) and clamped to its own range.
    let maint_kb = (mem_total_kb / 8).clamp(MAINT_MIN_KB, MAINT_MAX_KB);

    (work_mem_kb, maint_kb)
}

/// Build the `SET LOCAL temp_tablespaces = '<value>';` fragment from the configured
/// `pgrdf.staged_temp_tablespaces` value, or `None` to emit nothing.
///
/// This is the pure (no-SPI) core of routing the staged loader's temp spill (chiefly RESOLVE's forced
/// parallel 3-way hash join — see [`apply_session_gucs`]) off the PGDATA disk. It is split out as a
/// free function so it is unit-testable without a live backend.
///
/// * Empty / whitespace-only ⇒ `None`: the loader emits no `temp_tablespaces` override and inherits
///   the server default (the prior behaviour — no change where the operator hasn't opted in).
/// * Otherwise the value must be a tablespace-NAME LIST: the same comma-separated SQL-identifier list
///   the core `temp_tablespaces` GUC takes (e.g. `fast_ssd` or `ts_a, ts_b`). We VALIDATE it as such
///   ([`is_safe_tablespace_list`]) and reject anything that is not — a tablespace name is an
///   identifier, never a quoted/dotted/`;`-bearing string — so the interpolated value can never break
///   out of the single-quoted `SET LOCAL` statement. A rejected value ⇒ `Err(reason)`; the caller logs
///   it and falls back to the server default rather than emitting unsafe SQL.
///
/// The validated list is wrapped in single quotes exactly like `temp_tablespaces` expects. Validation
/// guarantees no `'` is present, so no further escaping is needed.
fn temp_tablespaces_set_fragment(configured: &str) -> Result<Option<String>, &'static str> {
    let v = configured.trim();
    if v.is_empty() {
        return Ok(None); // inherit the server default — unchanged behaviour
    }
    if !is_safe_tablespace_list(v) {
        return Err(
            "pgrdf.staged_temp_tablespaces must be a comma-separated list of plain tablespace \
             identifiers (letters, digits, underscore; no quotes, semicolons, or whitespace within a \
             name)",
        );
    }
    Ok(Some(format!("SET LOCAL temp_tablespaces = '{v}';")))
}

/// Is `s` a safe tablespace-name list — a non-empty comma-separated list of plain SQL identifiers,
/// each `[A-Za-z_][A-Za-z0-9_]*`? Used to gate `pgrdf.staged_temp_tablespaces` before it is
/// interpolated into a `SET LOCAL` statement (see [`temp_tablespaces_set_fragment`]). Rejects quotes,
/// semicolons, dots, and any other punctuation, so the value cannot break out of the single-quoted SQL
/// literal nor smuggle a second statement. Surrounding/separating spaces around a comma are tolerated
/// (`a, b`), but a space WITHIN a name is not — that keeps the grammar to bare identifiers only
/// (quoted identifiers, which alone could contain spaces, are deliberately out of scope).
fn is_safe_tablespace_list(s: &str) -> bool {
    let parts: Vec<&str> = s.split(',').map(str::trim).collect();
    if parts.is_empty() {
        return false;
    }
    parts.iter().all(|name| {
        let mut chars = name.chars();
        match chars.next() {
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
            _ => return false, // empty part (e.g. trailing comma) or bad leading char
        }
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Map `pgrdf.staged_resolve_strategy` to the `SET LOCAL` planner block RESOLVE runs before its 3-way
/// join CTAS. Returns `(chosen, sql)` where `chosen` is the strategy name actually applied (after any
/// fallback) and `sql` is the `SET LOCAL …` fragment.
///
/// The join OUTPUT is identical for every join method — RESOLVE writes the same rows regardless — so
/// this is purely a PERFORMANCE knob (the all-hash-join builds a multi-TB temp spill at 8.2 B rows;
/// the index-nested-loop path stays low-spill). This is split out as a free function so the
/// strategy → SQL mapping is unit-testable without a live backend.
///
/// * `"hash"` — force the all-hash-join (the historical behaviour, the known-safe fallback).
/// * `"index"` — force the low-spill index-nested-loop path.
/// * `"auto"` — emit no `enable_*` forcing; let the planner choose with the adaptive `work_mem` + the
///   dict resolve index + the parallel reloption already in place (still bumps `hash_mem_multiplier`).
/// * anything else — fall back to the `"hash"` block (known-safe behaviour); `chosen` is then `"hash"`
///   while the requested strategy was not, which the caller ([`resolve`]) detects to log a warning.
///
/// This fn is PURE (no SPI, no `ereport`) so it is unit-testable without a live backend; the
/// unrecognised-value warning is emitted by the caller, not here.
fn resolve_join_strategy_sql(strategy: &str) -> (&'static str, String) {
    // The historical forced all-hash-join block — also the known-safe fallback.
    const HASH_SQL: &str = "SET LOCAL enable_nestloop = off; \
         SET LOCAL enable_mergejoin = off; \
         SET LOCAL enable_indexscan = off; \
         SET LOCAL enable_indexonlyscan = off; \
         SET LOCAL enable_bitmapscan = off; \
         SET LOCAL enable_hashjoin = on; \
         SET LOCAL hash_mem_multiplier = 2;";
    // The low-spill index-nested-loop block.
    const INDEX_SQL: &str = "SET LOCAL enable_hashjoin = off; \
         SET LOCAL enable_mergejoin = off; \
         SET LOCAL enable_nestloop = on; \
         SET LOCAL enable_indexscan = on; \
         SET LOCAL enable_indexonlyscan = on; \
         SET LOCAL enable_bitmapscan = on; \
         SET LOCAL hash_mem_multiplier = 2;";
    // `auto`: no enable_* forcing — let the planner choose. Still bump hash_mem_multiplier.
    const AUTO_SQL: &str = "SET LOCAL hash_mem_multiplier = 2;";

    match strategy.trim() {
        "hash" => ("hash", HASH_SQL.to_string()),
        "index" => ("index", INDEX_SQL.to_string()),
        "auto" => ("auto", AUTO_SQL.to_string()),
        // Unrecognised value: fall back to the known-safe all-hash-join. The caller warns.
        _ => ("hash", HASH_SQL.to_string()),
    }
}

/// Re-apply the per-session parallel levers (§5) inside the worker's transaction. GUCs are
/// per-session and a dynamic worker starts with server defaults, so each phase that wants PG's
/// intra-query parallelism (DICT hash-agg, RESOLVE hash-join, INDEX maintenance) must `SET LOCAL`
/// them itself. `nproc` is the worker's own `num_cpus`; the staging/dict `parallel_workers`
/// reloption (set elsewhere) is what actually lifts the per-table worker cap, these GUCs raise the
/// session ceilings to match. `SET LOCAL` scopes them to this transaction only.
///
/// `work_mem` / `maintenance_work_mem` are HARDENED against OOM: instead of the prior FIXED 2 GB /
/// 16 GB they adapt to host RAM (see [`derive_work_mem_kb`]). On a big-RAM host they land at the
/// same 2 GB / 16 GB cap (no change); on a small-RAM host they shrink so RESOLVE's 3-way parallel
/// hash join spills to temp rather than risking OOM. This is hardening only — the definitive
/// 8.2 B-scale RESOLVE fix is deferred pending an at-scale diagnostic.
///
/// `temp_tablespaces` is routed from `pgrdf.staged_temp_tablespaces` (when set): RESOLVE forces a
/// parallel all-hash-join (`enable_mergejoin`/`enable_nestloop = off`, see [`resolve`]) whose spill is
/// roughly the size of the dictionary + the staged data and, at 8.2 B rows, was measured at ~3 TB.
/// PostgreSQL writes that spill under `base/pgsql_tmp` on the PGDATA disk by default; on a box whose
/// PGDATA sits on a small (~3.4 TB) volume that filled the disk (`No space left on device`). Emitting
/// `SET LOCAL temp_tablespaces` here moves EVERY staged phase's spill onto an operator-chosen,
/// roomier mount. Empty GUC ⇒ nothing emitted ⇒ the server default disk (unchanged).
pub fn apply_session_gucs() {
    let nproc = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(8)
        .max(1);
    let maint_workers = (nproc / 2).max(1);
    let mem_total = host_mem_total_bytes();
    let (work_mem_kb, maint_mem_kb) = derive_work_mem_kb(mem_total, nproc);

    // Optional `SET LOCAL temp_tablespaces = '…'` from pgrdf.staged_temp_tablespaces. The value is a
    // tablespace-name list validated to a bare-identifier grammar before it reaches the SQL (see
    // `temp_tablespaces_set_fragment`), so an unsafe value never lands here — it is logged and dropped,
    // and the spill falls back to the server default disk rather than emitting unsafe SQL.
    let temp_ts_sql =
        match temp_tablespaces_set_fragment(&crate::query::guc::staged_temp_tablespaces()) {
            Ok(Some(frag)) => frag,
            Ok(None) => String::new(), // empty GUC: inherit the server default temp tablespace
            Err(reason) => {
                pgrx::warning!(
                    "staged loader: ignoring pgrdf.staged_temp_tablespaces — {reason}; \
                 temp spill stays on the server default tablespace"
                );
                String::new()
            }
        };

    // T5 instrumentation: surface the self-tuning decision in the PG log so an operator can see what
    // the staged loader derived for this host (the foundation for later auto-routing). Pure logging —
    // it reports the values already computed above and changes no tuning math.
    let mem_total_desc = mem_total
        .map(|b| format!("{:.1}GiB", b as f64 / (1024.0 * 1024.0 * 1024.0)))
        .unwrap_or_else(|| "unmeasured".to_string());
    let configured_temp_ts = crate::query::guc::staged_temp_tablespaces();
    let temp_ts_desc = if configured_temp_ts.is_empty() {
        "server-default".to_string()
    } else {
        configured_temp_ts
    };
    pgrx::log!(
        "staged self-tune: MemTotal={mem_total_desc} nproc={nproc} work_mem={}MB \
         maintenance_work_mem={}MB max_parallel_workers={nproc} temp_tablespaces={temp_ts_desc}",
        work_mem_kb / 1024,
        maint_mem_kb / 1024
    );

    // One statement, semicolon-separated: cheaper than N SPI round-trips. All are GUCs that exist on
    // PG17; `SET LOCAL` confines them to this transaction (auto-reset on commit). work_mem /
    // maintenance_work_mem are set in kB (the smallest GUC unit) so the derived value applies exactly.
    // The temp_tablespaces fragment (when set) is appended so every phase's temp spill — chiefly
    // RESOLVE's forced parallel hash join — lands on the operator-chosen mount, not the PGDATA disk.
    let sql = format!(
        "SET LOCAL max_parallel_workers = {nproc}; \
         SET LOCAL max_parallel_workers_per_gather = {nproc}; \
         SET LOCAL max_parallel_maintenance_workers = {maint_workers}; \
         SET LOCAL enable_parallel_hash = on; \
         SET LOCAL parallel_setup_cost = 0; \
         SET LOCAL parallel_tuple_cost = 0; \
         SET LOCAL min_parallel_table_scan_size = 0; \
         SET LOCAL min_parallel_index_scan_size = 0; \
         SET LOCAL work_mem = '{work_mem_kb}kB'; \
         SET LOCAL maintenance_work_mem = '{maint_mem_kb}kB'; \
         {temp_ts_sql}"
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

/// The dedup SELECT that feeds `dict_lit` — one row per DISTINCT literal IDENTITY, i.e. per distinct
/// `(lexical value, datatype-IRI string, language tag)` triple, NOT per lexical value. This is the
/// single source of truth shared by [`dict`] and its regression test.
///
/// An RDF literal's identity is its `(value, datatype, language)` triple, mirroring the dict's
/// `unique_term (term_type, lexical_md5, datatype_iri_id, language_tag)` key. Grouping by `o_val`
/// alone (and stamping `max(o_dt)`/`max(o_lang)`) would collapse `"Berlin"@en`, `"Berlin"@de`,
/// `"1"^^xsd:integer` and a plain/lang `"1"` into ONE dict row carrying the MAX datatype AND the MAX
/// language — a row that is simultaneously typed and language-tagged (impossible in RDF) and loses
/// every variant but one. So the GROUP/SELECT key is the FULL `(o_val, o_dt, o_lang)`; each distinct
/// literal becomes its own dict id. `o_dt`/`o_lang` are NULLABLE and NULLS-DISTINCT under `GROUP BY`
/// the same way the dict's NULLS-DISTINCT `unique_term` treats them, so a plain `"1"` (o_dt NULL,
/// o_lang NULL) and `"1"^^xsd:string` (o_dt = the xsd:string IRI) stay distinct.
fn dict_lit_dedup_select(stg: &str, literal: i32) -> String {
    format!("SELECT o_val, o_dt, o_lang FROM {stg} WHERE o_type = {literal} GROUP BY o_val, o_dt, o_lang")
}

/// The object-side `ON` predicate of RESOLVE's quad-object → dict-id hash join (the `dobj` join in
/// [`resolve`]). Single source of truth, shared with the regression test.
///
/// `dobj` is the candidate `_pgrdf_dictionary` row; `st` is the staging row; `ddt` is a SECOND dict
/// alias LEFT-JOINed so a literal's datatype-IRI **string** can be compared (the dict stores the
/// datatype as a dict *id* in `datatype_iri_id`, not the IRI string). The match is:
///
/// * `term_type` + `lexical_md5` — the value, as before.
/// * `language_tag IS NOT DISTINCT FROM o_lang` — NULL-safe, so a plain/typed literal (lang NULL) and
///   a `@en` literal with the same value resolve to DIFFERENT ids; URIs/blanks have NULL lang and
///   `st.o_lang` is NULL for them, so this stays TRUE (no change to URI/blank resolution).
/// * `ddt.lexical_value IS NOT DISTINCT FROM o_dt` — `ddt` is the datatype IRI's own URI term
///   (`ddt.id = dobj.datatype_iri_id`); via the LEFT JOIN it is NULL exactly when `dobj.datatype_iri_id`
///   is NULL (URI/blank, or a no-datatype/lang literal), and `NULL IS NOT DISTINCT FROM st.o_dt` is
///   TRUE only when `st.o_dt` is itself NULL — again preserving URI/blank resolution unchanged while
///   making `"1"^^xsd:integer` vs a plain/lang `"1"` resolve to their own ids.
///
/// Without the datatype/language predicates the object join would be AMBIGUOUS now that `dict_lit`
/// keeps multiple rows per lexical value (see [`dict_lit_dedup_select`]): `(term_type, lexical_md5)`
/// alone would match several dict rows and bind the object to an arbitrary one.
fn resolve_object_join_on() -> &'static str {
    "ON dobj.term_type = st.o_type \
        AND dobj.lexical_md5 = decode(md5(st.o_val), 'hex') \
        AND dobj.language_tag IS NOT DISTINCT FROM st.o_lang \
        AND ddt.lexical_value IS NOT DISTINCT FROM st.o_dt"
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

    // ── (3) dict_lit: DISTINCT literals by FULL identity (lexical value, datatype IRI string,
    // language tag), ids continue. The dedup key is the WHOLE `(o_val, o_dt, o_lang)` triple — see
    // `dict_lit_dedup_select`: each distinct RDF literal gets its own dict id, so `"Berlin"@en` /
    // `"Berlin"@de` / `"1"^^xsd:integer` / a plain `"1"` are FOUR rows, not one collapsed row.
    let cta_lit = format!(
        "CREATE UNLOGGED TABLE {d_lit} WITH (parallel_workers = {nproc}) AS \
         SELECT {base}::bigint + (SELECT count(*) FROM {d_uri}) \
                              + (SELECT count(*) FROM {d_blank}) + row_number() OVER () AS id, \
                o_val AS lexical_value, o_dt, o_lang FROM ( {dedup} ) d",
        dedup = dict_lit_dedup_select(&stg, literal)
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
/// **Object join matches the FULL literal identity** (see [`resolve_object_join_on`]). `dict_lit` now
/// keeps one row per distinct `(value, datatype, language)` (see [`dict_lit_dedup_select`]), so the
/// object join can no longer match on `(term_type, lexical_md5)` alone — that would be AMBIGUOUS across
/// the several dict rows a single lexical value can now have. The `dobj` join therefore also requires
/// `language_tag IS NOT DISTINCT FROM o_lang` and (via a LEFT-JOINed `ddt` = the dict row of `dobj`'s
/// datatype IRI) `ddt.lexical_value IS NOT DISTINCT FROM o_dt`, both NULL-safe so URIs/blanks and
/// no-datatype/lang literals (all NULL `o_dt`/`o_lang`) still resolve exactly as before. So
/// `"5"^^xsd:int` and `"5"@en` resolve to their OWN ids — the prior lossy value-only collision is gone.
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

    // Set the planner join strategy for the 3-way resolve from `pgrdf.staged_resolve_strategy`. SET
    // LOCAL ⇒ scoped to this worker's transaction. The join OUTPUT is identical for any join method
    // (RESOLVE writes the same rows regardless), so this is a PERFORMANCE knob, not a correctness one:
    // `hash` forces the historical parallel all-hash-join (which spills multi-TB to temp at 8.2 B
    // rows); `index` forces the low-spill index-nested-loop path; `auto` (the default) lets the planner
    // choose using the adaptive work_mem + the dict resolve index + the parallel reloption in place.
    let requested_strategy = crate::query::guc::staged_resolve_strategy();
    let (resolve_strategy, resolve_sql) = resolve_join_strategy_sql(&requested_strategy);
    if resolve_strategy != requested_strategy.trim() {
        pgrx::warning!(
            "pgrdf.staged_resolve_strategy = '{requested_strategy}' is not one of auto|hash|index — \
             falling back to '{resolve_strategy}' (the known-safe all-hash-join)"
        );
    }
    pgrx::log!("staged RESOLVE: join strategy = {resolve_strategy}");
    Spi::run(&resolve_sql).expect("staged RESOLVE: set join-strategy GUCs failed");

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
    // The object is matched on its FULL literal identity, not value alone (see `resolve_object_join_on`
    // / `dict_lit_dedup_select`): `dobj` carries `language_tag` directly and a LEFT-JOINed `ddt` (the
    // dict row of `dobj`'s datatype IRI) supplies the datatype IRI STRING to compare against `o_dt`,
    // both via `IS NOT DISTINCT FROM` so NULLs (URIs/blanks, plain literals) match correctly. Columns
    // are emitted in the parent's order with explicit casts so ATTACH sees a structurally identical
    // table.
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
         LEFT JOIN pgrdf._pgrdf_dictionary ddt ON ddt.id = dobj.datatype_iri_id \
              {dobj_on}",
        dobj_on = resolve_object_join_on()
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

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use super::{dict_lit_dedup_select, parse_window_par, resolve_object_join_on, STAGE_COLS};
    use crate::storage::dict::term_type;
    use pgrx::prelude::*;

    /// Regression for the literal full-key dedup bug (release-blocking): the staged loader used to
    /// deduplicate literals by lexical VALUE alone — `GROUP BY o_val` with `max(o_dt)`/`max(o_lang)` —
    /// collapsing `"Berlin"@en`, `"Berlin"@de`, `"1"^^xsd:integer` and a plain/lang `"1"` into ONE dict
    /// row stamped with the MAX datatype AND the MAX language (a row that is simultaneously typed and
    /// language-tagged — impossible in RDF — and loses every variant but one). The matching value-only
    /// RESOLVE object join then bound every such object to that single id.
    ///
    /// This drives the EXACT production SQL — the same `dict_lit_dedup_select` and
    /// `resolve_object_join_on` the [`super::dict`]/[`super::resolve`] phases build — against a tiny
    /// hand-built staging + dictionary pair (no background-worker pool, so it runs in the ordinary
    /// `cargo pgrx test` harness), and asserts the literal IDENTITY is preserved end to end.
    #[pg_test]
    fn staged_literal_full_key_dedup_and_resolve() {
        let stg = "pgrdf._pgrdf_stg_littest";
        let uri = term_type::URI as i32;
        let blank = term_type::BLANK_NODE as i32;
        let literal = term_type::LITERAL as i32;

        // ── Fixture: the four bug literals (Berlin@en/@de, "1"^^xsd:integer, "1"@en) + a URI object
        // and a blank object (to prove URI/blank resolution is unchanged), each on a distinct subject
        // so every staged row resolves to exactly one quad. Mirrors STAGE_COLS.
        Spi::run(&format!(
            "DROP TABLE IF EXISTS {stg}; CREATE TABLE {stg} ({cols});",
            cols = STAGE_COLS
        ))
        .expect("create staging fixture");
        Spi::run(&format!(
            "INSERT INTO {stg}(s,p,o_type,o_val,o_dt,o_lang) VALUES \
             ('http://ex/a','http://ex/label',{literal},'Berlin',NULL,'en'), \
             ('http://ex/b','http://ex/label',{literal},'Berlin',NULL,'de'), \
             ('http://ex/c','http://ex/n',{literal},'1','http://www.w3.org/2001/XMLSchema#integer',NULL), \
             ('http://ex/d','http://ex/n',{literal},'1',NULL,'en'), \
             ('http://ex/e','http://ex/ref',{uri},'http://ex/target',NULL,NULL), \
             ('http://ex/f','http://ex/bn',{blank},'_:b0',NULL,NULL)"
        ))
        .expect("seed staging fixture");

        // ── DICT (the fixed full-key dedup feeds dict_lit) — the same shape super::dict builds, minus
        // the parallel_workers reloption (irrelevant to correctness). base = current MAX(id).
        let base: i64 =
            Spi::get_one::<i64>("SELECT COALESCE(MAX(id),0)::bigint FROM pgrdf._pgrdf_dictionary")
                .unwrap()
                .unwrap_or(0);
        // The dict_* are TEMP tables: each #[pg_test] runs in its own rolled-back transaction, so
        // they are fresh here and vanish at test end — no pre-drop needed.
        Spi::run(&format!(
            "CREATE TEMP TABLE dict_uri_lt AS \
             SELECT {base}::bigint + row_number() OVER () AS id, u AS lexical_value FROM ( \
               SELECT u FROM ( \
                 SELECT s AS u FROM {stg} WHERE s NOT LIKE '\\_:%' \
                 UNION ALL SELECT p FROM {stg} \
                 UNION ALL SELECT o_val FROM {stg} WHERE o_type = {uri} \
                 UNION ALL SELECT o_dt FROM {stg} WHERE o_type = {literal} AND o_dt IS NOT NULL \
               ) a GROUP BY u) d"
        ))
        .expect("dict_uri");
        Spi::run(&format!(
            "CREATE TEMP TABLE dict_blank_lt AS \
             SELECT {base}::bigint + (SELECT count(*) FROM dict_uri_lt) + row_number() OVER () AS id, \
                    b AS lexical_value FROM ( \
               SELECT b FROM ( \
                 SELECT s AS b FROM {stg} WHERE s LIKE '\\_:%' \
                 UNION ALL SELECT o_val FROM {stg} WHERE o_type = {blank} \
               ) a GROUP BY b) d"
        ))
        .expect("dict_blank");
        // The line under test: dict_lit's dedup SELECT is the production helper verbatim.
        Spi::run(&format!(
            "CREATE TEMP TABLE dict_lit_lt AS \
             SELECT {base}::bigint + (SELECT count(*) FROM dict_uri_lt) \
                                  + (SELECT count(*) FROM dict_blank_lt) + row_number() OVER () AS id, \
                    o_val AS lexical_value, o_dt, o_lang FROM ( {dedup} ) d",
            dedup = dict_lit_dedup_select(stg, literal)
        ))
        .expect("dict_lit (full-key dedup)");
        Spi::run(&format!(
            "CREATE TEMP TABLE dict_all_lt AS \
             SELECT id, {uri}::smallint AS term_type, lexical_value, NULL::bigint AS datatype_iri_id, \
                    NULL::text AS language_tag FROM dict_uri_lt \
             UNION ALL SELECT id, {blank}::smallint, lexical_value, NULL::bigint, NULL::text FROM dict_blank_lt \
             UNION ALL SELECT l.id, {literal}::smallint, l.lexical_value, u.id, l.o_lang \
                       FROM dict_lit_lt l LEFT JOIN dict_uri_lt u ON u.lexical_value = l.o_dt"
        ))
        .expect("dict_all");
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_dictionary (id, term_type, lexical_value, datatype_iri_id, language_tag) \
             OVERRIDING SYSTEM VALUE \
             SELECT id, term_type, lexical_value, datatype_iri_id, language_tag FROM dict_all_lt",
        )
        .expect("dict insert");
        Spi::run(
            "SELECT setval(pg_get_serial_sequence('pgrdf._pgrdf_dictionary','id'), \
                           GREATEST((SELECT COALESCE(MAX(id),1) FROM pgrdf._pgrdf_dictionary),1), true)",
        )
        .expect("resync identity");

        // ── Assertion 1: "Berlin"@en and "Berlin"@de are TWO distinct dict rows (not one). ──────────
        let berlin: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary \
             WHERE term_type = 3 AND lexical_value = 'Berlin'",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            berlin, 2,
            r#""Berlin"@en and "Berlin"@de must be two distinct dict rows"#
        );

        // ── Assertion 2: ZERO dict rows carry BOTH a datatype_iri_id AND a language_tag (the
        // impossible-in-RDF state the value-only dedup produced). ──────────────────────────────────
        let both: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary \
             WHERE datatype_iri_id IS NOT NULL AND language_tag IS NOT NULL",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            both, 0,
            "no dict row may have BOTH a datatype and a language tag"
        );

        // ── Assertion 3: "1"^^xsd:integer and a plain/lang "1" are distinct dict rows. ─────────────
        let ones: i64 = Spi::get_one(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary \
             WHERE term_type = 3 AND lexical_value = '1'",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            ones, 2,
            r#""1"^^xsd:integer and a lang/plain "1" must be distinct dict rows"#
        );

        // ── RESOLVE (the fixed full-key object join) — the same `resolve_object_join_on` predicate +
        // its `ddt` LEFT JOIN that super::resolve builds. Every staged row must resolve to exactly one
        // quad bound to the CORRECT object id (no value-only collision, no fan-out from the ddt join).
        Spi::run(&format!(
            "CREATE TEMP TABLE _q_littest AS \
             SELECT ds.id AS subject_id, dp.id AS predicate_id, dobj.id AS object_id \
             FROM {stg} st \
             JOIN pgrdf._pgrdf_dictionary ds \
                  ON ds.term_type = CASE WHEN st.s LIKE '\\_:%' THEN {blank} ELSE {uri} END \
                 AND ds.lexical_md5 = decode(md5(st.s), 'hex') \
             JOIN pgrdf._pgrdf_dictionary dp \
                  ON dp.term_type = {uri} AND dp.lexical_md5 = decode(md5(st.p), 'hex') \
             JOIN pgrdf._pgrdf_dictionary dobj \
             LEFT JOIN pgrdf._pgrdf_dictionary ddt ON ddt.id = dobj.datatype_iri_id \
                  {dobj_on}",
            dobj_on = resolve_object_join_on()
        ))
        .expect("resolve full-key join CTAS");

        // Exactly one quad per staged triple (6) — no loss, no fan-out from the extra ddt LEFT JOIN.
        let quads: i64 = Spi::get_one("SELECT count(*)::bigint FROM _q_littest")
            .unwrap()
            .unwrap();
        assert_eq!(quads, 6, "every staged triple resolves to exactly one quad");

        // Every object bound to the literal/term whose (value, datatype, language) it actually is —
        // proves the value-only collision is gone AND URI/blank objects still resolve (their NULL
        // o_dt/o_lang match via IS NOT DISTINCT FROM). Count rows where the bound object DISAGREES.
        let wrong: i64 = Spi::get_one(&format!(
            "SELECT count(*)::bigint FROM {stg} st \
             JOIN _q_littest q \
                  ON q.subject_id = ( \
                       SELECT id FROM pgrdf._pgrdf_dictionary \
                       WHERE lexical_md5 = decode(md5(st.s),'hex') \
                         AND term_type = CASE WHEN st.s LIKE '\\_:%' THEN {blank} ELSE {uri} END) \
             JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id \
             LEFT JOIN pgrdf._pgrdf_dictionary dt ON dt.id = o.datatype_iri_id \
             WHERE NOT ( o.lexical_value IS NOT DISTINCT FROM st.o_val \
                     AND dt.lexical_value IS NOT DISTINCT FROM st.o_dt \
                     AND o.language_tag  IS NOT DISTINCT FROM st.o_lang )"
        ))
        .unwrap()
        .unwrap();
        assert_eq!(
            wrong, 0,
            "each object must resolve to the dict row matching its full identity"
        );

        // Cleanup the persistent staging fixture (temp tables drop at txn end on their own).
        Spi::run(&format!("DROP TABLE IF EXISTS {stg}")).ok();
    }

    /// The staged loader's window parser is LENIENT: a malformed N-Triples line is skipped and COUNTED
    /// (`parse_skipped`-style), never a panic — the Wikidata control-byte robustness the streaming
    /// loader has. This exercises [`super::parse_window_par`] (the staged STAGE parser) directly: one
    /// good triple + one malformed line ⇒ one parsed row, skipped >= 1, no panic.
    #[pg_test]
    fn staged_parse_window_lenient_skips_malformed() {
        let lines = vec![
            "<http://ex/s> <http://ex/p> <http://ex/o> .".to_string(),
            "this is not a valid n-triples line at all".to_string(),
        ];
        let (rows, skipped) = parse_window_par(&lines);
        assert_eq!(rows.len(), 1, "the one well-formed triple is parsed");
        assert!(
            skipped >= 1,
            "the malformed line must be counted as skipped (got {skipped}), not panic"
        );
    }

    /// **Local proxy for "RESOLVE degrades to spill, not OOM."** No box is available to reproduce the
    /// 8.2 B-row failure, so this drives the RESOLVE hash-join SQL DIRECTLY (the same force-hash GUCs +
    /// `resolve_object_join_on` join `super::resolve` builds — NOT via the bgworker pool, matching the
    /// v0.6.12 test pattern) under a deliberately TINY `work_mem = '64kB'` over a fixture large enough
    /// to blow past 64 kB of hash table. PostgreSQL's batched hash join spills to temp files when the
    /// build side exceeds `work_mem`; the test asserts the join still returns the CORRECT row count
    /// with NO error — i.e. tight memory makes RESOLVE spill, it does not make it fail.
    #[pg_test]
    fn staged_resolve_hashjoin_spills_under_tight_work_mem() {
        let stg = "pgrdf._pgrdf_stg_spilltest";
        let uri = term_type::URI as i32;
        let blank = term_type::BLANK_NODE as i32;

        // N distinct URI triples — each subject/object a distinct URI, one shared predicate. 4000 rows
        // of ~40-byte keys is ~hundreds of kB of hash table, comfortably exceeding 64 kB so the build
        // side MUST batch/spill. Distinct subjects ⇒ every staged row resolves to exactly one quad.
        const N: i64 = 4000;
        Spi::run(&format!(
            "DROP TABLE IF EXISTS {stg}; CREATE TABLE {stg} ({cols});",
            cols = STAGE_COLS
        ))
        .expect("create spill staging fixture");
        Spi::run(&format!(
            "INSERT INTO {stg}(s,p,o_type,o_val,o_dt,o_lang) \
             SELECT 'http://ex/s' || g, 'http://ex/p', {uri}, 'http://ex/o' || g, NULL, NULL \
             FROM generate_series(1, {N}) g"
        ))
        .expect("seed spill staging fixture");

        // Intern every term the join probes (subjects, the predicate, objects) into the dictionary so
        // the 3-way join can resolve every row. row_number() over the distinct union mirrors DICT.
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_dictionary (term_type, lexical_value) \
             SELECT {uri}, t FROM ( \
                 SELECT s AS t FROM {stg} \
                 UNION SELECT p FROM {stg} \
                 UNION SELECT o_val FROM {stg} \
             ) d"
        ))
        .expect("intern spill dict terms");

        // Force the SAME plan shape RESOLVE uses (parallel-friendly hash joins only) AND clamp work_mem
        // to 64 kB so the hash build MUST spill. SET LOCAL ⇒ scoped to this test's transaction.
        // enable_hashjoin stays ON (batched hash joins spill automatically — nothing here prevents it).
        Spi::run(
            "SET LOCAL enable_nestloop = off; \
             SET LOCAL enable_mergejoin = off; \
             SET LOCAL enable_indexscan = off; \
             SET LOCAL enable_indexonlyscan = off; \
             SET LOCAL enable_bitmapscan = off; \
             SET LOCAL enable_hashjoin = on; \
             SET LOCAL hash_mem_multiplier = 2; \
             SET LOCAL work_mem = '64kB';",
        )
        .expect("set tight-memory hash-join GUCs");

        // The RESOLVE 3-way join, verbatim shape (ds=subject, dp=predicate, dobj=object + the ddt
        // LEFT JOIN), built with the production `resolve_object_join_on` predicate. Under 64 kB
        // work_mem this spills to temp files; it must still complete and resolve every row.
        Spi::run(&format!(
            "CREATE TEMP TABLE _q_spilltest AS \
             SELECT ds.id AS subject_id, dp.id AS predicate_id, dobj.id AS object_id \
             FROM {stg} st \
             JOIN pgrdf._pgrdf_dictionary ds \
                  ON ds.term_type = CASE WHEN st.s LIKE '\\_:%' THEN {blank} ELSE {uri} END \
                 AND ds.lexical_md5 = decode(md5(st.s), 'hex') \
             JOIN pgrdf._pgrdf_dictionary dp \
                  ON dp.term_type = {uri} AND dp.lexical_md5 = decode(md5(st.p), 'hex') \
             JOIN pgrdf._pgrdf_dictionary dobj \
             LEFT JOIN pgrdf._pgrdf_dictionary ddt ON ddt.id = dobj.datatype_iri_id \
                  {dobj_on}",
            dobj_on = resolve_object_join_on()
        ))
        .expect("RESOLVE hash join must SPILL (not error) under 64 kB work_mem");

        // Correct row count: exactly one quad per staged triple — the spill changed performance, not
        // the result. This is the local stand-in for "RESOLVE degrades to spill, not OOM."
        let quads: i64 = Spi::get_one("SELECT count(*)::bigint FROM _q_spilltest")
            .unwrap()
            .unwrap();
        assert_eq!(
            quads, N,
            "every staged triple resolves to exactly one quad even when the hash join spills to temp"
        );

        Spi::run(&format!("DROP TABLE IF EXISTS {stg}")).ok();
    }
}

/// Pure (no-Postgres) unit tests. Kept in a plain `#[cfg(test)]` module — NOT the `#[pg_schema]`
/// block above — so the arithmetic test compiles + runs as an ordinary `cargo test`/`cargo pgrx
/// test` unit test (the `#[pg_schema]` macro only carries `#[pg_test]`s into the in-database harness).
#[cfg(test)]
mod unit_tests {
    use super::{derive_work_mem_kb, resolve_join_strategy_sql, temp_tablespaces_set_fragment};

    /// Unit test of the RESOLVE memory-hardening arithmetic ([`super::derive_work_mem_kb`]) — pure,
    /// no Postgres. Five regimes:
    ///   * the E32 box (251 GiB, 32 cores) lands work_mem strictly WITHIN bounds and below the 2 GB
    ///     cap (~669 MB), because the OLD fixed 2 GB would imply a 384 GB 3-way hash budget on a
    ///     251 GiB box — this is the OOM the hardening removes; the budget now fits within half-RAM;
    ///   * a TINY-RAM host hits the 64 MB work_mem floor (it can't go lower) — from there the hash
    ///     SPILLS rather than OOMs (that floor is the only place the half-RAM bound is exceeded);
    ///   * an UNMEASURED host (`None`, e.g. the macOS dev box with no /proc/meminfo) returns the
    ///     EXACT prior fixed values (2 GB / 16 GB) so behaviour is unchanged where we can't measure;
    ///   * a MID-size host scales strictly between the floor and the cap (proves it actually scales);
    ///   * a HIGH-RAM-per-core host (768 GiB, 32 cores) pins work_mem at the 2 GB cap (unchanged).
    #[test]
    fn derive_work_mem_kb_bounds() {
        const WORK_MIN: u64 = 64 * 1024; // 64 MB in kB
        const WORK_MAX: u64 = 2 * 1024 * 1024; // 2 GB in kB
        const MAINT_MIN: u64 = 256 * 1024; // 256 MB in kB
        const MAINT_MAX: u64 = 16 * 1024 * 1024; // 16 GB in kB
        let gib = |n: u64| n * 1024 * 1024 * 1024;

        // E32: 251 GiB, 32 cores. work_mem = MemTotal/(12*32) ≈ 669 MB — within bounds, below the cap.
        let mem_251g = gib(251);
        let (work, maint) = derive_work_mem_kb(Some(mem_251g), 32);
        assert!(
            (WORK_MIN..=WORK_MAX).contains(&work),
            "a 251 GiB / 32-core host keeps work_mem within [64 MB, 2 GB] (got {work} kB)"
        );
        assert!(
            work < WORK_MAX,
            "at 251 GiB / 32 cores work_mem scales BELOW the 2 GB cap (got {work} kB) — the OLD fixed \
             2 GB implied a 384 GB 3-way hash budget on a 251 GiB box (the OOM this removes)"
        );
        assert!(
            (MAINT_MIN..=MAINT_MAX).contains(&maint),
            "maintenance_work_mem stays within [256 MB, 16 GB] (got {maint} kB)"
        );
        // The hardening invariant (holds wherever work_mem is above the floor): the 3-way
        // parallel-hash budget — work_mem * 2 (hash_mem_multiplier) * 32 (workers) * 3 (joins) — is
        // ≤ 50 % of RAM, so RESOLVE's hash tables fit in half the box instead of risking OOM.
        assert!(
            work * 2 * 32 * 3 <= (mem_251g / 1024) / 2,
            "the 3-way hash budget must stay within half of host RAM"
        );

        // Tiny-RAM host: 1 GiB, 8 cores. MemTotal/(12*8) ≈ 10.9 MB < the 64 MB floor ⇒ clamps UP to
        // the floor (the smallest work_mem we allow; the hash spills from there rather than OOMs).
        let (work_tiny, maint_tiny) = derive_work_mem_kb(Some(gib(1)), 8);
        assert_eq!(
            work_tiny, WORK_MIN,
            "a 1 GiB host floors work_mem at 64 MB (so RESOLVE's hash spills, not OOMs)"
        );
        assert_eq!(
            maint_tiny, MAINT_MIN,
            "a 1 GiB host floors maintenance_work_mem at 256 MB"
        );

        // Unmeasured host (None): the exact prior fixed values, so nothing changes where we can't probe.
        assert_eq!(
            derive_work_mem_kb(None, 32),
            (WORK_MAX, MAINT_MAX),
            "an unmeasured host keeps the prior fixed work_mem = 2 GB / maintenance_work_mem = 16 GB"
        );

        // A mid-size host (64 GiB, 16 cores) lands strictly between the floor and the cap, proving the
        // formula actually scales rather than always pinning to a bound.
        let (work_mid, _maint_mid) = derive_work_mem_kb(Some(gib(64)), 16);
        assert!(
            work_mid > WORK_MIN && work_mid < WORK_MAX,
            "a 64 GiB / 16-core host scales work_mem strictly between the floor and the cap (got {work_mid} kB)"
        );

        // A high-RAM-per-core host (768 GiB, 32 cores): MemTotal/(12*32) ≥ 2 GB ⇒ pinned at the cap,
        // i.e. IDENTICAL to the prior fixed 2 GB where the box is roomy enough to afford it.
        let (work_big, _maint_big) = derive_work_mem_kb(Some(gib(768)), 32);
        assert_eq!(
            work_big, WORK_MAX,
            "a 768 GiB / 32-core host pins work_mem at the 2 GB cap (unchanged from the fixed value)"
        );
    }

    /// Unit test of the temp-spill routing fragment ([`super::temp_tablespaces_set_fragment`]) — pure,
    /// no Postgres. This is the mechanism `pgrdf.staged_temp_tablespaces` rides into every staged
    /// phase's session GUCs (the RESOLVE hash-join spill that filled a small PGDATA disk at 8.2 B).
    /// Three regimes:
    ///   * a SET value (single tablespace, and a comma list with spaces) ⇒ a `SET LOCAL
    ///     temp_tablespaces = '<value>';` fragment carrying that exact value;
    ///   * an EMPTY / whitespace-only value ⇒ `None`, so the loader emits nothing and inherits the
    ///     server default disk (the prior, unchanged behaviour);
    ///   * an UNSAFE value (quote / semicolon / dotted name) ⇒ `Err`, so an injection attempt is
    ///     rejected and never reaches the SQL.
    #[test]
    fn temp_tablespaces_set_fragment_cases() {
        // Set: a single bare identifier ⇒ exactly the SET LOCAL fragment, value verbatim.
        assert_eq!(
            temp_tablespaces_set_fragment("fast_ssd"),
            Ok(Some("SET LOCAL temp_tablespaces = 'fast_ssd';".to_string())),
            "a configured tablespace name must be emitted as a temp_tablespaces SET LOCAL fragment"
        );
        // Set: a comma list with spaces (the temp_tablespaces grammar) is accepted as-is.
        assert_eq!(
            temp_tablespaces_set_fragment("  ts_a, ts_b  "),
            Ok(Some(
                "SET LOCAL temp_tablespaces = 'ts_a, ts_b';".to_string()
            )),
            "a comma-separated tablespace list (trimmed) must be accepted and emitted verbatim"
        );

        // Empty / whitespace ⇒ None: emit nothing, inherit the server default (no behaviour change).
        assert_eq!(
            temp_tablespaces_set_fragment(""),
            Ok(None),
            "an empty value emits nothing (inherit the server default temp tablespace)"
        );
        assert_eq!(
            temp_tablespaces_set_fragment("   "),
            Ok(None),
            "a whitespace-only value is treated as empty"
        );

        // Unsafe ⇒ Err: a single quote or a semicolon trying to break out / chain a statement, a dotted
        // name, and an empty list element (trailing comma) are all rejected before they reach the SQL.
        for bad in [
            "fast'; DROP TABLE pgrdf._pgrdf_dictionary; --",
            "ts'; SELECT 1",
            "a; b",
            "schema.ts",
            "ts_a,",
            "ts a", // space WITHIN a name (only bare identifiers are allowed)
            "*",
        ] {
            assert!(
                temp_tablespaces_set_fragment(bad).is_err(),
                "unsafe value {bad:?} must be rejected, not interpolated into SET LOCAL"
            );
        }
    }

    /// Unit test of the RESOLVE join-strategy mapping ([`super::resolve_join_strategy_sql`]) — pure,
    /// no Postgres. This is the mechanism `pgrdf.staged_resolve_strategy` rides into RESOLVE's planner
    /// `SET LOCAL` block. The join OUTPUT is identical for any join method, so this only changes which
    /// plan the forced block steers the planner to. Four regimes:
    ///   * `"hash"` ⇒ the historical forced all-hash-join (hashjoin on, nestloop off);
    ///   * `"index"` ⇒ the low-spill index-nested-loop path (hashjoin off, nestloop on);
    ///   * `"auto"` ⇒ no `enable_*` forcing at all (neither `enable_hashjoin` nor `enable_nestloop`),
    ///     just `hash_mem_multiplier`, so the planner is left to choose;
    ///   * an UNKNOWN value ⇒ falls back to the `"hash"` fragment (the known-safe behaviour) and reports
    ///     `chosen == "hash"` so the caller can detect the fallback.
    #[test]
    fn resolve_join_strategy_sql_cases() {
        // hash: force the all-hash-join.
        let (chosen, sql) = resolve_join_strategy_sql("hash");
        assert_eq!(chosen, "hash", "'hash' selects the hash strategy");
        assert!(
            sql.contains("enable_hashjoin = on"),
            "the hash fragment must turn enable_hashjoin on"
        );
        assert!(
            sql.contains("enable_nestloop = off"),
            "the hash fragment must turn enable_nestloop off"
        );

        // index: force the low-spill index-nested-loop path.
        let (chosen, sql) = resolve_join_strategy_sql("index");
        assert_eq!(chosen, "index", "'index' selects the index strategy");
        assert!(
            sql.contains("enable_hashjoin = off"),
            "the index fragment must turn enable_hashjoin off"
        );
        assert!(
            sql.contains("enable_nestloop = on"),
            "the index fragment must turn enable_nestloop on"
        );

        // auto: no enable_* forcing, only hash_mem_multiplier — let the planner choose.
        let (chosen, sql) = resolve_join_strategy_sql("auto");
        assert_eq!(chosen, "auto", "'auto' selects the auto strategy");
        assert!(
            !sql.contains("enable_hashjoin"),
            "the auto fragment must NOT force enable_hashjoin"
        );
        assert!(
            !sql.contains("enable_nestloop"),
            "the auto fragment must NOT force enable_nestloop"
        );
        assert!(
            sql.contains("hash_mem_multiplier"),
            "the auto fragment must still bump hash_mem_multiplier"
        );

        // Unknown value ⇒ fall back to the hash fragment (known-safe behaviour).
        let (chosen, sql) = resolve_join_strategy_sql("bogus");
        assert_eq!(
            chosen, "hash",
            "an unrecognised strategy falls back to the known-safe 'hash'"
        );
        assert_eq!(
            sql,
            resolve_join_strategy_sql("hash").1,
            "the unknown-value fragment is identical to the hash fragment"
        );
    }
}
