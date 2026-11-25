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

/// One row of the inventory, in column order.
type InventoryRow = (
    i64,
    Option<String>,
    i64,
    i64,
    bool,
    Option<String>,
    String,
    Option<String>,
    Option<i32>,
    Option<String>,
    Option<String>,
    Option<pgrx::datum::TimestampWithTimeZone>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    bool,
    Option<pgrx::datum::TimestampWithTimeZone>,
    Option<i64>,
);

/// Which graphs a listing asks for.
enum Which {
    All,
    Id(i64),
    Iri(String),
}

/// Does `current_user` see every graph? A superuser, a role holding the
/// storage owner's privileges, or one holding `pg_read_all_data`, which
/// can read every table already. USAGE, never membership: a role granted
/// WITH INHERIT FALSE does not hold the privileges until it SET ROLEs.
/// `pg_read_all_data` does not bypass row-level security; pgRDF uses none.
pub(crate) fn caller_sees_all_graphs() -> bool {
    // `has_privs_of_role` is the USAGE test (`pg_has_role(..., 'USAGE')`),
    // asked directly: no query on the listing's hot path.
    unsafe {
        let me = pgrx::pg_sys::GetUserId();
        crate::storage::partition::storage_owner_oid()
            .is_some_and(|owner| pgrx::pg_sys::has_privs_of_role(me, owner))
            || pgrx::pg_sys::has_privs_of_role(
                me,
                pgrx::pg_sys::Oid::from(pgrx::pg_sys::ROLE_PG_READ_ALL_DATA),
            )
    }
}

/// The graphs the caller owns: the test `require_graph_owner` uses, so
/// "may drop" and "may list" never disagree. `$1` is whether the caller
/// sees every graph, `$2` the caller's role oid; `j.r` is the registry row
/// as jsonb.
pub(crate) const LISTED_TO_CALLER: &str = "($1 OR EXISTS ( \
       SELECT 1 FROM pg_roles r \
        WHERE r.rolname = (j.r ->> 'owner') AND pg_has_role($2, r.oid, 'USAGE')))";

/// The inventory rows the caller is entitled to. Read as the storage
/// owner, with the scope evaluated for the caller's `current_user`
/// captured here: a role needs EXECUTE on the function and nothing else.
fn inventory_rows(which: Which) -> Vec<InventoryRow> {
    let caller = unsafe { pgrx::pg_sys::GetUserId() };
    let sees_all = caller_sees_all_graphs();
    let (by_id, by_iri): (Option<i64>, Option<String>) = match &which {
        Which::All => (None, None),
        Which::Id(id) => (Some(*id), None),
        Which::Iri(iri) => (None, Some(iri.clone())),
    };
    // The stored counts arrive with the 0.6.42 SQL. Without them every
    // graph is counted by scan, as before.
    let (count_columns, count_join) = if crate::storage::counts::present() {
        (
            crate::storage::counts::read_columns(),
            format!(
                "LEFT JOIN pgrdf._pgrdf_graph_counts c ON c.graph_id = g.graph_id LEFT JOIN {}",
                crate::storage::counts::read_laterals()
            ),
        )
    } else {
        (
            "NULL::bigint, NULL::bigint, 0::bigint, false, NULL::timestamptz".to_string(),
            String::new(),
        )
    };
    // One statement per form, each with a plain equality where it has
    // one: a filter of the shape "$3 IS NULL OR graph_id = $3" has no good
    // generic plan, so the plan cache would plan the statement again on
    // every call, and for one graph's row planning is most of the cost.
    let only = match &which {
        Which::All => "AND ($3::bigint IS NULL) AND ($4::text IS NULL)",
        Which::Id(_) => "AND g.graph_id = $3 AND ($4::text IS NULL)",
        Which::Iri(_) => "AND g.iri = $4 AND ($3::bigint IS NULL)",
    };
    let sql = format!(
        "SELECT g.graph_id, g.iri,
                COALESCE(g.locked, false), g.lock_reason,
                g.last_materialize_at IS NOT NULL, g.materialized_base_count,
                g.source_sha256, g.source_loads,
                -- 0.6.37: a locked graph's cached rdfc-1.0 digest, shown only
                -- while lock custody holds (both partition triggers present):
                -- the rule graph_digest uses to trust its cache.
                CASE WHEN g.locked AND (
                       SELECT count(*) FROM pg_trigger t
                       JOIN pg_class k ON k.oid = t.tgrelid
                       WHERE k.relnamespace = 'pgrdf'::regnamespace
                         AND k.relname = format('_pgrdf_quads_g%s', g.graph_id)
                         AND t.tgname IN ('pgrdf_lock_row', 'pgrdf_lock_truncate')
                     ) = 2
                     THEN g.locked_digest END,
                -- 0.6.41: the recorded owner, who created the graph (the
                -- declared identity on its create act), and the role holding
                -- the current lock. NULL on a graph older than the act record.
                (j.r ->> 'owner'), (j.r ->> 'created_at')::timestamptz,
                ca.workspace, ca.agent, ca.thread, la.acting_role,
                (SELECT a.act_id FROM pgrdf._pgrdf_graph_acts a
                  WHERE a.graph_id = g.graph_id ORDER BY a.act_id DESC LIMIT 1),
                {count_columns}
           FROM pgrdf._pgrdf_graphs g
          CROSS JOIN LATERAL (SELECT to_jsonb(g) AS r) j
           LEFT JOIN pgrdf._pgrdf_graph_acts ca
                  ON ca.act_id = (j.r ->> 'created_act')::bigint
           LEFT JOIN pgrdf._pgrdf_graph_acts la
                  ON la.act_id = (j.r ->> 'locked_act')::bigint AND g.locked
           {count_join}
          WHERE {LISTED_TO_CALLER}
            {only}
          ORDER BY g.graph_id"
    );
    // Reading only, and through read-only SPI all the way down: a call
    // that lists graphs takes no transaction id.
    let rows: Vec<InventoryRow> = crate::storage::partition::as_storage_owner_reading(|| {
        crate::storage::counts::plan_by_index();
        let mut rows = Vec::new();
        Spi::connect(|client| {
            // Prepared once per backend (read-only: no transaction id) and
            // reused: for one graph's row, planning this statement costs
            // more than running it.
            if !crate::query::plan_cache::contains(&sql) {
                let prepared = client
                    .prepare(
                        sql.as_str(),
                        &[
                            PgOid::BuiltIn(PgBuiltInOids::BOOLOID),
                            PgOid::BuiltIn(PgBuiltInOids::OIDOID),
                            PgOid::BuiltIn(PgBuiltInOids::INT8OID),
                            PgOid::BuiltIn(PgBuiltInOids::TEXTOID),
                        ],
                    )
                    .unwrap_or_else(|e| panic!("graph_inventory: prepare failed: {e}"))
                    .keep();
                crate::query::plan_cache::insert(sql.clone(), prepared);
            }
            let args = [
                sees_all.into(),
                caller.into(),
                by_id.into(),
                by_iri.clone().into(),
            ];
            let tup = crate::query::plan_cache::with_plan(&sql, |plan| {
                client
                    .select(
                        plan.expect("graph_inventory: plan is cached after insert"),
                        None,
                        &args,
                    )
                    .unwrap_or_else(|e| panic!("graph_inventory: enumeration failed: {e}"))
            });
            for row in tup {
                let graph_id = row.get::<i64>(1).unwrap().unwrap_or(0);
                let verified = row.get::<bool>(20).unwrap().unwrap_or(false);
                // A verified count is the stored one. Any other graph is
                // counted here and now: the numbers returned are exact
                // either way, and `counts_verified` says which it was.
                let (asserted, inferred) = match (
                    verified,
                    row.get::<i64>(17).unwrap(),
                    row.get::<i64>(18).unwrap(),
                ) {
                    (true, Some(a), Some(i)) => (a, i),
                    _ => crate::storage::counts::scanned(graph_id),
                };
                let materialized = row.get::<bool>(5).unwrap().unwrap_or(false);
                let base = row.get::<i64>(6).unwrap();
                let materialization = if !materialized {
                    if inferred > 0 { "unknown" } else { "never" }
                } else if base != Some(asserted) {
                    "stale"
                } else {
                    "current"
                };
                rows.push((
                    graph_id,
                    row.get::<String>(2).unwrap(),
                    asserted,
                    inferred,
                    row.get::<bool>(3).unwrap().unwrap_or(false),
                    row.get::<String>(4).unwrap(),
                    materialization.to_string(),
                    row.get::<String>(7).unwrap(),
                    row.get::<i32>(8).unwrap(),
                    row.get::<String>(9).unwrap(),
                    row.get::<String>(10).unwrap(),
                    row.get::<pgrx::datum::TimestampWithTimeZone>(11).unwrap(),
                    row.get::<String>(12).unwrap(),
                    row.get::<String>(13).unwrap(),
                    row.get::<String>(14).unwrap(),
                    row.get::<String>(15).unwrap(),
                    row.get::<i64>(19).unwrap().unwrap_or(0),
                    verified,
                    row.get::<pgrx::datum::TimestampWithTimeZone>(21).unwrap(),
                    row.get::<i64>(16).unwrap(),
                ));
            }
        });
        // One graph asked for and none returned: it does not exist, or it
        // is not the caller's. The two never look alike.
        if rows.is_empty() && !matches!(which, Which::All) {
            let exists = crate::storage::partition::read_one::<bool>(
                "SELECT EXISTS (SELECT 1 FROM pgrdf._pgrdf_graphs g \
                                 WHERE ($1::bigint IS NULL OR g.graph_id = $1) \
                                   AND ($2::text IS NULL OR g.iri = $2))",
                &[by_id.into(), by_iri.into()],
            )
            .unwrap_or(false);
            if exists {
                crate::refuse_with_hint(
                    PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
                    "pgrdf: graph_inventory: that graph is not listed to this role".to_string(),
                    "pgrdf.graph_inventory() lists the graphs this role owns; the graph's owner \
                     or the storage owner can hand it over with \
                     pgrdf.set_graph_owner(graph_id, role)."
                        .to_string(),
                );
            }
        }
        rows
    });
    rows
}

/// The graphs the caller is entitled to see, each with its identity, size
/// and lock state. An admin (a superuser, a role holding the storage
/// owner's privileges, or one holding `pg_read_all_data`) sees every
/// registered graph; any other role sees the graphs it owns. Empty graphs
/// are included. Sorted by `graph_id` so output is stable for diffing.
///
/// `asserted` and `inferred` are exact. A graph whose `counts_verified`
/// is true answered from the count the engine keeps as it writes; any
/// other graph was counted by scan for this call.
///
/// A listing rule, not secrecy: a role that may read triples can count
/// any graph by query, and the act record names every graph.
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
        name!(source_sha256, Option<String>),
        name!(source_loads, Option<i32>),
        name!(identity_digest, Option<String>),
        name!(owner, Option<String>),
        name!(created_at, Option<pgrx::datum::TimestampWithTimeZone>),
        name!(created_workspace, Option<String>),
        name!(created_agent, Option<String>),
        name!(created_thread, Option<String>),
        name!(locked_by, Option<String>),
        name!(change_seq, i64),
        name!(counts_verified, bool),
        name!(
            counts_scanned_at,
            Option<pgrx::datum::TimestampWithTimeZone>
        ),
        name!(last_act_id, Option<i64>),
    ),
> {
    TableIterator::new(inventory_rows(Which::All).into_iter())
}

/// One graph's inventory row, by id: the row `graph_inventory()` has for
/// it. Refuses 42501 for a graph that exists and is not listed to the
/// caller; returns no row for a graph that does not exist.
#[search_path(pgrdf, pg_temp)]
#[pg_extern(name = "graph_inventory")]
#[allow(clippy::type_complexity)]
fn graph_inventory_by_id(
    graph_id: i64,
) -> TableIterator<
    'static,
    (
        name!(graph_id, i64),
        name!(iri, Option<String>),
        name!(asserted, i64),
        name!(inferred, i64),
        name!(locked, bool),
        name!(lock_reason, Option<String>),
        name!(materialization, String),
        name!(source_sha256, Option<String>),
        name!(source_loads, Option<i32>),
        name!(identity_digest, Option<String>),
        name!(owner, Option<String>),
        name!(created_at, Option<pgrx::datum::TimestampWithTimeZone>),
        name!(created_workspace, Option<String>),
        name!(created_agent, Option<String>),
        name!(created_thread, Option<String>),
        name!(locked_by, Option<String>),
        name!(change_seq, i64),
        name!(counts_verified, bool),
        name!(
            counts_scanned_at,
            Option<pgrx::datum::TimestampWithTimeZone>
        ),
        name!(last_act_id, Option<i64>),
    ),
> {
    TableIterator::new(inventory_rows(Which::Id(graph_id)).into_iter())
}

/// One graph's inventory row, by IRI. A quoted number is an IRI here:
/// write `graph_inventory(42)` for the id form.
#[search_path(pgrdf, pg_temp)]
#[pg_extern(name = "graph_inventory")]
#[allow(clippy::type_complexity)]
fn graph_inventory_by_iri(
    iri: &str,
) -> TableIterator<
    'static,
    (
        name!(graph_id, i64),
        name!(iri, Option<String>),
        name!(asserted, i64),
        name!(inferred, i64),
        name!(locked, bool),
        name!(lock_reason, Option<String>),
        name!(materialization, String),
        name!(source_sha256, Option<String>),
        name!(source_loads, Option<i32>),
        name!(identity_digest, Option<String>),
        name!(owner, Option<String>),
        name!(created_at, Option<pgrx::datum::TimestampWithTimeZone>),
        name!(created_workspace, Option<String>),
        name!(created_agent, Option<String>),
        name!(created_thread, Option<String>),
        name!(locked_by, Option<String>),
        name!(change_seq, i64),
        name!(counts_verified, bool),
        name!(
            counts_scanned_at,
            Option<pgrx::datum::TimestampWithTimeZone>
        ),
        name!(last_act_id, Option<i64>),
    ),
> {
    TableIterator::new(inventory_rows(Which::Iri(iri.to_string())).into_iter())
}

/// The ids of the graphs the caller may act on (it owns them, or it holds
/// the storage owner's privileges), in order; with `only`, that graph
/// alone. Asking for one graph that exists and is somebody else's refuses
/// 42501; one that does not exist refuses 42704 under `what`.
pub(crate) fn listed_graph_ids(only: Option<i64>, what: &str) -> Vec<i64> {
    let caller = unsafe { pgrx::pg_sys::GetUserId() };
    // An act, not a look: the storage owner may, and the graph's owner.
    // A role that only reads everything (pg_read_all_data) may not leave
    // acts on other roles' graphs.
    let sees_all = crate::storage::partition::caller_owns_storage();
    crate::storage::partition::as_storage_owner(|| {
        let ids: Vec<i64> = Spi::connect(|client| {
            client
                .select(
                    &format!(
                        "SELECT g.graph_id FROM pgrdf._pgrdf_graphs g \
                          CROSS JOIN LATERAL (SELECT to_jsonb(g) AS r) j \
                          WHERE {LISTED_TO_CALLER} AND ($3::bigint IS NULL OR g.graph_id = $3) \
                          ORDER BY g.graph_id"
                    ),
                    None,
                    &[sees_all.into(), caller.into(), only.into()],
                )
                .unwrap_or_else(|e| panic!("{what}: graph lookup failed: {e}"))
                .filter_map(|row| row.get::<i64>(1).ok().flatten())
                .collect()
        });
        if let (true, Some(id)) = (ids.is_empty(), only) {
            let exists = Spi::get_one_with_args::<bool>(
                "SELECT EXISTS (SELECT 1 FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)",
                &[id.into()],
            )
            .unwrap_or_else(|e| panic!("{what}: graph lookup failed: {e}"))
            .unwrap_or(false);
            if exists {
                crate::refuse_with_hint(
                    PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
                    format!("pgrdf: {what}: graph {id} belongs to another role"),
                    "The graph's owner or the storage owner can run this, or hand the graph \
                     over with pgrdf.set_graph_owner(graph_id, role)."
                        .to_string(),
                );
            }
            crate::refuse(
                PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
                format!("pgrdf: {what}: no graph with id {id}"),
            );
        }
        ids
    })
}

/// Count a graph again and store what the scan finds: the cure for a
/// count that is not verified (`counts_verified` false in the
/// inventory), which is what a write made in plain SQL, a restore, or a
/// privileged role that switched the count triggers off leaves behind.
/// `recount_graph(NULL)` does it for every graph the caller owns, or for
/// every graph when the caller holds the storage owner's privileges.
///
/// One row per graph: the count found, the count that was stored, and
/// whether that one was verified. Changes no triple, so it runs on a
/// locked graph. Recorded as act `recount`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
#[allow(clippy::type_complexity)]
fn recount_graph(
    graph_id: default!(Option<i64>, "NULL"),
) -> TableIterator<
    'static,
    (
        name!(graph_id, i64),
        name!(asserted, i64),
        name!(inferred, i64),
        name!(was_asserted, Option<i64>),
        name!(was_inferred, Option<i64>),
        name!(was_verified, bool),
    ),
> {
    let mut out = Vec::new();
    for id in listed_graph_ids(graph_id, "recount_graph") {
        let was = crate::storage::counts::stored(id);
        let act = crate::storage::acts::begin(id, "recount");
        let (asserted, inferred) = crate::storage::counts::set_by_scan(id);
        let was_verified = was.map(|w| w.verified).unwrap_or(false);
        act.finish(serde_json::json!({
            "asserted": asserted,
            "inferred": inferred,
            "was_asserted": was.map(|w| w.asserted),
            "was_inferred": was.map(|w| w.inferred),
            "was_verified": was_verified,
        }));
        out.push((
            id,
            asserted,
            inferred,
            was.map(|w| w.asserted),
            was.map(|w| w.inferred),
            was_verified,
        ));
    }
    TableIterator::new(out.into_iter())
}

/// Totals over the graphs listed to the caller, in one row, with the
/// scope they cover stated beside them: `all` for an admin, `own` for any
/// other role. `graphs_total` is the number of graphs in the database for
/// every caller, so a client that compares listings can tell "not listed
/// to me" from "dropped". `unverified_graphs` counts the listed graphs
/// that were counted by scan for this call.
///
/// `terms`, the number of dictionary terms, belongs to the database and
/// not to a graph: it is NULL when the scope is `own`.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
#[allow(clippy::type_complexity)]
fn graph_totals() -> TableIterator<
    'static,
    (
        name!(scope, String),
        name!(graphs_listed, i64),
        name!(graphs_total, i64),
        name!(asserted, i64),
        name!(inferred, i64),
        name!(unverified_graphs, i64),
        name!(terms, Option<i64>),
    ),
> {
    let sees_all = caller_sees_all_graphs();
    let rows = inventory_rows(Which::All);
    let graphs_total = crate::storage::partition::as_storage_owner_reading(|| {
        crate::storage::partition::read_one::<i64>(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_graphs",
            &[],
        )
        .unwrap_or(0)
    });
    let terms = if sees_all {
        crate::storage::counts::dictionary_terms()
    } else {
        None
    };
    TableIterator::once((
        if sees_all { "all" } else { "own" }.to_string(),
        rows.len() as i64,
        graphs_total,
        rows.iter().map(|r| r.2).sum(),
        rows.iter().map(|r| r.3).sum(),
        rows.iter().filter(|r| !r.17).count() as i64,
        terms,
    ))
}

/// The materialization state of one registered graph, by the rule
/// `graph_inventory()` shows in its `materialization` column: `never`,
/// `unknown`, `stale` or `current`. `None` when the graph is not
/// registered.
pub(crate) fn materialization_of(graph_id: i64) -> Option<String> {
    Spi::get_one_with_args::<String>(
        "SELECT (SELECT CASE
                          WHEN g.last_materialize_at IS NULL THEN
                            CASE WHEN c.i > 0 THEN 'unknown' ELSE 'never' END
                          WHEN c.a IS DISTINCT FROM g.materialized_base_count THEN 'stale'
                          ELSE 'current'
                        END
                   FROM pgrdf._pgrdf_graphs g,
                        LATERAL (SELECT count(*) FILTER (WHERE NOT q.is_inferred) AS a,
                                        count(*) FILTER (WHERE q.is_inferred) AS i
                                   FROM pgrdf._pgrdf_quads q
                                  WHERE q.graph_id = g.graph_id) c
                  WHERE g.graph_id = $1)",
        &[graph_id.into()],
    )
    .unwrap_or_else(|e| panic!("materialization state read failed for graph {graph_id}: {e}"))
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

/// Storage objects not owned by the storage owner (#153, 0.6.40; #160).
///
/// The storage owner is the owner of `_pgrdf_quads`: graph creation,
/// clear, drop, lock and the act record run as that role. A consumer
/// that hands pgRDF's tables to its own role (one ALTER ... OWNER per
/// relation) leaves behind any relation a later release adds, and any
/// partition someone else created; acts on those can then be refused.
/// One row per drifted table, partition, standalone sequence (a
/// sequence linked to a column follows its table) or the `pgrdf` schema
/// itself. `cure` is the statement that realigns it. Empty when
/// everything agrees: empty is the answer.
///
/// `blocking` is true exactly when the storage owner lacks something
/// the engine does as that role on the object (#160):
///
/// | object | the engine, as the storage owner, needs |
/// |---|---|
/// | a partition | ownership: it drops, truncates, indexes and puts triggers on it |
/// | `_pgrdf_graphs` | SELECT and UPDATE: it reads ids, takes `LOCK TABLE ... SHARE ROW EXCLUSIVE`, and records owner and custody columns |
/// | `_pgrdf_graph_acts` | INSERT |
/// | `_pgrdf_graph_id_seq` | USAGE, SELECT and UPDATE |
/// | `_pgrdf_graph_act_id_seq` | USAGE |
/// | schema `pgrdf` | USAGE and CREATE: it creates partitions there |
/// | anything else (`_pgrdf_dictionary`, `_pgrdf_staged_ping`, ...) | nothing: listed, never blocking |
///
/// Holding the object owner's rights always satisfies the row.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
#[allow(clippy::type_complexity)]
fn ownership_drift() -> TableIterator<
    'static,
    (
        name!(relname, String),
        name!(kind, String),
        name!(owner, String),
        name!(storage_owner, String),
        name!(blocking, bool),
        name!(cure, String),
    ),
> {
    let owner: pgrx::pg_sys::Oid = Spi::get_one(
        "SELECT relowner FROM pg_catalog.pg_class \
         WHERE oid = 'pgrdf._pgrdf_quads'::regclass",
    )
    .expect("ownership_drift: storage owner lookup failed")
    .expect("pgrdf._pgrdf_quads has no owner");
    TableIterator::new(drift_rows_for(owner).into_iter())
}

/// One drift row: relname, kind, owner, storage_owner, blocking, cure.
pub(crate) type DriftRow = (String, String, String, String, bool, String);

/// [`ownership_drift`] computed for `storage_owner`. The exported
/// function passes the real one; tests pass a role with chosen grants,
/// so the blocking rule is checked without re-owning the storage.
pub(crate) fn drift_rows_for(storage_owner: pgrx::pg_sys::Oid) -> Vec<DriftRow> {
    let sql = "WITH so AS (SELECT $1::oid AS oid)
         SELECT * FROM (
         SELECT c.relname::text,
                CASE WHEN c.relkind = 'S' THEN 'sequence'
                     WHEN c.relispartition THEN 'partition'
                     ELSE 'table' END,
                pg_catalog.pg_get_userbyid(c.relowner)::text,
                pg_catalog.pg_get_userbyid(so.oid)::text,
                NOT (pg_catalog.pg_has_role(so.oid, c.relowner, 'USAGE')
                     OR CASE
                        WHEN c.relkind = 'S' AND c.relname = '_pgrdf_graph_id_seq' THEN
                             pg_catalog.has_sequence_privilege(so.oid, c.oid, 'USAGE')
                         AND pg_catalog.has_sequence_privilege(so.oid, c.oid, 'SELECT')
                         AND pg_catalog.has_sequence_privilege(so.oid, c.oid, 'UPDATE')
                        WHEN c.relkind = 'S' AND c.relname = '_pgrdf_graph_act_id_seq' THEN
                             pg_catalog.has_sequence_privilege(so.oid, c.oid, 'USAGE')
                        WHEN c.relkind = 'S' THEN true
                        WHEN c.relispartition THEN false
                        WHEN c.relname = '_pgrdf_graphs' THEN
                             pg_catalog.has_table_privilege(so.oid, c.oid, 'SELECT')
                         AND pg_catalog.has_table_privilege(so.oid, c.oid, 'UPDATE')
                        WHEN c.relname = '_pgrdf_graph_acts' THEN
                             pg_catalog.has_table_privilege(so.oid, c.oid, 'INSERT')
                        -- 0.6.42: the engine writes the count tables as the
                        -- storage owner, and their guard admits nobody else.
                        WHEN c.relname IN ('_pgrdf_graph_counts', '_pgrdf_graph_count_deltas',
                                           '_pgrdf_graph_count_bypass') THEN false
                        ELSE true
                        END),
                pg_catalog.format('ALTER %s pgrdf.%I OWNER TO %I',
                       CASE WHEN c.relkind = 'S' THEN 'SEQUENCE' ELSE 'TABLE' END,
                       c.relname, pg_catalog.pg_get_userbyid(so.oid))
         FROM pg_catalog.pg_class c, so
         WHERE c.relnamespace = 'pgrdf'::regnamespace
           AND c.relkind IN ('r', 'p', 'S')
           AND c.relowner <> so.oid
           AND NOT (c.relkind = 'S' AND EXISTS (
               SELECT 1 FROM pg_catalog.pg_depend d
               WHERE d.classid = 'pg_catalog.pg_class'::regclass
                 AND d.objid = c.oid
                 AND d.refclassid = 'pg_catalog.pg_class'::regclass
                 AND d.deptype IN ('a', 'i')))
         UNION ALL
         SELECT n.nspname::text, 'schema',
                pg_catalog.pg_get_userbyid(n.nspowner)::text,
                pg_catalog.pg_get_userbyid(so.oid)::text,
                NOT (pg_catalog.pg_has_role(so.oid, n.nspowner, 'USAGE')
                     OR (pg_catalog.has_schema_privilege(so.oid, n.oid, 'USAGE')
                         AND pg_catalog.has_schema_privilege(so.oid, n.oid, 'CREATE'))),
                pg_catalog.format('ALTER SCHEMA %I OWNER TO %I',
                       n.nspname, pg_catalog.pg_get_userbyid(so.oid))
         FROM pg_catalog.pg_namespace n, so
         WHERE n.nspname = 'pgrdf' AND n.nspowner <> so.oid
         ) d(relname, kind, owner, storage_owner, blocking, cure)
         ORDER BY kind = 'schema', relname";
    let mut rows = Vec::new();
    Spi::connect(|client| {
        let tup = client
            .select(sql, None, &[storage_owner.into()])
            .unwrap_or_else(|e| panic!("ownership_drift: enumeration failed: {e}"));
        for row in tup {
            rows.push((
                row.get::<String>(1).unwrap().unwrap_or_default(),
                row.get::<String>(2).unwrap().unwrap_or_default(),
                row.get::<String>(3).unwrap().unwrap_or_default(),
                row.get::<String>(4).unwrap().unwrap_or_default(),
                row.get::<bool>(5).unwrap().unwrap_or(true),
                row.get::<String>(6).unwrap().unwrap_or_default(),
            ));
        }
    });
    rows
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

    fn sha_hex(s: &str) -> String {
        Spi::get_one_with_args(
            "SELECT encode(sha256(convert_to($1, 'UTF8')), 'hex')",
            &[s.into()],
        )
        .unwrap()
        .unwrap()
    }

    fn inv_row(iri: &str) -> (Option<String>, Option<i32>, Option<String>) {
        Spi::get_three_with_args(
            "SELECT source_sha256, source_loads, identity_digest \
             FROM pgrdf.graph_inventory() WHERE iri = $1",
            &[iri.into()],
        )
        .unwrap()
    }

    /// #143: the inventory shows each graph's source digest and load count,
    /// and — for a locked graph whose digest was computed — its identity.
    #[pg_test]
    fn inventory_exposes_source_digest_and_identity() {
        let ttl = "<urn:inv:s> <urn:inv:p> <urn:inv:o> .\n";
        let g: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:inv:a')")
            .unwrap()
            .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle_verbose($1, $2)",
            &[ttl.into(), g.into()],
        )
        .unwrap();
        let (sha, loads, ident) = inv_row("urn:tdd:inv:a");
        assert_eq!(sha.as_deref(), Some(sha_hex(ttl).as_str()));
        assert_eq!(loads, Some(1));
        assert_eq!(ident, None, "an open graph has no cached identity");
        Spi::run(&format!("SELECT pgrdf.lock_graph({g}, 'checkpoint')")).unwrap();
        let d: String = Spi::get_one(&format!("SELECT pgrdf.graph_digest({g})"))
            .unwrap()
            .unwrap();
        let (_, _, ident) = inv_row("urn:tdd:inv:a");
        assert_eq!(ident.as_deref(), Some(d.as_str()));
    }

    /// #143: parse_nquads / parse_trig record the source digest — sha256
    /// of the UTF-8 content — when every quad landed in the target graph;
    /// a multi-graph load records nothing and leaves the count unchanged.
    #[pg_test]
    fn quad_parsers_record_source_digest_for_single_graph_loads() {
        let nq = "<urn:nq:s> <urn:nq:p> <urn:nq:o> .\n";
        let g: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:inv:nq')")
            .unwrap()
            .unwrap();
        Spi::run_with_args("SELECT pgrdf.parse_nquads($1, $2)", &[nq.into(), g.into()]).unwrap();
        let (sha, loads, _) = inv_row("urn:tdd:inv:nq");
        assert_eq!(sha.as_deref(), Some(sha_hex(nq).as_str()));
        assert_eq!(loads, Some(1));

        let trig_one = "<urn:tg:s> <urn:tg:p> <urn:tg:o> .\n";
        let gt: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:inv:trig')")
            .unwrap()
            .unwrap();
        Spi::run_with_args(
            "SELECT pgrdf.parse_trig($1, $2)",
            &[trig_one.into(), gt.into()],
        )
        .unwrap();
        let (sha, loads, _) = inv_row("urn:tdd:inv:trig");
        assert_eq!(sha.as_deref(), Some(sha_hex(trig_one).as_str()));
        assert_eq!(loads, Some(1));

        Spi::run("SELECT pgrdf.add_graph('urn:tdd:inv:named')").unwrap();
        let multi = "<urn:tg:s2> <urn:tg:p> <urn:tg:o> .\n\
                     <urn:tdd:inv:named> { <urn:tg:s3> <urn:tg:p> <urn:tg:o> . }\n";
        Spi::run_with_args(
            "SELECT pgrdf.parse_trig($1, $2)",
            &[multi.into(), gt.into()],
        )
        .unwrap();
        let (sha, loads, _) = inv_row("urn:tdd:inv:trig");
        assert_eq!(
            sha.as_deref(),
            Some(sha_hex(trig_one).as_str()),
            "unchanged"
        );
        assert_eq!(
            loads,
            Some(1),
            "a multi-graph load is not this graph's source"
        );
    }

    /// #143: create_graph claims a NEW IRI or refuses 42710; the HINT
    /// names the existing graph and what it was loaded from, so the
    /// caller can choose "reuse" or "rename" without another round trip.
    #[pg_test]
    fn create_graph_refuses_an_existing_iri_with_its_identity() {
        let id: i64 = Spi::get_one("SELECT pgrdf.create_graph('urn:tdd:cg:a')")
            .unwrap()
            .unwrap();
        let ttl = "<urn:cg:s> <urn:cg:p> <urn:cg:o> .\n";
        Spi::run_with_args(
            "SELECT pgrdf.parse_turtle_verbose($1, $2)",
            &[ttl.into(), id.into()],
        )
        .unwrap();
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.cg_try(q text) RETURNS text \
             LANGUAGE plpgsql AS $$ DECLARE st text; h text; BEGIN \
               EXECUTE q; RETURN 'ok'; \
             EXCEPTION WHEN OTHERS THEN \
               GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, h = PG_EXCEPTION_HINT; \
               RETURN st || '|' || coalesce(h, ''); END $$",
        )
        .unwrap();
        let got: String =
            Spi::get_one("SELECT pg_temp.cg_try('SELECT pgrdf.create_graph(''urn:tdd:cg:a'')')")
                .unwrap()
                .unwrap();
        assert!(got.starts_with("42710|"), "{got}");
        assert!(got.contains(&format!("graph_id {id}")), "{got}");
        assert!(
            got.contains(&sha_hex(ttl)),
            "the HINT carries the source digest: {got}"
        );
        let other: i64 = Spi::get_one("SELECT pgrdf.create_graph('urn:tdd:cg:b')")
            .unwrap()
            .unwrap();
        assert_ne!(other, id);
    }

    // ── 0.6.42: stored counts, the count state, one-graph forms, scope ──

    fn n(q: &str) -> Option<i64> {
        Spi::get_one::<i64>(q).unwrap()
    }

    fn t(q: &str) -> Option<String> {
        Spi::get_one::<String>(q).unwrap()
    }

    /// `q` as `role`: its one text value, or the SQLSTATE it raised.
    fn as_role_fn() {
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.inv_as(role text, q text) RETURNS text \
             LANGUAGE plpgsql AS $$ DECLARE r text; BEGIN \
               BEGIN \
                 EXECUTE format('SET LOCAL ROLE %I', role); \
                 EXECUTE q INTO r; \
                 RESET ROLE; \
                 RETURN coalesce(r, '<null>'); \
               EXCEPTION WHEN OTHERS THEN RETURN SQLSTATE; \
               END; \
             END $$",
        )
        .unwrap();
    }

    fn as_role(role: &str, q: &str) -> String {
        Spi::get_one_with_args::<String>("SELECT pg_temp.inv_as($1, $2)", &[role.into(), q.into()])
            .unwrap()
            .unwrap()
    }

    fn app_role(name: &str) {
        Spi::run(&format!("CREATE ROLE {name} NOLOGIN")).unwrap();
        Spi::run(&format!("GRANT USAGE ON SCHEMA pgrdf TO {name}")).unwrap();
        Spi::run(&format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON pgrdf._pgrdf_quads, pgrdf._pgrdf_graphs, \
             pgrdf._pgrdf_dictionary TO {name}"
        ))
        .unwrap();
    }

    const STATE: &str = "concat_ws('|', asserted, inferred, counts_verified, \
                         counts_scanned_at IS NOT NULL, change_seq > 0, last_act_id IS NOT NULL)";

    /// The row says what it stands on: the counts, that they are
    /// verified, when a scan last set them, and the graph's newest act.
    #[pg_test]
    fn inventory_carries_the_count_state() {
        let g = n("SELECT pgrdf.add_graph('urn:inv42:state')").unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> <urn:i:o> .', {g})"
        ))
        .unwrap();
        let row = t(&format!(
            "SELECT {STATE} FROM pgrdf.graph_inventory() WHERE graph_id = {g}"
        ));
        assert_eq!(row.as_deref(), Some("1|0|t|t|t|t"));
    }

    /// A write made in plain SQL on a graph's partition: the inventory
    /// still returns the exact count, and says it is no longer verified.
    /// Another graph is untouched.
    #[pg_test]
    fn a_direct_write_leaves_the_count_exact_and_unverified() {
        let g = n("SELECT pgrdf.add_graph('urn:inv42:direct')").unwrap();
        let h = n("SELECT pgrdf.add_graph('urn:inv42:bystander')").unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> <urn:i:o> .', {g})"
        ))
        .unwrap();
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_quads_g{g} (subject_id, predicate_id, object_id, graph_id) \
             VALUES (5, 6, 7, {g})"
        ))
        .unwrap();
        let row = |g: i64| {
            t(&format!(
                "SELECT concat_ws('|', asserted, counts_verified) \
                   FROM pgrdf.graph_inventory() WHERE graph_id = {g}"
            ))
            .unwrap()
        };
        assert_eq!(row(g), "2|f");
        assert_eq!(row(h), "0|t");
    }

    /// The same write through the parent table names no graph, so every
    /// graph's count is unverified, and every count is still exact.
    #[pg_test]
    fn a_direct_write_through_the_parent_marks_every_graph() {
        let g = n("SELECT pgrdf.add_graph('urn:inv42:parent')").unwrap();
        let h = n("SELECT pgrdf.add_graph('urn:inv42:other')").unwrap();
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id) \
             VALUES (5, 6, 7, {g})"
        ))
        .unwrap();
        let rows = t(&format!(
            "SELECT string_agg(concat_ws('|', asserted, counts_verified), ' ' ORDER BY graph_id) \
               FROM pgrdf.graph_inventory() WHERE graph_id IN ({g}, {h})"
        ));
        assert_eq!(rows.as_deref(), Some("1|f 0|f"));
    }

    /// One graph by id and by IRI: the row the list has; nothing for a
    /// graph that does not exist.
    #[pg_test]
    fn one_graph_by_id_and_by_iri() {
        let g = n("SELECT pgrdf.add_graph('urn:inv42:one')").unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> <urn:i:o> .', {g})"
        ))
        .unwrap();
        let listed = t(&format!(
            "SELECT to_jsonb(i)::text FROM pgrdf.graph_inventory() i WHERE graph_id = {g}"
        ));
        assert!(listed.is_some());
        let by_id = t(&format!(
            "SELECT to_jsonb(i)::text FROM pgrdf.graph_inventory({g}) i"
        ));
        let by_iri = t("SELECT to_jsonb(i)::text FROM pgrdf.graph_inventory('urn:inv42:one') i");
        assert_eq!(by_id, listed);
        assert_eq!(by_iri, listed);
        assert_eq!(
            n("SELECT count(*) FROM pgrdf.graph_inventory(987654321)"),
            Some(0)
        );
        assert_eq!(
            n("SELECT count(*) FROM pgrdf.graph_inventory('urn:inv42:no-such')"),
            Some(0)
        );
    }

    /// The listing follows the role: an admin sees every graph, any other
    /// role its own. Asking for one graph gives the row, 42501 for a graph
    /// that is somebody else's, and nothing for one that does not exist.
    #[pg_test]
    fn a_role_is_listed_its_own_graphs() {
        as_role_fn();
        // First of all, so this transaction queues on the partition DDL
        // gate before it holds any lock on the quad table: the GRANTs
        // below take one, and a graph being created elsewhere needs the
        // table exclusively.
        let old = n("SELECT pgrdf.add_graph('urn:inv42:ownerless')").unwrap();
        app_role("inv42_alice");
        app_role("inv42_bob");
        Spi::run("CREATE ROLE inv42_reader NOLOGIN").unwrap();
        Spi::run("GRANT USAGE ON SCHEMA pgrdf TO inv42_reader").unwrap();
        Spi::run("GRANT pg_read_all_data TO inv42_reader").unwrap();
        // Holds the role, not its privileges: it may SET ROLE to it, and
        // until it does it reads no more than before.
        app_role("inv42_lazy");
        Spi::run("GRANT pg_read_all_data TO inv42_lazy WITH INHERIT FALSE").unwrap();

        assert_ne!(
            as_role(
                "inv42_alice",
                "SELECT pgrdf.add_graph('urn:inv42:of-alice')::text"
            ),
            "42501"
        );
        as_role(
            "inv42_bob",
            "SELECT pgrdf.add_graph('urn:inv42:of-bob')::text",
        );
        let bob_graph = n("SELECT pgrdf.graph_id('urn:inv42:of-bob')").unwrap();
        as_role(
            "inv42_bob",
            &format!(
                "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> <urn:i:o> .', {bob_graph})::text"
            ),
        );
        // A graph nobody is recorded as owning (created before 0.6.41).
        Spi::run(&format!(
            "UPDATE pgrdf._pgrdf_graphs SET owner = NULL WHERE graph_id = {old}"
        ))
        .unwrap();

        let listed = "SELECT coalesce(string_agg(iri, ' ' ORDER BY iri), '') \
                        FROM pgrdf.graph_inventory() WHERE iri LIKE 'urn:inv42:o%'";
        let all = "urn:inv42:of-alice urn:inv42:of-bob urn:inv42:ownerless";
        assert_eq!(t(listed).as_deref(), Some(all), "the storage owner");
        assert_eq!(as_role("inv42_reader", listed), all, "pg_read_all_data");
        assert_eq!(as_role("inv42_alice", listed), "urn:inv42:of-alice");
        assert_eq!(as_role("inv42_bob", listed), "urn:inv42:of-bob");
        assert_eq!(as_role("inv42_lazy", listed), "", "INHERIT FALSE: not held");

        let one = |who: &str, what: &str| {
            as_role(
                who,
                &format!(
                    "SELECT count(*)::text || '|' || coalesce(max(asserted), -1) FROM pgrdf.graph_inventory({what})"
                ),
            )
        };
        assert_eq!(one("inv42_bob", &bob_graph.to_string()), "1|1", "yours");
        assert_eq!(
            one("inv42_bob", "'urn:inv42:of-bob'"),
            "1|1",
            "yours, by IRI"
        );
        assert_eq!(
            one("inv42_alice", &bob_graph.to_string()),
            "42501",
            "not yours"
        );
        assert_eq!(
            one("inv42_alice", "'urn:inv42:of-bob'"),
            "42501",
            "not yours, by IRI"
        );
        assert_eq!(one("inv42_alice", &old.to_string()), "42501", "nobody's");
        assert_eq!(one("inv42_alice", "987654321"), "0|-1", "no such graph");
        assert_eq!(
            one("inv42_reader", &bob_graph.to_string()),
            "1|1",
            "an admin"
        );
    }

    /// A count that a direct write left unverified is cured by a recount,
    /// which says what it found and what was stored, and is an act.
    #[pg_test]
    fn a_recount_restores_a_verified_count() {
        let g = n("SELECT pgrdf.add_graph('urn:inv42:recount')").unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> <urn:i:o> .', {g})"
        ))
        .unwrap();
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_quads_g{g} (subject_id, predicate_id, object_id, graph_id) \
             VALUES (5, 6, 7, {g})"
        ))
        .unwrap();
        let verified = |g: i64| {
            t(&format!(
                "SELECT concat_ws('|', asserted, counts_verified) FROM pgrdf.graph_inventory({g})"
            ))
            .unwrap()
        };
        assert_eq!(verified(g), "2|f");
        let integrity = |g: i64| {
            t(&format!(
                "SELECT concat_ws('|', j -> 'stored_count' ->> 'verified', \
                                  j -> 'stored_count' ->> 'consistent') \
                   FROM pgrdf.graph_integrity({g}) j"
            ))
            .unwrap()
        };
        assert_eq!(
            integrity(g),
            "false|true",
            "unverified says so; nothing is hidden"
        );
        let r = t(&format!(
            "SELECT concat_ws('|', graph_id = {g}, asserted, inferred, was_asserted, was_verified) \
               FROM pgrdf.recount_graph({g})"
        ));
        assert_eq!(r.as_deref(), Some("t|2|0|1|f"));
        assert_eq!(verified(g), "2|t");
        assert_eq!(integrity(g), "true|true");
        assert_eq!(
            t(&format!(
                "SELECT act FROM pgrdf.graph_acts({g}) ORDER BY act_id DESC LIMIT 1"
            ))
            .as_deref(),
            Some("recount")
        );
        // A direct write after the recount is a new bypass.
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_quads_g{g} (subject_id, predicate_id, object_id, graph_id) \
             VALUES (5, 6, 8, {g})"
        ))
        .unwrap();
        assert_eq!(verified(g), "3|f");
        // A write that rolls back leaves no mark behind.
        Spi::run(&format!("SELECT pgrdf.recount_graph({g})")).unwrap();
        as_role_fn();
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.undo(q text) RETURNS void LANGUAGE plpgsql AS $$ \
             BEGIN EXECUTE q; RAISE EXCEPTION 'undo'; EXCEPTION WHEN OTHERS THEN NULL; END $$",
        )
        .unwrap();
        Spi::run(&format!(
            "SELECT pg_temp.undo('INSERT INTO pgrdf._pgrdf_quads_g{g} \
               (subject_id, predicate_id, object_id, graph_id) VALUES (5, 6, 9, {g})')"
        ))
        .unwrap();
        assert_eq!(verified(g), "3|t");
    }

    /// recount_graph(NULL) cures every graph a parent-level write marked.
    #[pg_test]
    fn a_recount_of_everything_cures_a_write_through_the_parent() {
        let g = n("SELECT pgrdf.add_graph('urn:inv42:all-a')").unwrap();
        let h = n("SELECT pgrdf.add_graph('urn:inv42:all-b')").unwrap();
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id) \
             VALUES (5, 6, 7, {g})"
        ))
        .unwrap();
        let unverified = "SELECT unverified_graphs FROM pgrdf.graph_totals()";
        assert!(n(unverified).unwrap() >= 2);
        let cured = n("SELECT count(*) FROM pgrdf.recount_graph()").unwrap();
        assert!(cured >= 2, "every listed graph is recounted");
        assert_eq!(n(unverified), Some(0));
        let rows = t(&format!(
            "SELECT string_agg(concat_ws('|', asserted, counts_verified), ' ' ORDER BY graph_id) \
               FROM pgrdf.graph_inventory() WHERE graph_id IN ({g}, {h})"
        ));
        assert_eq!(rows.as_deref(), Some("1|t 0|t"));
        // The spent marks are gone: nothing is left for a reader to weigh.
        assert_eq!(
            n("SELECT count(*) FROM pgrdf._pgrdf_graph_count_bypass"),
            Some(0)
        );
    }

    #[pg_test]
    fn a_recount_refuses_what_is_not_listed_or_not_there() {
        as_role_fn();
        let g = n("SELECT pgrdf.add_graph('urn:inv42:not-yours')").unwrap();
        app_role("inv42_carol");
        assert_eq!(
            as_role(
                "inv42_carol",
                &format!("SELECT count(*)::text FROM pgrdf.recount_graph({g})")
            ),
            "42501"
        );
        assert_eq!(
            as_role(
                "inv42_carol",
                "SELECT count(*)::text FROM pgrdf.recount_graph(987654321)"
            ),
            "42704"
        );
        assert_eq!(
            as_role(
                "inv42_carol",
                "SELECT count(*)::text FROM pgrdf.recount_graph()"
            ),
            "0",
            "nothing listed, nothing recounted"
        );
        // Reading everything is not acting on everything: the role sees
        // the graph and may not leave an act on it.
        Spi::run("CREATE ROLE inv42_auditor NOLOGIN").unwrap();
        Spi::run("GRANT USAGE ON SCHEMA pgrdf TO inv42_auditor").unwrap();
        Spi::run("GRANT pg_read_all_data TO inv42_auditor").unwrap();
        assert_eq!(
            as_role(
                "inv42_auditor",
                &format!("SELECT count(*)::text FROM pgrdf.graph_inventory({g})")
            ),
            "1"
        );
        assert_eq!(
            as_role(
                "inv42_auditor",
                &format!("SELECT count(*)::text FROM pgrdf.recount_graph({g})")
            ),
            "42501"
        );
    }

    /// Totals state the scope they cover and never pass a part for the whole.
    #[pg_test]
    fn totals_state_their_scope() {
        as_role_fn();
        let g = n("SELECT pgrdf.add_graph('urn:inv42:total-admin')").unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> <urn:i:o> . <urn:i:s> <urn:i:p> <urn:i:x> .', {g})"
        ))
        .unwrap();
        app_role("inv42_dave");
        as_role(
            "inv42_dave",
            "SELECT pgrdf.add_graph('urn:inv42:total-dave')::text",
        );
        let dave_graph = n("SELECT pgrdf.graph_id('urn:inv42:total-dave')").unwrap();
        as_role(
            "inv42_dave",
            &format!(
                "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> <urn:i:o> .', {dave_graph})::text"
            ),
        );
        let all = t(
            "SELECT concat_ws('|', scope, graphs_listed = graphs_total, \
                            asserted = (SELECT count(*) FROM pgrdf._pgrdf_quads WHERE NOT is_inferred), \
                            unverified_graphs) FROM pgrdf.graph_totals()",
        );
        assert_eq!(all.as_deref(), Some("all|t|t|0"));
        let total = n("SELECT graphs_total FROM pgrdf.graph_totals()").unwrap();
        assert_eq!(
            as_role(
                "inv42_dave",
                "SELECT concat_ws('|', scope, graphs_listed, graphs_total, asserted, inferred, \
                                  coalesce(terms::text, 'null')) FROM pgrdf.graph_totals()"
            ),
            format!("own|1|{total}|1|0|null")
        );
    }

    /// The feed always answers with a cursor, and leaves out the acts of
    /// the transaction that is asking: those can still roll back.
    #[pg_test]
    fn the_act_feed_carries_a_cursor_and_hides_open_acts() {
        let g = n("SELECT pgrdf.add_graph('urn:inv42:feed')").unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:i:s> <urn:i:p> <urn:i:o> .', {g})"
        ))
        .unwrap();
        assert!(
            n(&format!("SELECT count(*) FROM pgrdf.graph_acts({g})")).unwrap() >= 2,
            "the acts are there for graph_acts"
        );
        let row = t(&format!(
            "SELECT concat_ws('|', count(*), count(act_id), count(complete_below), \
                              count(next_page)) FROM pgrdf.graph_act_feed(NULL, {g})"
        ));
        assert_eq!(row.as_deref(), Some("1|0|1|0"));
    }

    /// The count tables are the engine's. A role granted every table of the
    /// schema, as the install guide's grant does, can neither set a count
    /// nor remove the mark that says a count is unverified.
    #[pg_test]
    fn a_granted_role_cannot_write_the_count_tables() {
        as_role_fn();
        let g = n("SELECT pgrdf.add_graph('urn:inv42:guarded')").unwrap();
        Spi::run("CREATE ROLE inv42_granted NOLOGIN").unwrap();
        Spi::run("GRANT USAGE ON SCHEMA pgrdf TO inv42_granted").unwrap();
        Spi::run(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pgrdf TO inv42_granted",
        )
        .unwrap();
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_quads_g{g} (subject_id, predicate_id, object_id, graph_id) \
             VALUES (5, 6, 7, {g})"
        ))
        .unwrap();
        for forged in [
            format!(
                "UPDATE pgrdf._pgrdf_graph_counts SET asserted = 1000 WHERE graph_id = {g} RETURNING 'done'"
            ),
            format!("DELETE FROM pgrdf._pgrdf_graph_counts WHERE graph_id = {g} RETURNING 'done'"),
            "DELETE FROM pgrdf._pgrdf_graph_count_bypass RETURNING 'done'".to_string(),
            format!(
                "INSERT INTO pgrdf._pgrdf_graph_count_bypass (graph_id) VALUES ({g}) RETURNING 'done'"
            ),
            format!(
                "INSERT INTO pgrdf._pgrdf_graph_count_deltas (graph_id, d_asserted, d_inferred) VALUES ({g}, 9, 0) RETURNING 'done'"
            ),
        ] {
            assert_eq!(as_role("inv42_granted", &forged), "42501", "{forged}");
        }
        assert_eq!(
            t(&format!(
                "SELECT concat_ws('|', asserted, counts_verified) FROM pgrdf.graph_inventory({g})"
            ))
            .as_deref(),
            Some("1|f"),
            "the direct write is still on record"
        );
        // And the role's own engine writes are counted as anyone's.
        Spi::run("SELECT pgrdf.recount_graph()").unwrap();
        assert_ne!(
            as_role(
                "inv42_granted",
                &format!("SELECT pgrdf.put_quad(5, 6, 8, {g})::text")
            ),
            "42501"
        );
        assert_eq!(
            as_role(
                "inv42_granted",
                &format!(
                    "DELETE FROM pgrdf._pgrdf_graph_count_deltas WHERE graph_id = {g} RETURNING 'done'"
                )
            ),
            "42501"
        );
        assert_eq!(
            t(&format!(
                "SELECT concat_ws('|', asserted, counts_verified) FROM pgrdf.graph_inventory({g})"
            ))
            .as_deref(),
            Some("2|t")
        );
    }

    /// change_seq never goes back: a recount takes the marks of the direct
    /// writes it cures with it.
    #[pg_test]
    fn change_seq_never_goes_back() {
        let g = n("SELECT pgrdf.add_graph('urn:inv42:seq')").unwrap();
        let seq = |g: i64| {
            n(&format!(
                "SELECT change_seq FROM pgrdf.graph_inventory({g})"
            ))
            .unwrap()
        };
        let mut last = seq(g);
        let mut step = |what: &str, q: String| {
            Spi::run(&q).unwrap();
            let now = seq(g);
            assert!(now > last, "{what}: change_seq went from {last} to {now}");
            last = now;
        };
        step(
            "an engine write",
            format!("SELECT pgrdf.put_quad(1, 2, 3, {g})"),
        );
        for o in 4..=8 {
            step(
                "a direct write",
                format!(
                    "INSERT INTO pgrdf._pgrdf_quads_g{g} (subject_id, predicate_id, object_id, graph_id) \
                     VALUES (1, 2, {o}, {g})"
                ),
            );
        }
        step(
            "the recount that cures them",
            format!("SELECT pgrdf.recount_graph({g})"),
        );
        step("a clear", format!("SELECT pgrdf.clear_graph({g})"));
    }
}
