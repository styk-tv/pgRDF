-- 10-dict-roundtrip — put_term + get_term + dedup semantics.
-- Wrapped in BEGIN/ROLLBACK so the dict stays clean between runs.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- put_term returns the same id for the same (term_type, value) pair.
SELECT pgrdf.put_term('http://example.com/alice', 1::smallint)
     = pgrdf.put_term('http://example.com/alice', 1::smallint)
  AS dedups;

-- Distinct values give distinct ids.
SELECT pgrdf.put_term('http://example.com/x', 1::smallint)
    <> pgrdf.put_term('http://example.com/y', 1::smallint)
  AS distinct_ids;

-- Same lexical value, different term_type, gives distinct ids
-- (URI vs blank-node vs literal).
SELECT pgrdf.put_term('shared', 1::smallint)
    <> pgrdf.put_term('shared', 3::smallint)
  AS type_separates;

-- get_term is the inverse of put_term for the simple-term case.
SELECT pgrdf.get_term(pgrdf.put_term('roundtrip', 3::smallint)) = 'roundtrip'
  AS roundtrip;

-- get_term on a missing id returns NULL.
SELECT pgrdf.get_term(9223372036854775807) IS NULL AS missing_is_null;

ROLLBACK;
