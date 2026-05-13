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

SELECT count(*)::int FROM pg_class
 WHERE relnamespace = 'pgrdf'::regnamespace AND relname = '_pgrdf_dictionary';

SELECT count(*)::int FROM pg_class
 WHERE relnamespace = 'pgrdf'::regnamespace AND relname LIKE '_pgrdf_quads%';

SELECT count(*)::int FROM pg_indexes
 WHERE schemaname = 'pgrdf' AND indexname IN ('_pgrdf_idx_spo','_pgrdf_idx_pos','_pgrdf_idx_osp');

SELECT extversion FROM pg_extension WHERE extname='pgrdf';
