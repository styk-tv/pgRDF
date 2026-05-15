-- 73-add-graph-populates-iri.sql
--
-- Phase A slice 119 — `pgrdf.add_graph(id BIGINT)` (the existing
-- integer-keyed UDF) now populates the `_pgrdf_graphs` IRI mapping
-- table (landed in slice 120) with a synthetic IRI of the form
-- `urn:pgrdf:graph:{id}` on each successful partition creation.
--
-- Invariants locked by this file:
--
--   1. Baseline immediately after `CREATE EXTENSION` is exactly one
--      row — the seed `(0, 'urn:pgrdf:graph:0')` from the schema SQL.
--   2. `pgrdf.add_graph(42)` adds a second row whose IRI is the
--      synthetic `urn:pgrdf:graph:42` shape.
--   3. A repeat `pgrdf.add_graph(42)` call is idempotent: no extra
--      row, no error from the `ON CONFLICT (graph_id) DO NOTHING`
--      clause inside the UDF.
--   4. Multiple distinct ids each land their own synthetic-IRI row.
--   5. The materialised IRI strings match the literal
--      `urn:pgrdf:graph:<id>` shape exactly (no whitespace, no
--      trailing newline, identical to the LLD v0.4 §3.1 contract).
--
-- A refactor that drops the INSERT, changes the IRI prefix, omits
-- the ON CONFLICT clause, or fires the INSERT on the existing-partition
-- path (which would still be safe, but would silently shift the
-- behaviour contract) trips this baseline.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Baseline — only the seed row ─────────────────────────────────
SELECT count(*)::bigint AS baseline FROM pgrdf._pgrdf_graphs;

-- ─── Adding a graph populates the IRI mapping ─────────────────────
SELECT pgrdf.add_graph(42) AS created_now;
SELECT count(*)::bigint AS after_one_add FROM pgrdf._pgrdf_graphs;
SELECT iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 42;

-- ─── Idempotent re-add — no duplicate, no error ───────────────────
SELECT pgrdf.add_graph(42) AS created_again;
SELECT count(*)::bigint AS after_reuse FROM pgrdf._pgrdf_graphs WHERE graph_id = 42;

-- ─── Multiple distinct ids each get their own row ─────────────────
SELECT pgrdf.add_graph(100) AS add_100;
SELECT pgrdf.add_graph(101) AS add_101;
SELECT count(*)::bigint AS total_after_three FROM pgrdf._pgrdf_graphs;

-- ─── Final IRI shape across all bound graph_ids ───────────────────
SELECT graph_id, iri
  FROM pgrdf._pgrdf_graphs
 ORDER BY graph_id;

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
