-- 00-smoke — install state after `CREATE EXTENSION pgrdf`.
--
-- Idempotent: re-runs cleanly because of IF NOT EXISTS. Uses -A -t -q
-- so output is one value per line; expected file is matched exactly.

CREATE EXTENSION IF NOT EXISTS pgrdf;

SELECT pgrdf.version();

SELECT count(*)::int FROM pg_class
 WHERE relnamespace = 'pgrdf'::regnamespace AND relname = '_pgrdf_dictionary';

SELECT count(*)::int FROM pg_class
 WHERE relnamespace = 'pgrdf'::regnamespace AND relname LIKE '_pgrdf_quads%';

SELECT count(*)::int FROM pg_indexes
 WHERE schemaname = 'pgrdf' AND indexname IN ('_pgrdf_idx_spo','_pgrdf_idx_pos','_pgrdf_idx_osp');

SELECT extversion FROM pg_extension WHERE extname='pgrdf';
