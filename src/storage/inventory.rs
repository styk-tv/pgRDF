//! E1 (SPEC.pgRDF.LIB.v0.6.34) — the supported graph-inventory surface.
//!
//! Before this module, "which graphs exist, how big are they, which
//! partitions are orphaned" was answerable only by joining
//! `pgrdf._pgrdf_graphs`, `_pgrdf_quads`, `pg_class` and `pg_inherits`
//! by hand. Measured across the two external consumers: 54 direct
//! private-table references in one, a full private-SQL inventory in the
//! other — every one a re-implementation of this crate's internals that
//! a storage change breaks with no signal. Both consumers ranked this
//! surface as the highest-value emission in the whole LIB set.
//!
//! Contract (LIB C3): with this module present, no supported client
//! path needs to read a `pgrdf._pgrdf_*` relation or a `pg_catalog`
//! internal. The parity questions it answers:
//!   Q1 which graphs exist (id + iri)      → `graph_inventory`
//!   Q2 asserted/inferred counts per graph → `graph_inventory`
//!   Q5 which partitions are orphaned      → `orphan_partitions`
//! plus the E4 fact no client ledger can state correctly (only the
//! engine sees every connection's writes): `materialization` =
//! current | stale | never | unknown.
//! (Q3 term lexical value → `pgrdf.get_term`; Q4 graphs containing a
//! subject → SPARQL `GRAPH ?g` — both already supported.)

use pgrx::prelude::*;

/// Every registered graph with its identity, size and lock state.
/// Empty graphs are included (an empty graph exists; absence refuses —
/// the same distinction `graph_digest` enforces). Sorted by `graph_id`
/// so output is stable for diffing.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
// pgrx's SQL generator requires the literal tuple type here — a type
// alias fails schema generation ("unexpected generic argument"), so the
// complex-type lint is allowed rather than obeyed.
#[allow(clippy::type_complexity)]
fn graph_inventory() -> TableIterator<
    'static,
    (
        name!(graph_id, i64),
        name!(iri, Option<String>),
        name!(asserted, i64),
        name!(inferred, i64),
        name!(locked, bool),
        name!(lock_reason, Option<String>),
        name!(materialization, String),
    ),
> {
    let mut rows = Vec::new();
    Spi::connect(|client| {
        let tup = client
            .select(
                "SELECT g.graph_id, g.iri,
                        COALESCE(c.a, 0) AS asserted,
                        COALESCE(c.i, 0) AS inferred,
                        COALESCE(g.locked, false) AS locked,
                        g.lock_reason,
                        CASE
                          WHEN g.last_materialize_at IS NULL THEN
                            CASE WHEN COALESCE(c.i, 0) > 0 THEN 'unknown' ELSE 'never' END
                          WHEN COALESCE(c.a, 0) IS DISTINCT FROM g.materialized_base_count
                            THEN 'stale'
                          ELSE 'current'
                        END AS materialization
                 FROM pgrdf._pgrdf_graphs g
                 LEFT JOIN (
                     SELECT graph_id,
                            count(*) FILTER (WHERE NOT is_inferred) AS a,
                            count(*) FILTER (WHERE is_inferred) AS i
                     FROM pgrdf._pgrdf_quads GROUP BY graph_id
                 ) c ON c.graph_id = g.graph_id
                 ORDER BY g.graph_id",
                None,
                &[],
            )
            .unwrap_or_else(|e| panic!("graph_inventory: enumeration failed: {e}"));
        for row in tup {
            rows.push((
                row.get::<i64>(1).unwrap().unwrap_or(0),
                row.get::<String>(2).unwrap(),
                row.get::<i64>(3).unwrap().unwrap_or(0),
                row.get::<i64>(4).unwrap().unwrap_or(0),
                row.get::<bool>(5).unwrap().unwrap_or(false),
                row.get::<String>(6).unwrap(),
                row.get::<String>(7)
                    .unwrap()
                    .unwrap_or_else(|| "unknown".into()),
            ));
        }
    });
    TableIterator::new(rows.into_iter())
}

/// Quad partitions with no `_pgrdf_graphs` registration — unreachable
/// by SPARQL and therefore by any graph-level export or digest. They
/// are counted here so their existence is at least visible; capturing
/// them needs a different mechanism (LIB §14).
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn orphan_partitions() -> TableIterator<'static, (name!(relname, String),)> {
    let mut rows = Vec::new();
    Spi::connect(|client| {
        let tup = client
            .select(
                "SELECT pc.relname::text
                 FROM pg_class pc
                 JOIN pg_inherits i ON i.inhrelid = pc.oid
                 JOIN pg_class par ON par.oid = i.inhparent
                                  AND par.relname = '_pgrdf_quads'
                 WHERE pc.relname <> '_pgrdf_quads_default'
                   AND NOT EXISTS (
                       SELECT 1 FROM pgrdf._pgrdf_graphs g
                       WHERE pc.relname = '_pgrdf_quads_g' || g.graph_id
                   )
                 ORDER BY pc.relname",
                None,
                &[],
            )
            .unwrap_or_else(|e| panic!("orphan_partitions: enumeration failed: {e}"));
        for row in tup {
            if let Ok(Some(name)) = row.get::<String>(1) {
                rows.push((name,));
            }
        }
    });
    TableIterator::new(rows.into_iter())
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// E1 parity Q1/Q2: the inventory agrees with the private tables it
    /// exists to retire — same transaction, both directions. This is
    /// the artifact that lets a consumer delete its private-table SQL
    /// and KNOW nothing was lost (TDD §4).
    #[pg_test]
    fn inventory_parity_with_private_tables() {
        Spi::run("SELECT pgrdf.add_graph('urn:inv:a')").unwrap();
        Spi::run("SELECT pgrdf.add_graph('urn:inv:b')").unwrap();
        Spi::run(
            "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> \"v\" .', pgrdf.graph_id('urn:inv:a'))",
        )
        .unwrap();
        let (pub_n, priv_n) = Spi::get_two::<i64, i64>(
            "SELECT (SELECT count(*) FROM pgrdf.graph_inventory()),
                    (SELECT count(*) FROM pgrdf._pgrdf_graphs)",
        )
        .unwrap();
        assert_eq!(pub_n, priv_n, "Q1 parity: same graph count both ways");
        let (pub_a, priv_a) = Spi::get_two::<i64, i64>(
            "SELECT (SELECT asserted FROM pgrdf.graph_inventory()
                     WHERE iri = 'urn:inv:a'),
                    (SELECT count(*) FROM pgrdf._pgrdf_quads
                     WHERE graph_id = pgrdf.graph_id('urn:inv:a')
                       AND NOT is_inferred)",
        )
        .unwrap();
        assert_eq!(pub_a, priv_a, "Q2 parity: same asserted count both ways");
        assert_eq!(pub_a, Some(1));
    }

    /// Lock state surfaces in the inventory — the engine's real lock,
    /// not a shadow copy.
    #[pg_test]
    fn inventory_carries_lock_state() {
        Spi::run("SELECT pgrdf.add_graph('urn:inv:locked')").unwrap();
        Spi::run("SELECT pgrdf.lock_graph(pgrdf.graph_id('urn:inv:locked'), 'inv test')").unwrap();
        let (locked, reason) = Spi::get_two::<bool, String>(
            "SELECT locked, lock_reason FROM pgrdf.graph_inventory()
             WHERE iri = 'urn:inv:locked'",
        )
        .unwrap();
        assert_eq!(locked, Some(true));
        assert_eq!(reason.as_deref(), Some("inv test"));
    }
}
