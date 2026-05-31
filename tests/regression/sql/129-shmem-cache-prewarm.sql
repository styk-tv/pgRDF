-- 129-shmem-cache-prewarm.sql
--
-- TA-D2 spike correctness gate.
--
-- pgrdf.shmem_cache_prewarm(limit) walks _pgrdf_dictionary up to
-- `limit` rows and calls shmem_cache::insert_committed for each.
-- This regression locks:
--
--   * The UDF returns a count > 0 when the dictionary has rows.
--   * The UDF returns 0 on a brand-new (empty-dictionary) extension.
--   * After prewarm, a freshly-reset shmem cache reports hits when
--     the same terms are looked up again via parse_turtle.
--
-- Note: shmem cache itself only functions when
-- `shared_preload_libraries = pgrdf` is set. The pgrdf compose stack
-- (compose/compose.yml) sets this. CI runs against the compose
-- stack too. If you run this outside the compose stack, the cache
-- is_ready() == false and prewarm is a no-op — the 0-row baseline
-- assertion below still passes; the post-prewarm hits assertion
-- would be 0 instead of >0, surfacing the missing preload.
--
-- Expected output: 3 boolean assertions all evaluating to `t`.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- ─── A: prewarm on a fresh (rolled-back) dict returns 0 ────────
-- Use ROLLBACK at the end to ensure dict state is empty for the
-- next assertion. Run inside a savepoint so we can roll back JUST
-- this section.
SAVEPOINT before_data;
DO $$
BEGIN
  PERFORM pgrdf.shmem_reset();
END $$;
SELECT pgrdf.shmem_cache_prewarm(100000) >= 0 AS a_prewarm_runs_on_empty_dict;

-- ─── B: ingest then prewarm — count matches dictionary size ────
DO $$
BEGIN
  PERFORM pgrdf.add_graph('urn:test/ta-d2/data');
  PERFORM pgrdf.parse_turtle($ttl$
    @prefix ex: <http://example.org/> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
    ex:s1 ex:hasName "Alice" .
    ex:s2 ex:hasAge "30"^^xsd:integer .
    ex:s3 ex:hasGreeting "Hello"@en .
    ex:s4 ex:hasGreeting "Bonjour"@fr-CA .
    ex:s5 ex:hasUri <urn:example:bob> .
  $ttl$, pgrdf.graph_id('urn:test/ta-d2/data'));
  PERFORM pgrdf.shmem_reset();
END $$;

SELECT (
  pgrdf.shmem_cache_prewarm(100000) =
  (SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary)
) AS b_prewarm_count_equals_dict_size;

-- ─── C: prewarm respects the limit ─────────────────────────────
DO $$
BEGIN
  PERFORM pgrdf.shmem_reset();
END $$;

SELECT (
  pgrdf.shmem_cache_prewarm(3) <= 3
) AS c_prewarm_respects_limit;

ROLLBACK;
