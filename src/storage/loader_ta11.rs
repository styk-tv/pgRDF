//! TA-11 spike — measure batch-size sensitivity in the bulk INSERT path
//! before attempting the heap_multi_insert direct C-API variant.
//!
//! Phase-0 (v0.5.26) showed insert_ms is 19% of LUBM-1 ingest time
//! (312 ms for 100k triples ≈ 3.1 µs per triple via the prepared
//! `INSERT ... unnest($1,$2,$3)` path). The TA-11 spike's job is to
//! quantify whether heap_multi_insert (PG's low-level C-API bulk
//! insert) would move that number, and if so by how much.
//!
//! Before writing 200+ lines of unsafe Rust against pgrx::pg_sys, we
//! measure where the existing prepared-unnest path's time actually
//! goes. The dominant cost contributor at the LUBM-1 scale is one of:
//!
//! 1. **SPI roundtrip** (per-batch fixed cost). The 100k triples /
//!    BATCH_SIZE=1000 = 100 batches → 100 SPI roundtrips. If a single
//!    huge batch (everything in one call) is much faster than 100
//!    small batches, then SPI roundtrip dominates → heap_multi_insert
//!    has plausible payoff (it skips SPI entirely).
//! 2. **Per-row insert work** (heap insert, WAL, FSM). If a single
//!    huge batch is roughly the same speed as 100 small batches, then
//!    per-row work dominates and heap_multi_insert (which still does
//!    the per-row insert work) has limited payoff.
//!
//! The spike exposes:
//!
//! - `pgrdf.spike_ta11_batch_sweep(triple_count INT, batch_size INT)
//!   -> JSONB` — generates `triple_count` synthetic triples (s, p, o,
//!   g = sequential bigints) and inserts them in batches of
//!   `batch_size` into a temp `_pgrdf_ta11_target` table via the same
//!   prepared `unnest` SQL the baseline ingest path uses. Returns
//!   timing breakdown.
//!
//! Comparison sweep (LUBM-1-scale 100,000 triples / batch_size
//! ∈ {100, 1000 [current default], 10000, 100000}) is invoked from
//! `tests/perf/lubm/spike-ta11.sh` (or manually); results live in
//! `tests/perf/lubm/spike-ta11.lubm-1.{json,md}`.
//!
//! The spike does NOT touch the production `_pgrdf_quads` partitioned
//! table — it uses a flat temp table so partition routing complexity
//! doesn't confound the measurement. A production version (TA-7
//! landing) would need partition routing on top of whichever path
//! wins.

use pgrx::prelude::*;
use serde_json::json;
use std::time::Instant;

/// SQL for the batch insert. Mirrors the BASELINE QUAD_INSERT_SQL
/// (loader.rs:50) but targets a flat temp table so partition routing
/// doesn't confound the measurement.
const TA11_INSERT_SQL: &str = "INSERT INTO pgrdf_ta11_target \
    (subject_id, predicate_id, object_id, graph_id) \
    SELECT s, p, o, $4 \
      FROM unnest($1::bigint[], $2::bigint[], $3::bigint[]) AS t(s, p, o)";

/// SQL for setting up the spike target table — flat, un-partitioned,
/// dropped before each spike run so warm-cache effects are bounded.
const TA11_SETUP_SQL: &str = "
    DROP TABLE IF EXISTS pgrdf_ta11_target;
    CREATE UNLOGGED TABLE pgrdf_ta11_target (
        subject_id   bigint NOT NULL,
        predicate_id bigint NOT NULL,
        object_id    bigint NOT NULL,
        graph_id     bigint NOT NULL
    );
";

/// Phase 1 of TA-11 — measure batch-size sensitivity of the existing
/// prepared-unnest path against a flat (un-partitioned) target. See
/// the module docstring for what this informs.
///
/// SQL: `pgrdf.spike_ta11_batch_sweep(triple_count INT DEFAULT 100000,
///       batch_size INT DEFAULT 1000) -> JSONB`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn spike_ta11_batch_sweep(
    triple_count: default!(i32, 100000),
    batch_size: default!(i32, 1000),
) -> pgrx::JsonB {
    let triple_count = triple_count.max(0) as usize;
    let batch_size = batch_size.max(1) as usize;

    // Setup: drop + recreate the target table so each run starts
    // clean. UNLOGGED so WAL cost doesn't dominate the measurement
    // (we're profiling the INSERT path, not durability).
    Spi::run(TA11_SETUP_SQL).expect("spike_ta11_batch_sweep: setup failed");

    let mut batch_s: Vec<i64> = Vec::with_capacity(batch_size);
    let mut batch_p: Vec<i64> = Vec::with_capacity(batch_size);
    let mut batch_o: Vec<i64> = Vec::with_capacity(batch_size);

    let start = Instant::now();
    let mut batches_flushed: i64 = 0;
    let mut spi_ns: u128 = 0;

    for i in 0..triple_count {
        // Synthetic triple values — sequential bigints. The point is
        // not realism; it's measuring the INSERT path under known
        // bounded values that don't trigger dict/parsing cost.
        batch_s.push(i as i64);
        batch_p.push((i + 1) as i64);
        batch_o.push((i + 2) as i64);

        if batch_s.len() >= batch_size {
            let t = Instant::now();
            flush_one(&mut batch_s, &mut batch_p, &mut batch_o);
            spi_ns += t.elapsed().as_nanos();
            batches_flushed += 1;
        }
    }
    if !batch_s.is_empty() {
        let t = Instant::now();
        flush_one(&mut batch_s, &mut batch_p, &mut batch_o);
        spi_ns += t.elapsed().as_nanos();
        batches_flushed += 1;
    }
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    let spi_ms = spi_ns as f64 / 1_000_000.0;

    pgrx::JsonB(json!({
        "path":             "unnest_baseline_flat_target",
        "triple_count":     triple_count as i64,
        "batch_size":       batch_size as i64,
        "batches_flushed":  batches_flushed,
        "elapsed_ms":       elapsed_ms,
        "spi_ms":           spi_ms,
        "per_triple_us":    (elapsed_ms * 1000.0) / (triple_count.max(1) as f64),
        "per_batch_us":     if batches_flushed > 0 {
            (spi_ms * 1000.0) / (batches_flushed as f64)
        } else {
            0.0
        },
    }))
}

fn flush_one(s: &mut Vec<i64>, p: &mut Vec<i64>, o: &mut Vec<i64>) {
    let s_arr: Vec<i64> = std::mem::take(s);
    let p_arr: Vec<i64> = std::mem::take(p);
    let o_arr: Vec<i64> = std::mem::take(o);
    // graph_id is constant for the spike (only the BULK INSERT
    // mechanic is being measured, not partition routing).
    let g: i64 = 1;
    Spi::run_with_args(
        TA11_INSERT_SQL,
        &[s_arr.into(), p_arr.into(), o_arr.into(), g.into()],
    )
    .expect("spike_ta11_batch_sweep: insert failed");
}
