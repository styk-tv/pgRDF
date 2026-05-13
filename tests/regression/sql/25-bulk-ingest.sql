-- 25-bulk-ingest — verify the dict cache + batched quad INSERT path
-- on the deterministic 100-triple synthetic fixture
-- (fixtures/regression/synth-100.ttl).
--
-- Fixture properties (see synth-100.sh):
--   100 triples, 10 subjects × 5 predicates × 100 objects.
--   115 distinct terms total. 300 term references → 185 cache hits.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Run the ingest in verbose mode and read structured stats out of
-- the JSONB return value. Each predicate is a boolean assertion so
-- the test is robust to minor count drift if the fixture changes.
WITH r AS (
  SELECT pgrdf.load_turtle_verbose('/fixtures/regression/synth-100.ttl', 250) AS j
)
SELECT
  (j->>'triples')::int         = 100   AS triples_correct,
  (j->>'dict_db_calls')::int   = 115   AS db_calls_exact,
  (j->>'dict_cache_hits')::int = 185   AS cache_hits_exact,
  (j->>'quad_batches')::int    = 1     AS single_batch
  FROM r;

-- The quads landed on the named partition (since add_graph wasn't
-- called for 250, they go to _pgrdf_quads_default).
SELECT pgrdf.count_quads(250) = 100 AS quads_in_graph_correct;

-- Second load into a different graph still benefits from the cache;
-- since the cache is per-call we get the same hit numbers.
WITH r2 AS (
  SELECT pgrdf.load_turtle_verbose('/fixtures/regression/synth-100.ttl', 251) AS j
)
SELECT (j->>'dict_cache_hits')::int = 185 AS cache_per_call_independent
  FROM r2;

-- Across the two graphs, the dictionary deduped the 115 distinct
-- terms. Both graphs' quads point at the SAME term ids.
SELECT count(DISTINCT lit.id)::int = 115 AS dict_deduped_across_graphs
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary lit
    ON lit.id IN (q.subject_id, q.predicate_id, q.object_id)
 WHERE q.graph_id IN (250, 251);

ROLLBACK;
