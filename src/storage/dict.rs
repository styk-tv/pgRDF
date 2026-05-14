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
#[pg_extern]
fn put_term(value: &str, term_type: i16) -> i64 {
    put_term_full(value, term_type, None, None)
}

/// Reverse lookup: ID → lexical value. Returns NULL if the ID is not
/// present in the dictionary.
///
/// SQL surface: `pgrdf.get_term(id BIGINT) → TEXT`.
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
