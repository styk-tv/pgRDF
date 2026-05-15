-- 76-graph-id-lookup.sql
--
-- Phase A slice 116 — `pgrdf.graph_id(iri TEXT) → BIGINT` lookup
-- (LLD v0.4 §3.2). Read-only resolution of an IRI back to its
-- integer `graph_id` in `_pgrdf_graphs`, or `NULL` when the IRI is
-- not bound. No side effects, no panic on miss. Marked STRICT so
-- a NULL argument short-circuits to NULL output without invoking
-- the function body.
--
-- Invariants locked by this file:
--
--   1. The seed binding (0, 'urn:pgrdf:graph:0') is reachable via
--      `graph_id('urn:pgrdf:graph:0')` immediately after
--      `CREATE EXTENSION` — returns 0.
--   2. After `add_graph('http://example.org/g1')` (slice 118),
--      `graph_id('http://example.org/g1')` returns the same id
--      the overload allocated (1).
--   3. After `add_graph(42, 'http://example.org/g42')` (slice 117),
--      `graph_id('http://example.org/g42')` returns 42.
--   4. After `add_graph(99)` (slice 119 synthetic binding),
--      `graph_id('urn:pgrdf:graph:99')` returns 99 — the
--      synthetic IRI is queryable through the same surface.
--   5. Lookup miss returns NULL (no error raised).
--   6. Empty-string IRI returns NULL — slice 118 rejects empty on
--      add, so no row can ever exist with iri = ''.
--   7. NULL input returns NULL — `#[pg_extern(strict)]` contract.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Baseline: seed graph_id 0 is bound to urn:pgrdf:graph:0 ──────
SELECT pgrdf.graph_id('urn:pgrdf:graph:0') AS seed_lookup;

-- ─── Bind a new graph via the IRI overload (slice 118) ───────────
SELECT pgrdf.add_graph('http://example.org/g1') AS allocated;
SELECT pgrdf.graph_id('http://example.org/g1') AS g1_lookup;

-- ─── Bind a specific (id, iri) pair (slice 117) ──────────────────
SELECT pgrdf.add_graph(42::bigint, 'http://example.org/g42') AS explicit;
SELECT pgrdf.graph_id('http://example.org/g42') AS g42_lookup;

-- ─── Lookup miss returns NULL ────────────────────────────────────
SELECT pgrdf.graph_id('http://example.org/never-bound') IS NULL AS miss_returns_null;

-- ─── Empty string is also a miss (slice 118 rejects empty on add) ─
SELECT pgrdf.graph_id('') IS NULL AS empty_returns_null;

-- ─── Round-trip with integer add_graph (slice 119 synthetic binding) ─
SELECT pgrdf.add_graph(99::bigint) AS int_add;
SELECT pgrdf.graph_id('urn:pgrdf:graph:99') AS synthetic_lookup;

-- ─── NULL input is strictly NULL output (#[pg_extern(strict)]) ───
SELECT pgrdf.graph_id(NULL::text) IS NULL AS null_in_null_out;

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
