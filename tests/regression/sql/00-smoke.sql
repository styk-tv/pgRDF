-- 00-smoke — install state after `CREATE EXTENSION pgrdf`.
--
-- Idempotent across fresh-PGDATA and re-runs:
--  * IF NOT EXISTS handles either path,
--  * SET client_min_messages=WARNING suppresses the "extension … already
--    exists" NOTICE that fires on the second-and-later runs (otherwise
--    the output diff would depend on whether pg-data was wiped).

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

SELECT pgrdf.version();

-- Check the canonical schema layout exists. Use explicit names so the
-- count stays stable even if `pgrdf.add_graph(...)` has created extra
-- per-graph partitions during earlier work in this session.
SELECT count(*)::int FROM pg_class
 WHERE relnamespace = 'pgrdf'::regnamespace
   AND relname IN ('_pgrdf_dictionary','_pgrdf_quads','_pgrdf_quads_default');

-- The hexastore covering indexes. POS and OSP live on the partitioned
-- table and propagate to its partitions. Since 0.6.41 the
-- subject-predicate-object index is per partition and UNIQUE (it is what
-- makes a graph a set of triples); the default partition's is checked
-- here, so per-graph partitions don't shift the count.
SELECT count(*)::int FROM pg_indexes
 WHERE schemaname = 'pgrdf'
   AND indexname IN ('_pgrdf_idx_pos','_pgrdf_idx_osp','_pgrdf_quads_default_spo_uniq');

SELECT extversion FROM pg_extension WHERE extname='pgrdf';
