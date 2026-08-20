//! Process-wide dictionary cache backed by PostgreSQL shared memory.
//!
//! Implements LLD §4.1 — a fixed-capacity, open-addressed hash table in
//! Postgres shmem that caches `(database_oid, term_type, lexical_value,
//! datatype_id, language) → dict_id` mappings across backends and across calls. The
//! per-call HashMap in [`super::loader`] sits on top: a load picks up
//! "saw this term inside this Turtle file" with zero locks; the shmem
//! cache then catches "saw this term in any backend since the
//! postmaster started".
//!
//! Acceptance (LLD §4.1):
//! * Hit-path latency well under 1 µs (LWLock share + two slot probes).
//! * Cross-backend reuse: a second connection's first `put_term` for an
//!   already-warmed term hits shmem, never the dictionary table.
//!
//! Database scoping. This table lives in postmaster shared memory and
//! is therefore process-wide, while `_pgrdf_dictionary` is an ordinary
//! table and therefore per-database. The same lexical term is a
//! DIFFERENT `dict_id` in each database, so the database OID is part
//! of the key: without it a fingerprint warmed by database A is a hit
//! in database B and resolves to an id that means another term there,
//! and the caller writes quads pointing at ids it does not own. The
//! generation counter does not cover this — every database on the
//! instance shares one counter and one keyspace. `super::staged::pool`
//! carries `MyDatabaseId` explicitly for the same reason.
//!
//! Transactional safety. Dictionary INSERTs can be rolled back, so
//! freshly inserted (key → id) pairs are STAGED in a per-backend
//! pending list and only published to shmem on `XACT_EVENT_COMMIT`.
//! SELECT-found rows are already committed and go directly to shmem.
//!
//! Capacity: 16 384 slots × 32 B = 512 KiB shmem. Open-addressed with
//! linear probing up to [`PROBE_DEPTH`]; full streak → evict the
//! canonical slot. 64-bit Fingerprint is stored as a u128 pair so
//! false-hit probability is ~2⁻¹²⁸ at fleet scale (one shared hasher
//! seed per half).

use pgrx::callbacks::{
    PgSubXactCallbackEvent, PgXactCallbackEvent, register_subxact_callback, register_xact_callback,
};
use pgrx::prelude::*;
use pgrx::{PGRXSharedMemory, PgAtomic, PgLwLock, pg_shmem_init};
use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

const SLOTS: usize = 16_384;
const PROBE_DEPTH: usize = 8;

#[derive(Copy, Clone, Default)]
#[repr(C)]
pub(crate) struct DictCacheSlot {
    key_hash1: u64,
    key_hash2: u64,
    /// Generation that wrote this slot. Compared against the global
    /// `GENERATION` counter on lookup; mismatch means the slot is
    /// stale (typically post `DROP EXTENSION pgrdf; CREATE EXTENSION`)
    /// and is treated as cold.
    generation: u64,
    dict_id: i64,
    occupied: u8,
    _pad: [u8; 7],
}
unsafe impl PGRXSharedMemory for DictCacheSlot {}

impl DictCacheSlot {
    /// `const`-friendly default so we can initialise the shmem array
    /// via `[default_const(); SLOTS]` (Default::default is not const).
    const fn default_const() -> Self {
        Self {
            key_hash1: 0,
            key_hash2: 0,
            generation: 0,
            dict_id: 0,
            occupied: 0,
            _pad: [0; 7],
        }
    }
}

static DICT_CACHE: PgLwLock<[DictCacheSlot; SLOTS]> =
    unsafe { PgLwLock::new(c"pgrdf_dict_cache_v1") };

pub(crate) static HITS: PgAtomic<AtomicU64> = unsafe { PgAtomic::new(c"pgrdf_dict_cache_hits") };
static MISSES: PgAtomic<AtomicU64> = unsafe { PgAtomic::new(c"pgrdf_dict_cache_misses") };
static INSERTS: PgAtomic<AtomicU64> = unsafe { PgAtomic::new(c"pgrdf_dict_cache_inserts") };
static EVICTIONS: PgAtomic<AtomicU64> = unsafe { PgAtomic::new(c"pgrdf_dict_cache_evictions") };
/// Current cache generation. Bumped by [`reset`] / `pgrdf.shmem_reset()`
/// after `DROP EXTENSION pgrdf; CREATE EXTENSION` to invalidate every
/// slot in one atomic increment. Starts at 1 so the all-zero initial
/// slot state reads as stale (slot.generation 0 ≠ current 1).
static GENERATION: PgAtomic<AtomicU64> = unsafe { PgAtomic::new(c"pgrdf_dict_cache_generation") };
/// Phase E group E1 (LLD v0.4 §7.2): count of SPARQL property-path
/// solutions truncated because the walk hit `pgrdf.path_max_depth`.
/// E1 only *scaffolds* this — the counter exists, initialises to 0,
/// is zeroed by `pgrdf.shmem_reset()`, and surfaces on
/// `pgrdf.stats()` as `path_depth_truncations`. The actual increment
/// is wired in Phase E group E2 once the recursive CTE exists (a
/// depth guard is meaningless without recursion). Cross-backend
/// cumulative, like the dict-cache counters above.
static PATH_DEPTH_TRUNCATIONS: PgAtomic<AtomicU64> =
    unsafe { PgAtomic::new(c"pgrdf_path_depth_truncations") };

/// #114: count of query translations REFUSED because a group-level
/// construct (FILTER / triple / OPTIONAL / MINUS / VALUES) sat
/// alongside a UNION, where the union assembly paths would have
/// silently dropped it and widened the result set. The refusal is the
/// fix; this counter is the belt-and-braces signal — any future
/// translation path that skips a clause instead of refusing MUST
/// increment it, so "this answer is silently incomplete" is always
/// detectable on the first call. Surfaces on `pgrdf.stats()` as
/// `filter_clauses_dropped`. Cross-backend cumulative; incremented
/// BEFORE the refusal error is raised (shmem survives the abort).
static FILTER_CLAUSES_DROPPED: PgAtomic<AtomicU64> =
    unsafe { PgAtomic::new(c"pgrdf_filter_clauses_dropped") };

/// Register shmem requests + startup hooks for the dict cache and
/// its counters + generation flag. Must be called from inside
/// `_PG_init` and ONLY when
/// `process_shared_preload_libraries_in_progress == true`. The macro
/// expansion installs hook chains and writes per-static pointers;
/// running it outside the postmaster scan installs hooks that will
/// never fire and leaks the pointer slots.
pub fn init_in_postmaster() {
    // `[T; N]: Default` only holds for N <= 32 in std, so build the
    // initial array explicitly via Copy.
    pg_shmem_init!(DICT_CACHE = [DictCacheSlot::default_const(); SLOTS]);
    pg_shmem_init!(HITS);
    pg_shmem_init!(MISSES);
    pg_shmem_init!(INSERTS);
    pg_shmem_init!(EVICTIONS);
    pg_shmem_init!(GENERATION = AtomicU64::new(1));
    pg_shmem_init!(PATH_DEPTH_TRUNCATIONS);
    pg_shmem_init!(FILTER_CLAUSES_DROPPED);
    mark_ready();
}

fn current_generation() -> u64 {
    if !is_ready() {
        return 0;
    }
    GENERATION.get().load(Ordering::Relaxed)
}

/// Atomically invalidate every shmem slot. After this returns,
/// lookups for previously-cached terms read as cold and refill from
/// the dictionary table. Use after `DROP EXTENSION pgrdf` so the new
/// extension's dict id space doesn't collide with the stale cache.
pub fn reset() {
    if !is_ready() {
        return;
    }
    GENERATION.get().fetch_add(1, Ordering::Relaxed);
    // Phase E group E1: `path_depth_truncations` is an absolute
    // counter (not generation-versioned like the dict slots), so
    // `pgrdf.shmem_reset()` must zero it directly for tests that
    // assert a clean `0` baseline (LLD v0.4 §7.2 / regression
    // invariant I).
    PATH_DEPTH_TRUNCATIONS.get().store(0, Ordering::Relaxed);
    FILTER_CLAUSES_DROPPED.get().store(0, Ordering::Relaxed);
}

/// Increment the property-path depth-truncation counter by one.
///
/// Phase E group E1 SCAFFOLD only — no caller increments yet because
/// no recursive CTE exists until Phase E group E2. E2's recursive
/// property-path translator calls this each time a solution path is
/// truncated at `pgrdf.path_max_depth`. `#[allow(dead_code)]` keeps
/// clippy quiet until that first caller lands.
#[allow(dead_code)]
pub fn note_path_depth_truncation() {
    if !is_ready() {
        return;
    }
    PATH_DEPTH_TRUNCATIONS.get().fetch_add(1, Ordering::Relaxed);
}

/// #114: record one would-have-been-dropped clause event. Called at
/// the refusal site immediately before the error is raised (and by
/// any future path that skips a clause it cannot apply).
pub fn note_filter_clause_dropped() {
    if !is_ready() {
        return;
    }
    FILTER_CLAUSES_DROPPED.get().fetch_add(1, Ordering::Relaxed);
}

/// Set true inside `_PG_init` only when Postgres is running the
/// postmaster's shared_preload_libraries scan — that's the only
/// context where `pg_shmem_init!` can successfully register the
/// shmem-request and shmem-startup hooks. In a backend that lazy-
/// loads the .so the static stays false, every lookup short-circuits,
/// and we revert to the per-call HashMap path.
static SHMEM_READY: AtomicBool = AtomicBool::new(false);

pub fn mark_ready() {
    SHMEM_READY.store(true, Ordering::Relaxed);
}

pub fn is_ready() -> bool {
    SHMEM_READY.load(Ordering::Relaxed)
}

// SipHash via DefaultHasher; collision-resistant enough for two
// independent seeds (u128 fingerprint), within budget of the per-
// lookup 1 µs target. Switching to ahash / fxhash is a v0.4 perf
// follow-up; the win in a hashmap with ~16 K slots is modest.
const SEED_A: u64 = 0x9E37_79B9_7F4A_7C15; // golden ratio
const SEED_B: u64 = 0xC4F1_7B5E_9D0A_3E27; // unrelated odd 64-bit

/// The database this backend is connected to. Every cache key is
/// scoped by it — see the module header. Taken per call rather than
/// cached in a static: a background worker in `super::staged::pool`
/// does not inherit the coordinator's database and connects to its
/// own, so a value latched at `_PG_init` in the postmaster would be
/// wrong for every backend that inherits it.
fn current_db_oid() -> u32 {
    unsafe { pg_sys::MyDatabaseId }.to_u32()
}

/// `db_oid` is a parameter rather than being read inside, so the
/// keyspace separation is provable in a plain unit test without a
/// second database — the pgrx harness runs one.
fn fingerprint(
    db_oid: u32,
    term_type: i16,
    value: &str,
    datatype_id: Option<i64>,
    language: Option<&str>,
) -> (u64, u64) {
    let mut h1 = DefaultHasher::new();
    SEED_A.hash(&mut h1);
    db_oid.hash(&mut h1);
    term_type.hash(&mut h1);
    value.hash(&mut h1);
    datatype_id.hash(&mut h1);
    language.hash(&mut h1);
    let mut h2 = DefaultHasher::new();
    SEED_B.hash(&mut h2);
    db_oid.hash(&mut h2);
    term_type.hash(&mut h2);
    value.hash(&mut h2);
    datatype_id.hash(&mut h2);
    language.hash(&mut h2);
    (h1.finish(), h2.finish())
}

/// Try to resolve a term out of the cross-backend shmem cache.
/// Returns `None` if shmem is not initialised (extension was loaded
/// outside `shared_preload_libraries`) or if the slot is cold.
pub fn lookup(
    term_type: i16,
    value: &str,
    datatype_id: Option<i64>,
    language: Option<&str>,
) -> Option<i64> {
    if !is_ready() {
        return None;
    }
    let r#gen = current_generation();
    let (h1, h2) = fingerprint(current_db_oid(), term_type, value, datatype_id, language);
    let table = DICT_CACHE.share();
    let start = (h1 as usize) % SLOTS;
    for i in 0..PROBE_DEPTH {
        let slot = &table[(start + i) % SLOTS];
        if slot.occupied != 0
            && slot.generation == r#gen
            && slot.key_hash1 == h1
            && slot.key_hash2 == h2
        {
            HITS.get().fetch_add(1, Ordering::Relaxed);
            return Some(slot.dict_id);
        }
    }
    MISSES.get().fetch_add(1, Ordering::Relaxed);
    None
}

// Per-backend list of (fingerprint, dict_id, subxact_id) entries staged
// inside the current transaction. Published on commit; discarded on
// abort. Each entry carries the subtransaction id it was staged under
// (#127): a PL/pgSQL EXCEPTION block aborts only a SUBtransaction, the
// dictionary INSERT rolls back with it, and the outer transaction still
// commits — so without the tag the commit flush publishes a dict id
// that has no row, and every later intern of that exact lexical value
// resolves to the dangling id until `pgrdf.shmem_reset()`. The
// thread-local makes the lifetime trivially per-backend; pgrx's
// register_xact_callback handles the per-txn part.
thread_local! {
    static PENDING: RefCell<Vec<(u64, u64, i64, pg_sys::SubTransactionId)>> =
        const { RefCell::new(Vec::new()) };
    static REGISTERED: RefCell<bool> = const { RefCell::new(false) };
}

/// Stage a freshly-INSERTed dict row to be published on commit.
pub fn stage_for_commit(
    term_type: i16,
    value: &str,
    datatype_id: Option<i64>,
    language: Option<&str>,
    dict_id: i64,
) {
    if !is_ready() {
        return;
    }
    let (h1, h2) = fingerprint(current_db_oid(), term_type, value, datatype_id, language);
    let subxid = unsafe { pg_sys::GetCurrentSubTransactionId() };
    PENDING.with(|p| p.borrow_mut().push((h1, h2, dict_id, subxid)));
    register_xact_callbacks_once();
}

/// Insert a known-committed (SELECT-found) row directly into shmem.
pub fn insert_committed(
    term_type: i16,
    value: &str,
    datatype_id: Option<i64>,
    language: Option<&str>,
    dict_id: i64,
) {
    if !is_ready() {
        return;
    }
    let (h1, h2) = fingerprint(current_db_oid(), term_type, value, datatype_id, language);
    insert_slot(h1, h2, dict_id);
}

fn register_xact_callbacks_once() {
    let needs_register = REGISTERED.with(|r| {
        if *r.borrow() {
            false
        } else {
            *r.borrow_mut() = true;
            true
        }
    });
    if !needs_register {
        return;
    }
    register_xact_callback(PgXactCallbackEvent::Commit, || {
        flush_pending();
        REGISTERED.with(|r| *r.borrow_mut() = false);
    });
    register_xact_callback(PgXactCallbackEvent::Abort, || {
        PENDING.with(|p| p.borrow_mut().clear());
        REGISTERED.with(|r| *r.borrow_mut() = false);
    });
    // #127: a subtransaction abort must take its staged entries with it.
    // Subtransaction ids are assigned monotonically within a backend's
    // transaction, and while a subtransaction is open every newer id is
    // nested inside it — so on abort of `my_subid`, exactly the entries
    // tagged >= `my_subid` are the ones whose dictionary INSERTs rolled
    // back. Entries from subtransactions that already committed carry
    // ids below any later-aborting sibling and survive; if an ANCESTOR
    // aborts, their ids are above its and they are correctly dropped
    // with it. pgrx clears subxact hooks at top-level txn end, so this
    // registration rides the same once-per-transaction flag as the
    // callbacks above.
    register_subxact_callback(
        PgSubXactCallbackEvent::AbortSub,
        |my_subid, _parent_subid| {
            PENDING.with(|p| {
                p.borrow_mut()
                    .retain(|&(_, _, _, subxid)| subxid < my_subid)
            });
        },
    );
}

fn flush_pending() {
    let drained: Vec<(u64, u64, i64, pg_sys::SubTransactionId)> =
        PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (h1, h2, dict_id, _subxid) in drained {
        insert_slot(h1, h2, dict_id);
    }
}

fn insert_slot(h1: u64, h2: u64, dict_id: i64) {
    let r#gen = current_generation();
    let mut table = DICT_CACHE.exclusive();
    let start = (h1 as usize) % SLOTS;
    for i in 0..PROBE_DEPTH {
        let idx = (start + i) % SLOTS;
        // Treat any slot with a stale generation as if it were empty
        // — it cannot be trusted any more and is fair game to reuse.
        let slot_usable = table[idx].occupied != 0 && table[idx].generation == r#gen;
        if !slot_usable {
            table[idx] = DictCacheSlot {
                key_hash1: h1,
                key_hash2: h2,
                generation: r#gen,
                dict_id,
                occupied: 1,
                _pad: [0; 7],
            };
            INSERTS.get().fetch_add(1, Ordering::Relaxed);
            return;
        }
        if table[idx].key_hash1 == h1 && table[idx].key_hash2 == h2 {
            // Concurrent insert from another backend already landed
            // here. Refresh dict_id (idempotent — same row in fact)
            // and exit.
            table[idx].dict_id = dict_id;
            return;
        }
    }
    // Probe streak full — evict canonical slot. Cold terms get
    // displaced first which keeps the hot-set sticky.
    let idx = start;
    table[idx] = DictCacheSlot {
        key_hash1: h1,
        key_hash2: h2,
        generation: r#gen,
        dict_id,
        occupied: 1,
        _pad: [0; 7],
    };
    EVICTIONS.get().fetch_add(1, Ordering::Relaxed);
    INSERTS.get().fetch_add(1, Ordering::Relaxed);
}

/// Aggregate stats snapshot for `pgrdf.stats()`.
pub struct Snapshot {
    pub ready: bool,
    pub slots: usize,
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
    /// Phase E group E1 (LLD v0.4 §7.2): cumulative property-path
    /// depth truncations. Always 0 in E1 (no recursive CTE to
    /// truncate yet); E2 starts incrementing it. Surfaces on
    /// `pgrdf.stats()` as `path_depth_truncations`.
    pub path_depth_truncations: u64,
    /// #114: clauses a translation path could not apply — refused (or,
    /// in any future silent path, skipped). Non-zero means callers hit
    /// the group-construct-over-UNION refusal or a path is dropping
    /// clauses; either way the number is the signal.
    pub filter_clauses_dropped: u64,
}

/// Test-only view of how many staged entries are pending publish.
/// The #127 regression test asserts this returns to its prior value
/// after a subtransaction abort — the poison was precisely a pending
/// entry surviving the subxact rollback and publishing on the outer
/// commit.
#[cfg(any(test, feature = "pg_test"))]
pub fn pending_len() -> usize {
    PENDING.with(|p| p.borrow().len())
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        ready: is_ready(),
        slots: SLOTS,
        hits: if is_ready() {
            HITS.get().load(Ordering::Relaxed)
        } else {
            0
        },
        misses: if is_ready() {
            MISSES.get().load(Ordering::Relaxed)
        } else {
            0
        },
        inserts: if is_ready() {
            INSERTS.get().load(Ordering::Relaxed)
        } else {
            0
        },
        evictions: if is_ready() {
            EVICTIONS.get().load(Ordering::Relaxed)
        } else {
            0
        },
        path_depth_truncations: if is_ready() {
            PATH_DEPTH_TRUNCATIONS.get().load(Ordering::Relaxed)
        } else {
            0
        },
        filter_clauses_dropped: if is_ready() {
            FILTER_CLAUSES_DROPPED.get().load(Ordering::Relaxed)
        } else {
            0
        },
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use super::*;
    use crate::storage::dict::term_type;

    /// shared_preload_libraries='pgrdf' is set in pg_test config;
    /// _PG_init runs in the postmaster path; SHMEM_READY is true.
    #[pg_test]
    fn shmem_ready_in_test() {
        assert!(is_ready(), "shmem cache must be initialised in pg_test");
    }

    /// Inserting via the committed path and looking back up returns
    /// the same id. Acceptance: the cache primitive — not the dict
    /// integration — is correct on the hit path.
    #[pg_test]
    fn shmem_roundtrip_via_committed() {
        let key_value = "http://example.com/shmem-test-1";
        insert_committed(term_type::URI, key_value, None, None, 4242);
        let got = lookup(term_type::URI, key_value, None, None);
        assert_eq!(got, Some(4242));
    }

    /// Two different keys live in different slots.
    #[pg_test]
    fn shmem_disambiguates_keys() {
        insert_committed(
            term_type::URI,
            "http://example.com/shmem-test-2a",
            None,
            None,
            100,
        );
        insert_committed(
            term_type::URI,
            "http://example.com/shmem-test-2b",
            None,
            None,
            200,
        );
        assert_eq!(
            lookup(
                term_type::URI,
                "http://example.com/shmem-test-2a",
                None,
                None
            ),
            Some(100)
        );
        assert_eq!(
            lookup(
                term_type::URI,
                "http://example.com/shmem-test-2b",
                None,
                None
            ),
            Some(200)
        );
    }

    /// Datatype-id and language-tag are part of the key — terms with
    /// the same lexical value but different datatypes don't collide.
    #[pg_test]
    fn shmem_datatype_in_key() {
        insert_committed(term_type::LITERAL, "42", None, None, 1);
        insert_committed(term_type::LITERAL, "42", Some(7), None, 2);
        assert_eq!(lookup(term_type::LITERAL, "42", None, None), Some(1));
        assert_eq!(lookup(term_type::LITERAL, "42", Some(7), None), Some(2));
    }

    /// The database OID is part of the key — the same term in two
    /// databases must not share a slot.
    ///
    /// This is a plain unit test, not a `#[pg_test]`, deliberately:
    /// the harness runs ONE database, so the cross-database case is
    /// unreachable through `lookup`/`insert_committed`. Passing the
    /// oid to `fingerprint` explicitly is what makes the separation
    /// provable here rather than asserted in a comment. It is also
    /// the defect's shape — the previous key had no database input at
    /// all, so a fingerprint warmed by database A hit in database B.
    #[test]
    fn fingerprint_separates_databases() {
        let a = fingerprint(16384, term_type::URI, "http://example.com/x", None, None);
        let b = fingerprint(16385, term_type::URI, "http://example.com/x", None, None);
        assert_ne!(
            a, b,
            "identical terms in different databases must not share a cache slot"
        );

        // Both halves must move. They carry independent seeds, and a
        // key that separated on only one would halve the u128 the
        // false-hit budget is stated against.
        assert_ne!(a.0, b.0, "hash half 1 must include the database oid");
        assert_ne!(a.1, b.1, "hash half 2 must include the database oid");

        // Same database, same term still agrees — scoping the key
        // must not make it unstable within a database.
        let c = fingerprint(16384, term_type::URI, "http://example.com/x", None, None);
        assert_eq!(a, c, "the key must stay stable within one database");
    }

    /// #127: a term first interned inside a subtransaction that aborts
    /// must NOT stay staged for publish — the dictionary INSERT rolled
    /// back with the subxact, so publishing the pair on the outer
    /// commit poisons the cache with a dict id that has no row. The
    /// PL/pgSQL EXCEPTION block is the real-world trigger: the caught
    /// error aborts only the subtransaction and the outer transaction
    /// goes on to commit.
    #[pg_test]
    fn shmem_subxact_abort_discards_staged_entries() {
        let before = pending_len();
        Spi::run(
            "DO $$ BEGIN \
               BEGIN \
                 PERFORM pgrdf.put_term('urn:pgrdf-test:subxact-poison-1', 1::smallint); \
                 RAISE EXCEPTION 'simulated refusal'; \
               EXCEPTION WHEN OTHERS THEN NULL; \
               END; \
             END $$;",
        )
        .expect("DO block must succeed (exception is swallowed)");
        assert_eq!(
            pending_len(),
            before,
            "entries staged inside an aborted subtransaction must be discarded, \
             not published on the outer commit"
        );
    }

    /// #127 companion: staging that happens in a subtransaction that
    /// COMMITS must survive to the outer publish list — the abort
    /// cleanup must not over-discard.
    #[pg_test]
    fn shmem_subxact_commit_keeps_staged_entries() {
        let before = pending_len();
        Spi::run(
            "DO $$ BEGIN \
               BEGIN \
                 PERFORM pgrdf.put_term('urn:pgrdf-test:subxact-commit-1', 1::smallint); \
               EXCEPTION WHEN OTHERS THEN NULL; \
               END; \
             END $$;",
        )
        .expect("DO block must succeed");
        assert!(
            pending_len() > before,
            "a term interned in a committed subtransaction must stay staged for publish"
        );
    }

    /// #127 boundary: entries staged BEFORE a subtransaction started
    /// belong to the outer transaction and must survive that
    /// subtransaction's abort — the cleanup drops only ids at-or-above
    /// the aborting subxact's.
    #[pg_test]
    fn shmem_subxact_abort_keeps_earlier_entries() {
        Spi::run("SELECT pgrdf.put_term('urn:pgrdf-test:pre-subxact-1', 1::smallint);")
            .expect("top-level intern must succeed");
        let staged = pending_len();
        assert!(staged > 0, "top-level intern must stage an entry");
        Spi::run(
            "DO $$ BEGIN \
               BEGIN \
                 PERFORM pgrdf.put_term('urn:pgrdf-test:subxact-poison-2', 1::smallint); \
                 RAISE EXCEPTION 'simulated refusal'; \
               EXCEPTION WHEN OTHERS THEN NULL; \
               END; \
             END $$;",
        )
        .expect("DO block must succeed");
        assert_eq!(
            pending_len(),
            staged,
            "a subtransaction abort must drop only its own staged entries, \
             not the outer transaction's"
        );
    }

    /// Counters increment on hit / miss.
    #[pg_test]
    fn shmem_counters_advance() {
        let before = snapshot();
        // Miss
        assert!(lookup(term_type::URI, "http://example.com/cold-miss", None, None).is_none());
        let after_miss = snapshot();
        assert!(after_miss.misses > before.misses);

        // Insert + hit
        insert_committed(
            term_type::URI,
            "http://example.com/warm-hit",
            None,
            None,
            9999,
        );
        let _ = lookup(term_type::URI, "http://example.com/warm-hit", None, None);
        let after_hit = snapshot();
        assert!(after_hit.hits > after_miss.hits);
        assert!(after_hit.inserts > before.inserts);
    }
}
