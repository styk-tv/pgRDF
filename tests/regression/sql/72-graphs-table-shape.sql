-- 72-graphs-table-shape.sql
--
-- Locks the shape contract for the v0.4 `_pgrdf_graphs` IRI ↔
-- graph_id mapping table (LLD v0.4 §3.1). Schema-only this slice
-- (countdown 120): no UDF surface yet, so this file pins the
-- on-disk shape — the table exists in the `pgrdf` schema, the
-- columns carry the expected types + NOT NULL, the PRIMARY KEY +
-- UNIQUE constraints are in place, the default-partition seed row
-- `(0, 'urn:pgrdf:graph:0')` is present, and no other rows leak
-- in from auto-seeding. Subsequent slices (118-115) add UDFs on
-- top of this contract; if they slip a column rename or drop the
-- seed, this baseline catches it.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Table exists in the pgrdf schema ─────────────────────────────
SELECT EXISTS(
  SELECT 1
    FROM pg_catalog.pg_class      c
    JOIN pg_catalog.pg_namespace  n ON n.oid = c.relnamespace
   WHERE c.relname  = '_pgrdf_graphs'
     AND n.nspname  = 'pgrdf'
) AS table_exists;

-- ─── Column types + NOT NULL contract ─────────────────────────────
-- Every user column must satisfy its expected (type, NOT NULL)
-- shape. `attnum > 0` skips system columns. `bool_and` collapses
-- per-row predicates to a single boolean; any drift trips the
-- baseline.
SELECT bool_and(
  CASE attname
    WHEN 'graph_id' THEN format_type(atttypid, atttypmod) = 'bigint' AND attnotnull
    WHEN 'iri'      THEN format_type(atttypid, atttypmod) = 'text'   AND attnotnull
    ELSE FALSE
  END
) AS columns_correct
  FROM pg_catalog.pg_attribute
 WHERE attrelid = 'pgrdf._pgrdf_graphs'::regclass
   AND attnum  > 0
   AND NOT attisdropped;

-- ─── PRIMARY KEY on graph_id ──────────────────────────────────────
SELECT EXISTS(
  SELECT 1
    FROM pg_catalog.pg_constraint
   WHERE conrelid = 'pgrdf._pgrdf_graphs'::regclass
     AND contype  = 'p'
) AS has_pk;

-- ─── UNIQUE on iri ────────────────────────────────────────────────
SELECT EXISTS(
  SELECT 1
    FROM pg_catalog.pg_constraint
   WHERE conrelid = 'pgrdf._pgrdf_graphs'::regclass
     AND contype  = 'u'
) AS has_unique_iri;

-- ─── Seed row for default partition ───────────────────────────────
SELECT graph_id, iri
  FROM pgrdf._pgrdf_graphs
 WHERE graph_id = 0;

-- ─── No spurious extra rows from initial seed ─────────────────────
SELECT count(*)::bigint AS row_count
  FROM pgrdf._pgrdf_graphs;

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
