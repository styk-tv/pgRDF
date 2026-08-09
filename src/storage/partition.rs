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

    // (4) Create it. `part_name` is caller-built from a BIGINT (no
    // user input in identifier position); `graph_id` is a constant
    // in the LIST value position which Postgres accepts in DDL.
    let sql = format!(
        "CREATE TABLE IF NOT EXISTS pgrdf.{} \
         PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN ({})",
        part_name, graph_id
    );
    Spi::run(&sql).expect("create_quads_partition: CREATE TABLE failed");

    // (5) Replicate the parent's ACL onto the new partition.
    inherit_parent_acl(part_name);
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
    use pgrx::prelude::*;

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
}
