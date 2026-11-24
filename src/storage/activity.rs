//! Who is inside a graph-level act right now: `pgrdf.graph_activity()`.
//!
//! A backend fills one shared-memory slot when it enters a graph-level act
//! (a write act, a whole-graph read, a validation) and clears it when the
//! act ends, on success or error. It reports and never blocks: custody
//! stays with `lock_graph`.
//!
//! The table has a fixed number of slots. When more backends than that are
//! inside an act at once, the surplus are simply not listed; nothing waits
//! and nothing fails. Without `shared_preload_libraries = 'pgrdf'` there is
//! no shared memory and the function returns no rows.

use core::sync::atomic::{AtomicBool, Ordering};
use pgrx::prelude::*;
use pgrx::{PGRXSharedMemory, PgLwLock, pg_shmem_init};

/// Backends listed at once.
pub const SLOTS: usize = 256;
const NAME_CAP: usize = 64;
const TEXT_CAP: usize = 128;
const ACT_CAP: usize = 16;

/// How the listed backend takes part in the act.
pub mod part {
    /// An ordinary backend running the call.
    pub const BACKEND: u8 = 0;
    /// The backend that called the staged loader and waits for its workers.
    pub const COORDINATOR: u8 = 1;
    /// A staged-loader background worker, carrying its caller's identity.
    pub const WORKER: u8 = 2;
}

/// A fixed inline string: shared memory cannot hold pointers.
#[derive(Copy, Clone)]
#[repr(C)]
struct Inline<const N: usize> {
    len: u16,
    bytes: [u8; N],
}

impl<const N: usize> Inline<N> {
    const fn empty() -> Self {
        Self {
            len: 0,
            bytes: [0; N],
        }
    }

    /// Keep as much of `value` as fits, cut on a character boundary.
    fn of(value: Option<&str>) -> Self {
        let mut out = Self::empty();
        if let Some(v) = value {
            let mut n = v.len().min(N);
            while n > 0 && !v.is_char_boundary(n) {
                n -= 1;
            }
            out.bytes[..n].copy_from_slice(&v.as_bytes()[..n]);
            out.len = n as u16;
        }
        out
    }

    fn get(&self) -> Option<String> {
        let n = (self.len as usize).min(N);
        (n > 0).then(|| String::from_utf8_lossy(&self.bytes[..n]).into_owned())
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
struct Slot {
    pid: i32,
    db_oid: u32,
    part: u8,
    graph_id: i64,
    /// Start of the act, as a PostgreSQL TimestampTz (microseconds).
    started: i64,
    act: Inline<ACT_CAP>,
    session_role: Inline<NAME_CAP>,
    acting_role: Inline<NAME_CAP>,
    application_name: Inline<NAME_CAP>,
    requester: Inline<TEXT_CAP>,
    workspace: Inline<TEXT_CAP>,
    agent: Inline<TEXT_CAP>,
    thread: Inline<TEXT_CAP>,
    on_behalf_of: Inline<TEXT_CAP>,
    harness: Inline<TEXT_CAP>,
}
unsafe impl PGRXSharedMemory for Slot {}

impl Slot {
    const fn empty() -> Self {
        Self {
            pid: 0,
            db_oid: 0,
            part: part::BACKEND,
            graph_id: 0,
            started: 0,
            act: Inline::empty(),
            session_role: Inline::empty(),
            acting_role: Inline::empty(),
            application_name: Inline::empty(),
            requester: Inline::empty(),
            workspace: Inline::empty(),
            agent: Inline::empty(),
            thread: Inline::empty(),
            on_behalf_of: Inline::empty(),
            harness: Inline::empty(),
        }
    }
}

static ACTIVITY: PgLwLock<[Slot; SLOTS]> = unsafe { PgLwLock::new(c"pgrdf_graph_activity") };
static SHMEM_READY: AtomicBool = AtomicBool::new(false);

/// Register the slot table. Postmaster only, beside the other segments.
pub fn init_in_postmaster() {
    pg_shmem_init!(ACTIVITY = [Slot::empty(); SLOTS]);
    SHMEM_READY.store(true, Ordering::Relaxed);
}

fn is_ready() -> bool {
    SHMEM_READY.load(Ordering::Relaxed)
}

/// The identity a slot shows; the act record carries the same fields.
pub(crate) struct Shown<'a> {
    pub session_role: &'a str,
    pub acting_role: &'a str,
    pub application_name: Option<&'a str>,
    pub requester: Option<&'a str>,
    pub workspace: Option<&'a str>,
    pub agent: Option<&'a str>,
    pub thread: Option<&'a str>,
    pub on_behalf_of: Option<&'a str>,
    pub harness: Option<&'a str>,
}

/// List this backend as inside `act` on `graph_id`. Reuses this backend's
/// slot, else takes a free one; with none free the act is not listed.
pub(crate) fn enter(graph_id: i64, act: &str, part: u8, who: &Shown<'_>) {
    if !is_ready() {
        return;
    }
    let pid = unsafe { pgrx::pg_sys::MyProcPid };
    let db_oid: u32 = unsafe { pgrx::pg_sys::MyDatabaseId }.into();
    let started = unsafe { pgrx::pg_sys::GetCurrentTimestamp() };
    let mut slots = ACTIVITY.exclusive();
    let idx = slots
        .iter()
        .position(|s| s.pid == pid)
        .or_else(|| slots.iter().position(|s| s.pid == 0));
    let Some(idx) = idx else {
        return;
    };
    slots[idx] = Slot {
        pid,
        db_oid,
        part,
        graph_id,
        started,
        act: Inline::of(Some(act)),
        session_role: Inline::of(Some(who.session_role)),
        acting_role: Inline::of(Some(who.acting_role)),
        application_name: Inline::of(who.application_name),
        requester: Inline::of(who.requester),
        workspace: Inline::of(who.workspace),
        agent: Inline::of(who.agent),
        thread: Inline::of(who.thread),
        on_behalf_of: Inline::of(who.on_behalf_of),
        harness: Inline::of(who.harness),
    };
}

/// Stop listing this backend.
pub(crate) fn leave() {
    if !is_ready() {
        return;
    }
    let pid = unsafe { pgrx::pg_sys::MyProcPid };
    let mut slots = ACTIVITY.exclusive();
    for s in slots.iter_mut().filter(|s| s.pid == pid) {
        *s = Slot::empty();
    }
}

/// Backends inside a pgRDF graph-level act right now, in this database:
/// write acts, whole-graph reads (export, manifest, digests) and
/// validation. `part` is `backend`, or `coordinator` / `worker` for a
/// staged load.
///
/// Visibility follows `pg_stat_activity`: only this database's backends are
/// listed, and a session of another role shows only its `pid` and
/// `session_role` unless the caller holds that role's privileges or
/// `pg_read_all_stats`. A slot whose backend is gone is not listed.
#[search_path(pgrdf, pg_temp)]
#[pg_extern(stable)]
#[allow(clippy::type_complexity)]
fn graph_activity(
    graph_id: default!(Option<i64>, "NULL"),
) -> TableIterator<
    'static,
    (
        name!(pid, i32),
        name!(graph_id, Option<i64>),
        name!(act, Option<String>),
        name!(started, Option<pgrx::datum::TimestampWithTimeZone>),
        name!(part, Option<String>),
        name!(session_role, Option<String>),
        name!(acting_role, Option<String>),
        name!(application_name, Option<String>),
        name!(requester, Option<String>),
        name!(workspace, Option<String>),
        name!(agent, Option<String>),
        name!(thread, Option<String>),
        name!(on_behalf_of, Option<String>),
        name!(harness, Option<String>),
    ),
> {
    let mut rows = Vec::new();
    if !is_ready() {
        return TableIterator::new(rows.into_iter());
    }
    let db_oid: u32 = unsafe { pgrx::pg_sys::MyDatabaseId }.into();
    // Copy out under the lock, then release it before any SQL runs.
    let live: Vec<Slot> = {
        let slots = ACTIVITY.share();
        slots
            .iter()
            .filter(|s| s.pid != 0 && s.db_oid == db_oid)
            .filter(|s| graph_id.is_none_or(|g| g == s.graph_id))
            .copied()
            .collect()
    };
    if live.is_empty() {
        return TableIterator::new(rows.into_iter());
    }
    // One read of pg_stat_activity: which of these backends exist, and
    // which may this caller see in full.
    let pids: Vec<i32> = live.iter().map(|s| s.pid).collect();
    let seen: std::collections::HashMap<i32, (bool, Option<String>)> = Spi::connect(|c| {
        c.select(
            "SELECT a.pid, \
                    a.usesysid IS NOT NULL \
                      AND (pg_catalog.pg_has_role(current_user, a.usesysid, 'USAGE') \
                           OR pg_catalog.pg_has_role(current_user, 'pg_read_all_stats', 'USAGE')), \
                    a.usename::text \
               FROM pg_catalog.pg_stat_activity a \
              WHERE a.pid = ANY($1)",
            None,
            &[pids.into()],
        )
        .expect("graph_activity: pg_stat_activity read failed")
        .map(|r| {
            (
                r.get::<i32>(1).unwrap().unwrap_or(0),
                (
                    r.get::<bool>(2).unwrap().unwrap_or(false),
                    r.get::<String>(3).unwrap(),
                ),
            )
        })
        .collect()
    });
    for s in live {
        let Some((visible, usename)) = seen.get(&s.pid) else {
            continue; // the backend is gone; the slot is stale
        };
        if !visible {
            rows.push((
                s.pid,
                None,
                None,
                None,
                None,
                usename.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            ));
            continue;
        }
        let started =
            pgrx::datum::TimestampWithTimeZone::try_from(s.started as pgrx::pg_sys::TimestampTz)
                .ok();
        let part = match s.part {
            part::COORDINATOR => "coordinator",
            part::WORKER => "worker",
            _ => "backend",
        };
        rows.push((
            s.pid,
            Some(s.graph_id),
            s.act.get(),
            started,
            Some(part.to_string()),
            s.session_role.get(),
            s.acting_role.get(),
            s.application_name.get(),
            s.requester.get(),
            s.workspace.get(),
            s.agent.get(),
            s.thread.get(),
            s.on_behalf_of.get(),
            s.harness.get(),
        ));
    }
    rows.sort_by_key(|r| r.0);
    TableIterator::new(rows.into_iter())
}
