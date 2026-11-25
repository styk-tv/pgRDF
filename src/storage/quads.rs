//! The one place quads are written: a graph is a set of triples.
//!
//! No two rows of one graph share `(subject_id, predicate_id, object_id)`.
//! PostgreSQL enforces it: every graph partition carries a unique index on
//! that key (the default partition on `(graph_id, subject, predicate,
//! object)`, because it holds several graph ids). Every writer inserts
//! through the parent table with `ON CONFLICT DO NOTHING`, which honours
//! the partition's own unique index, keeps the privilege rule callers have
//! always had (a grant on the parent is enough), and drops a repeat whether
//! it comes from the same statement, an earlier batch or an earlier load.
//!
//! **Asserted beats inferred.** A triple a caller asserts may already be
//! stored as inferred. The insert then stores nothing, and the row is
//! turned asserted instead: left inferred, the next `materialize` would
//! delete it with the other inferred rows and the assertion would be lost.
//!
//! **A graph that already holds repeats** (loaded before 0.6.41) has no
//! unique index, because one cannot be built over repeats and an upgrade
//! never rewrites data. Writes to such a graph check for the row
//! themselves, so they add no further repeats; `duplicate_report()` lists
//! these graphs and `dedup_graph()` cures one.

use crate::query::plan_cache;
use pgrx::datum::DatumWithOid;
use pgrx::pg_sys::{Oid, PgBuiltInOids};
use pgrx::prelude::*;

// The index builder, shared by graph creation's siblings: the bulk and
// staged loaders (rebuild after a load that dropped the index), the upgrade
// (try it on every existing partition) and dedup_graph (cure, then build).
//
// Returns 0 when the unique index was built over clean rows, the number of
// surplus rows removed when `dedup` is true and repeats were found, and -1
// when repeats were found and `dedup` is false: the partition then gets a
// plain index on the same key, so queries keep their plan, and stays
// without the constraint. Of each group of repeats the asserted row is
// kept over an inferred one. The failed unique build runs in the
// function's own subtransaction, so it leaves nothing behind.
pgrx::extension_sql!(
    r#"
CREATE FUNCTION _set_index(part regclass, dedup boolean) RETURNS bigint
LANGUAGE plpgsql AS $fn$
DECLARE
  rel  text := (SELECT c.relname FROM pg_catalog.pg_class c WHERE c.oid = part);
  cols text := CASE WHEN rel = '_pgrdf_quads_default'
                    THEN 'graph_id, subject_id, predicate_id, object_id'
                    ELSE 'subject_id, predicate_id, object_id' END;
  removed bigint := 0;
BEGIN
  -- Dropped only where present, so a caller sees no "does not exist,
  -- skipping" notice for the one that is not there.
  IF pg_catalog.to_regclass(pg_catalog.format('pgrdf.%I', rel || '_spo_uniq')) IS NOT NULL THEN
    EXECUTE pg_catalog.format('DROP INDEX pgrdf.%I', rel || '_spo_uniq');
  END IF;
  IF pg_catalog.to_regclass(pg_catalog.format('pgrdf.%I', rel || '_spo_idx')) IS NOT NULL THEN
    EXECUTE pg_catalog.format('DROP INDEX pgrdf.%I', rel || '_spo_idx');
  END IF;
  BEGIN
    EXECUTE pg_catalog.format('CREATE UNIQUE INDEX %I ON pgrdf.%I (%s) INCLUDE (is_inferred)',
                              rel || '_spo_uniq', rel, cols);
    RETURN 0;
  EXCEPTION WHEN unique_violation THEN
    IF NOT dedup THEN
      EXECUTE pg_catalog.format('CREATE INDEX %I ON pgrdf.%I (%s) INCLUDE (is_inferred)',
                                rel || '_spo_idx', rel, cols);
      RETURN -1;
    END IF;
  END;
  EXECUTE pg_catalog.format(
    'WITH gone AS (
       DELETE FROM pgrdf.%1$I q
        USING (SELECT ctid,
                      pg_catalog.row_number() OVER (PARTITION BY %2$s ORDER BY is_inferred, ctid) AS rn
                 FROM pgrdf.%1$I) r
        WHERE q.ctid = r.ctid AND r.rn > 1
       RETURNING 1)
     SELECT pg_catalog.count(*) FROM gone', rel, cols)
    INTO removed;
  EXECUTE pg_catalog.format('CREATE UNIQUE INDEX %I ON pgrdf.%I (%s) INCLUDE (is_inferred)',
                            rel || '_spo_uniq', rel, cols);
  RETURN removed;
END
$fn$;

-- The default partition is created by the install script, so its index is
-- created here; a graph partition gets its own when it is created.
SELECT _set_index('_pgrdf_quads_default'::regclass, false);
"#,
    name = "set_index_v0_6_41",
    requires = ["schema_v0_2_0"],
);

/// The relation that stores `graph_id`: its own partition when it has
/// one, else the default partition. Unqualified name.
pub(crate) fn partition_of(graph_id: i64) -> String {
    let name = format!("_pgrdf_quads_g{graph_id}");
    let own = Spi::get_one_with_args::<bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_class \
                        WHERE relnamespace = 'pgrdf'::regnamespace AND relname = $1)",
        &[name.as_str().into()],
    )
    .expect("partition lookup failed")
    .unwrap_or(false);
    if own {
        name
    } else {
        "_pgrdf_quads_default".to_string()
    }
}

/// Does the relation that stores `graph_id` carry the unique index, i.e.
/// does PostgreSQL enforce set semantics for it?
pub(crate) fn set_enforced(graph_id: i64) -> bool {
    let part = partition_of(graph_id);
    Spi::get_one_with_args::<bool>(
        "SELECT EXISTS(SELECT 1 FROM pg_catalog.pg_index i \
                         JOIN pg_catalog.pg_class c ON c.oid = i.indexrelid \
                        WHERE i.indrelid = pg_catalog.format('pgrdf.%I', $1::text)::regclass \
                          AND i.indisunique AND i.indisvalid \
                          AND c.relname = $1 || '_spo_uniq')",
        &[part.as_str().into()],
    )
    .expect("set-index lookup failed")
    .unwrap_or(false)
}

/// Build the unique index on `graph_id`'s partition; see `_set_index`.
/// The caller decides who runs it (index DDL needs the table's owner).
pub(crate) fn build_set_index(graph_id: i64, dedup: bool) -> i64 {
    let part = partition_of(graph_id);
    // The dedup inside deletes rows of the partition: an engine write.
    crate::storage::counts::engine_write(|| {
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf._set_index(pg_catalog.format('pgrdf.%I', $1::text)::regclass, $2)",
            &[part.as_str().into(), dedup.into()],
        )
    })
    .expect("building the set index failed")
    .unwrap_or(0)
}

/// What one write stored.
#[derive(Default, Clone, Copy)]
pub(crate) struct Written {
    /// New rows.
    pub stored: i64,
    /// Rows that were inferred and are asserted now.
    pub made_asserted: i64,
}

const INSERT_ENFORCED: &str = "WITH ins AS ( \
       INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred) \
       SELECT s, p, o, $4, $5 \
         FROM unnest($1::bigint[], $2::bigint[], $3::bigint[]) AS t(s, p, o) \
       ON CONFLICT DO NOTHING \
       RETURNING 1) \
     SELECT count(*)::bigint FROM ins";

const INSERT_UNENFORCED: &str = "WITH ins AS ( \
       INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred) \
       SELECT DISTINCT s, p, o, $4, $5 \
         FROM unnest($1::bigint[], $2::bigint[], $3::bigint[]) AS t(s, p, o) \
        WHERE NOT EXISTS ( \
              SELECT 1 FROM pgrdf._pgrdf_quads q \
               WHERE q.graph_id = $4 AND q.subject_id = t.s \
                 AND q.predicate_id = t.p AND q.object_id = t.o) \
       RETURNING 1) \
     SELECT count(*)::bigint FROM ins";

/// No conflict handling at all: for a load into a partition whose unique
/// index was dropped for the duration and is rebuilt (with a dedup, if the
/// build finds repeats) when the load ends.
const INSERT_PLAIN: &str = "WITH ins AS ( \
       INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred) \
       SELECT s, p, o, $4, $5 \
         FROM unnest($1::bigint[], $2::bigint[], $3::bigint[]) AS t(s, p, o) \
       RETURNING 1) \
     SELECT count(*)::bigint FROM ins";

const PROMOTE: &str = "WITH up AS ( \
       UPDATE pgrdf._pgrdf_quads q SET is_inferred = false \
         FROM (SELECT DISTINCT s, p, o \
                 FROM unnest($1::bigint[], $2::bigint[], $3::bigint[]) AS t(s, p, o)) b \
        WHERE q.graph_id = $4 AND q.is_inferred \
          AND q.subject_id = b.s AND q.predicate_id = b.p AND q.object_id = b.o \
       RETURNING 1) \
     SELECT pg_catalog.count(*)::bigint FROM up";

/// How a batch is written.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// The partition carries the unique index.
    Enforced,
    /// The partition holds repeats and has no unique index.
    Unenforced,
    /// The partition's unique index is dropped for a bulk load.
    Plain,
}

impl Mode {
    /// The mode for an ordinary write to `graph_id`.
    pub(crate) fn of(graph_id: i64) -> Self {
        if set_enforced(graph_id) {
            Mode::Enforced
        } else {
            Mode::Unenforced
        }
    }
}

/// Run one of the batch statements through the per-backend plan cache
/// (prepared once per backend, executed for every batch of every load).
fn run_batch(
    sql: &'static str,
    s: Vec<i64>,
    p: Vec<i64>,
    o: Vec<i64>,
    graph_id: i64,
    inferred: Option<bool>,
) -> i64 {
    let int8: Oid = PgBuiltInOids::INT8OID.into();
    let int8_array: Oid = PgBuiltInOids::INT8ARRAYOID.into();
    let boolean: Oid = PgBuiltInOids::BOOLOID.into();
    Spi::connect_mut(|client| {
        if !plan_cache::contains(sql) {
            let mut arg_oids = vec![
                PgOid::BuiltIn(PgBuiltInOids::INT8ARRAYOID),
                PgOid::BuiltIn(PgBuiltInOids::INT8ARRAYOID),
                PgOid::BuiltIn(PgBuiltInOids::INT8ARRAYOID),
                PgOid::BuiltIn(PgBuiltInOids::INT8OID),
            ];
            if inferred.is_some() {
                arg_oids.push(PgOid::BuiltIn(PgBuiltInOids::BOOLOID));
            }
            let prepared = client
                .prepare_mut(sql, &arg_oids)
                .expect("quad write: prepare failed")
                .keep();
            plan_cache::insert(sql.to_string(), prepared);
            plan_cache::record_miss();
        } else {
            plan_cache::record_hit();
        }
        // SAFETY: each (value, oid) pair matches by construction.
        let mut datums: Vec<DatumWithOid<'_>> = unsafe {
            vec![
                DatumWithOid::new(s, int8_array),
                DatumWithOid::new(p, int8_array),
                DatumWithOid::new(o, int8_array),
                DatumWithOid::new(graph_id, int8),
            ]
        };
        if let Some(inferred) = inferred {
            datums.push(unsafe { DatumWithOid::new(inferred, boolean) });
        }
        // The engine is writing: the count bypass marker stands down.
        let _writing = crate::storage::counts::Writing::begin();
        plan_cache::with_plan(sql, |plan| {
            let plan = plan.expect("quad write: plan must be cached after insert");
            client
                .update(plan, None, &datums)
                .expect("quad write failed")
                .first()
                .get_one::<i64>()
                .expect("quad write: count column")
                .unwrap_or(0)
        })
    })
}

/// Store a batch of asserted triples in `graph_id`. Repeats are dropped,
/// and rows that were only inferred become asserted.
pub(crate) fn insert_batch(
    graph_id: i64,
    mode: Mode,
    s: Vec<i64>,
    p: Vec<i64>,
    o: Vec<i64>,
) -> Written {
    let n = s.len() as i64;
    if n == 0 {
        return Written::default();
    }
    if mode == Mode::Plain {
        let stored = run_batch(INSERT_PLAIN, s, p, o, graph_id, Some(false));
        crate::storage::counts::gained(graph_id, stored, 0);
        return Written {
            stored,
            made_asserted: 0,
        };
    }
    let sql = match mode {
        Mode::Enforced => INSERT_ENFORCED,
        _ => INSERT_UNENFORCED,
    };
    let stored = run_batch(sql, s.clone(), p.clone(), o.clone(), graph_id, Some(false));
    // Every triple found its place: nothing can be waiting to be made
    // asserted. Otherwise some met an existing row, and any of those may
    // be an inferred one.
    let made_asserted = if stored < n {
        promote(graph_id, s, p, o)
    } else {
        0
    };
    // A promoted row leaves the inferred count and joins the asserted one.
    crate::storage::counts::gained(graph_id, stored + made_asserted, -made_asserted);
    Written {
        stored,
        made_asserted,
    }
}

/// Turn the listed triples asserted where they are stored as inferred.
/// As the storage owner: asserting a triple is authorised by INSERT, and
/// this fixed statement changes nothing but that flag on those rows.
fn promote(graph_id: i64, s: Vec<i64>, p: Vec<i64>, o: Vec<i64>) -> i64 {
    crate::storage::partition::as_storage_owner(|| run_batch(PROMOTE, s, p, o, graph_id, None))
}

/// Store a batch of inferred triples. An inferred triple never replaces
/// or changes a stored one.
pub(crate) fn insert_inferred_batch(
    graph_id: i64,
    mode: Mode,
    s: Vec<i64>,
    p: Vec<i64>,
    o: Vec<i64>,
) -> i64 {
    if s.is_empty() {
        return 0;
    }
    let sql = match mode {
        Mode::Enforced => INSERT_ENFORCED,
        Mode::Unenforced => INSERT_UNENFORCED,
        Mode::Plain => INSERT_PLAIN,
    };
    let stored = run_batch(sql, s, p, o, graph_id, Some(true));
    crate::storage::counts::gained(graph_id, 0, stored);
    stored
}

const ROW_INSERT: &str = "WITH ins AS ( \
       INSERT INTO pgrdf._pgrdf_quads \
             (subject_id, predicate_id, object_id, graph_id, is_inferred) \
       SELECT $1, $2, $3, $4, false \
        WHERE NOT EXISTS ( \
              SELECT 1 FROM pgrdf._pgrdf_quads \
               WHERE subject_id = $1 AND predicate_id = $2 \
                 AND object_id = $3 AND graph_id = $4) \
       ON CONFLICT DO NOTHING \
       RETURNING 1) \
     SELECT count(*)::bigint FROM ins";

/// [`ROW_INSERT`] with the graph's count delta in the same statement: a
/// single-row write outside any act pays for one statement, not two.
const ROW_INSERT_COUNTED: &str = "WITH ins AS ( \
       INSERT INTO pgrdf._pgrdf_quads \
             (subject_id, predicate_id, object_id, graph_id, is_inferred) \
       SELECT $1, $2, $3, $4, false \
        WHERE NOT EXISTS ( \
              SELECT 1 FROM pgrdf._pgrdf_quads \
               WHERE subject_id = $1 AND predicate_id = $2 \
                 AND object_id = $3 AND graph_id = $4) \
       ON CONFLICT DO NOTHING \
       RETURNING 1), \
     n AS (SELECT count(*)::bigint AS c FROM ins), \
     d AS (INSERT INTO pgrdf._pgrdf_graph_count_deltas (graph_id, d_asserted, d_inferred) \
           SELECT $4, n.c, 0 FROM n WHERE n.c > 0) \
     SELECT c FROM n";

/// Run one of the single-row statements through the per-backend plan
/// cache, with the engine's writing mark up.
fn run_row(sql: &'static str, s: i64, p: i64, o: i64, graph_id: i64) -> i64 {
    let int8: Oid = PgBuiltInOids::INT8OID.into();
    Spi::connect_mut(|client| {
        if !plan_cache::contains(sql) {
            let prepared = client
                .prepare_mut(sql, &[PgOid::BuiltIn(PgBuiltInOids::INT8OID); 4])
                .expect("quad write: prepare failed")
                .keep();
            plan_cache::insert(sql.to_string(), prepared);
            plan_cache::record_miss();
        } else {
            plan_cache::record_hit();
        }
        // SAFETY: each (value, oid) pair matches by construction.
        let datums: Vec<DatumWithOid<'_>> = unsafe {
            vec![
                DatumWithOid::new(s, int8),
                DatumWithOid::new(p, int8),
                DatumWithOid::new(o, int8),
                DatumWithOid::new(graph_id, int8),
            ]
        };
        let _writing = crate::storage::counts::Writing::begin();
        plan_cache::with_plan(sql, |plan| {
            let plan = plan.expect("quad write: plan must be cached after insert");
            client
                .update(plan, None, &datums)
                .expect("quad write failed")
                .first()
                .get_one::<i64>()
                .expect("quad write: count column")
                .unwrap_or(0)
        })
    })
}

/// Store one asserted triple (SPARQL UPDATE, JSON ingest, `put_quad`).
/// The existence check makes it correct on a partition without the unique
/// index too; `ON CONFLICT` settles a race on one that has it.
pub(crate) fn insert_row(graph_id: i64, s: i64, p: i64, o: i64) -> Written {
    // Inside an act the call's tally gathers what was stored and one
    // delta is written when the act ends. Outside one (`put_quad`), the
    // delta rides in the row's own statement.
    // An id without a count (never registered: the row lands in the
    // default partition) records none.
    let own_delta = !crate::storage::counts::tally_open()
        && crate::storage::counts::present()
        && crate::storage::counts::counted(graph_id);
    let stored = run_row(
        if own_delta {
            ROW_INSERT_COUNTED
        } else {
            ROW_INSERT
        },
        s,
        p,
        o,
        graph_id,
    );
    let made_asserted = if stored == 0 {
        promote(graph_id, vec![s], vec![p], vec![o])
    } else {
        0
    };
    if own_delta {
        if stored > 0 {
            crate::storage::counts::maybe_fold(graph_id);
        }
        crate::storage::counts::gained(graph_id, made_asserted, -made_asserted);
    } else {
        crate::storage::counts::gained(graph_id, stored + made_asserted, -made_asserted);
    }
    Written {
        stored,
        made_asserted,
    }
}

/// Remove one triple from `graph_id` (SPARQL DELETE). Returns the rows
/// removed: none or one, or more on a partition that still holds repeats.
pub(crate) fn delete_row(graph_id: i64, s: i64, p: i64, o: i64) -> i64 {
    let (asserted, inferred) = crate::storage::counts::engine_write(|| {
        Spi::get_two_with_args::<i64, i64>(
            "WITH d AS ( \
               DELETE FROM pgrdf._pgrdf_quads \
                WHERE subject_id = $1 AND predicate_id = $2 \
                  AND object_id = $3 AND graph_id = $4 \
               RETURNING is_inferred) \
             SELECT count(*) FILTER (WHERE NOT is_inferred)::bigint, \
                    count(*) FILTER (WHERE is_inferred)::bigint FROM d",
            &[s.into(), p.into(), o.into(), graph_id.into()],
        )
    })
    .unwrap_or_else(|e| panic!("quad delete failed: {e}"));
    let (asserted, inferred) = (asserted.unwrap_or(0), inferred.unwrap_or(0));
    crate::storage::counts::gained(graph_id, -asserted, -inferred);
    asserted + inferred
}

/// Remove the rows of `graph_id` with a DELETE: all of them, or only the
/// inferred ones. Returns the rows removed.
pub(crate) fn delete_graph_rows(graph_id: i64, only_inferred: bool) -> i64 {
    let (asserted, inferred) = crate::storage::counts::engine_write(|| {
        Spi::get_two_with_args::<i64, i64>(
            "WITH d AS ( \
               DELETE FROM pgrdf._pgrdf_quads \
                WHERE graph_id = $1 AND (is_inferred OR NOT $2) \
               RETURNING is_inferred) \
             SELECT count(*) FILTER (WHERE NOT is_inferred)::bigint, \
                    count(*) FILTER (WHERE is_inferred)::bigint FROM d",
            &[graph_id.into(), only_inferred.into()],
        )
    })
    .unwrap_or_else(|e| panic!("graph delete failed: {e}"));
    let (asserted, inferred) = (asserted.unwrap_or(0), inferred.unwrap_or(0));
    crate::storage::counts::gained(graph_id, -asserted, -inferred);
    asserted + inferred
}

/// One row of [`duplicate_report`].
type ReportRow = (
    Option<i64>,
    Option<String>,
    i64,
    i64,
    i64,
    i64,
    i64,
    Option<bool>,
    Option<bool>,
    Option<String>,
    String,
);

/// Graphs whose partition does not carry the unique index, with what each
/// holds: `rows`, `distinct_rows`, `surplus` (rows a set would not have),
/// `groups` (triples stored more than once) and `max_multiplicity`. A
/// graph loaded before 0.6.41 from a source that repeated triples is in
/// this state; `dedup_graph(graph_id)` cures it. `note` says what to do.
/// A locked graph is cured in three acts: unlock, dedup, lock; its
/// recorded digest was computed over the repeats when
/// `locked_digest_engine` is NULL.
///
/// One more row, with `graph_id` NULL, appears when the dictionary holds
/// the same term under two ids (two loads that introduced the same new
/// term at the same moment): `surplus` then counts the extra ids. A triple
/// can sit in one graph twice under those two ids without being a repeat
/// by id.
///
/// Only partitions without the index are scanned, so on a healthy database
/// this reads the catalog and the dictionary and returns nothing: empty is
/// the answer.
#[search_path(pgrdf, pg_temp)]
#[pg_extern(stable)]
#[allow(clippy::type_complexity)]
fn duplicate_report() -> TableIterator<
    'static,
    (
        name!(graph_id, Option<i64>),
        name!(iri, Option<String>),
        name!(rows, i64),
        name!(distinct_rows, i64),
        name!(surplus, i64),
        name!(groups, i64),
        name!(max_multiplicity, i64),
        name!(set_enforced, Option<bool>),
        name!(locked, Option<bool>),
        name!(locked_digest_engine, Option<String>),
        name!(note, String),
    ),
> {
    let mut out: Vec<ReportRow> = Vec::new();
    // Partitions of the quad table without a valid unique set index.
    let parts: Vec<String> = Spi::connect(|c| {
        c.select(
            "SELECT c.relname::text \
               FROM pg_catalog.pg_inherits i JOIN pg_catalog.pg_class c ON c.oid = i.inhrelid \
              WHERE i.inhparent = 'pgrdf._pgrdf_quads'::regclass \
                AND NOT EXISTS (SELECT 1 FROM pg_catalog.pg_index x \
                                  JOIN pg_catalog.pg_class ic ON ic.oid = x.indexrelid \
                                 WHERE x.indrelid = c.oid AND x.indisunique AND x.indisvalid \
                                   AND ic.relname = c.relname || '_spo_uniq') \
              ORDER BY 1",
            None,
            &[],
        )
        .expect("duplicate_report: partition scan failed")
        .map(|r| r.get::<String>(1).unwrap().unwrap_or_default())
        .collect()
    });
    for part in parts {
        // Counted per graph id: the default partition holds several.
        let mut found: Vec<(i64, i64, i64, i64, i64)> = Spi::connect(|c| {
            c.select(
                &format!(
                    "SELECT graph_id, sum(m)::bigint, count(*)::bigint, \
                            (count(*) FILTER (WHERE m > 1))::bigint, max(m)::bigint \
                       FROM (SELECT graph_id, count(*) AS m FROM pgrdf.{part} \
                              GROUP BY graph_id, subject_id, predicate_id, object_id) g \
                      GROUP BY graph_id ORDER BY graph_id"
                ),
                None,
                &[],
            )
            .expect("duplicate_report: partition read failed")
            .map(|r| {
                (
                    r.get::<i64>(1).unwrap().unwrap_or(0),
                    r.get::<i64>(2).unwrap().unwrap_or(0),
                    r.get::<i64>(3).unwrap().unwrap_or(0),
                    r.get::<i64>(4).unwrap().unwrap_or(0),
                    r.get::<i64>(5).unwrap().unwrap_or(0),
                )
            })
            .collect()
        });
        // An empty partition without the index is still a finding.
        if found.is_empty()
            && let Some(id) = part
                .strip_prefix("_pgrdf_quads_g")
                .and_then(|n| n.parse::<i64>().ok())
        {
            found.push((id, 0, 0, 0, 0));
        }
        for (graph_id, rows, distinct, groups, max_m) in found {
            let (iri, locked, engine) = Spi::get_three_with_args::<String, bool, String>(
                "SELECT g.iri, g.locked, pg_catalog.to_jsonb(g) ->> 'locked_digest_engine' \
                   FROM pgrdf._pgrdf_graphs g WHERE g.graph_id = $1",
                &[graph_id.into()],
            )
            .unwrap_or((None, None, None));
            let surplus = rows - distinct;
            let note = if surplus == 0 {
                format!("no repeats; pgrdf.dedup_graph({graph_id}) builds the unique index")
            } else if locked == Some(true) && engine.is_none() {
                format!(
                    "locked, and its recorded digest was computed over the repeats: \
                     pgrdf.unlock_graph({graph_id}, '<reason>'), pgrdf.dedup_graph({graph_id}), \
                     then pgrdf.lock_graph({graph_id}, '<reason>') to record the digest of the set"
                )
            } else if locked == Some(true) {
                format!(
                    "locked: pgrdf.unlock_graph({graph_id}, '<reason>'), \
                     pgrdf.dedup_graph({graph_id}), then lock again"
                )
            } else {
                format!(
                    "pgrdf.dedup_graph({graph_id}) removes the {surplus} surplus row(s) and \
                     builds the unique index"
                )
            };
            out.push((
                Some(graph_id),
                iri,
                rows,
                distinct,
                surplus,
                groups,
                max_m,
                Some(false),
                locked,
                engine,
                note,
            ));
        }
    }
    // The same term under two dictionary ids.
    let dict = Spi::connect(|c| {
        let r = c
            .select(
                "SELECT coalesce(sum(m), 0)::bigint, count(*)::bigint, \
                        (count(*) FILTER (WHERE m > 1))::bigint, coalesce(max(m), 0)::bigint \
                   FROM (SELECT count(*) AS m FROM pgrdf._pgrdf_dictionary \
                          GROUP BY term_type, lexical_md5, datatype_iri_id, language_tag) d",
                None,
                &[],
            )
            .expect("duplicate_report: dictionary read failed")
            .first();
        (
            r.get::<i64>(1).unwrap().unwrap_or(0),
            r.get::<i64>(2).unwrap().unwrap_or(0),
            r.get::<i64>(3).unwrap().unwrap_or(0),
            r.get::<i64>(4).unwrap().unwrap_or(0),
        )
    });
    if dict.0 > dict.1 {
        out.push((
            None,
            None,
            dict.0,
            dict.1,
            dict.0 - dict.1,
            dict.2,
            dict.3,
            None,
            None,
            None,
            "the dictionary holds the same term under more than one id: a triple can then be \
             stored twice in one graph under different ids, which no index sees"
                .to_string(),
        ));
    }
    out.sort_by_key(|r| (r.0.is_none(), r.0));
    TableIterator::new(out.into_iter())
}

/// Remove the repeated rows of a graph and build its unique index. Returns
/// the number of surplus rows removed (0 for a graph that held none).
///
/// Of each group of repeats one row is kept, an asserted one before an
/// inferred one, so the graph's set of triples is unchanged; a current
/// materialization stays current. Refuses 55P03 on a locked graph (unlock,
/// dedup, lock). Needs what `clear_graph` needs: SELECT and DELETE on the
/// quad table. Recorded as a `dedup` act. Called for graph 0 it cures the
/// default partition, which also holds the rows of any graph id without a
/// partition of its own.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn dedup_graph(graph_id: i64) -> i64 {
    if graph_id < 0 {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
            format!("dedup_graph: graph_id must be >= 0, got {graph_id}"),
        );
    }
    let registered = graph_id == 0
        || Spi::get_one_with_args::<bool>(
            "SELECT EXISTS(SELECT 1 FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)",
            &[graph_id.into()],
        )
        .expect("dedup_graph: graph lookup failed")
        .unwrap_or(false);
    if !registered {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
            format!("dedup_graph: no graph with id {graph_id}"),
        );
    }
    crate::storage::lock::require_unlocked(graph_id, "dedup_graph");
    crate::storage::acts::require_graph_owner(graph_id, "dedup_graph");
    crate::storage::partition::require_graph_ddl_privilege(
        crate::storage::partition::GraphDdl::Clear,
        "dedup_graph",
    );
    if set_enforced(graph_id) {
        return 0; // the index is there: the graph is a set
    }
    crate::storage::partition::acquire_partition_ddl_gate();
    let act = crate::storage::acts::begin(graph_id, "dedup");
    let asserted = || -> i64 {
        Spi::get_one_with_args::<i64>(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads \
              WHERE graph_id = $1 AND NOT is_inferred",
            &[graph_id.into()],
        )
        .expect("dedup_graph: count failed")
        .unwrap_or(0)
    };
    let before = asserted();
    let removed =
        crate::storage::partition::as_storage_owner(|| build_set_index(graph_id, true)).max(0);
    let asserted_removed = if removed > 0 { before - asserted() } else { 0 };
    if removed > 0 {
        // 0.6.42: the dedup removed rows it does not split by side; it
        // read the whole partition already, so the count is set by scan.
        crate::storage::counts::set_by_scan(graph_id);
    }
    if asserted_removed > 0 {
        // The materialization was stamped with a row count; the set it was
        // computed over has not changed, so the stamp follows the count.
        crate::storage::partition::as_storage_owner(|| {
            Spi::run_with_args(
                "UPDATE pgrdf._pgrdf_graphs \
                    SET materialized_base_count = materialized_base_count - $2 \
                  WHERE graph_id = $1 AND materialized_base_count IS NOT NULL",
                &[graph_id.into(), asserted_removed.into()],
            )
            .expect("dedup_graph: materialization stamp update failed");
        });
    }
    act.finish(serde_json::json!({ "removed": removed, "asserted_removed": asserted_removed }));
    removed
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    fn one<T: pgrx::datum::FromDatum + pgrx::datum::IntoDatum>(q: &str) -> Option<T> {
        Spi::get_one::<T>(q).unwrap()
    }

    fn rows(g: i64) -> i64 {
        one(&format!(
            "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = {g}"
        ))
        .unwrap()
    }

    /// Groups of rows of one graph that share subject, predicate and object.
    fn repeated(g: i64) -> i64 {
        one(&format!(
            "SELECT count(*) FROM (SELECT 1 FROM pgrdf._pgrdf_quads WHERE graph_id = {g} \
               GROUP BY subject_id, predicate_id, object_id HAVING count(*) > 1) r"
        ))
        .unwrap()
    }

    /// Run `q` in a subtransaction: 'ok' or its SQLSTATE.
    fn try_fn() {
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.q_try(q text) RETURNS text LANGUAGE plpgsql AS $$ \
             BEGIN EXECUTE q; RETURN 'ok'; EXCEPTION WHEN OTHERS THEN RETURN SQLSTATE; END $$",
        )
        .unwrap();
    }

    fn attempt(q: &str) -> String {
        Spi::get_one_with_args::<String>("SELECT pg_temp.q_try($1)", &[q.into()])
            .unwrap()
            .unwrap()
    }

    /// Put a graph in the state a pre-0.6.41 load could leave: repeated
    /// rows, and therefore no unique index. The ONE place tests create
    /// repeats.
    pub(crate) fn make_graph_hold_repeats(g: i64) {
        let part = format!("_pgrdf_quads_g{g}");
        Spi::run(&format!("DROP INDEX pgrdf.{part}_spo_uniq")).unwrap();
        Spi::run(&format!(
            "INSERT INTO pgrdf.{part} (subject_id, predicate_id, object_id, graph_id, is_inferred) \
             SELECT subject_id, predicate_id, object_id, graph_id, is_inferred FROM pgrdf.{part}"
        ))
        .unwrap();
        Spi::run(&format!(
            "CREATE INDEX {part}_spo_idx ON pgrdf.{part} (subject_id, predicate_id, object_id) \
             INCLUDE (is_inferred)"
        ))
        .unwrap();
    }

    /// Every way of putting triples into a graph stores a set: a repeat
    /// inside one call, and the same call made twice, add nothing.
    #[pg_test]
    fn every_write_route_stores_a_set() {
        // Three statements, two distinct triples.
        let nt = "<urn:s1:a> <urn:s1:p> \"v\" .\n<urn:s1:a> <urn:s1:p> \"v\" .\n\
                  <urn:s1:a> <urn:s1:q> <urn:s1:b> .\n";
        let path = format!("/tmp/pgrdf-s1-{}.nt", std::process::id());
        std::fs::write(&path, nt).unwrap();
        let row = |o: &str| {
            format!(
                "'{{\"subject\": {{\"type\": \"iri\", \"value\": \"urn:s1:a\"}}, \
                   \"predicate\": {{\"type\": \"iri\", \"value\": \"urn:s1:p\"}}, \
                   \"object\": {{\"type\": \"literal\", \"value\": \"{o}\"}}}}'::jsonb"
            )
        };
        let routes: Vec<(&str, Box<dyn Fn(i64, &str) -> String>)> = vec![
            (
                "parse_turtle",
                Box::new(|g, _| format!("SELECT pgrdf.parse_turtle($s1${nt}$s1$, {g})")),
            ),
            (
                "parse_turtle_verbose",
                Box::new(|g, _| format!("SELECT pgrdf.parse_turtle_verbose($s1${nt}$s1$, {g})")),
            ),
            (
                "parse_turtle_dict_batched",
                Box::new(|g, _| {
                    format!("SELECT pgrdf.parse_turtle_dict_batched($s1${nt}$s1$, {g})")
                }),
            ),
            (
                "parse_nquads",
                Box::new(|g, _| format!("SELECT pgrdf.parse_nquads($s1${nt}$s1$, {g})")),
            ),
            (
                "parse_trig",
                Box::new(|_, iri| format!("SELECT pgrdf.parse_trig($s1$<{iri}> {{ {nt} }}$s1$)")),
            ),
            (
                "load_turtle",
                Box::new(|g, _| {
                    format!("SELECT pgrdf.load_turtle('{path}', {g}, 'http://base.example/')")
                }),
            ),
            (
                "load_turtle bulk",
                Box::new(|g, _| format!("SELECT pgrdf.load_turtle('{path}', {g}, NULL, true)")),
            ),
            (
                "load_turtle_dict_batched",
                Box::new(|g, _| format!("SELECT pgrdf.load_turtle_dict_batched('{path}', {g})")),
            ),
            (
                "put_construct_rows",
                Box::new(|g, _| {
                    format!(
                        "SELECT pgrdf.put_construct_rows(ARRAY[{}, {}, {}], {g})",
                        row("v"),
                        row("v"),
                        row("w")
                    )
                }),
            ),
            (
                "sparql INSERT DATA",
                Box::new(|_, iri| {
                    format!(
                        "SELECT * FROM pgrdf.sparql('INSERT DATA {{ GRAPH <{iri}> {{ \
                         <urn:s1:a> <urn:s1:p> \"v\" . <urn:s1:a> <urn:s1:p> \"v\" . \
                         <urn:s1:a> <urn:s1:q> <urn:s1:b> }} }}')"
                    )
                }),
            ),
        ];
        let mut wrong = Vec::new();
        for (i, (name, sql)) in routes.iter().enumerate() {
            let iri = format!("urn:tdd:s1:route{i}");
            let g: i64 = one(&format!("SELECT pgrdf.add_graph('{iri}')")).unwrap();
            for round in 1..=2 {
                Spi::run(&sql(g, &iri)).unwrap();
                if rows(g) != 2 || repeated(g) != 0 {
                    wrong.push(format!(
                        "{name}, call {round}: {} rows, {} repeated",
                        rows(g),
                        repeated(g)
                    ));
                }
                // 0.6.42: and the stored count is verified and true.
                if let Err(e) = crate::storage::counts::testing::truth(g) {
                    wrong.push(format!("{name}, call {round}: {e}"));
                }
            }
        }
        let _ = std::fs::remove_file(&path);
        assert!(
            wrong.is_empty(),
            "routes that did not store a set: {wrong:#?}"
        );
    }

    /// PostgreSQL enforces it: a repeat written with plain SQL is refused,
    /// in a graph's own partition and in the default partition.
    #[pg_test]
    fn the_table_refuses_a_repeat() {
        try_fn();
        let g: i64 = one("SELECT pgrdf.add_graph('urn:tdd:s1:table')").unwrap();
        for target in [g, 0, 880_777 /* no partition: the default one */] {
            let ins = format!(
                "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id) \
                 VALUES (9001, 9002, 9003, {target})"
            );
            assert_eq!(attempt(&ins), "ok", "graph {target}: first row");
            assert_eq!(attempt(&ins), "23505", "graph {target}: the repeat");
        }
        assert!(crate::storage::quads::set_enforced(g));
        assert!(crate::storage::quads::set_enforced(0));
    }

    /// A triple that is asserted while it is stored as inferred becomes
    /// asserted. Left inferred, the next materialize would delete it.
    #[pg_test]
    fn an_asserted_triple_replaces_its_inferred_copy() {
        let sub = "<http://www.w3.org/2000/01/rdf-schema#subClassOf>";
        let ways: Vec<(&str, Box<dyn Fn(i64, &str) -> String>)> = vec![
            (
                "parse_turtle",
                Box::new(|g, _| {
                    format!("SELECT pgrdf.parse_turtle('<urn:pr:x> a <urn:pr:D> .', {g})")
                }),
            ),
            (
                "sparql",
                Box::new(|_, iri| {
                    format!(
                        "SELECT * FROM pgrdf.sparql('INSERT DATA {{ GRAPH <{iri}> {{ <urn:pr:x> a <urn:pr:D> }} }}')"
                    )
                }),
            ),
            (
                "put_construct_row",
                Box::new(|g, _| {
                    format!(
                        "SELECT pgrdf.put_construct_row('{{\"subject\": {{\"type\": \"iri\", \"value\": \"urn:pr:x\"}}, \
                         \"predicate\": {{\"type\": \"iri\", \"value\": \"http://www.w3.org/1999/02/22-rdf-syntax-ns#type\"}}, \
                         \"object\": {{\"type\": \"iri\", \"value\": \"urn:pr:D\"}}}}'::jsonb, {g})"
                    )
                }),
            ),
        ];
        for (i, (name, assert_it)) in ways.iter().enumerate() {
            let iri = format!("urn:tdd:s1:promote{i}");
            let g: i64 = one(&format!("SELECT pgrdf.add_graph('{iri}')")).unwrap();
            Spi::run(&format!(
                "SELECT pgrdf.parse_turtle('<urn:pr:x> a <urn:pr:C> . <urn:pr:C> {sub} <urn:pr:D> .', {g})"
            ))
            .unwrap();
            Spi::run(&format!("SELECT pgrdf.materialize({g}, 'rdfs')")).unwrap();
            let state = || -> String {
                one::<String>(&format!(
                    "SELECT coalesce(string_agg(q.is_inferred::text, ','), 'absent') \
                       FROM pgrdf._pgrdf_quads q \
                       JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id \
                       JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id \
                      WHERE q.graph_id = {g} AND s.lexical_value = 'urn:pr:x' \
                        AND o.lexical_value = 'urn:pr:D'"
                ))
                .unwrap()
            };
            assert_eq!(state(), "true", "{name}: x a D starts as inferred");
            Spi::run(&assert_it(g, &iri)).unwrap();
            assert_eq!(
                state(),
                "false",
                "{name}: asserting it makes the row asserted"
            );
            // The premise goes away; the asserted triple must stay.
            Spi::run(&format!(
                "SELECT * FROM pgrdf.sparql('DELETE DATA {{ GRAPH <{iri}> {{ <urn:pr:C> {sub} <urn:pr:D> }} }}')"
            ))
            .unwrap();
            Spi::run(&format!("SELECT pgrdf.materialize({g}, 'rdfs')")).unwrap();
            assert_eq!(state(), "false", "{name}: it survives the next materialize");
        }
    }

    /// A graph that already holds repeats gains no more from any write,
    /// is listed by duplicate_report(), and is cured by dedup_graph().
    #[pg_test]
    fn a_graph_holding_repeats_is_reported_and_cured() {
        try_fn();
        let ttl = "<urn:d:a> <urn:d:p> <urn:d:b> . <urn:d:a> <urn:d:q> \"v\" . \
                   <urn:d:a> a <urn:d:C> . <urn:d:C> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <urn:d:D> .";
        let g: i64 = one("SELECT pgrdf.add_graph('urn:tdd:s3:dirty')").unwrap();
        Spi::run(&format!("SELECT pgrdf.parse_turtle('{ttl}', {g})")).unwrap();
        let clean: i64 =
            one("SELECT count(*) FROM pgrdf.duplicate_report() WHERE iri = 'urn:tdd:s3:dirty'")
                .unwrap();
        assert_eq!(clean, 0, "a clean graph is not listed");
        // The state a pre-0.6.41 load left: every asserted row twice, and
        // a materialization computed over that.
        make_graph_hold_repeats(g);
        assert_eq!(rows(g), 8);
        Spi::run(&format!("SELECT pgrdf.materialize({g}, 'rdfs')")).unwrap();
        assert_eq!(
            rows(g),
            9,
            "eight asserted rows (four triples), one inferred"
        );
        let report = || -> Option<String> {
            // A scalar subquery: no row reads as NULL, not as an SPI error.
            one(&format!(
                "SELECT (SELECT concat_ws('|', rows, distinct_rows, surplus, groups, max_multiplicity, set_enforced) \
                   FROM pgrdf.duplicate_report() WHERE graph_id = {g})"
            ))
        };
        assert_eq!(report().as_deref(), Some("9|5|4|4|2|f"));
        // Writes check for the row themselves: nothing more is added.
        Spi::run(&format!("SELECT pgrdf.parse_turtle('{ttl}', {g})")).unwrap();
        Spi::run(&format!(
            "SELECT * FROM pgrdf.sparql('INSERT DATA {{ GRAPH <urn:tdd:s3:dirty> {{ <urn:d:a> <urn:d:p> <urn:d:b> }} }}')"
        ))
        .unwrap();
        assert_eq!(rows(g), 9, "a graph holding repeats gains no more");
        // A locked graph is cured in three acts: unlock, dedup, lock.
        Spi::run(&format!("SELECT pgrdf.lock_graph({g}, 'sealed')")).unwrap();
        assert_eq!(attempt(&format!("SELECT pgrdf.dedup_graph({g})")), "55P03");
        Spi::run(&format!("SELECT pgrdf.unlock_graph({g}, 'to cure')")).unwrap();
        let removed: i64 = one(&format!("SELECT pgrdf.dedup_graph({g})")).unwrap();
        assert_eq!(removed, 4);
        assert_eq!(rows(g), 5);
        assert_eq!(repeated(g), 0);
        assert!(
            crate::storage::quads::set_enforced(g),
            "the cure builds the unique index"
        );
        assert_eq!(report(), None, "a cured graph is not listed");
        let state: String = one(&format!(
            "SELECT concat_ws('|', asserted, inferred, materialization) \
               FROM pgrdf.graph_inventory() WHERE graph_id = {g}"
        ))
        .unwrap();
        assert_eq!(
            state, "4|1|current",
            "the set did not change, so the inferences still hold"
        );
        let act: String = one(&format!(
            "SELECT detail ->> 'removed' FROM pgrdf.graph_acts({g}) WHERE act = 'dedup'"
        ))
        .unwrap();
        assert_eq!(act, "4");
        // Curing a clean graph removes nothing.
        let again: i64 = one(&format!("SELECT pgrdf.dedup_graph({g})")).unwrap();
        assert_eq!(again, 0);
        assert_eq!(attempt("SELECT pgrdf.dedup_graph(880778)"), "42704");
    }

    /// copy_graph adds what the destination lacks and says how much that
    /// was; copying again adds nothing. Repeats in the source are not
    /// carried over.
    #[pg_test]
    fn copy_stores_a_set_and_reports_what_it_stored() {
        let src: i64 = one("SELECT pgrdf.add_graph('urn:tdd:s1:copy-src')").unwrap();
        let dst: i64 = one("SELECT pgrdf.add_graph('urn:tdd:s1:copy-dst')").unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:c:a> <urn:c:p> <urn:c:b> . <urn:c:a> <urn:c:p> <urn:c:c> .', {src})"
        ))
        .unwrap();
        let first: i64 = one(&format!("SELECT pgrdf.copy_graph({src}, {dst})")).unwrap();
        let second: i64 = one(&format!("SELECT pgrdf.copy_graph({src}, {dst})")).unwrap();
        assert_eq!((first, second), (2, 0));
        assert_eq!((rows(dst), repeated(dst)), (2, 0));
        make_graph_hold_repeats(src);
        let clean: i64 = one("SELECT pgrdf.add_graph('urn:tdd:s1:copy-clean')").unwrap();
        let n: i64 = one(&format!("SELECT pgrdf.copy_graph({src}, {clean})")).unwrap();
        assert_eq!((n, rows(clean), repeated(clean)), (2, 2, 0));
        let carved: i64 = one(&format!(
            "SELECT pgrdf.carve_graph({src}, 'urn:c:p', pgrdf.add_graph('urn:tdd:s1:carved'))"
        ))
        .unwrap();
        assert_eq!(carved, 2, "a carve stores the set too");
    }

    /// Every loader says what it read, what it stored and what it dropped.
    #[pg_test]
    fn loaders_report_read_stored_and_dropped() {
        let nt = "<urn:r:a> <urn:r:p> \"v\" .\n<urn:r:a> <urn:r:p> \"v\" .\n<urn:r:a> <urn:r:q> <urn:r:b> .\n";
        let g: i64 = one("SELECT pgrdf.add_graph('urn:tdd:s2:g')").unwrap();
        let load = |g: i64| -> String {
            one::<String>(&format!(
                "SELECT (pgrdf.parse_turtle_verbose($s2${nt}$s2$, {g}) -> 'load')::text"
            ))
            .unwrap()
        };
        let want = |read: i64, stored: i64, promoted: i64, dropped: i64, empty: bool| -> String {
            one::<String>(&format!(
                "SELECT jsonb_build_object('triples_read', {read}, 'parse_skipped', 0, \
                        'triples_stored', {stored}, 'inferred_made_asserted', {promoted}, \
                        'duplicates_dropped', {dropped}, 'graph_was_empty', {empty})::text"
            ))
            .unwrap()
        };
        assert_eq!(load(g), want(3, 2, 0, 1, true));
        // The same report, for callers on the same connection.
        let last: String = one("SELECT (pgrdf.last_call_stats() -> 'load')::text").unwrap();
        assert_eq!(last, want(3, 2, 0, 1, true));
        assert_eq!(load(g), want(3, 0, 0, 3, false), "a reload stores nothing");
        // And on the act row.
        let acts: i64 = one(&format!(
            "SELECT count(*) FROM pgrdf.graph_acts({g}) \
              WHERE act = 'load' AND detail -> 'load' ->> 'triples_read' = '3' \
                AND detail -> 'load' ->> 'triples_stored' IN ('2', '0')"
        ))
        .unwrap();
        assert_eq!(acts, 2);
        // Quad loaders report it too.
        let nq: String = one(&format!(
            "SELECT (pgrdf.parse_nquads($s2${nt}$s2$, {g}) -> 'load' ->> 'duplicates_dropped')"
        ))
        .unwrap();
        assert_eq!(nq, "3");
        // SPARQL reports what it stored, not what it was asked to store.
        let up: String = one(
            "SELECT string_agg(j -> '_update' ->> 'triples_inserted', ',') FROM pgrdf.sparql( \
               'INSERT DATA { GRAPH <urn:tdd:s2:g> { <urn:r:a> <urn:r:p> \"v\" . <urn:r:new> <urn:r:p> \"w\" } }') j",
        )
        .unwrap();
        assert_eq!(up, "1", "one of the two triples was already there");
        let act: String = one(&format!(
            "SELECT concat_ws('|', detail ->> 'inserted', detail ->> 'deleted') \
               FROM pgrdf.graph_acts({g}) WHERE act = 'update'"
        ))
        .unwrap();
        assert_eq!(act, "1|0");
    }
}
