//! Dictionary CRUD.
//!
//! Phase 2.0: SPI-backed put / get with at-write dedup.
//! Phase 3 step 1: every `put_term_full` first consults the process-
//! wide shmem cache (LLD §4.1). On hit it returns immediately, never
//! touching the table. On miss it falls through to SELECT (warm
//! shmem with the committed id), then INSERT (stage the new id for
//! shmem publication on commit).

use crate::storage::shmem_cache;
use pgrx::prelude::*;

/// Term-type discriminator. Matches `_pgrdf_dictionary.term_type` (SMALLINT)
/// in [`sql/schema_v0_2_0.sql`].
pub mod term_type {
    pub const URI: i16 = 1;
    pub const BLANK_NODE: i16 = 2;
    pub const LITERAL: i16 = 3;
}

/// Internal put with the full dictionary key (value, type, datatype id,
/// language tag). `IS NOT DISTINCT FROM` is used on the lookup so NULL
/// rows participate in dedup (Postgres default `UNIQUE` treats NULL as
/// always-distinct, which would otherwise leak duplicates of untyped
/// literals — at minor cost to lookup-by-index latency).
///
/// Called by both the public `put_term` and the Turtle loader. Not a
/// pg_extern in its own right; SQL surface goes through `put_term`.
pub(crate) fn put_term_full(
    value: &str,
    term_type: i16,
    datatype_id: Option<i64>,
    language: Option<&str>,
) -> i64 {
    // Phase 3 step 1: shmem cache hit avoids both SELECT and INSERT.
    if let Some(id) = shmem_cache::lookup(term_type, value, datatype_id, language) {
        return id;
    }
    let existing: Option<i64> = Spi::get_one_with_args(
        "SELECT (
            SELECT id FROM pgrdf._pgrdf_dictionary
             WHERE term_type = $1
               AND lexical_value = $2
               AND datatype_iri_id IS NOT DISTINCT FROM $3
               AND language_tag    IS NOT DISTINCT FROM $4
             LIMIT 1)",
        &[
            term_type.into(),
            value.into(),
            datatype_id.into(),
            language.into(),
        ],
    )
    .expect("put_term_full: select failed");
    if let Some(id) = existing {
        // SELECT hit. Stage rather than publish-immediately: in a
        // write transaction the row we just found may have been
        // INSERTed by THIS txn and could still be rolled back. The
        // commit-deferred publish keeps shmem in lockstep with the
        // dictionary table.
        shmem_cache::stage_for_commit(term_type, value, datatype_id, language, id);
        return id;
    }
    let id: i64 = Spi::get_one_with_args(
        "INSERT INTO pgrdf._pgrdf_dictionary
            (term_type, lexical_value, datatype_iri_id, language_tag)
         VALUES ($1, $2, $3, $4) RETURNING id",
        &[
            term_type.into(),
            value.into(),
            datatype_id.into(),
            language.into(),
        ],
    )
    .expect("put_term_full: insert failed")
    .expect("put_term_full: INSERT … RETURNING returned no row");
    // INSERT path: row is still in-flight. Stage the mapping for
    // shmem publication on COMMIT — on ABORT it is silently dropped
    // so we never publish ids that don't survive in the table.
    shmem_cache::stage_for_commit(term_type, value, datatype_id, language, id);
    id
}

/// Insert a simple term (no datatype, no language tag) and return its
/// ID. If the (term_type, lexical_value) pair already exists with NULL
/// datatype + language, returns the existing ID without inserting.
///
/// SQL surface: `pgrdf.put_term(value TEXT, term_type SMALLINT) → BIGINT`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn put_term(value: &str, term_type: i16) -> i64 {
    put_term_full(value, term_type, None, None)
}

/// Batch variant: resolve N terms in **two SPI calls** instead of N
/// independent shmem-cache-miss → SELECT → INSERT trips. Returns ids
/// in INPUT ORDER (positionally matched to `terms`).
///
/// **Why this exists — TA-D3 spike (v0.5.27+)**. Phase-0 profiling
/// against LUBM-1 (`_WIP/SPIKE.TRACK-A.phase0-findings.md`) showed
/// dict resolution is 73% of total ingest time — 26,473 distinct
/// `put_term_full` calls × ~42 µs each. The per-term SPI roundtrip
/// dominates. Replacing N roundtrips with 2 per batch should yield
/// 5-50× speedup on the dict phase (rough estimate; measured by the
/// caller after each spike batch).
///
/// Algorithm:
///
/// 1. **Bulk-insert** any missing rows:
///    `INSERT INTO _pgrdf_dictionary
///       SELECT * FROM unnest(...) ON CONFLICT DO NOTHING`
///    — silent on duplicates; the existing rows are NOT updated and
///    DO NOT return.
/// 2. **Bulk-lookup** all ids by joining the input arrays back
///    against the dictionary on the four-column UNIQUE.
///
/// `IS NOT DISTINCT FROM` is used in the join so NULL datatype /
/// language tags match correctly (matching `put_term_full`'s
/// single-term behavior).
///
/// **Shmem cache integration**: This batch path bypasses the per-
/// backend shmem cache on the read side (would require N cache
/// lookups defeating the batching). On the write side it ALSO
/// skips the per-row `stage_for_commit` for now — the spike is
/// validating "does dict batching help?", not "does shmem still
/// warm correctly?". TA-D2 spike covers shmem behavior.
///
/// Returns `Vec<i64>` with `result[i]` being the resolved id for
/// `terms[i]`. Panics on SPI failure or on missing post-insert
/// lookup (which would indicate a UNIQUE constraint mismatch — a
/// bug, not user-recoverable).
pub(crate) fn put_terms_batch(terms: &[(i16, String, Option<i64>, Option<String>)]) -> Vec<i64> {
    if terms.is_empty() {
        return Vec::new();
    }
    // Build the four parallel arrays for unnest. Order MUST match
    // across all four; we reconstruct (term_type, lexical_value,
    // datatype, language) on the SQL side via positional unnest.
    let term_types: Vec<i16> = terms.iter().map(|t| t.0).collect();
    let lexicals: Vec<String> = terms.iter().map(|t| t.1.clone()).collect();
    let datatypes: Vec<Option<i64>> = terms.iter().map(|t| t.2).collect();
    let languages: Vec<Option<String>> = terms.iter().map(|t| t.3.clone()).collect();

    // Step 1: bulk insert MISSING rows only.
    //
    // CAREFUL — the natural form
    //   `... ON CONFLICT (...) DO NOTHING`
    // does NOT work here. PostgreSQL's UNIQUE constraint defaults to
    // `NULLS DISTINCT` semantics: a row with `(URI, 'ex:p', NULL,
    // NULL)` does NOT conflict with another `(URI, 'ex:p', NULL,
    // NULL)` because the NULLs aren't equal. That makes ON CONFLICT
    // silently INSERT duplicate URI / blank-node rows whose
    // `datatype_iri_id` and `language_tag` are both NULL. The
    // subsequent JOIN-back (which uses `IS NOT DISTINCT FROM`) then
    // matches multiple dict rows for one input position and the
    // result Vec's per-position slot races between them — caller
    // sees an unstable id, breaks every downstream query against
    // the affected predicates.
    //
    // (TA-D3 spike v0.5.27 shipped with the ON CONFLICT form. Its
    // own parity gate didn't catch this — that test compares
    // BASELINE vs SPIKE in adjacent graphs but the duplicate rows
    // only show up when a SECOND batched ingest call reuses a term
    // from a prior batched call in the same backend. TA-7's
    // multi-graph regression 130 catches it.)
    //
    // Workaround at the SQL level: replace the ON CONFLICT clause
    // with a `WHERE NOT EXISTS` anti-join that uses `IS NOT DISTINCT
    // FROM` for the NULL columns. This matches `put_term_full`'s
    // pre-INSERT SELECT exactly. Concurrent backends could still
    // race (two parallel ingests of the same NULL-containing term
    // would each pass NOT EXISTS and each INSERT) — that's filed as
    // a TA-NEW-W follow-up: a schema migration to
    // `UNIQUE NULLS NOT DISTINCT` (PG 15+), which lets ON CONFLICT
    // resume working and is race-safe. **v0.5.39 ATTEMPTED THE
    // MIGRATION** but found the NULLS NOT DISTINCT form introduces
    // PG-level deadlocks under pgrx parallel test execution (see
    // `_WIP/DECISION.TA-NEW-W.nulls-not-distinct-deferred.md` for
    // the incident analysis); shipping it requires a retry-loop on
    // `ERRCODE_T_R_DEADLOCK_DETECTED` (SQLSTATE 40P01) which is
    // designed for v0.7+. Until then the `WHERE NOT EXISTS` form
    // stays.
    Spi::run_with_args(
        "INSERT INTO pgrdf._pgrdf_dictionary
             (term_type, lexical_value, datatype_iri_id, language_tag)
         SELECT t.tt, t.lv, t.di, t.lt
         FROM unnest($1::int2[], $2::text[], $3::int8[], $4::text[])
              AS t(tt, lv, di, lt)
         WHERE NOT EXISTS (
             SELECT 1 FROM pgrdf._pgrdf_dictionary d
              WHERE d.term_type = t.tt
                AND d.lexical_value = t.lv
                AND d.datatype_iri_id IS NOT DISTINCT FROM t.di
                AND d.language_tag    IS NOT DISTINCT FROM t.lt
         )",
        &[
            term_types.clone().into(),
            lexicals.clone().into(),
            datatypes.clone().into(),
            languages.clone().into(),
        ],
    )
    .expect("put_terms_batch: insert failed");

    // Step 2: bulk lookup. Returns rows in JOIN order (input order
    // preserved by the WITH ORDINALITY pattern). The result is a
    // dense Vec<i64> with one id per input position.
    //
    // Use WITH ORDINALITY on the unnest so we can sort the result
    // back to input order regardless of how PG executes the join.
    let mut result: Vec<i64> = vec![0; terms.len()];
    let table = Spi::connect(|client| {
        client
            .select(
                "SELECT t.ord, d.id
                 FROM unnest($1::int2[], $2::text[], $3::int8[], $4::text[])
                      WITH ORDINALITY AS t(tt, lv, di, lt, ord)
                 JOIN pgrdf._pgrdf_dictionary d
                   ON d.term_type = t.tt
                  AND d.lexical_value = t.lv
                  AND d.datatype_iri_id IS NOT DISTINCT FROM t.di
                  AND d.language_tag    IS NOT DISTINCT FROM t.lt",
                None,
                &[
                    term_types.into(),
                    lexicals.into(),
                    datatypes.into(),
                    languages.into(),
                ],
            )
            .expect("put_terms_batch: lookup failed")
            .into_iter()
            .map(|row| {
                let ord: i64 = row.get(1).expect("ord").expect("ord NULL");
                let id: i64 = row.get(2).expect("id").expect("id NULL");
                (ord, id)
            })
            .collect::<Vec<_>>()
    });
    for (ord, id) in table {
        // ord is 1-based per WITH ORDINALITY; convert to 0-based.
        let idx = (ord - 1) as usize;
        if idx < result.len() {
            result[idx] = id;
        }
    }
    result
}

/// Reverse lookup: ID → lexical value. Returns NULL if the ID is not
/// present in the dictionary.
///
/// SQL surface: `pgrdf.get_term(id BIGINT) → TEXT`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn get_term(id: i64) -> Option<String> {
    Spi::get_one_with_args(
        "SELECT (SELECT lexical_value FROM pgrdf._pgrdf_dictionary WHERE id = $1)",
        &[id.into()],
    )
    .expect("get_term: select failed")
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn put_term_dedups() {
        let a = Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.put_term('http://example.com/a', 1::smallint)",
            &[],
        )
        .unwrap()
        .unwrap();
        let b = Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.put_term('http://example.com/a', 1::smallint)",
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(a, b);
    }

    #[pg_test]
    fn put_term_separates() {
        let a = Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.put_term('http://example.com/x', 1::smallint)",
            &[],
        )
        .unwrap()
        .unwrap();
        let b = Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.put_term('http://example.com/y', 1::smallint)",
            &[],
        )
        .unwrap()
        .unwrap();
        assert_ne!(a, b);
    }

    #[pg_test]
    fn term_roundtrip() {
        let id = Spi::get_one_with_args::<i64>("SELECT pgrdf.put_term('hello', 3::smallint)", &[])
            .unwrap()
            .unwrap();
        let back: Option<String> =
            Spi::get_one_with_args("SELECT pgrdf.get_term($1)", &[id.into()]).unwrap();
        assert_eq!(back.as_deref(), Some("hello"));
    }

    #[pg_test]
    fn get_term_missing() {
        let v: Option<String> =
            Spi::get_one_with_args("SELECT pgrdf.get_term($1)", &[i64::MAX.into()]).unwrap();
        assert!(v.is_none());
    }
}
