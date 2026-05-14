//! Process-wide dictionary cache backed by PostgreSQL shared memory.
//!
//! Implements LLD §4.1 — a fixed-capacity, open-addressed hash table in
//! Postgres shmem that caches `(term_type, lexical_value, datatype_id,
//! language) → dict_id` mappings across backends and across calls. The
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

use pgrx::callbacks::{register_xact_callback, PgXactCallbackEvent};
use pgrx::prelude::*;
use pgrx::{pg_shmem_init, PGRXSharedMemory, PgAtomic, PgLwLock};
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

fn fingerprint(
    term_type: i16,
    value: &str,
    datatype_id: Option<i64>,
    language: Option<&str>,
) -> (u64, u64) {
    let mut h1 = DefaultHasher::new();
    SEED_A.hash(&mut h1);
    term_type.hash(&mut h1);
    value.hash(&mut h1);
    datatype_id.hash(&mut h1);
    language.hash(&mut h1);
    let mut h2 = DefaultHasher::new();
    SEED_B.hash(&mut h2);
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
    let gen = current_generation();
    let (h1, h2) = fingerprint(term_type, value, datatype_id, language);
    let table = DICT_CACHE.share();
    let start = (h1 as usize) % SLOTS;
    for i in 0..PROBE_DEPTH {
        let slot = &table[(start + i) % SLOTS];
        if slot.occupied != 0
            && slot.generation == gen
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

// Per-backend list of (fingerprint, dict_id) entries staged inside the
// current transaction. Published on commit; discarded on abort. The
// thread-local makes the lifetime trivially per-backend; pgrx's
// register_xact_callback handles the per-txn part.
thread_local! {
    static PENDING: RefCell<Vec<(u64, u64, i64)>> = const { RefCell::new(Vec::new()) };
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
    let (h1, h2) = fingerprint(term_type, value, datatype_id, language);
    PENDING.with(|p| p.borrow_mut().push((h1, h2, dict_id)));
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
    let (h1, h2) = fingerprint(term_type, value, datatype_id, language);
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
}

fn flush_pending() {
    let drained: Vec<(u64, u64, i64)> = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for (h1, h2, dict_id) in drained {
        insert_slot(h1, h2, dict_id);
    }
}

fn insert_slot(h1: u64, h2: u64, dict_id: i64) {
    let gen = current_generation();
    let mut table = DICT_CACHE.exclusive();
    let start = (h1 as usize) % SLOTS;
    for i in 0..PROBE_DEPTH {
        let idx = (start + i) % SLOTS;
        // Treat any slot with a stale generation as if it were empty
        // — it cannot be trusted any more and is fair game to reuse.
        let slot_usable = table[idx].occupied != 0 && table[idx].generation == gen;
        if !slot_usable {
            table[idx] = DictCacheSlot {
                key_hash1: h1,
                key_hash2: h2,
                generation: gen,
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
        generation: gen,
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
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use super::*;
    use crate::storage::dict::term_type;
    use pgrx::prelude::*;

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
