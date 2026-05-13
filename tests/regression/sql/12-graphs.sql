-- 12-graphs — add_graph creates a named partition; put_quad routes
-- tuples into it based on the graph_id key.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Create a named partition for graph 42. add_graph returns true when
-- it created the partition, false when it was already there.
SELECT pgrdf.add_graph(42) AS created_now;

-- Partition is in the catalog as `_pgrdf_quads_g42`.
SELECT count(*)::int AS partition_exists
  FROM pg_class
 WHERE relnamespace = 'pgrdf'::regnamespace
   AND relname = '_pgrdf_quads_g42';

-- add_graph is idempotent — second call returns false.
SELECT pgrdf.add_graph(42) AS created_again;

WITH ids AS (
  SELECT
    pgrdf.put_term('http://example.com/s', 1::smallint) AS s,
    pgrdf.put_term('http://example.com/p', 1::smallint) AS p,
    pgrdf.put_term('http://example.com/in42', 1::smallint) AS o
)
SELECT pgrdf.put_quad(s, p, o, 42) FROM ids;

-- The tuple landed in _pgrdf_quads_g42, NOT the default partition.
SELECT count(*)::int AS in_g42 FROM pgrdf._pgrdf_quads_g42;
SELECT count(*)::int AS in_default FROM pgrdf._pgrdf_quads_default WHERE graph_id = 42;

-- Count via UDF agrees.
SELECT pgrdf.count_quads(42) AS n_g42;

ROLLBACK;
