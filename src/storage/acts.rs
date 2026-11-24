//! Graph ownership and the act record.
//!
//! Two facts pgRDF keeps per graph, beside PostgreSQL's own privileges:
//!
//! * **the graph owner**: the role that created the graph, by name. A
//!   record, not a PostgreSQL relation owner: partitions stay owned by the
//!   storage owner. `pgrdf.graph_owner_enforce` (default off) makes it gate
//!   the destructive acts.
//! * **the act record**: one row per graph-level act (create, load, clear,
//!   lock, ...), never per triple, written in the act's own transaction, so
//!   only committed acts are recorded.
//!
//! Each act row keeps two kinds of identity apart. *Verified* is what
//! PostgreSQL authenticated: `session_user` and `current_user`. *Declared*
//! is what the client says about itself and is never used to authorise
//! anything: `application_name`, `ckp.requester`, and the five `pgrdf.*`
//! settings (workspace, agent, thread, on_behalf_of, harness).

use crate::storage::activity;
use pgrx::prelude::*;
use serde_json::Value;
use std::cell::Cell;

// Fresh installs get the columns, the act table and its guards here; the
// upgrade script carries the same statements.
//
// * `owner` is a role NAME, the way pg_dump records ownership. A regrole
//   would make the registry's COPY abort on restore whenever a role is
//   missing on the target (#152 again).
// * The act table's id comes from its own sequence, read by the engine as
//   the storage owner, so writing an act needs no grant from the caller.
// * The two triggers refuse every direct write unless the writer holds the
//   table owner's privileges: the documented `GRANT ... ON ALL TABLES IN
//   SCHEMA pgrdf` would otherwise let any application role forge, change
//   or delete rows.
pgrx::extension_sql!(
    r#"
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS owner TEXT;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS created_act BIGINT;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS locked_act BIGINT;

CREATE SEQUENCE _pgrdf_graph_act_id_seq MINVALUE 1 START 1;
CREATE TABLE _pgrdf_graph_acts (
    act_id           BIGINT PRIMARY KEY,
    graph_id         BIGINT NOT NULL,
    graph_iri        TEXT,
    act              TEXT NOT NULL,
    at               TIMESTAMPTZ NOT NULL,
    xact             xid8 NOT NULL,
    session_role     TEXT NOT NULL,
    acting_role      TEXT NOT NULL,
    application_name TEXT,
    requester        TEXT,
    workspace        TEXT,
    agent            TEXT,
    thread           TEXT,
    on_behalf_of     TEXT,
    harness          TEXT,
    detail           JSONB
);
CREATE INDEX _pgrdf_graph_acts_graph_idx ON _pgrdf_graph_acts (graph_id, act_id);
GRANT SELECT ON _pgrdf_graph_acts TO PUBLIC;
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graph_acts', '');
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graph_act_id_seq', '');

CREATE FUNCTION _refuse_act_write() RETURNS trigger
LANGUAGE plpgsql AS $fn$
BEGIN
  IF pg_catalog.pg_has_role(
       current_user,
       (SELECT relowner FROM pg_catalog.pg_class WHERE oid = TG_RELID),
       'USAGE') THEN
    IF TG_LEVEL = 'STATEMENT' THEN RETURN NULL; END IF;
    IF TG_OP = 'DELETE' THEN RETURN OLD; END IF;
    RETURN NEW;
  END IF;
  RAISE EXCEPTION USING
    ERRCODE = '42501',
    MESSAGE = pg_catalog.format(
      'pgrdf: the act record is written by the engine: direct %s refused', TG_OP),
    HINT = 'Read it with pgrdf.graph_acts().';
END
$fn$;
CREATE TRIGGER pgrdf_acts_row BEFORE INSERT OR UPDATE OR DELETE ON _pgrdf_graph_acts
  FOR EACH ROW EXECUTE FUNCTION _refuse_act_write();
CREATE TRIGGER pgrdf_acts_truncate BEFORE TRUNCATE ON _pgrdf_graph_acts
  FOR EACH STATEMENT EXECUTE FUNCTION _refuse_act_write();

CREATE FUNCTION graph_acts(graph_id BIGINT DEFAULT NULL, since TIMESTAMPTZ DEFAULT NULL)
RETURNS SETOF _pgrdf_graph_acts
LANGUAGE sql STABLE
AS $fn$
  SELECT a.* FROM pgrdf._pgrdf_graph_acts a
   WHERE ($1 IS NULL OR a.graph_id = $1)
     AND ($2 IS NULL OR a.at >= $2)
   ORDER BY a.act_id
$fn$;
"#,
    name = "graph_acts_v0_6_41",
    requires = ["graph_locked_digest_v0_6_37"],
);

/// Who is acting, read once when an act starts.
pub(crate) struct Identity {
    /// The act table exists. False while the extension's SQL is older
    /// than the library (swapped, `ALTER EXTENSION UPDATE` not yet run):
    /// acts then run unrecorded instead of failing.
    present: bool,
    at: pgrx::datum::TimestampWithTimeZone,
    graph_iri: Option<String>,
    pub(crate) session_role: String,
    pub(crate) acting_role: String,
    pub(crate) application_name: Option<String>,
    pub(crate) requester: Option<String>,
    pub(crate) workspace: Option<String>,
    pub(crate) agent: Option<String>,
    pub(crate) thread: Option<String>,
    pub(crate) on_behalf_of: Option<String>,
    pub(crate) harness: Option<String>,
}

impl Identity {
    /// Read the verified and the declared identity as they stand now.
    /// Must run before any switch to the storage owner: `current_user`
    /// would otherwise always be that role.
    pub(crate) fn capture(graph_id: i64) -> Self {
        Spi::connect(|c| {
            let row = c
                .select(
                    "SELECT pg_catalog.to_regclass('pgrdf._pgrdf_graph_acts') IS NOT NULL, \
                            pg_catalog.clock_timestamp(), \
                            (SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = $1), \
                            session_user::text, current_user::text, \
                            nullif(pg_catalog.current_setting('application_name', true), ''), \
                            nullif(pg_catalog.current_setting('ckp.requester', true), ''), \
                            nullif(pg_catalog.current_setting('pgrdf.workspace', true), ''), \
                            nullif(pg_catalog.current_setting('pgrdf.agent', true), ''), \
                            nullif(pg_catalog.current_setting('pgrdf.thread', true), ''), \
                            nullif(pg_catalog.current_setting('pgrdf.on_behalf_of', true), ''), \
                            nullif(pg_catalog.current_setting('pgrdf.harness', true), '')",
                    None,
                    &[graph_id.into()],
                )
                .expect("act identity read failed")
                .first();
            let text = |n: usize| row.get::<String>(n).expect("act identity column");
            Identity {
                present: row
                    .get::<bool>(1)
                    .expect("act table probe")
                    .unwrap_or(false),
                at: row
                    .get::<pgrx::datum::TimestampWithTimeZone>(2)
                    .expect("act start time")
                    .expect("clock_timestamp() is never NULL"),
                graph_iri: text(3),
                session_role: text(4).unwrap_or_default(),
                acting_role: text(5).unwrap_or_default(),
                application_name: text(6),
                requester: text(7),
                workspace: text(8),
                agent: text(9),
                thread: text(10),
                on_behalf_of: text(11),
                harness: text(12),
            }
        })
    }

    fn shown(&self) -> activity::Shown<'_> {
        activity::Shown {
            session_role: &self.session_role,
            acting_role: &self.acting_role,
            application_name: self.application_name.as_deref(),
            requester: self.requester.as_deref(),
            workspace: self.workspace.as_deref(),
            agent: self.agent.as_deref(),
            thread: self.thread.as_deref(),
            on_behalf_of: self.on_behalf_of.as_deref(),
            harness: self.harness.as_deref(),
        }
    }

    /// Write one act row, as the storage owner, and return its id. `None`
    /// when nothing can be written: a read-only transaction, or SQL older
    /// than the library. Recording never fails the act it records.
    fn write(&self, graph_id: i64, act: &str, detail: Value) -> Option<i64> {
        if !self.present || unsafe { pgrx::pg_sys::XactReadOnly } {
            return None;
        }
        let act_id: i64 = crate::storage::partition::as_storage_owner(|| {
            Spi::get_one_with_args::<i64>(
                "INSERT INTO pgrdf._pgrdf_graph_acts \
                   (act_id, graph_id, graph_iri, act, at, xact, session_role, acting_role, \
                    application_name, requester, workspace, agent, thread, on_behalf_of, \
                    harness, detail) \
                 VALUES (pg_catalog.nextval('pgrdf._pgrdf_graph_act_id_seq'), $1, \
                         coalesce($2, (SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)), \
                         $3, $4, pg_catalog.pg_current_xact_id(), $5, $6, $7, $8, $9, $10, $11, \
                         $12, $13, $14) \
                 RETURNING act_id",
                &[
                    graph_id.into(),
                    self.graph_iri.as_deref().into(),
                    act.into(),
                    self.at.into(),
                    self.session_role.as_str().into(),
                    self.acting_role.as_str().into(),
                    self.application_name.as_deref().into(),
                    self.requester.as_deref().into(),
                    self.workspace.as_deref().into(),
                    self.agent.as_deref().into(),
                    self.thread.as_deref().into(),
                    self.on_behalf_of.as_deref().into(),
                    self.harness.as_deref().into(),
                    pgrx::JsonB(detail).into(),
                ],
            )
            .expect("recording the act failed")
            .expect("the act insert returned no id")
        });
        if crate::query::guc::notify_acts() {
            // Outside the owner switch, and after the row: the notification
            // is delivered only if this transaction commits.
            Spi::run_with_args(
                "SELECT pg_catalog.pg_notify('pgrdf_acts', $1)",
                &[format!("{act_id}:{graph_id}:{act}").into()],
            )
            .expect("notifying the act failed");
        }
        Some(act_id)
    }
}

thread_local! {
    /// How many acts this backend is inside. Only the outermost one is
    /// listed in `graph_activity()` and, `create` aside, recorded.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// One graph-level act in progress. [`begin`] it before the work and
/// [`ActScope::finish`] it after; dropped unfinished (an error unwinding
/// through it) it records nothing and stops being listed.
pub(crate) struct ActScope {
    graph_id: i64,
    act: &'static str,
    identity: Identity,
    outermost: bool,
}

/// Enter `act` on `graph_id`: read who is acting, and list this backend
/// in `graph_activity()`.
///
/// An act that calls another pgRDF function (a move copies and drops, a
/// load may create its graph) is recorded once, under its own name: a
/// nested scope records nothing, except `create`, which is a fact of its
/// own whoever asked for it.
pub(crate) fn begin(graph_id: i64, act: &'static str) -> ActScope {
    let identity = Identity::capture(graph_id);
    let outermost = DEPTH.with(|d| {
        let depth = d.get();
        d.set(depth + 1);
        depth == 0
    });
    if outermost {
        activity::enter(graph_id, act, activity::part::BACKEND, &identity.shown());
    }
    ActScope {
        graph_id,
        act,
        identity,
        outermost,
    }
}

impl ActScope {
    /// The act succeeded: record it. Returns the act id when a row was
    /// written.
    pub(crate) fn finish(self, detail: Value) -> Option<i64> {
        if self.outermost || self.act == "create" {
            self.identity.write(self.graph_id, self.act, detail)
        } else {
            None
        }
    }

    /// Record a further row of the same act on another graph (the source
    /// of a move, each graph a SPARQL update wrote to).
    pub(crate) fn also(&self, graph_id: i64, act: &str, detail: Value) -> Option<i64> {
        if self.outermost {
            // The other graph's IRI is read when the row is written.
            let other = Identity {
                present: self.identity.present,
                at: self.identity.at,
                graph_iri: None,
                session_role: self.identity.session_role.clone(),
                acting_role: self.identity.acting_role.clone(),
                application_name: self.identity.application_name.clone(),
                requester: self.identity.requester.clone(),
                workspace: self.identity.workspace.clone(),
                agent: self.identity.agent.clone(),
                thread: self.identity.thread.clone(),
                on_behalf_of: self.identity.on_behalf_of.clone(),
                harness: self.identity.harness.clone(),
            };
            other.write(graph_id, act, detail)
        } else {
            None
        }
    }

    /// The role acting, as read when the act started.
    pub(crate) fn acting_role(&self) -> &str {
        &self.identity.acting_role
    }
}

impl Drop for ActScope {
    fn drop(&mut self) {
        DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
        if self.outermost {
            activity::leave();
        }
    }
}

/// List this backend in `graph_activity()` for a whole-graph read or a
/// validation. Nothing is recorded; the listing ends when the returned
/// scope is dropped.
pub(crate) fn reading(graph_id: i64, what: &'static str) -> ActScope {
    begin(graph_id, what)
}

/// Record an act that has already happened, in one step.
pub(crate) fn record(graph_id: i64, act: &'static str, detail: Value) -> Option<i64> {
    begin(graph_id, act).finish(detail)
}

/// Does the act table exist? False while the installed SQL is older than
/// the library.
pub(crate) fn schema_present() -> bool {
    Spi::get_one::<bool>("SELECT pg_catalog.to_regclass('pgrdf._pgrdf_graph_acts') IS NOT NULL")
        .expect("act table probe failed")
        .unwrap_or(false)
}

/// A graph was created: record who owns it and the act that created it.
/// Called from the one place every creating route ends in.
pub(crate) fn record_created(graph_id: i64) {
    if !schema_present() {
        return;
    }
    let act = begin(graph_id, "create");
    let owner = act.acting_role().to_string();
    let iri: Option<String> = Spi::get_one_with_args(
        "SELECT (SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)",
        &[graph_id.into()],
    )
    .expect("record_created: iri lookup failed");
    let act_id = act.finish(serde_json::json!({ "iri": iri }));
    // The engine's own bookkeeping, so it runs as the storage owner: a
    // role authorised to create a graph (INSERT) need not hold UPDATE on
    // the registry, and must not be able to choose what is recorded.
    crate::storage::partition::as_storage_owner(|| {
        Spi::run_with_args(
            "UPDATE pgrdf._pgrdf_graphs \
                SET owner = coalesce(owner, $2), \
                    created_at = coalesce(created_at, pg_catalog.clock_timestamp()), \
                    created_act = coalesce(created_act, $3) \
              WHERE graph_id = $1",
            &[graph_id.into(), owner.as_str().into(), act_id.into()],
        )
        .expect("record_created: registry update failed");
    });
}

/// With `pgrdf.graph_owner_enforce` on, refuse 42501 unless the caller
/// holds the privileges of the graph's recorded owner or of the storage
/// owner. Off (the default), or for a graph with no recorded owner,
/// privileges alone decide, as before graphs had owners.
///
/// The test is USAGE, never membership: a login that may `SET ROLE` to
/// every person's role without inheriting them (the shape a shared server
/// uses) holds nobody's graphs until it does switch. An owner whose role
/// no longer exists leaves the graph to the storage owner.
pub(crate) fn require_graph_owner(graph_id: i64, caller: &str) {
    if !crate::query::guc::graph_owner_enforce() || !schema_present() {
        return;
    }
    let row = Spi::get_two_with_args::<String, bool>(
        "SELECT g.owner, \
                coalesce(pg_catalog.pg_has_role(current_user, r.oid, 'USAGE'), false) \
                OR pg_catalog.pg_has_role(current_user, \
                     (SELECT relowner FROM pg_catalog.pg_class \
                       WHERE oid = 'pgrdf._pgrdf_quads'::regclass), 'USAGE') \
           FROM pgrdf._pgrdf_graphs g \
           LEFT JOIN pg_catalog.pg_roles r ON r.rolname = g.owner \
          WHERE g.graph_id = $1",
        &[graph_id.into()],
    );
    let Ok((Some(owner), allowed)) = row else {
        return; // no such graph, or no recorded owner
    };
    if allowed.unwrap_or(false) {
        return;
    }
    let role: String = Spi::get_one("SELECT pg_catalog.quote_ident(current_user::text)")
        .expect("current_user lookup failed")
        .unwrap_or_default();
    crate::refuse_with_hint(
        pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
        format!(
            "{caller}: graph {graph_id} is owned by {owner}; role {role} is neither that owner \
             nor the storage owner (pgrdf.graph_owner_enforce is on)"
        ),
        format!(
            "Act as {owner}, or have {owner} or the storage owner run \
             pgrdf.set_graph_owner({graph_id}, '<role>')."
        ),
    );
}

/// Hand a graph to another role. Returns the new owner's name.
///
/// Allowed for whoever holds the privileges of the current owner, and for
/// the storage owner. A graph with no recorded owner (created before
/// 0.6.41) can be claimed by a role allowed to drop it. As with
/// `ALTER ... OWNER TO`, the caller must be able to act as the new owner,
/// unless it is the storage owner. The transfer is recorded as an act.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn set_graph_owner(graph_id: i64, role: &str) -> String {
    let fn_name = "set_graph_owner";
    let current = Spi::get_two_with_args::<bool, String>(
        "SELECT true, owner FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
        &[graph_id.into()],
    );
    let Ok((Some(true), current_owner)) = current else {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
            format!("{fn_name}: no graph with id {graph_id}"),
        );
    };
    // The target must be an existing role; its stored spelling is the
    // catalog's. `can_be` follows ALTER ... OWNER TO: PG 16 asks for the
    // SET option, earlier servers for membership.
    let can_be = if cfg!(any(feature = "pg14", feature = "pg15")) {
        "MEMBER"
    } else {
        "SET"
    };
    let target = Spi::get_three_with_args::<String, bool, bool>(
        &format!(
            "SELECT r.rolname::text, \
                    pg_catalog.pg_has_role(current_user, r.oid, '{can_be}'), \
                    pg_catalog.pg_has_role(current_user, \
                      (SELECT relowner FROM pg_catalog.pg_class \
                        WHERE oid = 'pgrdf._pgrdf_quads'::regclass), 'USAGE') \
               FROM pg_catalog.pg_roles r WHERE r.rolname = $1"
        ),
        &[role.into()],
    );
    let Ok((Some(new_owner), can_be_new, is_storage_owner)) = target else {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
            format!("{fn_name}: role \"{role}\" does not exist"),
        );
    };
    let is_storage_owner = is_storage_owner.unwrap_or(false);
    if !is_storage_owner {
        let me: String = Spi::get_one("SELECT pg_catalog.quote_ident(current_user::text)")
            .expect("current_user lookup failed")
            .unwrap_or_default();
        let holds_current = match &current_owner {
            Some(owner) => Spi::get_one_with_args::<bool>(
                "SELECT coalesce((SELECT pg_catalog.pg_has_role(current_user, r.oid, 'USAGE') \
                                    FROM pg_catalog.pg_roles r WHERE r.rolname = $1), false)",
                &[owner.as_str().into()],
            )
            .expect("owner check failed")
            .unwrap_or(false),
            // Not recorded: the grants that would let the caller drop the
            // graph let it claim the graph.
            None => {
                crate::storage::partition::require_graph_ddl_privilege(
                    crate::storage::partition::GraphDdl::Drop,
                    fn_name,
                );
                true
            }
        };
        if !holds_current {
            crate::refuse_with_hint(
                pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
                format!(
                    "{fn_name}: graph {graph_id} is owned by {}; role {me} is neither that \
                     owner nor the storage owner",
                    current_owner.as_deref().unwrap_or("nobody")
                ),
                "The current owner or the storage owner transfers a graph.".to_string(),
            );
        }
        if !can_be_new.unwrap_or(false) {
            crate::refuse_with_hint(
                pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INSUFFICIENT_PRIVILEGE,
                format!(
                    "{fn_name}: role {me} cannot act as {new_owner}, so cannot give it a graph"
                ),
                format!("GRANT {new_owner} TO {me}, or have the storage owner transfer the graph."),
            );
        }
    }
    let act = begin(graph_id, "set_owner");
    // Authorised above by ownership, not by a grant on the registry.
    crate::storage::partition::as_storage_owner(|| {
        Spi::run_with_args(
            "UPDATE pgrdf._pgrdf_graphs SET owner = $2 WHERE graph_id = $1",
            &[graph_id.into(), new_owner.as_str().into()],
        )
        .expect("set_graph_owner: update failed");
    });
    act.finish(serde_json::json!({ "from": current_owner, "to": new_owner }));
    new_owner
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// Run `q` as `role` in a subtransaction: 'ok', or SQLSTATE|hint.
    fn try_fn() {
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.act_as(role text, q text) RETURNS text \
             LANGUAGE plpgsql AS $$ DECLARE st text; h text; BEGIN \
               BEGIN \
                 IF role IS NOT NULL THEN EXECUTE format('SET LOCAL ROLE %I', role); END IF; \
                 EXECUTE q; \
                 RESET ROLE; \
                 RETURN 'ok'; \
               EXCEPTION WHEN OTHERS THEN \
                 GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, h = PG_EXCEPTION_HINT; \
                 RETURN st || '|' || coalesce(h, ''); \
               END; \
             END $$",
        )
        .unwrap();
    }

    fn act_as(role: Option<&str>, q: &str) -> String {
        Spi::get_one_with_args::<String>("SELECT pg_temp.act_as($1, $2)", &[role.into(), q.into()])
            .unwrap()
            .unwrap()
    }

    /// A role holding the grants the install guide documents.
    fn app_role(name: &str) {
        Spi::run(&format!("CREATE ROLE {name} NOLOGIN")).unwrap();
        Spi::run(&format!("GRANT USAGE ON SCHEMA pgrdf TO {name}")).unwrap();
        Spi::run(&format!(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON pgrdf._pgrdf_quads, pgrdf._pgrdf_graphs, \
             pgrdf._pgrdf_dictionary TO {name}"
        ))
        .unwrap();
    }

    fn one<T: pgrx::datum::FromDatum + pgrx::datum::IntoDatum>(q: &str) -> Option<T> {
        Spi::get_one::<T>(q).unwrap()
    }

    /// The acts of one graph, oldest first, as `act` names.
    fn acts_of(graph: i64) -> Vec<String> {
        Spi::connect(|c| {
            c.select(
                &format!("SELECT act FROM pgrdf.graph_acts({graph}) ORDER BY act_id"),
                None,
                &[],
            )
            .unwrap()
            .map(|r| r.get::<String>(1).unwrap().unwrap())
            .collect()
        })
    }

    /// A graph is owned by the role in effect when it was created, by
    /// name, and creating it is recorded as an act of that role.
    #[pg_test]
    fn creating_a_graph_records_its_owner_and_a_create_act() {
        crate::storage::partition::acquire_partition_ddl_gate();
        let g: i64 = one("SELECT pgrdf.add_graph('urn:tdd:a1:created')").unwrap();
        let (owner, has_at, act) = Spi::get_three::<String, bool, i64>(&format!(
            "SELECT owner, created_at IS NOT NULL, created_act \
             FROM pgrdf._pgrdf_graphs WHERE graph_id = {g}"
        ))
        .unwrap();
        let me: String = one("SELECT current_user::text").unwrap();
        assert_eq!(owner.as_deref(), Some(me.as_str()));
        assert_eq!(has_at, Some(true));
        let act = act.expect("the creating act is recorded on the graph");
        let ok: bool = one(&format!(
            "SELECT act = 'create' AND graph_id = {g} AND graph_iri = 'urn:tdd:a1:created' \
                AND acting_role = current_user AND session_role = session_user \
                AND xact = pg_current_xact_id() AND at <= clock_timestamp() \
             FROM pgrdf.graph_acts({g}) WHERE act_id = {act}"
        ))
        .unwrap();
        assert!(
            ok,
            "the create act carries the graph, the roles and the transaction"
        );
        // Every creating route records: by id, by (id, iri), create_graph.
        Spi::run("SELECT pgrdf.add_graph(880041)").unwrap();
        Spi::run("SELECT pgrdf.add_graph(880042, 'urn:tdd:a1:pair')").unwrap();
        let c: i64 = one("SELECT pgrdf.create_graph('urn:tdd:a1:claimed')").unwrap();
        for id in [880041, 880042, c] {
            assert_eq!(acts_of(id), vec!["create"], "graph {id}");
            let owner: Option<String> = one(&format!(
                "SELECT owner FROM pgrdf._pgrdf_graphs WHERE graph_id = {id}"
            ));
            assert_eq!(owner.as_deref(), Some(me.as_str()), "graph {id}");
        }
        // Asking for an existing graph again creates nothing and records nothing.
        Spi::run("SELECT pgrdf.add_graph('urn:tdd:a1:created')").unwrap();
        assert_eq!(acts_of(g), vec!["create"]);
    }

    /// After SET ROLE the graph belongs to the role acted as, and the act
    /// row shows both the login and that role.
    #[pg_test]
    fn set_role_decides_the_owner_and_the_act_shows_both_roles() {
        crate::storage::partition::acquire_partition_ddl_gate();
        try_fn();
        app_role("pgrdf_a1_worker");
        assert_eq!(
            act_as(
                Some("pgrdf_a1_worker"),
                "SELECT pgrdf.add_graph('urn:tdd:a1:setrole')"
            ),
            "ok"
        );
        let ok: bool = one(
            "SELECT g.owner = 'pgrdf_a1_worker' AND a.acting_role = 'pgrdf_a1_worker' \
                AND a.session_role = session_user::text AND a.session_role <> a.acting_role \
             FROM pgrdf._pgrdf_graphs g JOIN pgrdf._pgrdf_graph_acts a ON a.act_id = g.created_act \
             WHERE g.iri = 'urn:tdd:a1:setrole'",
        )
        .unwrap();
        assert!(ok);
    }

    /// Each kind of graph-level act leaves exactly one row, with what a
    /// reader needs in `detail`.
    #[pg_test]
    fn each_graph_act_is_recorded_once() {
        crate::storage::partition::acquire_partition_ddl_gate();
        let g: i64 = one("SELECT pgrdf.add_graph('urn:tdd:a2:g')").unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.parse_turtle('<urn:a2:s> a <urn:a2:C> . \
             <urn:a2:C> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <urn:a2:D> .', {g})"
        ))
        .unwrap();
        Spi::run(&format!("SELECT pgrdf.materialize({g}, 'rdfs')")).unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.put_construct_row('{{\"subject\": {{\"type\": \"iri\", \"value\": \"urn:a2:s\"}}, \
               \"predicate\": {{\"type\": \"iri\", \"value\": \"urn:a2:p\"}}, \
               \"object\": {{\"type\": \"literal\", \"value\": \"v\"}}}}'::jsonb, {g})"
        ))
        .unwrap();
        Spi::run(&format!(
            "SELECT pgrdf.lock_graph({g}, 'sealed for review')"
        ))
        .unwrap();
        Spi::run(&format!("SELECT pgrdf.unlock_graph({g}, 'review done')")).unwrap();
        let d: i64 = one("SELECT pgrdf.add_graph('urn:tdd:a2:copy')").unwrap();
        Spi::run(&format!("SELECT pgrdf.copy_graph({g}, {d})")).unwrap();
        Spi::run(&format!("SELECT pgrdf.clear_graph({g})")).unwrap();
        assert_eq!(
            acts_of(g),
            vec![
                "create",
                "load",
                "materialize",
                "ingest",
                "lock",
                "unlock",
                "clear"
            ]
        );
        assert_eq!(acts_of(d), vec!["create", "copy"]);
        let detail = |graph: i64, act: &str, path: &str| -> Option<String> {
            one(&format!(
                "SELECT detail #>> '{{{path}}}' FROM pgrdf.graph_acts({graph}) WHERE act = '{act}'"
            ))
        };
        assert_eq!(
            detail(g, "lock", "reason").as_deref(),
            Some("sealed for review")
        );
        assert_eq!(
            detail(g, "unlock", "reason").as_deref(),
            Some("review done")
        );
        assert_eq!(detail(g, "materialize", "profile").as_deref(), Some("rdfs"));
        assert_eq!(
            detail(d, "copy", "from").as_deref(),
            Some(g.to_string().as_str())
        );
        assert!(detail(g, "clear", "rows").is_some());
        // The lock's act is on the registry row while the lock holds.
        Spi::run(&format!("SELECT pgrdf.lock_graph({d}, 'kept')")).unwrap();
        let ok: bool = one(&format!(
            "SELECT g.locked_act = (SELECT max(act_id) FROM pgrdf.graph_acts({d}) WHERE act = 'lock') \
             FROM pgrdf._pgrdf_graphs g WHERE graph_id = {d}"
        ))
        .unwrap();
        assert!(ok, "locked_act names the lock act");
        Spi::run(&format!("SELECT pgrdf.unlock_graph({d}, 'released')")).unwrap();
        // Drop keeps the graph's IRI on the act; move records both ends.
        let m: i64 = one("SELECT pgrdf.add_graph('urn:tdd:a2:moved')").unwrap();
        Spi::run(&format!("SELECT pgrdf.move_graph({d}, {m})")).unwrap();
        assert_eq!(acts_of(m), vec!["create", "move"]);
        assert_eq!(acts_of(d).last().map(String::as_str), Some("move"));
        Spi::run(&format!("SELECT pgrdf.drop_graph({m}, true)")).unwrap();
        let iri: Option<String> = one(&format!(
            "SELECT graph_iri FROM pgrdf.graph_acts({m}) WHERE act = 'drop'"
        ));
        assert_eq!(iri.as_deref(), Some("urn:tdd:a2:moved"));
        // since: only acts from that moment on.
        let n: i64 = one(&format!(
            "SELECT count(*) FROM pgrdf.graph_acts({g}, clock_timestamp())"
        ))
        .unwrap();
        assert_eq!(n, 0);
    }

    /// The declared settings are copied as they stand when the act starts,
    /// so two acts on one connection carry their own values.
    #[pg_test]
    fn declared_identity_is_copied_per_act() {
        crate::storage::partition::acquire_partition_ddl_gate();
        Spi::run("SET LOCAL pgrdf.workspace = 'atlas'").unwrap();
        Spi::run("SET LOCAL pgrdf.agent = 'loader-1'").unwrap();
        Spi::run("SET LOCAL pgrdf.thread = '65d9f3b9'").unwrap();
        Spi::run("SET LOCAL pgrdf.on_behalf_of = 'ada'").unwrap();
        Spi::run("SET LOCAL pgrdf.harness = 'claude-code/2.1.330'").unwrap();
        Spi::run("SET LOCAL application_name = 'pgrdf-mcp:h1'").unwrap();
        let a: i64 = one("SELECT pgrdf.add_graph('urn:tdd:a3:first')").unwrap();
        Spi::run("SET LOCAL pgrdf.workspace = 'routes'").unwrap();
        Spi::run("SET LOCAL pgrdf.agent = ''").unwrap();
        let b: i64 = one("SELECT pgrdf.add_graph('urn:tdd:a3:second')").unwrap();
        let row = |g: i64| -> String {
            one::<String>(&format!(
                "SELECT concat_ws('|', coalesce(workspace, '-'), coalesce(agent, '-'), \
                        coalesce(thread, '-'), coalesce(on_behalf_of, '-'), coalesce(harness, '-'), \
                        coalesce(application_name, '-')) \
                 FROM pgrdf.graph_acts({g}) WHERE act = 'create'"
            ))
            .unwrap()
        };
        assert_eq!(
            row(a),
            "atlas|loader-1|65d9f3b9|ada|claude-code/2.1.330|pgrdf-mcp:h1"
        );
        assert_eq!(
            row(b),
            "routes|-|65d9f3b9|ada|claude-code/2.1.330|pgrdf-mcp:h1",
            "an empty setting is recorded as not stated"
        );
        // The inventory shows who created each graph.
        let inv: String = one(&format!(
            "SELECT concat_ws('|', owner, created_workspace, coalesce(created_agent, '-'), created_thread) \
             FROM pgrdf.graph_inventory() WHERE graph_id = {b}"
        ))
        .unwrap();
        let me: String = one("SELECT current_user::text").unwrap();
        assert_eq!(inv, format!("{me}|routes|-|65d9f3b9"));
    }

    /// The declared settings refuse what a reader could not trust on
    /// sight: over 128 bytes (22001) and control characters (22023). The
    /// `pgrdf.` prefix is reserved, so a misspelt name is an error.
    #[pg_test]
    fn declared_settings_are_checked_at_set() {
        crate::storage::partition::acquire_partition_ddl_gate();
        try_fn();
        let long = "x".repeat(129);
        assert!(
            act_as(None, &format!("SET LOCAL pgrdf.workspace = '{long}'")).starts_with("22001|")
        );
        assert_eq!(
            act_as(
                None,
                &format!("SET LOCAL pgrdf.workspace = '{}'", "x".repeat(128))
            ),
            "ok"
        );
        for name in ["workspace", "agent", "thread", "on_behalf_of", "harness"] {
            assert!(
                act_as(None, &format!("SET LOCAL pgrdf.{name} = E'a\\nb'")).starts_with("22023|"),
                "pgrdf.{name} must refuse a control character"
            );
        }
        if cfg!(not(feature = "pg14")) {
            assert!(
                !act_as(None, "SET LOCAL pgrdf.workspce = 'typo'").starts_with("ok"),
                "a misspelt pgrdf.* name must be an error"
            );
        }
    }

    /// The act record is written by the engine. A role holding every
    /// grant on the table still cannot forge, change or delete a row.
    #[pg_test]
    fn act_rows_cannot_be_forged() {
        crate::storage::partition::acquire_partition_ddl_gate();
        try_fn();
        app_role("pgrdf_a2_forger");
        Spi::run("GRANT ALL ON pgrdf._pgrdf_graph_acts TO pgrdf_a2_forger").unwrap();
        Spi::run("GRANT ALL ON SEQUENCE pgrdf._pgrdf_graph_act_id_seq TO pgrdf_a2_forger").unwrap();
        let g: i64 = one("SELECT pgrdf.add_graph('urn:tdd:a2:forge')").unwrap();
        let forger = Some("pgrdf_a2_forger");
        assert!(
            act_as(
                forger,
                &format!(
                    "INSERT INTO pgrdf._pgrdf_graph_acts (act_id, graph_id, act, at, xact, \
                     session_role, acting_role) VALUES (999999999, {g}, 'drop', now(), \
                     pg_current_xact_id(), 'someone', 'someone')"
                )
            )
            .starts_with("42501|")
        );
        assert!(
            act_as(
                forger,
                "UPDATE pgrdf._pgrdf_graph_acts SET acting_role = 'x'"
            )
            .starts_with("42501|")
        );
        assert!(act_as(forger, "DELETE FROM pgrdf._pgrdf_graph_acts").starts_with("42501|"));
        assert!(act_as(forger, "TRUNCATE pgrdf._pgrdf_graph_acts").starts_with("42501|"));
        // The same role's own act is still recorded, by the engine.
        assert_eq!(
            act_as(forger, "SELECT pgrdf.add_graph('urn:tdd:a2:forger-own')"),
            "ok"
        );
        let who: String = one(
            "SELECT acting_role FROM pgrdf._pgrdf_graph_acts a JOIN pgrdf._pgrdf_graphs g \
               ON g.created_act = a.act_id WHERE g.iri = 'urn:tdd:a2:forger-own'",
        )
        .unwrap();
        assert_eq!(who, "pgrdf_a2_forger");
    }

    /// set_graph_owner hands a graph to another role and records it.
    #[pg_test]
    fn ownership_is_transferred_and_recorded() {
        crate::storage::partition::acquire_partition_ddl_gate();
        try_fn();
        app_role("pgrdf_a1_from");
        app_role("pgrdf_a1_to");
        assert_eq!(
            act_as(
                Some("pgrdf_a1_from"),
                "SELECT pgrdf.add_graph('urn:tdd:a1:xfer')"
            ),
            "ok"
        );
        let g: i64 = one("SELECT pgrdf.graph_id('urn:tdd:a1:xfer')").unwrap();
        // The owner cannot give the graph to a role it cannot act as.
        assert!(
            act_as(
                Some("pgrdf_a1_from"),
                &format!("SELECT pgrdf.set_graph_owner({g}, 'pgrdf_a1_to')")
            )
            .starts_with("42501|")
        );
        assert!(
            act_as(
                None,
                &format!("SELECT pgrdf.set_graph_owner({g}, 'pgrdf_a1_nobody')")
            )
            .starts_with("42704|")
        );
        // The storage owner can.
        assert_eq!(
            act_as(
                None,
                &format!("SELECT pgrdf.set_graph_owner({g}, 'pgrdf_a1_to')")
            ),
            "ok"
        );
        let ok: bool = one(&format!(
            "SELECT g.owner = 'pgrdf_a1_to' AND a.detail ->> 'from' = 'pgrdf_a1_from' \
                AND a.detail ->> 'to' = 'pgrdf_a1_to' \
             FROM pgrdf._pgrdf_graphs g, pgrdf.graph_acts({g}) a \
             WHERE g.graph_id = {g} AND a.act = 'set_owner'"
        ))
        .unwrap();
        assert!(ok);
        // A role that is not the owner cannot take the graph.
        assert!(
            act_as(
                Some("pgrdf_a1_from"),
                &format!("SELECT pgrdf.set_graph_owner({g}, 'pgrdf_a1_from')")
            )
            .starts_with("42501|")
        );
    }

    /// With pgrdf.graph_owner_enforce on, the destructive acts belong to
    /// the graph owner and the storage owner. With it off, grants decide,
    /// as before. The declared settings never change who may act.
    #[pg_test]
    fn owner_enforcement_gates_destructive_acts_only_when_on() {
        crate::storage::partition::acquire_partition_ddl_gate();
        try_fn();
        app_role("pgrdf_d8_owner");
        app_role("pgrdf_d8_other");
        assert_eq!(
            act_as(
                Some("pgrdf_d8_owner"),
                "SELECT pgrdf.add_graph('urn:tdd:d8:g')"
            ),
            "ok"
        );
        let g: i64 = one("SELECT pgrdf.graph_id('urn:tdd:d8:g')").unwrap();
        let other = Some("pgrdf_d8_other");
        Spi::run("SET LOCAL pgrdf.graph_owner_enforce = on").unwrap();
        Spi::run("SET LOCAL pgrdf.on_behalf_of = 'pgrdf_d8_owner'").unwrap();
        for q in [
            format!("SELECT pgrdf.lock_graph({g}, 'mine now')"),
            format!("SELECT pgrdf.clear_graph({g})"),
            format!("SELECT pgrdf.drop_graph({g}, true)"),
        ] {
            let got = act_as(other, &q);
            assert!(got.starts_with("42501|"), "{q}: {got}");
            assert!(
                got.contains("pgrdf_d8_owner"),
                "the HINT names the owner: {got}"
            );
        }
        // Not destructive: a non-owner with the grants still loads.
        assert_eq!(
            act_as(
                other,
                &format!("SELECT pgrdf.parse_turtle('<urn:d8:s> <urn:d8:p> <urn:d8:o> .', {g})")
            ),
            "ok"
        );
        // The owner acts; so does the storage owner.
        assert_eq!(
            act_as(
                Some("pgrdf_d8_owner"),
                &format!("SELECT pgrdf.lock_graph({g}, 'owner')")
            ),
            "ok"
        );
        assert_eq!(
            act_as(
                None,
                &format!("SELECT pgrdf.unlock_graph({g}, 'storage owner')")
            ),
            "ok"
        );
        // A graph with no recorded owner stays governed by grants alone.
        Spi::run("SELECT pgrdf.add_graph('urn:tdd:d8:unowned')").unwrap();
        Spi::run("UPDATE pgrdf._pgrdf_graphs SET owner = NULL WHERE iri = 'urn:tdd:d8:unowned'")
            .unwrap();
        assert_eq!(
            act_as(other, "SELECT pgrdf.clear_graph('urn:tdd:d8:unowned')"),
            "ok"
        );
        // An owner that no longer exists: only the storage owner may act.
        Spi::run("UPDATE pgrdf._pgrdf_graphs SET owner = 'pgrdf_d8_gone' WHERE iri = 'urn:tdd:d8:unowned'").unwrap();
        assert!(
            act_as(other, "SELECT pgrdf.clear_graph('urn:tdd:d8:unowned')").starts_with("42501|")
        );
        assert_eq!(
            act_as(None, "SELECT pgrdf.clear_graph('urn:tdd:d8:unowned')"),
            "ok"
        );
        // Off: behaviour is what it was before ownership existed.
        Spi::run("SET LOCAL pgrdf.graph_owner_enforce = off").unwrap();
        assert_eq!(
            act_as(other, &format!("SELECT pgrdf.drop_graph({g}, true)")),
            "ok"
        );
    }

    /// graph_activity lists a backend while it is inside a graph-level
    /// act, with its declared identity, and nothing once the act ends.
    #[pg_test]
    fn graph_activity_lists_an_act_while_it_runs() {
        crate::storage::partition::acquire_partition_ddl_gate();
        let g: i64 = one("SELECT pgrdf.add_graph('urn:tdd:a3:live')").unwrap();
        Spi::run("SET LOCAL pgrdf.workspace = 'live-ws'").unwrap();
        let mine = || -> i64 {
            one("SELECT count(*) FROM pgrdf.graph_activity() WHERE pid = pg_backend_pid()").unwrap()
        };
        assert_eq!(mine(), 0, "idle: not listed");
        {
            let _act = crate::storage::acts::begin(g, "load");
            let row: String = one(&format!(
                "SELECT concat_ws('|', graph_id, act, workspace, acting_role = current_user, \
                        started <= clock_timestamp()) \
                 FROM pgrdf.graph_activity({g}) WHERE pid = pg_backend_pid()"
            ))
            .unwrap();
            assert_eq!(row, format!("{g}|load|live-ws|t|t"));
            let elsewhere: i64 =
                one("SELECT count(*) FROM pgrdf.graph_activity(0) WHERE pid = pg_backend_pid()")
                    .unwrap();
            assert_eq!(elsewhere, 0, "the graph filter applies");
        }
        assert_eq!(mine(), 0, "the act ended: not listed");
        // Dropped without finishing (an error path) writes no act row.
        assert_eq!(acts_of(g), vec!["create"]);
    }
}
