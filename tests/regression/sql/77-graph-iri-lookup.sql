-- 77-graph-iri-lookup.sql
--
-- Phase A slice 115 — `pgrdf.graph_iri(id BIGINT) → TEXT` lookup
-- (LLD v0.4 §3.2). Read-only resolution of an integer `graph_id`
-- back to its bound IRI in `_pgrdf_graphs`, or `NULL` when the id
-- is not bound. No side effects, no panic on miss. Marked STRICT
-- so a NULL argument short-circuits to NULL output without invoking
-- the function body.
--
-- Symmetric inverse of slice 116's `pgrdf.graph_id(iri)`. Together
-- the two UDFs close the §3.2 IRI ↔ graph_id lookup surface — the
-- last §3.2 UDF row landing in this slice.
--
-- Invariants locked by this file:
--
--   1. The seed binding (0, 'urn:pgrdf:graph:0') is reachable via
--      `graph_iri(0)` immediately after `CREATE EXTENSION` —
--      returns 'urn:pgrdf:graph:0'.
--   2. After `add_graph('http://example.org/g1')` (slice 118),
--      `graph_iri(1)` returns the IRI the overload bound.
--   3. After `add_graph(42, 'http://example.org/g42')` (slice 117),
--      `graph_iri(42)` returns 'http://example.org/g42'.
--   4. After `add_graph(99)` (slice 119 synthetic binding),
--      `graph_iri(99)` returns 'urn:pgrdf:graph:99'.
--   5. Lookup miss returns NULL (no error raised).
--   6. Round-trip: `graph_id(graph_iri(id)) = id` for any bound id
--      — exercises both slice-115 and slice-116 lookups against the
--      same binding.
--   7. NULL input returns NULL — `#[pg_extern(strict)]` contract.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- ─── Baseline: seed graph_id 0 binds urn:pgrdf:graph:0 ────────────
SELECT pgrdf.graph_iri(0::bigint) AS seed_lookup;

-- ─── Bind via IRI and look up by allocated id (slice 118) ────────
SELECT pgrdf.add_graph('http://example.org/g1') AS allocated;
SELECT pgrdf.graph_iri(1::bigint) AS g1_lookup;

-- ─── Explicit (id, iri) binding (slice 117) ──────────────────────
SELECT pgrdf.add_graph(42::bigint, 'http://example.org/g42');
SELECT pgrdf.graph_iri(42::bigint) AS g42_lookup;

-- ─── Integer add — synthetic IRI is queryable (slice 119) ────────
SELECT pgrdf.add_graph(99::bigint);
SELECT pgrdf.graph_iri(99::bigint) AS synthetic_lookup;

-- ─── Lookup miss returns NULL ────────────────────────────────────
SELECT pgrdf.graph_iri(99999::bigint) IS NULL AS miss_returns_null;

-- ─── Round-trip: id → iri → id should match (slice 116 inverse) ──
WITH r AS (SELECT pgrdf.graph_iri(42::bigint) AS iri)
SELECT pgrdf.graph_id(iri) = 42 AS roundtrip FROM r;

-- ─── NULL input is strictly NULL output (#[pg_extern(strict)]) ───
SELECT pgrdf.graph_iri(NULL::bigint) IS NULL AS null_in_null_out;

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
