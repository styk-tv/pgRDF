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
        name!(source_sha256, Option<String>),
        name!(source_loads, Option<i32>),
        name!(identity_digest, Option<String>),
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
                        END AS materialization,
                        g.source_sha256,
                        g.source_loads,
                        -- 0.6.37: a locked graph's cached rdfc-1.0 digest,
                        -- shown only while lock custody holds (both
                        -- partition triggers present) — the same rule
                        -- graph_digest uses to trust its cache.
                        CASE WHEN g.locked AND (
                               SELECT count(*) FROM pg_trigger t
                               JOIN pg_class c ON c.oid = t.tgrelid
                               WHERE c.relnamespace = 'pgrdf'::regnamespace
                                 AND c.relname = format('_pgrdf_quads_g%s', g.graph_id)
                                 AND t.tgname IN ('pgrdf_lock_row', 'pgrdf_lock_truncate')
                             ) = 2
                             THEN g.locked_digest END AS identity_digest
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
                row.get::<String>(8).unwrap(),
                row.get::<i32>(9).unwrap(),
                row.get::<String>(10).unwrap(),
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

/// Storage relations not owned by the storage owner (#153, 0.6.40).
///
/// The storage owner is the owner of `_pgrdf_quads`: graph creation,
/// clear and drop act as that role. A consumer that hands pgRDF's tables
/// to its own role (one ALTER ... OWNER per relation) leaves behind any
/// relation a later release adds, and any partition someone else
/// created; acts on those can then be refused. One row per drifted
/// table, partition or standalone sequence (a sequence linked to a
/// column follows its table). `blocking` is true when the storage owner
/// holds neither the relation owner's rights nor, for a sequence,
/// USAGE, SELECT and UPDATE on it. `cure` is the statement that
/// realigns it. Empty when everything agrees: empty is the answer.
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
    let mut rows = Vec::new();
    Spi::connect(|client| {
        let tup = client
            .select(
                "WITH so AS (
                     SELECT relowner AS oid FROM pg_catalog.pg_class
                     WHERE oid = 'pgrdf._pgrdf_quads'::regclass)
                 SELECT c.relname::text,
                        CASE WHEN c.relkind = 'S' THEN 'sequence'
                             WHEN c.relispartition THEN 'partition'
                             ELSE 'table' END,
                        pg_catalog.pg_get_userbyid(c.relowner)::text,
                        pg_catalog.pg_get_userbyid(so.oid)::text,
                        NOT (pg_catalog.pg_has_role(so.oid, c.relowner, 'USAGE')
                             OR (c.relkind = 'S'
                                 AND pg_catalog.has_sequence_privilege(so.oid, c.oid, 'USAGE')
                                 AND pg_catalog.has_sequence_privilege(so.oid, c.oid, 'SELECT')
                                 AND pg_catalog.has_sequence_privilege(so.oid, c.oid, 'UPDATE'))),
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
                 ORDER BY 1",
                None,
                &[],
            )
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
}
