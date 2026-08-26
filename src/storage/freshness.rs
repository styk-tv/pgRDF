//! E4 (SPEC.pgRDF.LIB.v0.6.34) — materialization freshness as an engine
//! fact.
//!
//! "Is this graph's materialization current?" is a question only the
//! engine can answer from one place for every connection — any
//! client-side ledger sees only its own writes (measured: a consumer
//! re-counts live triples on every call precisely because it is
//! guessing, and a fleet-wide "inferred = 0, entailment never ran"
//! claim stood for weeks while the true count was 1464).
//!
//! DESIGN, chosen for zero write-path contention: `materialize` — rare
//! and already heavyweight — records WHEN it ran and the asserted
//! count it ran OVER (`last_materialize_at`, `materialized_base_count`
//! on `_pgrdf_graphs`). Nothing on the hot write paths touches the
//! catalog row: a first draft stamped every write there, and the
//! parallel test suite immediately exposed the hazard (the stamp's row
//! lock, held to commit, collides with `add_graph`'s table lock and
//! serializes concurrent writers). The fact is derived at read time in
//! `graph_inventory()`:
//!
//!   `never`   — never materialized, no inferred rows
//!   `unknown` — inferred rows but no record (graphs from pre-0.6.34,
//!               until their next materialize) — honest, never guessed
//!   `stale`   — asserted count differs from the count materialized over
//!   `current` — counts agree
//!
//! STATED LIMIT: a write that leaves the asserted count unchanged (a
//! delete matched by an insert) reads `current`. This is the same
//! count-comparison consumers already relied on client-side — now
//! computed engine-side over every connection's writes, with the
//! timestamp on record. A stronger content-marker (digest at
//! materialize time) is a deliberate non-goal until it can be priced;
//! the column layout admits it additively.
//!
//! POLICY STAYS WITH THE CALLER (LIB §4): the engine emits the fact;
//! refusing on staleness belongs to the consumer or a future
//! fail-closed GUC.

use pgrx::prelude::*;

/// Record a successful materialization: when, and over how many
/// asserted triples. Called by `materialize` only — never from a hot
/// write path (see the module docs for why that matters).
pub(crate) fn stamp_materialize(graph_id: i64, base_count: i64) {
    Spi::run_with_args(
        "UPDATE pgrdf._pgrdf_graphs \
         SET last_materialize_at = clock_timestamp(), \
             materialized_base_count = $2 \
         WHERE graph_id = $1",
        &[graph_id.into(), base_count.into()],
    )
    .unwrap_or_else(|e| panic!("freshness: materialize stamp failed for graph {graph_id}: {e}"));
}

// Idempotent-ALTER pattern (same as the 0.6.28 lock columns): correct on
// fresh install and in the generated full-install SQL; the upgrade path
// ships identical statements in sql/pgrdf--0.6.33--0.6.34.sql.
pgrx::extension_sql!(
    r#"
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS last_materialize_at     TIMESTAMPTZ;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS materialized_base_count BIGINT;
"#,
    name = "graph_freshness_columns_v0_6_34",
    requires = ["schema_v0_4_0_graphs"],
);

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// E4 end-to-end: never → materialize → current → write → stale →
    /// re-materialize → current. The fact a client-side ledger could
    /// never state correctly across connections.
    #[pg_test]
    fn materialization_state_tracks_writes() {
        Spi::run("SELECT pgrdf.add_graph('urn:fresh:a')").unwrap();
        let state = |()| -> String {
            Spi::get_one::<String>(
                "SELECT materialization FROM pgrdf.graph_inventory()
                 WHERE iri = 'urn:fresh:a'",
            )
            .unwrap()
            .unwrap()
        };
        assert_eq!(state(()), "never", "no materialize yet, no inferred rows");
        Spi::run(
            "SELECT pgrdf.parse_turtle(
                '@prefix ex: <http://e/> . ex:a a ex:T . ex:T <http://www.w3.org/2000/01/rdf-schema#subClassOf> ex:S .',
                pgrdf.graph_id('urn:fresh:a'))",
        )
        .unwrap();
        Spi::run("SELECT pgrdf.materialize(pgrdf.graph_id('urn:fresh:a'))").unwrap();
        assert_eq!(state(()), "current", "materialized, nothing written since");
        Spi::run(
            "SELECT pgrdf.parse_turtle('<urn:f:s> <urn:f:p> \"v\" .', pgrdf.graph_id('urn:fresh:a'))",
        )
        .unwrap();
        assert_eq!(
            state(()),
            "stale",
            "asserted content changed after materialization"
        );
        Spi::run("SELECT pgrdf.materialize(pgrdf.graph_id('urn:fresh:a'))").unwrap();
        assert_eq!(state(()), "current", "re-materialize clears staleness");
    }

    /// #107 completed: the SPARQL UPDATE path now takes the lock fence —
    /// INSERT DATA into a locked graph refused nothing before this cut
    /// (a consumer compensated client-side). Asserted by code (55P03).
    #[pg_test]
    fn sparql_update_respects_graph_lock() {
        use pgrx::pg_sys::errcodes::PgSqlErrorCode;
        use pgrx::pg_sys::panic::CaughtError;
        Spi::run("SELECT pgrdf.add_graph('urn:fresh:upd')").unwrap();
        Spi::run(
            "SELECT count(*) FROM pgrdf.sparql(
                'INSERT DATA { GRAPH <urn:fresh:upd> { <urn:u:s> <urn:u:p> <urn:u:o> } }')",
        )
        .unwrap();
        Spi::run("SELECT pgrdf.lock_graph(pgrdf.graph_id('urn:fresh:upd'), 'fence test')").unwrap();
        let code = pgrx::PgTryBuilder::new(|| {
            Spi::run(
                "SELECT count(*) FROM pgrdf.sparql(
                    'INSERT DATA { GRAPH <urn:fresh:upd> { <urn:u:s2> <urn:u:p> <urn:u:o> } }')",
            )
            .unwrap();
            None
        })
        .catch_others(|e| match &e {
            CaughtError::PostgresError(r)
            | CaughtError::ErrorReport(r)
            | CaughtError::RustPanic { ereport: r, .. } => Some(r.sql_error_code()),
        })
        .execute();
        assert_eq!(
            code,
            Some(PgSqlErrorCode::ERRCODE_LOCK_NOT_AVAILABLE),
            "INSERT DATA into a locked graph must refuse 55P03 — the fence hole is closed"
        );
    }
}
