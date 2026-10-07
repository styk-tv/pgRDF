//! Serialised `_pgrdf_quads` partition DDL.
//!
//! `CREATE TABLE … PARTITION OF pgrdf._pgrdf_quads …` takes an
//! `AccessExclusiveLock` on the partitioned **parent**
//! `pgrdf._pgrdf_quads`. When two sessions each already hold
//! row/relation locks on the parent (every `add_graph` /
//! partition-creating test fixture does) and both try to escalate to
//! that parent-level `AccessExclusiveLock` at once, Postgres detects a
//! deadlock and aborts one with `deadlock detected`.
//!
//! The pgrx test harness spawns one Postgres backend per worker thread
//! against a **shared** `$PGDATA`, so parallel `#[pg_test]`s that each
//! create a partition race exactly this window. The historical
//! mitigation was `RUST_TEST_THREADS=1` in CI (commit `84d4efc`), which
//! removed the race but tripled the test-job wall time.
//!
//! This module replaces that workaround with a **transaction-scoped
//! advisory lock**: every partition-creating path serialises through a
//! single fixed advisory key, so concurrent callers *queue* on the
//! advisory lock instead of *deadlocking* on the parent's catalog lock.
//! `pg_advisory_xact_lock` is released automatically at transaction end
//! (the pgrx `#[pg_test]` auto-rollback boundary), so no lock leaks
//! across test cases — unlike a session-scoped lock which would.
//!
//! Reference: this is the infra fix described alongside the earlier CI
//! green-rescue commits `84d4efc` / `d464214`; it restores parallel
//! test threads.

use pgrx::prelude::*;

/// Fixed advisory-lock key gating **all** `_pgrdf_quads` partition
/// creation. The value is the ASCII bytes of `"pgrd"` packed
/// big-endian into the low 32 bits (`0x70='p' 0x67='g' 0x72='r'
/// 0x64='d'`) — an arbitrary but stable, self-documenting constant.
/// Any other partition-DDL path in the codebase MUST take this same
/// key (via [`create_quads_partition`] or
/// [`create_quads_partition_named`]) or the serialisation guarantee
/// is void.
///
/// Digit grouping is `0x_7067_7264` rather than `0x70_67_72_64` only
/// to keep clippy's `mistyped_literal_suffixes` lint from misreading
/// the trailing `_64` as a malformed `i64` suffix; the numeric value
/// is identical (`1_886_613_604`).
const PARTITION_DDL_LOCK_KEY: i64 = 0x_7067_7264; // "pgrd"

/// Does a relation named `part_name` already exist in the `pgrdf`
/// schema? Used as both the lock-free fast path and the
/// under-lock re-check.
fn partition_exists(part_name: &str) -> bool {
    Spi::get_one_with_args::<bool>(
        "SELECT EXISTS(
            SELECT 1 FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relname = $1
         )",
        &[part_name.into()],
    )
    .expect("create_quads_partition: existence check failed")
    .unwrap_or(false)
}

/// Acquire the shared partition-DDL gate explicitly, *without*
/// creating anything.
///
/// **Why this exists — global lock-order discipline.** The `add_graph`
/// family has TWO serialisation points that can deadlock under
/// parallel callers:
///
/// 1. The `_pgrdf_quads` parent's `AccessExclusiveLock` (taken by
///    `CREATE TABLE … PARTITION OF`).
/// 2. `_pgrdf_graphs`' row/table lock (the IRI-keyed overloads do
///    `LOCK TABLE pgrdf._pgrdf_graphs IN SHARE ROW EXCLUSIVE MODE`,
///    and the integer overload `INSERT … ON CONFLICT`s into it).
///
/// If one caller takes the advisory gate then `_pgrdf_graphs`, while
/// another takes `_pgrdf_graphs` then (via re-entry) the advisory
/// gate, Postgres deadlocks on `_pgrdf_graphs` instead of the parent
/// — the exact failure observed when only [`create_quads_partition`]
/// took the gate. The cure is a single global order: **the advisory
/// gate is always the OUTERMOST lock**. Every `add_graph` overload
/// calls this first, before any `_pgrdf_graphs` lock/insert, so all
/// paths agree on `advisory → _pgrdf_graphs → partition-catalog`.
///
/// `pg_advisory_xact_lock` is re-entrant within a transaction: the
/// nested acquisition inside [`create_quads_partition`] just bumps
/// the hold count and returns immediately, so calling this up-front
/// is cheap and correct.
pub(crate) fn acquire_partition_ddl_gate() {
    Spi::run_with_args(
        "SELECT pg_advisory_xact_lock($1)",
        &[PARTITION_DDL_LOCK_KEY.into()],
    )
    .expect("acquire_partition_ddl_gate: pg_advisory_xact_lock failed");
}

/// Core serialised partition-creation routine.
///
/// `part_name` is the *unqualified* relation name (no schema prefix,
/// no quoting); it is always constructed by callers from a validated
/// non-negative `BIGINT`, so there is no user input in the SQL
/// identifier position. `graph_id` is the `FOR VALUES IN (…)` list
/// value.
///
/// Flow (see module docs for the why):
///
/// 1. **Fast path** — lock-free `pg_class` existence check. The
///    overwhelmingly common case is "partition already exists"
///    (`add_graph` is idempotent and called repeatedly); short-
///    circuiting here keeps callers off the advisory lock entirely so
///    they never serialise unnecessarily.
/// 2. **Slow path** — `pg_advisory_xact_lock(PARTITION_DDL_LOCK_KEY)`.
///    Concurrent creators now queue here instead of racing the
///    parent's `AccessExclusiveLock`.
/// 3. **Re-check under the lock** — another session may have created
///    the partition while we waited on the advisory lock. The lock
///    makes the check+create atomic, so the loser of the race must
///    observe the partition and skip its own `CREATE`.
/// 4. `CREATE TABLE IF NOT EXISTS … PARTITION OF …` — belt-and-
///    suspenders. `IF NOT EXISTS` on `CREATE TABLE … PARTITION OF`
///    is valid on every supported server (PG 14–17; supported since
///    PG 10). With the advisory lock + re-check it should never
///    actually fire, but it removes the last theoretical window.
fn create_partition_impl(part_name: &str, graph_id: i64) {
    // (1) Fast path: already there → nothing to do, no lock taken.
    if partition_exists(part_name) {
        return;
    }

    // (2) Slow path: serialise the DDL critical section. Transaction-
    // scoped (NOT session-scoped) so it releases at the pgrx
    // #[pg_test] rollback boundary and never leaks across cases.
    // Re-entrant if the caller already took the gate up-front (the
    // `add_graph` overloads do — see `acquire_partition_ddl_gate`).
    acquire_partition_ddl_gate();

    // (3) Re-check under the lock — the partition may have appeared
    // while we were queued on the advisory lock.
    if partition_exists(part_name) {
        return;
    }

    // (4) Authority: creating a partition is authorised by the caller's
    // INSERT on the quad + graph tables, not by owning the parent
    // (SPEC 0.6.37 §3.1). Refuses 42501 with the cure before any DDL.
    require_graph_ddl_privilege(GraphDdl::Create, "add_graph");
    // Step (7) below moves the graph-id sequence as the storage owner;
    // refuse with the cure before any DDL if that owner cannot (#153).
    if graph_id_seq_present() {
        require_graph_id_seq_usable("add_graph");
    }

    // (5) Create it, as the storage owner. `part_name` is caller-built
    // from a BIGINT (no user input in identifier position); `graph_id`
    // is a constant in the LIST value position which Postgres accepts
    // in DDL.
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS pgrdf.{} \
         PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN ({})",
        part_name, graph_id
    );
    as_storage_owner(|| {
        Spi::run(&sql).expect("create_quads_partition: CREATE TABLE failed");
        // (6) Replicate the parent's ACL onto the new partition.
        inherit_parent_acl(part_name);
        // (7) #150: advance the graph-id sequence past this id, so an id
        // bound explicitly (add_graph(id), add_graph(id, iri)) is never
        // allocated later. Under the DDL gate, so concurrent creates
        // cannot move the mark backwards. Skipped while the extension's
        // SQL predates the sequence (library swapped, ALTER EXTENSION
        // UPDATE not yet run); the upgrade script seeds it past every id.
        if !graph_id_seq_present() {
            return;
        }
        Spi::run_with_args(
            "SELECT pg_catalog.setval('pgrdf._pgrdf_graph_id_seq', $1) \
              WHERE $1 > (SELECT last_value FROM pgrdf._pgrdf_graph_id_seq)",
            &[graph_id.into()],
        )
        .expect("create_quads_partition: advancing the graph-id sequence failed");
    });
}

/// Does `pgrdf._pgrdf_graph_id_seq` exist? It arrives with the 0.6.39 SQL;
/// a library newer than the installed SQL runs without it until
/// `ALTER EXTENSION pgrdf UPDATE`.
pub(crate) fn graph_id_seq_present() -> bool {
    Spi::get_one::<bool>("SELECT pg_catalog.to_regclass('pgrdf._pgrdf_graph_id_seq') IS NOT NULL")
        .expect("graph-id sequence lookup failed")
        .unwrap_or(false)
}

/// Can `role` allocate graph ids? Graph creation runs `nextval`, a read
/// of `last_value` and `setval` on `_pgrdf_graph_id_seq` as the storage
/// owner, so that role needs the sequence owner's rights, or USAGE,
/// SELECT and UPDATE on it. USAGE alone passes `nextval` and then
/// refuses at `setval` (#153).
pub(crate) fn graph_id_seq_usable_by(role: pgrx::pg_sys::Oid) -> bool {
    Spi::get_one_with_args::<bool>(
        "SELECT pg_catalog.pg_has_role($1, s.relowner, 'USAGE') \
             OR (pg_catalog.has_sequence_privilege($1, s.oid, 'USAGE') \
                 AND pg_catalog.has_sequence_privilege($1, s.oid, 'SELECT') \
                 AND pg_catalog.has_sequence_privilege($1, s.oid, 'UPDATE')) \
           FROM pg_catalog.pg_class s \
          WHERE s.oid = 'pgrdf._pgrdf_graph_id_seq'::regclass",
        &[role.into()],
    )
    .expect("graph-id sequence privilege check failed")
    .unwrap_or(false)
}

/// Refuse graph creation, with the cure, when the storage owner cannot
/// use the graph-id sequence (#153). The 0.6.39 upgrade created the
/// sequence owned by whoever ran `ALTER EXTENSION UPDATE`; where the
/// storage tables had been given to another role, every new graph then
/// failed with a bare 42501 on `nextval`. 55000 names the state and the
/// HINT carries the one statement that fixes it.
pub(crate) fn require_graph_id_seq_usable(fn_name: &str) {
    let (owner, owner_name, seq_owner) = Spi::get_three::<pgrx::pg_sys::Oid, String, String>(
        "SELECT q.relowner, \
                pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(q.relowner)::text), \
                pg_catalog.quote_ident(pg_catalog.pg_get_userbyid(s.relowner)::text) \
           FROM pg_catalog.pg_class q, pg_catalog.pg_class s \
          WHERE q.oid = 'pgrdf._pgrdf_quads'::regclass \
            AND s.oid = 'pgrdf._pgrdf_graph_id_seq'::regclass",
    )
    .expect("graph-id sequence owner lookup failed");
    let owner = owner.expect("pgrdf._pgrdf_quads has no owner");
    if graph_id_seq_usable_by(owner) {
        return;
    }
    let owner_name = owner_name.unwrap_or_default();
    crate::refuse_with_hint(
        pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_OBJECT_NOT_IN_PREREQUISITE_STATE,
        format!(
            "{fn_name}: the storage owner {owner_name} cannot use the graph-id sequence \
             pgrdf._pgrdf_graph_id_seq (owned by {}), so no graph id can be allocated",
            seq_owner.unwrap_or_default()
        ),
        format!("ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO {owner_name}"),
    );
}

/// The graph-DDL acts, each authorised by a table privilege the caller
/// already holds. Create and drop change both `_pgrdf_quads` and
/// `_pgrdf_graphs`; clear empties the graph's partition and leaves its
/// `_pgrdf_graphs` row alone, so it asks for the quad table only.
#[derive(Clone, Copy)]
pub(crate) enum GraphDdl {
    Create,
    Drop,
    Clear,
}

impl GraphDdl {
    fn privilege(self) -> &'static str {
        match self {
            GraphDdl::Create => "INSERT",
            GraphDdl::Drop | GraphDdl::Clear => "DELETE",
        }
    }
    fn act(self) -> &'static str {
        match self {
            GraphDdl::Create => "creating",
            GraphDdl::Drop => "dropping",
            GraphDdl::Clear => "clearing",
        }
    }
    fn tables(self) -> &'static str {
        match self {
            GraphDdl::Create | GraphDdl::Drop => "pgrdf._pgrdf_quads, pgrdf._pgrdf_graphs",
            GraphDdl::Clear => "pgrdf._pgrdf_quads",
        }
    }
    fn tables_prose(self) -> &'static str {
        match self {
            GraphDdl::Create | GraphDdl::Drop => "pgrdf._pgrdf_quads and pgrdf._pgrdf_graphs",
            GraphDdl::Clear => "pgrdf._pgrdf_quads",
        }
    }
}

/// Does `current_user` hold the privilege that authorises `kind`?
fn holds_graph_ddl_privilege(kind: GraphDdl) -> bool {
    // SELECT too: every graph-DDL path reads before it writes. A comma
    // list in has_table_privilege means ANY, so each privilege is asked
    // separately.
    let sql = match kind {
        GraphDdl::Create | GraphDdl::Drop => {
            "SELECT has_table_privilege('pgrdf._pgrdf_quads', 'SELECT') \
                AND has_table_privilege('pgrdf._pgrdf_graphs', 'SELECT') \
                AND has_table_privilege('pgrdf._pgrdf_quads', $1) \
                AND has_table_privilege('pgrdf._pgrdf_graphs', $1)"
        }
        GraphDdl::Clear => {
            "SELECT has_table_privilege('pgrdf._pgrdf_quads', 'SELECT') \
                AND has_table_privilege('pgrdf._pgrdf_quads', $1)"
        }
    };
    Spi::get_one_with_args::<bool>(sql, &[kind.privilege().into()])
        .expect("graph DDL privilege check failed")
        .unwrap_or(false)
}

/// Refuse 42501 unless `current_user` holds the privilege for `kind`.
/// The HINT is the exact GRANT that cures it.
pub(crate) fn require_graph_ddl_privilege(kind: GraphDdl, caller: &str) {
    if holds_graph_ddl_privilege(kind) {
        return;
    }
    let role: String = Spi::get_one("SELECT quote_ident(current_user::text)")
        .expect("current_user lookup failed")
        .unwrap_or_default();
    let privilege = kind.privilege();
    crate::refuse_with_hint(
        pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
        format!(
            "{caller}: {} a graph requires SELECT and {privilege} on {}; \
             role {role} does not hold them",
            kind.act(),
            kind.tables_prose()
        ),
        format!("GRANT SELECT, {privilege} ON {} TO {role}", kind.tables()),
    );
}

/// Run `f` as the owner of `_pgrdf_quads`, for partition DDL only.
///
/// PostgreSQL demands ownership of the parent to create, attach or drop
/// a partition, and no GRANT confers it. Callers authorise themselves
/// first with [`require_graph_ddl_privilege`]; this then switches the
/// user id the way PostgreSQL does for maintenance commands:
/// `SECURITY_RESTRICTED_OPERATION` + `SECURITY_LOCAL_USERID_CHANGE`, a
/// new GUC nest level, and `search_path = pg_catalog, pg_temp`. The
/// caller is restored in a `finally`, so an ERROR caught in the same
/// transaction (PgTryBuilder) never leaves the session elevated. `f`
/// must run fixed, engine-built statements only. This is the one
/// elevation in the add/clear/drop path, disclosed in the elevation census.
pub(crate) fn as_storage_owner<R>(f: impl FnOnce() -> R) -> R {
    let owner: pgrx::pg_sys::Oid = Spi::get_one(
        "SELECT relowner FROM pg_catalog.pg_class \
         WHERE oid = 'pgrdf._pgrdf_quads'::regclass",
    )
    .expect("storage owner lookup failed")
    .expect("pgrdf._pgrdf_quads has no owner");
    let mut save_userid = pgrx::pg_sys::InvalidOid;
    let mut save_sec: std::ffi::c_int = 0;
    let nest_level;
    unsafe {
        pgrx::pg_sys::GetUserIdAndSecContext(&mut save_userid, &mut save_sec);
        pgrx::pg_sys::SetUserIdAndSecContext(
            owner,
            save_sec
                | (pgrx::pg_sys::SECURITY_LOCAL_USERID_CHANGE
                    | pgrx::pg_sys::SECURITY_RESTRICTED_OPERATION)
                    as std::ffi::c_int,
        );
        nest_level = pgrx::pg_sys::NewGUCNestLevel();
    }
    pgrx::PgTryBuilder::new(std::panic::AssertUnwindSafe(|| {
        Spi::run("SET LOCAL search_path = pg_catalog, pg_temp")
            .expect("owner switch: restricting search_path failed");
        f()
    }))
    .finally(|| unsafe {
        pgrx::pg_sys::AtEOXact_GUC(false, nest_level);
        pgrx::pg_sys::SetUserIdAndSecContext(save_userid, save_sec);
    })
    .execute()
}

/// Does `current_user` hold the privileges of the storage owner (the
/// owner of `_pgrdf_quads`)? The staged loader is an owner lane: its
/// internal DDL (staging tables, CTAS, ATTACH) runs in background workers
/// as the caller, which needs this.
pub(crate) fn caller_owns_storage() -> bool {
    Spi::get_one::<bool>(
        "SELECT pg_has_role(current_user, c.relowner, 'USAGE') \
           FROM pg_catalog.pg_class c WHERE c.oid = 'pgrdf._pgrdf_quads'::regclass",
    )
    .expect("storage owner check failed")
    .unwrap_or(false)
}

/// Can `current_user` create graphs? True when it holds SELECT and INSERT on both
/// `_pgrdf_quads` and `_pgrdf_graphs` — the exact rule `add_graph`
/// enforces. Consumers read this instead of inferring from ownership.
#[pg_extern]
fn can_create_graphs() -> bool {
    holds_graph_ddl_privilege(GraphDdl::Create)
}

/// Can `current_user` drop graphs? True when it holds SELECT and DELETE on both
/// `_pgrdf_quads` and `_pgrdf_graphs` — the rule `drop_graph` enforces.
#[pg_extern]
fn can_drop_graphs() -> bool {
    holds_graph_ddl_privilege(GraphDdl::Drop)
}

/// Can `current_user` clear graphs? True when it holds SELECT and DELETE on
/// `_pgrdf_quads` — the rule `clear_graph` enforces.
#[pg_extern]
fn can_clear_graphs() -> bool {
    holds_graph_ddl_privilege(GraphDdl::Clear)
}

/// Copy `pgrdf._pgrdf_quads`'s grants onto a freshly created partition.
///
/// **Postgres does not propagate ACLs to partitions.** A partition is
/// owned by whoever ran the `CREATE`, with no grants, regardless of what
/// the parent carries. A downstream `SECURITY DEFINER` function owned by
/// a non-superuser role can therefore read the parent but not the
/// partition holding the rows — `permission denied for table
/// _pgrdf_quads_g<id>` — and no caller can grant at the right moment,
/// because the table does not exist until we make it and its name is our
/// internal detail (issue #96).
///
/// The parent's ACL is the right template: a consumer grants once on
/// `pgrdf._pgrdf_quads` and every partition created afterwards follows.
/// Granting after the fact only covers graphs that already exist, which
/// is the part that kept re-breaking for later consumers.
///
/// `relacl IS NULL` means default, owner-only privileges — `aclexplode`
/// yields no rows and nothing is granted, so a deployment that never
/// granted anything sees no behaviour change.
fn inherit_parent_acl(part_name: &str) {
    // Grantee 0 is PUBLIC, which has no entry in pg_authid and must be
    // spelled as a bare keyword rather than a quoted identifier.
    let sql = format!(
        r#"DO $pgrdf_acl$
        DECLARE r record;
        BEGIN
          FOR r IN
            SELECT a.privilege_type AS priv,
                   CASE WHEN a.grantee = 0 THEN 'PUBLIC'
                        ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee
            FROM pg_class c, aclexplode(c.relacl) a
            WHERE c.oid = 'pgrdf._pgrdf_quads'::regclass
          LOOP
            EXECUTE format('GRANT %s ON pgrdf.%I TO %s', r.priv, {part}, r.grantee);
          END LOOP;
        END
        $pgrdf_acl$;"#,
        part = spi_quote_literal(part_name),
    );
    Spi::run(&sql).expect("create_quads_partition: ACL inheritance failed");
}

/// Single-quote a value for embedding in SQL. `part_name` is built
/// internally from a BIGINT, so this is belt-and-braces rather than a
/// live injection surface — but the value crosses into a `DO` body where
/// it is no longer in identifier position, so it gets quoted properly.
fn spi_quote_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Create the canonical `_pgrdf_quads_g{graph_id}` partition,
/// serialised against all other partition DDL via the shared xact
/// advisory lock. Idempotent: a no-op if the partition already
/// exists. This is the single production entry point — every
/// `add_graph` overload routes here.
pub(crate) fn create_quads_partition(graph_id: i64) {
    let part_name = format!("_pgrdf_quads_g{}", graph_id);
    create_partition_impl(&part_name, graph_id);
}

/// Create a partition with an **explicit** relation name (e.g. the
/// `_pgrdf_quads_test501` fixtures hand-rolled by `#[pg_test]`s in
/// `query::executor` / `storage::graphs`), sharing the exact same
/// advisory-lock gate as [`create_quads_partition`].
///
/// This is option (b) from the fix plan: rather than renaming the
/// test partitions to the `g{id}` scheme (invasive, touches dozens
/// of assertions that match on `_pgrdf_quads_test*`), the test
/// fixtures keep their names but route their DDL through the same
/// serialising gate — which is what actually makes parallel
/// `cargo pgrx test` deadlock-free.
#[cfg(any(test, feature = "pg_test"))]
pub(crate) fn create_quads_partition_named(part_name: &str, graph_id: i64) {
    create_partition_impl(part_name, graph_id);
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use pgrx::pg_sys::panic::CaughtError;
    use pgrx::prelude::*;

    /// A NOLOGIN role holding `privs` on the quad and graph tables — the
    /// shape of a trusted non-owner writer (the MCP role on the bench).
    /// The partition-DDL gate is taken FIRST: GRANT locks the parent, and
    /// the module rule is that the advisory gate is the outermost lock.
    fn writer_role(name: &str, privs: &str) {
        crate::storage::partition::acquire_partition_ddl_gate();
        Spi::run(&format!("CREATE ROLE {name} NOLOGIN")).unwrap();
        Spi::run(&format!("GRANT USAGE ON SCHEMA pgrdf TO {name}")).unwrap();
        Spi::run(&format!(
            "GRANT {privs} ON pgrdf._pgrdf_quads, pgrdf._pgrdf_graphs, \
             pgrdf._pgrdf_dictionary TO {name}"
        ))
        .unwrap();
    }

    /// Run `sql` as `role` and return the SQLSTATE and HINT it raised, or
    /// None when it succeeded. The caller identity is read straight from
    /// `GetUserId()` after the catch (no SPI on a caught error), so the
    /// test also proves the owner switch restored the caller.
    fn as_role_caught(role: &str, sql: &str) -> (Option<String>, Option<String>, bool) {
        Spi::run(&format!("SET ROLE {role}")).unwrap();
        let caller = unsafe { pgrx::pg_sys::GetUserId() };
        let sql = sql.to_string();
        let (code, hint) = pgrx::PgTryBuilder::new(|| {
            Spi::run(&sql).unwrap();
            (None, None)
        })
        .catch_others(|e| match &e {
            CaughtError::PostgresError(r)
            | CaughtError::ErrorReport(r)
            | CaughtError::RustPanic { ereport: r, .. } => (
                Some(format!("{:?}", r.sql_error_code())),
                r.hint().map(str::to_string),
            ),
        })
        .execute();
        let restored = unsafe { pgrx::pg_sys::GetUserId() } == caller;
        (code, hint, restored)
    }

    /// F1 (SPEC 0.6.37 §3.1): a trusted non-owner holding INSERT on the
    /// quad + graph tables creates a graph. Only the fixed partition DDL
    /// runs as the storage owner; the partition is owned by the parent's
    /// owner, keeps the #96 ACL inheritance, and the caller is restored.
    #[pg_test]
    fn nonowner_with_insert_creates_graph() {
        writer_role("pgrdf_f1_writer", "SELECT, INSERT, DELETE");
        let (code, _hint, restored) = as_role_caught(
            "pgrdf_f1_writer",
            "SELECT pgrdf.add_graph('urn:tdd:f1:created')",
        );
        assert_eq!(
            code, None,
            "a writer with INSERT must be able to create a graph"
        );
        assert!(
            restored,
            "the caller must be restored after the owner switch"
        );
        Spi::run("RESET ROLE").unwrap();
        let owned_like_parent = Spi::get_one::<bool>(
            "SELECT c.relowner = p.relowner FROM pg_class c, pg_class p \
             WHERE p.oid = 'pgrdf._pgrdf_quads'::regclass AND c.oid = ( \
               SELECT format('pgrdf._pgrdf_quads_g%s', graph_id)::regclass \
               FROM pgrdf._pgrdf_graphs WHERE iri = 'urn:tdd:f1:created')",
        )
        .unwrap()
        .unwrap_or(false);
        assert!(
            owned_like_parent,
            "the partition is owned by the storage owner"
        );
        let writer_can_insert = Spi::get_one::<bool>(
            "SELECT has_table_privilege('pgrdf_f1_writer', ( \
               SELECT format('pgrdf._pgrdf_quads_g%s', graph_id) \
               FROM pgrdf._pgrdf_graphs WHERE iri = 'urn:tdd:f1:created'), 'INSERT')",
        )
        .unwrap()
        .unwrap_or(false);
        assert!(writer_can_insert, "#96 ACL inheritance still applies");
    }

    /// F1: without INSERT the create refuses 42501 before any DDL, and the
    /// HINT carries the exact grant that cures it.
    #[pg_test]
    fn nonowner_without_insert_refused_with_cure() {
        writer_role("pgrdf_f1_reader", "SELECT");
        let (code, hint, restored) = as_role_caught(
            "pgrdf_f1_reader",
            "SELECT pgrdf.add_graph('urn:tdd:f1:refused')",
        );
        assert_eq!(code.as_deref(), Some("ERRCODE_INSUFFICIENT_PRIVILEGE"));
        assert!(restored, "the caller identity is unchanged after a refusal");
        let hint = hint.expect("the refusal must carry a HINT");
        assert!(
            hint.contains(
                "GRANT SELECT, INSERT ON pgrdf._pgrdf_quads, pgrdf._pgrdf_graphs TO pgrdf_f1_reader"
            ),
            "the HINT names the cure, got: {hint}"
        );
    }

    /// F1: a writer holding DELETE drops a graph; without DELETE the drop
    /// refuses 42501 with the DELETE grant as the cure.
    #[pg_test]
    fn nonowner_drop_needs_delete() {
        Spi::run("SELECT pgrdf.add_graph('urn:tdd:f1:to-drop')").unwrap();
        Spi::run("SELECT pgrdf.add_graph('urn:tdd:f1:kept')").unwrap();
        writer_role("pgrdf_f1_dropper", "SELECT, INSERT, DELETE");
        let (code, _h, restored) = as_role_caught(
            "pgrdf_f1_dropper",
            "SELECT pgrdf.drop_graph('urn:tdd:f1:to-drop', true)",
        );
        assert_eq!(
            code, None,
            "a writer with DELETE must be able to drop a graph"
        );
        assert!(restored);
        Spi::run("RESET ROLE").unwrap();
        writer_role("pgrdf_f1_nodelete", "SELECT, INSERT");
        let (code, hint, _r) = as_role_caught(
            "pgrdf_f1_nodelete",
            "SELECT pgrdf.drop_graph('urn:tdd:f1:kept', true)",
        );
        assert_eq!(code.as_deref(), Some("ERRCODE_INSUFFICIENT_PRIVILEGE"));
        assert!(hint.unwrap_or_default().contains(
            "GRANT SELECT, DELETE ON pgrdf._pgrdf_quads, pgrdf._pgrdf_graphs TO pgrdf_f1_nodelete"
        ));
    }

    /// F1: an ERROR raised INSIDE the owner switch, caught in the same
    /// transaction, must not leave the session running as the storage
    /// owner. An event trigger refuses the partition CREATE TABLE, so
    /// the failure happens while the switch is active.
    #[pg_test]
    fn error_inside_owner_switch_restores_caller() {
        Spi::run(
            "CREATE FUNCTION pgrdf_f1_block() RETURNS event_trigger LANGUAGE plpgsql AS \
             $$ BEGIN RAISE EXCEPTION 'f1 probe: partition DDL blocked'; END $$",
        )
        .unwrap();
        Spi::run(
            "CREATE EVENT TRIGGER pgrdf_f1_block ON ddl_command_start \
             WHEN TAG IN ('CREATE TABLE') EXECUTE FUNCTION pgrdf_f1_block()",
        )
        .unwrap();
        writer_role("pgrdf_f1_victim", "SELECT, INSERT, DELETE");
        let (code, _hint, restored) = as_role_caught(
            "pgrdf_f1_victim",
            "SELECT pgrdf.add_graph('urn:tdd:f1:blocked')",
        );
        assert!(code.is_some(), "the blocked DDL must raise");
        assert!(
            restored,
            "a caught error inside the switch must restore the caller"
        );
    }

    /// F1: the capability predicates answer from the caller's grants.
    #[pg_test]
    fn can_create_and_drop_graphs_follow_grants() {
        writer_role("pgrdf_f1_full", "SELECT, INSERT, DELETE");
        writer_role("pgrdf_f1_ro", "SELECT");
        Spi::run("SET ROLE pgrdf_f1_full").unwrap();
        let full =
            Spi::get_one::<bool>("SELECT pgrdf.can_create_graphs() AND pgrdf.can_drop_graphs()")
                .unwrap()
                .unwrap();
        Spi::run("SET ROLE pgrdf_f1_ro").unwrap();
        let ro =
            Spi::get_one::<bool>("SELECT pgrdf.can_create_graphs() OR pgrdf.can_drop_graphs()")
                .unwrap()
                .unwrap();
        Spi::run("RESET ROLE").unwrap();
        assert!(full, "INSERT+DELETE on both tables reads true/true");
        assert!(!ro, "SELECT only reads false/false");
    }

    /// F1 through the SPARQL route: CREATE GRAPH / DROP GRAPH dispatch to
    /// add_graph / drop_graph as the caller, so a granted non-owner can do
    /// both through pgrdf.sparql() as well as SQL, and an ungranted one is
    /// refused with the same cure.
    #[pg_test]
    fn nonowner_creates_and_drops_graphs_through_sparql_update() {
        writer_role("pgrdf_f1_sparql", "SELECT, INSERT, DELETE");
        let (code, _h, restored) = as_role_caught(
            "pgrdf_f1_sparql",
            "SELECT * FROM pgrdf.sparql('CREATE GRAPH <urn:tdd:f1:sparql>')",
        );
        assert_eq!(code, None, "CREATE GRAPH as a granted non-owner");
        assert!(restored);
        Spi::run("RESET ROLE").unwrap();
        let exists: bool = Spi::get_one(
            "SELECT EXISTS(SELECT 1 FROM pgrdf._pgrdf_graphs WHERE iri = 'urn:tdd:f1:sparql')",
        )
        .unwrap()
        .unwrap();
        assert!(exists);
        let (code, _h, _r) = as_role_caught(
            "pgrdf_f1_sparql",
            "SELECT * FROM pgrdf.sparql('DROP GRAPH <urn:tdd:f1:sparql>')",
        );
        assert_eq!(code, None, "DROP GRAPH as a granted non-owner");
        Spi::run("RESET ROLE").unwrap();
        writer_role("pgrdf_f1_sparql_ro", "SELECT");
        let (code, hint, _r) = as_role_caught(
            "pgrdf_f1_sparql_ro",
            "SELECT * FROM pgrdf.sparql('CREATE GRAPH <urn:tdd:f1:sparql2>')",
        );
        assert_eq!(code.as_deref(), Some("ERRCODE_INSUFFICIENT_PRIVILEGE"));
        assert!(
            hint.unwrap_or_default()
                .contains("GRANT SELECT, INSERT ON pgrdf._pgrdf_quads")
        );
    }

    /// A partition created after a grant on the parent carries that
    /// grant. This is issue #96: without it a downstream `SECURITY
    /// DEFINER` function owned by a non-superuser role reads the parent
    /// fine and gets `permission denied for table _pgrdf_quads_g<id>`,
    /// because Postgres does not propagate ACLs to partitions.
    #[pg_test]
    fn partition_inherits_parent_grants() {
        // Take the DDL gate FIRST. `GRANT ... ON pgrdf._pgrdf_quads`
        // locks the parent, and the module's rule is that the advisory
        // gate is the OUTERMOST lock — locking the parent before it
        // inverts the order against any parallel test doing
        // gate-then-parent, and Postgres deadlocks. `create_quads_partition`
        // re-enters the gate harmlessly.
        crate::storage::partition::acquire_partition_ddl_gate();

        Spi::run("CREATE ROLE pgrdf_acl_probe NOLOGIN").unwrap();
        Spi::run("GRANT SELECT, INSERT ON pgrdf._pgrdf_quads TO pgrdf_acl_probe").unwrap();

        // Grant FIRST, create SECOND — the ordering the fix exists to
        // make work. Granting afterwards was always possible; it just
        // never covered graphs created later.
        crate::storage::partition::create_quads_partition(960_001);

        let has_select = Spi::get_one::<bool>(
            "SELECT has_table_privilege('pgrdf_acl_probe', \
             'pgrdf._pgrdf_quads_g960001', 'SELECT')",
        )
        .unwrap()
        .unwrap_or(false);
        let has_insert = Spi::get_one::<bool>(
            "SELECT has_table_privilege('pgrdf_acl_probe', \
             'pgrdf._pgrdf_quads_g960001', 'INSERT')",
        )
        .unwrap()
        .unwrap_or(false);

        assert!(has_select, "partition must inherit SELECT from the parent");
        assert!(has_insert, "partition must inherit INSERT from the parent");

        // A privilege the parent does NOT carry must not appear — the
        // fix copies the parent's ACL, it does not widen it.
        let has_delete = Spi::get_one::<bool>(
            "SELECT has_table_privilege('pgrdf_acl_probe', \
             'pgrdf._pgrdf_quads_g960001', 'DELETE')",
        )
        .unwrap()
        .unwrap_or(true);
        assert!(
            !has_delete,
            "partition must not gain privileges the parent lacks"
        );
    }

    /// With no grants on the parent, nothing is granted on the partition.
    /// `relacl IS NULL` means default owner-only privileges, so a
    /// deployment that never granted anything sees no change.
    #[pg_test]
    fn partition_with_ungranted_parent_stays_owner_only() {
        Spi::run("CREATE ROLE pgrdf_acl_probe2 NOLOGIN").unwrap();
        crate::storage::partition::create_quads_partition(960_002);

        let has_select = Spi::get_one::<bool>(
            "SELECT has_table_privilege('pgrdf_acl_probe2', \
             'pgrdf._pgrdf_quads_g960002', 'SELECT')",
        )
        .unwrap()
        .unwrap_or(true);
        assert!(!has_select, "an unrelated role must not gain access");
    }
    /// #145: clear_graph TRUNCATEs the partition, a privilege outside the
    /// documented grant set. A writer holding SELECT + DELETE on the quad
    /// table clears a graph (the graph row stays); without DELETE it is
    /// refused 42501 with a cure that names the quad table only.
    #[pg_test]
    fn nonowner_clear_needs_delete_on_quads() {
        let g: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:145:clear')")
            .unwrap()
            .unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:c:s> <urn:c:p> <urn:c:o> . \
             <urn:c:s> <urn:c:p> <urn:c:o2> .', {g})"
        ))
        .unwrap();
        writer_role("pgrdf_145_clearer", "SELECT, INSERT, DELETE");
        let (code, _h, restored) = as_role_caught(
            "pgrdf_145_clearer",
            "SELECT pgrdf.clear_graph('urn:tdd:145:clear')",
        );
        assert_eq!(
            code, None,
            "a writer with DELETE must be able to clear a graph"
        );
        assert!(
            restored,
            "the caller must be restored after the owner switch"
        );
        Spi::run("RESET ROLE").unwrap();
        let (left, kept): (Option<i64>, Option<bool>) = Spi::get_two(&format!(
            "SELECT (SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = {g}), \
                    EXISTS(SELECT 1 FROM pgrdf._pgrdf_graphs WHERE graph_id = {g})"
        ))
        .unwrap();
        assert_eq!(left, Some(0), "every triple is gone");
        assert_eq!(kept, Some(true), "the graph itself remains");

        writer_role("pgrdf_145_nodelete", "SELECT, INSERT");
        let (code, hint, restored) = as_role_caught(
            "pgrdf_145_nodelete",
            "SELECT pgrdf.clear_graph('urn:tdd:145:clear')",
        );
        assert_eq!(code.as_deref(), Some("ERRCODE_INSUFFICIENT_PRIVILEGE"));
        assert!(restored);
        let hint = hint.expect("the refusal must carry a HINT");
        assert_eq!(
            hint, "GRANT SELECT, DELETE ON pgrdf._pgrdf_quads TO pgrdf_145_nodelete",
            "clear asks for the quad table only"
        );
    }

    /// #145: a role granted on the parent AFTER a graph's partition was
    /// created holds nothing on that partition (#96 copies the ACL only at
    /// creation). clear_graph counts through the parent and truncates as
    /// the owner, so such a role still clears the graph.
    #[pg_test]
    fn clear_works_for_role_granted_after_partition() {
        crate::storage::partition::acquire_partition_ddl_gate();
        let g: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:145:late')")
            .unwrap()
            .unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:l:s> <urn:l:p> <urn:l:o> .', {g})"
        ))
        .unwrap();
        writer_role("pgrdf_145_late", "SELECT, INSERT, DELETE");
        let on_partition = Spi::get_one::<bool>(&format!(
            "SELECT has_table_privilege('pgrdf_145_late', 'pgrdf._pgrdf_quads_g{g}', 'SELECT')"
        ))
        .unwrap()
        .unwrap();
        assert!(
            !on_partition,
            "precondition: no grant reached the old partition"
        );
        let (code, _h, _r) = as_role_caught(
            "pgrdf_145_late",
            &format!("SELECT pgrdf.clear_graph({g}::bigint)"),
        );
        assert_eq!(code, None, "clear must not need a grant on the partition");
        Spi::run("RESET ROLE").unwrap();
        let left: i64 = Spi::get_one(&format!(
            "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = {g}"
        ))
        .unwrap()
        .unwrap();
        assert_eq!(left, 0);
    }

    /// #145: the lock fence still comes first. A granted writer clearing a
    /// locked graph is refused 55P03, never elevated into a TRUNCATE.
    #[pg_test]
    fn nonowner_clear_of_locked_graph_refuses_lock() {
        let g: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:145:locked')")
            .unwrap()
            .unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:k:s> <urn:k:p> <urn:k:o> .', {g})"
        ))
        .unwrap();
        Spi::run(&format!("SELECT pgrdf.lock_graph({g}, 'tdd 145')")).unwrap();
        writer_role("pgrdf_145_locked", "SELECT, INSERT, DELETE");
        let (code, _h, restored) = as_role_caught(
            "pgrdf_145_locked",
            &format!("SELECT pgrdf.clear_graph({g}::bigint)"),
        );
        // No SPI after a caught ERROR: the transaction is aborted.
        assert_eq!(code.as_deref(), Some("ERRCODE_LOCK_NOT_AVAILABLE"));
        assert!(restored);
    }

    /// #145: can_clear_graphs() answers the rule clear_graph enforces:
    /// SELECT + DELETE on the quad table, nothing on the graph table.
    #[pg_test]
    fn can_clear_graphs_follows_grants() {
        crate::storage::partition::acquire_partition_ddl_gate();
        Spi::run("CREATE ROLE pgrdf_145_qdel NOLOGIN").unwrap();
        Spi::run("GRANT USAGE ON SCHEMA pgrdf TO pgrdf_145_qdel").unwrap();
        Spi::run("GRANT SELECT, DELETE ON pgrdf._pgrdf_quads TO pgrdf_145_qdel").unwrap();
        writer_role("pgrdf_145_ro", "SELECT");
        Spi::run("SET ROLE pgrdf_145_qdel").unwrap();
        let (clear, drop): (Option<bool>, Option<bool>) =
            Spi::get_two("SELECT pgrdf.can_clear_graphs(), pgrdf.can_drop_graphs()").unwrap();
        Spi::run("SET ROLE pgrdf_145_ro").unwrap();
        let ro = Spi::get_one::<bool>("SELECT pgrdf.can_clear_graphs()")
            .unwrap()
            .unwrap();
        Spi::run("RESET ROLE").unwrap();
        assert_eq!(
            clear,
            Some(true),
            "DELETE on the quad table is enough to clear"
        );
        assert_eq!(
            drop,
            Some(false),
            "dropping still needs the graph table too"
        );
        assert!(!ro, "SELECT only cannot clear");
    }

    /// Every graph-writing entry point, run as a non-owner holding exactly
    /// the grants guide/01-install.md documents for an application role
    /// (plus pg_read_server_files, which the file loaders document
    /// separately), must succeed. F1 tested create and drop one by one and
    /// clear slipped through (#145); this walks the whole write surface.
    ///
    /// The second half keeps it whole: every stable function in surface()
    /// must be classified here as exercised, read-only or owner-lane, so a
    /// new export cannot ship without someone deciding which it is.
    #[pg_test]
    fn documented_grants_cover_every_graph_write() {
        use std::collections::BTreeSet;
        const EXERCISED: &[&str] = &[
            "add_graph",
            "create_graph",
            "parse_turtle",
            "parse_turtle_verbose",
            "parse_nquads",
            "parse_trig",
            "load_turtle",
            "load_turtle_verbose",
            "load_turtle_streaming",
            "copy_graph",
            "carve_graph",
            "materialize",
            "lock_graph",
            "unlock_graph",
            "sparql",
            "clear_graph",
            "move_graph",
            "drop_graph",
        ];
        const READ_ONLY: &[&str] = &[
            "build_id",
            "can_clear_graphs",
            "can_create_graphs",
            "can_drop_graphs",
            "construct",
            "count_quads",
            "describe",
            "export_graph",
            "get_term",
            "graph_diff",
            "graph_diff_summary",
            "graph_digest",
            "graph_id",
            "graph_integrity",
            "graph_inventory",
            "graph_iri",
            "graph_manifest",
            "last_call_stats",
            "orphan_partitions",
            "ownership_drift",
            "shacl_capability",
            "sparql_parse",
            "sparql_sql",
            "stats",
            "structural_digest",
            "surface",
            "validate",
            "version",
        ];
        // Refused up front for a non-owner, by design (F4), or operator
        // maintenance of the shared term cache rather than a graph write.
        const OWNER_LANE: &[&str] = &[
            "load_turtle_staged",
            "load_turtle_staged_run",
            "shmem_reset",
        ];

        crate::storage::partition::acquire_partition_ddl_gate();
        Spi::run("CREATE ROLE pgrdf_145_app NOLOGIN").unwrap();
        Spi::run("GRANT USAGE ON SCHEMA pgrdf TO pgrdf_145_app").unwrap();
        Spi::run(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pgrdf TO pgrdf_145_app",
        )
        .unwrap();
        Spi::run("GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pgrdf TO pgrdf_145_app").unwrap();
        Spi::run("GRANT pg_read_server_files TO pgrdf_145_app").unwrap();

        let path = format!("/tmp/pgrdf-145-matrix-{}.nt", std::process::id());
        std::fs::write(&path, "<urn:m:f> <urn:m:p> <urn:m:o> .\n").unwrap();

        let steps: Vec<(&str, String)> = vec![
            ("add_graph", "SELECT pgrdf.add_graph('urn:m:a')".into()),
            ("create_graph", "SELECT pgrdf.create_graph('urn:m:b')".into()),
            ("parse_turtle", "SELECT pgrdf.parse_turtle('<urn:m:s> <urn:m:p> <urn:m:o> . \
              <urn:m:s> a <urn:m:C> . <urn:m:C> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <urn:m:D> .', \
              pgrdf.graph_id('urn:m:a'))".into()),
            ("parse_turtle_verbose", "SELECT pgrdf.parse_turtle_verbose('<urn:m:s> <urn:m:q> <urn:m:o> .', \
              pgrdf.graph_id('urn:m:a'))".into()),
            ("parse_nquads", "SELECT pgrdf.parse_nquads('<urn:m:s> <urn:m:p> <urn:m:o> <urn:m:nq> .')".into()),
            ("parse_trig", "SELECT pgrdf.parse_trig('<urn:m:tg> { <urn:m:s> <urn:m:p> <urn:m:o> . }')".into()),
            ("load_turtle", format!("SELECT pgrdf.load_turtle('{path}', pgrdf.add_graph('urn:m:f1'))")),
            ("load_turtle", format!("SELECT pgrdf.load_turtle('{path}', pgrdf.add_graph('urn:m:f2'), NULL, true)")),
            ("load_turtle_verbose", format!("SELECT pgrdf.load_turtle_verbose('{path}', pgrdf.add_graph('urn:m:f3'))")),
            ("load_turtle_streaming", format!("SELECT pgrdf.load_turtle_streaming('{path}', pgrdf.add_graph('urn:m:f4'))")),
            ("copy_graph", "SELECT pgrdf.copy_graph(pgrdf.graph_id('urn:m:a'), pgrdf.graph_id('urn:m:b'))".into()),
            ("copy_graph", "SELECT pgrdf.copy_graph('urn:m:a', 'urn:m:b')".into()),
            ("carve_graph", "SELECT pgrdf.carve_graph(pgrdf.graph_id('urn:m:a'), 'urn:m:p', pgrdf.add_graph('urn:m:c1'))".into()),
            ("carve_graph", "SELECT pgrdf.carve_graph(pgrdf.graph_id('urn:m:a'), ARRAY['urn:m:s'], pgrdf.add_graph('urn:m:c2'), 1)".into()),
            ("materialize", "SELECT pgrdf.materialize(pgrdf.graph_id('urn:m:a'), 'rdfs')".into()),
            ("lock_graph", "SELECT pgrdf.lock_graph(pgrdf.graph_id('urn:m:a'), 'matrix')".into()),
            ("graph_digest", "SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:m:a'))".into()),
            ("unlock_graph", "SELECT pgrdf.unlock_graph(pgrdf.graph_id('urn:m:a'), 'matrix')".into()),
            ("sparql", "SELECT * FROM pgrdf.sparql('INSERT DATA { GRAPH <urn:m:a> { <urn:m:x> <urn:m:p> <urn:m:y> } }')".into()),
            ("sparql", "SELECT * FROM pgrdf.sparql('DELETE DATA { GRAPH <urn:m:a> { <urn:m:x> <urn:m:p> <urn:m:y> } }')".into()),
            ("sparql", "SELECT * FROM pgrdf.sparql('DELETE WHERE { GRAPH <urn:m:b> { ?s <urn:m:q> ?o } }')".into()),
            ("sparql", "SELECT * FROM pgrdf.sparql('CREATE GRAPH <urn:m:sp>')".into()),
            ("sparql", "SELECT * FROM pgrdf.sparql('INSERT DATA { GRAPH <urn:m:sp> { <urn:m:x> <urn:m:p> <urn:m:y> } }')".into()),
            ("sparql", "SELECT * FROM pgrdf.sparql('CLEAR GRAPH <urn:m:sp>')".into()),
            ("sparql", "SELECT * FROM pgrdf.sparql('DROP GRAPH <urn:m:sp>')".into()),
            ("clear_graph", "SELECT pgrdf.clear_graph(pgrdf.graph_id('urn:m:c1'))".into()),
            ("clear_graph", "SELECT pgrdf.clear_graph('urn:m:b')".into()),
            ("move_graph", "SELECT pgrdf.move_graph(pgrdf.graph_id('urn:m:c2'), pgrdf.add_graph('urn:m:mv1'))".into()),
            ("add_graph", "SELECT pgrdf.add_graph('urn:m:mv2')".into()),
            ("move_graph", "SELECT pgrdf.move_graph('urn:m:mv1', 'urn:m:mv2')".into()),
            ("drop_graph", "SELECT pgrdf.drop_graph(pgrdf.graph_id('urn:m:c1'), true)".into()),
            ("drop_graph", "SELECT pgrdf.drop_graph('urn:m:b', true)".into()),
        ];
        for (name, sql) in &steps {
            let (code, hint, restored) = as_role_caught("pgrdf_145_app", sql);
            // Stop at the first refusal: a caught ERROR aborts the
            // transaction, so later steps could only fail spuriously.
            if code.is_some() || !restored {
                let _ = std::fs::remove_file(&path);
                panic!(
                    "a non-owner with the documented grants was refused at {name}: \
                     {code:?} hint={hint:?} restored={restored}\n  {sql}"
                );
            }
        }
        Spi::run("RESET ROLE").unwrap();
        let _ = std::fs::remove_file(&path);

        let exercised: BTreeSet<&str> = steps.iter().map(|(n, _)| *n).collect();
        for name in EXERCISED {
            assert!(
                exercised.contains(name),
                "{name} is listed as exercised but has no step"
            );
        }
        let stable: Vec<String> = Spi::connect(|c| {
            c.select(
                "SELECT DISTINCT name FROM pgrdf.surface() WHERE class = 'stable' ORDER BY 1",
                None,
                &[],
            )
            .unwrap()
            .map(|r| r.get::<String>(1).unwrap().unwrap())
            .collect()
        });
        let unclassified: Vec<&String> = stable
            .iter()
            .filter(|n| {
                let n = n.as_str();
                !EXERCISED.contains(&n) && !READ_ONLY.contains(&n) && !OWNER_LANE.contains(&n)
            })
            .collect();
        assert!(
            unclassified.is_empty(),
            "stable functions not classified in this matrix (exercise each writer as a \
             non-owner, or list it as read-only / owner-lane): {unclassified:?}"
        );
    }
    /// #150: a dropped graph's id is never handed out again. With
    /// MAX(graph_id) + 1 allocation, dropping the highest-numbered graph
    /// gave its id to the next graph created, so a numeric id held across
    /// a drop and a create silently named a different graph.
    #[pg_test]
    fn dropped_graph_id_is_never_reused() {
        let a: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:150:a')")
            .unwrap()
            .unwrap();
        Spi::run(&format!("SELECT pgrdf.drop_graph({a}::bigint, true)")).unwrap();
        let b: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:150:b')")
            .unwrap()
            .unwrap();
        assert!(
            b > a,
            "the id of a dropped graph must not be reused: {a} then {b}"
        );
        // The same IRI re-created gets a new id too: the IRI is the
        // identity, the id names one incarnation.
        let a2: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:150:a')")
            .unwrap()
            .unwrap();
        assert!(a2 > b, "a re-created IRI gets a fresh id: {a2} after {b}");
    }

    /// #150: an id bound explicitly above the allocator's mark advances
    /// it, so the allocator never hands that id out later, even after
    /// the explicit graph is dropped.
    #[pg_test]
    fn explicit_id_advances_the_allocator() {
        let base: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:150:base')")
            .unwrap()
            .unwrap();
        let high = base + 1000;
        Spi::run(&format!(
            "SELECT pgrdf.add_graph({high}::bigint, 'urn:tdd:150:explicit')"
        ))
        .unwrap();
        Spi::run(&format!("SELECT pgrdf.drop_graph({high}::bigint, true)")).unwrap();
        let next: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:150:after')")
            .unwrap()
            .unwrap();
        assert!(
            next > high,
            "an explicitly bound id must never be allocated later: {next} <= {high}"
        );
    }

    /// #150: an id already bound before the sequence knew about it (a
    /// database upgraded with graphs above the mark) is skipped, never
    /// collided with.
    #[pg_test]
    fn allocator_skips_ids_already_in_use() {
        // Hold the DDL gate for the whole test: every allocation happens
        // behind it, so no parallel test can take seed+1 in between.
        crate::storage::partition::acquire_partition_ddl_gate();
        let seed: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:150:seed')")
            .unwrap()
            .unwrap();
        // Bind the next two ids behind the allocator's back (row only, as
        // an older install could have left them), without moving the mark.
        Spi::run(&format!(
            "INSERT INTO pgrdf._pgrdf_graphs (graph_id, iri) VALUES \
             ({}, 'urn:tdd:150:taken1'), ({}, 'urn:tdd:150:taken2')",
            seed + 1,
            seed + 2
        ))
        .unwrap();
        let next: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:150:skip')")
            .unwrap()
            .unwrap();
        assert!(next > seed + 2, "taken ids are skipped: got {next}");
    }

    /// #153: graph creation runs nextval, a read of last_value and setval
    /// on the graph-id sequence as the storage owner, so that owner needs
    /// the sequence owner's rights or USAGE, SELECT and UPDATE on it.
    /// USAGE alone passes nextval and then refuses at setval.
    #[pg_test]
    fn graph_id_seq_usable_needs_usage_select_and_update() {
        // The gate first: every reader of the sequence takes it, so the
        // grant and owner changes below cannot interleave with them.
        crate::storage::partition::acquire_partition_ddl_gate();
        Spi::run("CREATE ROLE pgrdf_153_owner NOLOGIN").unwrap();
        let role: pgrx::pg_sys::Oid = Spi::get_one("SELECT 'pgrdf_153_owner'::regrole::oid")
            .unwrap()
            .unwrap();
        let usable = || crate::storage::partition::graph_id_seq_usable_by(role);
        assert!(!usable(), "a role with no rights cannot allocate ids");
        Spi::run("GRANT USAGE ON SEQUENCE pgrdf._pgrdf_graph_id_seq TO pgrdf_153_owner").unwrap();
        assert!(!usable(), "USAGE alone is not enough");
        Spi::run("GRANT SELECT, UPDATE ON SEQUENCE pgrdf._pgrdf_graph_id_seq TO pgrdf_153_owner")
            .unwrap();
        assert!(usable(), "USAGE, SELECT and UPDATE are enough");
        Spi::run("REVOKE ALL ON SEQUENCE pgrdf._pgrdf_graph_id_seq FROM pgrdf_153_owner").unwrap();
        Spi::run("ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO pgrdf_153_owner").unwrap();
        assert!(usable(), "owning the sequence is enough");
    }

    /// #153: ownership_drift() lists each storage relation the storage
    /// owner does not own, says whether that blocks the engine, and gives
    /// the statement that realigns it. Running the cures empties it.
    #[pg_test]
    fn ownership_drift_reports_and_cures() {
        crate::storage::partition::acquire_partition_ddl_gate();
        let g: i64 = Spi::get_one("SELECT pgrdf.add_graph('urn:tdd:153:drift')")
            .unwrap()
            .unwrap();
        let part = format!("_pgrdf_quads_g{g}");
        let watched = format!("relname IN ('_pgrdf_graph_id_seq', '{part}')");
        let count = || -> i64 {
            Spi::get_one(&format!(
                "SELECT count(*) FROM pgrdf.ownership_drift() WHERE {watched}"
            ))
            .unwrap()
            .unwrap()
        };
        assert_eq!(count(), 0, "a fresh install has no drift");

        Spi::run("CREATE ROLE pgrdf_153_other NOLOGIN").unwrap();
        Spi::run(&format!(
            "ALTER TABLE pgrdf.{part} OWNER TO pgrdf_153_other"
        ))
        .unwrap();
        Spi::run("ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO pgrdf_153_other").unwrap();

        let (storage_owner, superuser) = Spi::get_two::<String, bool>(
            "SELECT r.rolname::text, r.rolsuper FROM pg_class c JOIN pg_roles r ON r.oid = c.relowner \
             WHERE c.oid = 'pgrdf._pgrdf_quads'::regclass",
        )
        .unwrap();
        let (storage_owner, superuser) = (storage_owner.unwrap(), superuser.unwrap());
        let rows: Vec<(String, String, String, String, bool, String)> = Spi::connect(|c| {
            c.select(
                &format!(
                    "SELECT relname, kind, owner, storage_owner, blocking, cure \
                     FROM pgrdf.ownership_drift() WHERE {watched} ORDER BY relname"
                ),
                None,
                &[],
            )
            .unwrap()
            .map(|r| {
                (
                    r.get::<String>(1).unwrap().unwrap(),
                    r.get::<String>(2).unwrap().unwrap(),
                    r.get::<String>(3).unwrap().unwrap(),
                    r.get::<String>(4).unwrap().unwrap(),
                    r.get::<bool>(5).unwrap().unwrap(),
                    r.get::<String>(6).unwrap().unwrap(),
                )
            })
            .collect()
        });
        assert_eq!(rows.len(), 2, "both drifted relations are listed: {rows:?}");
        let (seq, prt) = if rows[0].0 == "_pgrdf_graph_id_seq" {
            (&rows[0], &rows[1])
        } else {
            (&rows[1], &rows[0])
        };
        assert_eq!(prt.0, part);
        assert_eq!(prt.1, "partition");
        assert_eq!(seq.1, "sequence");
        for r in [seq, prt] {
            assert_eq!(r.2, "pgrdf_153_other");
            assert_eq!(r.3, storage_owner);
            // A superuser storage owner holds every role's rights, so the
            // drift is reported but blocks nothing.
            assert_eq!(
                r.4, !superuser,
                "blocking follows the owner's rights: {r:?}"
            );
        }
        assert_eq!(
            seq.5,
            format!("ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO {storage_owner}")
        );
        assert_eq!(
            prt.5,
            format!("ALTER TABLE pgrdf.{part} OWNER TO {storage_owner}")
        );

        for (_, _, _, _, _, cure) in &rows {
            Spi::run(cure).unwrap();
        }
        assert_eq!(count(), 0, "running the cures removes the drift");
    }
}
