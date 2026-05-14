-- 51-plan-cache.sql
--
-- Phase 3 step 2 (LLD §4.2) acceptance test. Verifies that repeated
-- SPARQL queries with identical structural shape (and even varying
-- IRI constants) reuse a cached prepared statement.
--
-- Counters are cumulative since postmaster start, so the test
-- snapshots `plan_cache_hits / misses / inserts` before the
-- experiment and asserts deltas.
--
-- Hand-computed expectations:
--
--   Block A: same query 5 times.
--     5 calls share one shape. First prepares (1 miss + 1 insert);
--     four hit the cache (4 hits). Deltas: hits=+4, misses=+1.
--
--   Block B: parameter variation — query the same shape but with
--           a different IRI constant. Plan parameterises every dict
--           id; SQL string is byte-identical to block A so it stays
--           cached. Deltas: hits=+1, misses=0.
--
--   Block C: structurally different query (FILTER added). Distinct
--           SQL string ⇒ new cache entry. Deltas: hits=0, misses=+1.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph(8800);
SELECT pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:a ex:p1 ex:b .
     ex:c ex:p2 ex:d .
     ex:e ex:p1 ex:f .',
    8800);

-- ── snapshot 0 ─────────────────────────────────────────────────────
SELECT (j->>'plan_cache_hits')::bigint    AS h0,
       (j->>'plan_cache_misses')::bigint  AS m0,
       (j->>'plan_cache_inserts')::bigint AS i0
  FROM (SELECT pgrdf.stats() AS j) s \gset

-- ── Block A: same query 5 times ───────────────────────────────────
SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o }');
SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o }');
SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o }');
SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o }');
SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o }');

SELECT (j->>'plan_cache_hits')::bigint   - :h0 = 4 AS block_a_hits_4,
       (j->>'plan_cache_misses')::bigint - :m0 = 1 AS block_a_miss_1,
       (j->>'plan_cache_inserts')::bigint - :i0 = 1 AS block_a_insert_1
  FROM (SELECT pgrdf.stats() AS j) s;

-- ── snapshot 1 ────────────────────────────────────────────────────
SELECT (j->>'plan_cache_hits')::bigint    AS h1,
       (j->>'plan_cache_misses')::bigint  AS m1,
       (j->>'plan_cache_inserts')::bigint AS i1
  FROM (SELECT pgrdf.stats() AS j) s \gset

-- ── Block B: same SHAPE, different constants ──────────────────────
-- Both queries have a constant predicate; they translate to identical
-- parameterised SQL with the only difference being the constant dict
-- id that becomes $1. Plan cache key is the SQL string — same SQL,
-- same key — so the second call hits.
SELECT count(*) FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/> SELECT ?s WHERE { ?s ex:p1 ?o }');
SELECT count(*) FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/> SELECT ?s WHERE { ?s ex:p2 ?o }');

SELECT (j->>'plan_cache_hits')::bigint   - :h1 = 1 AS block_b_hits_1,
       (j->>'plan_cache_misses')::bigint - :m1 = 1 AS block_b_miss_1
  FROM (SELECT pgrdf.stats() AS j) s;

-- ── snapshot 2 ────────────────────────────────────────────────────
SELECT (j->>'plan_cache_hits')::bigint    AS h2,
       (j->>'plan_cache_misses')::bigint  AS m2
  FROM (SELECT pgrdf.stats() AS j) s \gset

-- ── Block C: structurally distinct query ──────────────────────────
SELECT count(*) FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/> SELECT ?s WHERE { ?s ?p ?o FILTER(?p = ex:p1) }');

SELECT (j->>'plan_cache_hits')::bigint   - :h2 = 0 AS block_c_no_hits,
       (j->>'plan_cache_misses')::bigint - :m2 = 1 AS block_c_one_miss
  FROM (SELECT pgrdf.stats() AS j) s;

-- ── plan_cache_clear empties the local cache ──────────────────────
SELECT pgrdf.plan_cache_clear() >= 2 AS cleared_at_least_two;
SELECT (j->>'plan_cache_local_size')::int = 0 AS local_size_zero_after_clear
  FROM (SELECT pgrdf.stats() AS j) s;

-- ── Cleanup ───────────────────────────────────────────────────────
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
