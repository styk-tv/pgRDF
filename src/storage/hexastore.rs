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
    true
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
