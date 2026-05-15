//! Hexastore CRUD + partition management.
//!
//! Phase 2.0: writes go through SPI INSERTs. The bulk loader using
//! `COPY … FROM STDIN (FORMAT BINARY)` lands in Phase 2.1 per
//! [`docs/02-storage.md`].

use pgrx::prelude::*;

/// Insert one quad into the partitioned hexastore. The graph_id
/// argument routes the tuple to its named partition (created via
/// `pgrdf.add_graph(graph_id)`) or the default partition otherwise.
///
/// SQL surface:
/// `pgrdf.put_quad(s BIGINT, p BIGINT, o BIGINT, g BIGINT DEFAULT 0)`.
#[pg_extern]
fn put_quad(s: i64, p: i64, o: i64, g: default!(i64, 0)) {
    Spi::run_with_args(
        "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id)
         VALUES ($1, $2, $3, $4)",
        &[s.into(), p.into(), o.into(), g.into()],
    )
    .expect("put_quad: insert failed");
}

/// Count quads in a graph (0 = default partition).
///
/// SQL surface:
/// `pgrdf.count_quads(g BIGINT DEFAULT 0) → BIGINT`.
#[pg_extern]
fn count_quads(g: default!(i64, 0)) -> i64 {
    // `SELECT count(*)` always returns exactly one row, so we don't
    // need the scalar-subquery trick `get_term` uses.
    Spi::get_one_with_args::<i64>(
        "SELECT count(*)::BIGINT FROM pgrdf._pgrdf_quads WHERE graph_id = $1",
        &[g.into()],
    )
    .expect("count_quads: select failed")
    .unwrap_or(0)
}

/// Create a LIST partition of `_pgrdf_quads` for the named graph.
/// Idempotent. Required before `put_quad(.., g)` for non-default `g`
/// values to physically land in their own partition (otherwise tuples
/// route to `_pgrdf_quads_default`). Returns TRUE if the partition
/// was created on this call, FALSE if it already existed.
///
/// Phase A slice 119 — on the partition-creating path, also inserts
/// `(g, 'urn:pgrdf:graph:' || g::text)` into `_pgrdf_graphs` so v0.3
/// callers automatically populate the IRI ↔ graph_id mapping landed
/// in slice 120. `ON CONFLICT (graph_id) DO NOTHING` preserves
/// idempotency of the UDF as a whole (the seed row + repeat calls
/// never error). Synthetic IRI shape `urn:pgrdf:graph:{id}` matches
/// the seed row from slice 120 and the LLD v0.4 §3.1 contract.
///
/// SQL surface: `pgrdf.add_graph(g BIGINT) → BOOLEAN`.
#[pg_extern]
fn add_graph(g: i64) -> bool {
    if g < 0 {
        panic!("add_graph: graph_id must be >= 0, got {}", g);
    }
    let part_name = format!("_pgrdf_quads_g{}", g);
    let exists: bool = Spi::get_one_with_args(
        "SELECT EXISTS(
            SELECT 1 FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relname = $1
         )",
        &[part_name.as_str().into()],
    )
    .expect("add_graph: existence check failed")
    .unwrap_or(false);
    if exists {
        return false;
    }
    // `part_name` is a string we constructed from a BIGINT (no user
    // input in a SQL identifier position), so direct interpolation is
    // safe. `g` is bound via the LIST value position which Postgres
    // accepts as a constant in DDL.
    let sql = format!(
        "CREATE TABLE pgrdf.{} PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN ({})",
        part_name, g
    );
    Spi::run(&sql).expect("add_graph: CREATE TABLE failed");
    // Slice 119 — bind the synthetic IRI for this graph_id in
    // `_pgrdf_graphs`. `ON CONFLICT (graph_id) DO NOTHING` keeps the
    // UDF re-entrant: if a prior writer (or a future explicit
    // `add_graph(id, iri)` overload) already bound `g`, we leave that
    // binding intact rather than clobber it.
    Spi::run_with_args(
        "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) \
         VALUES ($1, 'urn:pgrdf:graph:' || $1::text) \
         ON CONFLICT (graph_id) DO NOTHING",
        &[g.into()],
    )
    .unwrap_or_else(|e| panic!("add_graph: failed to insert synthetic IRI for graph_id {g}: {e}"));
    true
}

/// Create or look up a named graph identified by an IRI; auto-allocates
/// a fresh integer `graph_id`, inserts the binding into
/// `_pgrdf_graphs`, and creates the matching LIST partition of
/// `_pgrdf_quads`.
///
/// Idempotent on the IRI: if `iri` is already bound, returns the
/// existing `graph_id` without creating a second partition or
/// duplicating the binding.
///
/// Allocation strategy: the smallest unused positive integer, computed
/// via `COALESCE(MAX(graph_id), 0) + 1`. Concurrent allocate-and-insert
/// sequences are serialised by a `LOCK TABLE _pgrdf_graphs IN SHARE
/// ROW EXCLUSIVE MODE` taken before the SELECT-MAX so two simultaneous
/// callers can't both compute the same id and race the INSERT (the
/// `UNIQUE(iri)` constraint would catch one of them, but the lock
/// makes it impossible to lose). The lock releases at transaction end
/// per Postgres semantics. For v0.4.1 we accept this simple approach;
/// a sequence-based allocator is a future option if contention proves
/// real on the wire.
///
/// IRI is bound to the `_pgrdf_graphs` row *before* `add_graph(id)`
/// runs so the slice-119 synthetic-IRI insert path inside the integer
/// overload no-ops via `ON CONFLICT (graph_id) DO NOTHING`, leaving
/// the user-supplied IRI intact.
///
/// IRI syntax is **not** validated against RFC 3987 here — we have no
/// oxiri dependency in v0.4.1. Empty / whitespace-only strings panic
/// with the stable `add_graph:` prefix; everything else is accepted
/// verbatim. Tighter validation is a future slice.
///
/// SQL surface: `pgrdf.add_graph(iri TEXT) → BIGINT` (overload of the
/// integer-keyed `pgrdf.add_graph(g BIGINT) → BOOLEAN` above; pgrx
/// surfaces both Rust functions under the same SQL name via the
/// `name = "add_graph"` attribute, and Postgres dispatches on the
/// argument types).
#[pg_extern(name = "add_graph")]
fn add_graph_iri(iri: &str) -> i64 {
    if iri.trim().is_empty() {
        panic!("add_graph: iri must be non-empty");
    }

    // Serialise concurrent allocate-and-insert. SHARE ROW EXCLUSIVE
    // blocks other writers (including itself) but not readers; the
    // lock releases at transaction end. This is the v0.4.1 mitigation
    // for the `MAX(graph_id) + 1 → INSERT` race.
    Spi::run("LOCK TABLE pgrdf._pgrdf_graphs IN SHARE ROW EXCLUSIVE MODE")
        .unwrap_or_else(|e| panic!("add_graph: lock _pgrdf_graphs failed: {e}"));

    // Idempotent path: if the IRI is already bound, return its id
    // without touching the partition or the table. The inner SELECT
    // is wrapped in a scalar subquery so SPI always sees exactly one
    // row back (NULL when the IRI is not yet bound). Without the
    // wrapper, an empty result trips SPI with a
    // "SpiTupleTable positioned before the start or after the end"
    // error rather than yielding `None`. Same idiom as `put_term` in
    // `dict.rs`.
    let existing: Option<i64> = Spi::get_one_with_args(
        "SELECT (SELECT graph_id FROM pgrdf._pgrdf_graphs WHERE iri = $1 LIMIT 1)",
        &[iri.into()],
    )
    .unwrap_or_else(|e| panic!("add_graph: lookup existing iri failed: {e}"));
    if let Some(id) = existing {
        return id;
    }

    // Allocate the next id — smallest positive integer not yet in
    // use. Seed row `(0, 'urn:pgrdf:graph:0')` makes MAX always >= 0
    // post-CREATE-EXTENSION, so this branch always yields >= 1.
    let next: i64 = Spi::get_one("SELECT COALESCE(MAX(graph_id), 0) + 1 FROM pgrdf._pgrdf_graphs")
        .unwrap_or_else(|e| panic!("add_graph: allocate next id failed: {e}"))
        .expect("add_graph: COALESCE returned NULL (impossible)");

    // Bind the IRI *before* the integer overload runs. The integer
    // overload's slice-119 synthetic-IRI INSERT carries
    // `ON CONFLICT (graph_id) DO NOTHING`, so it sees this row and
    // no-ops — preserving the user-supplied IRI verbatim.
    Spi::run_with_args(
        "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) VALUES ($1, $2)",
        &[next.into(), iri.into()],
    )
    .unwrap_or_else(|e| panic!("add_graph: insert iri binding failed: {e}"));

    // Create the partition via the existing integer overload. We
    // re-enter through the SQL surface so any future change to the
    // partition-creation idiom stays single-sourced in
    // `add_graph(g BIGINT)` above.
    Spi::run_with_args(
        "SELECT pgrdf.add_graph($1::bigint)",
        &[next.into()],
    )
    .unwrap_or_else(|e| panic!("add_graph: partition creation failed: {e}"));

    next
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::prelude::*;

    #[pg_test]
    fn put_quad_then_count() {
        let s = Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.put_term('http://example.com/s', 1::smallint)",
            &[],
        )
        .unwrap()
        .unwrap();
        let p = Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.put_term('http://example.com/p', 1::smallint)",
            &[],
        )
        .unwrap()
        .unwrap();
        let o = Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.put_term('http://example.com/o', 1::smallint)",
            &[],
        )
        .unwrap()
        .unwrap();

        Spi::run_with_args(
            "SELECT pgrdf.put_quad($1, $2, $3)",
            &[s.into(), p.into(), o.into()],
        )
        .unwrap();

        let n: i64 = Spi::get_one("SELECT pgrdf.count_quads()").unwrap().unwrap();
        assert!(n >= 1, "expected at least 1 quad in default graph, got {n}");
    }

    #[pg_test]
    fn add_graph_creates_partition_idempotently() {
        // Use a graph id unlikely to clash with other tests.
        let g: i64 = 9001;
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g.into()]).unwrap();
        Spi::run_with_args("SELECT pgrdf.add_graph($1)", &[g.into()]).unwrap();
        let part_count: i64 = Spi::get_one_with_args(
            "SELECT count(*)::BIGINT FROM pg_class
             WHERE relnamespace = 'pgrdf'::regnamespace
               AND relname = '_pgrdf_quads_g9001'",
            &[],
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            part_count, 1,
            "expected exactly one partition for graph 9001"
        );
    }
}
