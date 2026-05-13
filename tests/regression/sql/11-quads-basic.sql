-- 11-quads-basic — put_quad + count_quads on the default partition.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- put_quad returns void, so we just call it; success is observable
-- via count_quads below.
WITH ids AS (
  SELECT
    pgrdf.put_term('http://example.com/s', 1::smallint) AS s,
    pgrdf.put_term('http://example.com/p', 1::smallint) AS p,
    pgrdf.put_term('http://example.com/o1', 1::smallint) AS o1,
    pgrdf.put_term('http://example.com/o2', 1::smallint) AS o2,
    pgrdf.put_term('http://example.com/o3', 1::smallint) AS o3
)
SELECT pgrdf.put_quad(s, p, o1), pgrdf.put_quad(s, p, o2), pgrdf.put_quad(s, p, o3)
FROM ids;

-- count_quads on default graph (g=0) reflects what we just inserted.
SELECT pgrdf.count_quads() AS n_default;

-- Quads land in the default partition because we haven't added g=0
-- explicitly (0 is the LIST partition catch-all per the schema).
SELECT count(*)::int AS n_default_partition
  FROM pgrdf._pgrdf_quads_default;

ROLLBACK;
