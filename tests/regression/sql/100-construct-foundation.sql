-- 100-construct-foundation.sql
--
-- Phase D slice 59 — `pgrdf.construct(q TEXT) → SETOF JSONB`
-- foundation. Lands the sibling UDF documented in LLD v0.4 §6.1,
-- routing CONSTRUCT through the existing SELECT-side BGP translator
-- and emitting one structured-term row per (solution, template
-- triple) pair. Slice 59 narrows to CONSTANT-ONLY templates;
-- slice 58 (file `101-construct-variable-templates.sql`) widened to
-- variable substitution. Blank nodes in templates still panic until
-- slice 57.
--
-- Output row shape (per LLD §6.1):
--
--   {"subject":   {"type": "iri",     "value": "..."},
--    "predicate": {"type": "iri",     "value": "..."},
--    "object":    {"type": "literal", "value": "...",
--                  "datatype": "...", "language": "..."}}
--
-- Invariants locked by this file:
--
--   A. Constant-only template, single solution — one row carrying
--      the constant template, structured shape verified.
--   B. Constant-only template, N solutions — N identical rows (one
--      per solution per W3C 1.1 §16.2's "solution sequence is the
--      BGP's; multiplicity matters").
--   C. Multi-position constant types — IRI subject, IRI predicate,
--      typed-literal object (xsd:integer). Datatype IRI surfaces
--      verbatim in the term cell.
--   D. Empty solution set — zero rows. CONSTRUCT does not synthesise
--      output when the WHERE matches nothing.
--   E. Reject non-CONSTRUCT — `pgrdf.construct('SELECT …')` panics
--      with the stable `pgrdf.construct: not a CONSTRUCT query`
--      prefix.
--   F. Reject blank node in template — slice 58 narrowed-scope
--      guard with `pgrdf.construct: slice 58 supports variables
--      and constants; blank nodes land in slice 57` prefix.
--   G. Reject literal in subject — `pgrdf.construct: literal not
--      allowed in subject/predicate position` prefix (legal RDF).
--   H. Reject DISTINCT/ORDER BY/aggregate wrapping on the WHERE —
--      stable `pgrdf.construct: DISTINCT / ORDER BY / GROUP BY /
--      aggregates not supported (W3C 1.1 §16.2)` prefix family.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — captures SQLERRM from a wrapped EXECUTE and asserts the
-- expected substring is present. Same shape as `93-update-insert-data.sql`.
CREATE OR REPLACE FUNCTION _check_error(label TEXT, sql TEXT, expected_fragment TEXT)
RETURNS TEXT
LANGUAGE plpgsql AS $$
DECLARE
  msg TEXT;
BEGIN
  BEGIN
    EXECUTE sql;
    RETURN format('%s: !!! unexpected success !!!', label);
  EXCEPTION WHEN OTHERS THEN
    msg := SQLERRM;
  END;
  IF position(expected_fragment IN msg) > 0 THEN
    RETURN format('%s: t', label);
  ELSE
    RETURN format('%s: f (got: %s)', label, left(msg, 80));
  END IF;
END
$$;

-- Seed: one default-graph triple for the single-solution case +
-- three triples sharing a predicate for the multi-solution case.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:s ex:p ex:o .
   ex:s1 ex:k ex:o1 .
   ex:s2 ex:k ex:o2 .
   ex:s3 ex:k ex:o3 .',
  0);

-- ─── Invariant A: constant template, one solution, one row ───────
SELECT count(*)::bigint AS a_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/t1> <http://example.com/t2> "x" } '
    'WHERE { <http://example.com/s> <http://example.com/p> <http://example.com/o> }'
  ) AS s(j);

SELECT
  (j->'subject'->>'type')      AS s_type,
  (j->'subject'->>'value')     AS s_value,
  (j->'predicate'->>'type')    AS p_type,
  (j->'predicate'->>'value')   AS p_value,
  (j->'object'->>'type')       AS o_type,
  (j->'object'->>'value')      AS o_value,
  (j->'object'->>'datatype')   AS o_datatype
FROM pgrdf.construct(
  'CONSTRUCT { <http://example.com/t1> <http://example.com/t2> "x" } '
  'WHERE { <http://example.com/s> <http://example.com/p> <http://example.com/o> }'
) AS s(j);

-- ─── Invariant B: constant template, three solutions, three rows ─
SELECT count(*)::bigint AS b_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/tag> <http://example.com/k> "v" } '
    'WHERE { ?s <http://example.com/k> ?o }'
  ) AS s(j);

-- All three rows must carry the same constant template payload —
-- verify by counting distinct row shapes.
SELECT count(DISTINCT j)::bigint AS b_distinct_rows
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/tag> <http://example.com/k> "v" } '
    'WHERE { ?s <http://example.com/k> ?o }'
  ) AS s(j);

-- ─── Invariant C: typed-literal object ───────────────────────────
SELECT
  (j->'object'->>'type')       AS c_o_type,
  (j->'object'->>'value')      AS c_o_value,
  (j->'object'->>'datatype')   AS c_o_datatype
FROM pgrdf.construct(
  'CONSTRUCT { <http://example.com/x> <http://example.com/y> '
  '  "42"^^<http://www.w3.org/2001/XMLSchema#integer> } '
  'WHERE { <http://example.com/s> <http://example.com/p> <http://example.com/o> }'
) AS s(j);

-- ─── Invariant D: empty solution set yields zero rows ────────────
SELECT count(*)::bigint AS d_row_count
  FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/t1> <http://example.com/t2> "x" } '
    'WHERE { ?s <http://example.com/never-loaded> ?o }'
  ) AS s(j);

-- ─── Invariant E: reject non-CONSTRUCT (SELECT) ──────────────────
SELECT _check_error(
  'e-rejects-select',
  $$SELECT * FROM pgrdf.construct('SELECT ?s WHERE { ?s ?p ?o }')$$,
  $$pgrdf.construct: not a CONSTRUCT query$$
);

-- ─── Invariant F: reject blank node in template (slice 58 narrow) ─
SELECT _check_error(
  'f-rejects-bnode-in-template',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/s> <http://example.com/t> _:b } WHERE { ?s ?p ?o }')$$,
  $$pgrdf.construct: slice 58 supports variables and constants; blank nodes land in slice 57$$
);

-- ─── Invariant G: reject literal in subject (legal RDF) ─────────
SELECT _check_error(
  'g-rejects-literal-in-subject',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { "lit" <http://example.com/p> "x" } WHERE { ?s ?p ?o }')$$,
  $$pgrdf.construct: literal not allowed in subject/predicate position$$
);

-- ─── Invariant H: reject DISTINCT/ORDER BY/aggregate on WHERE ────
-- Sub-SELECT inside WHERE carries a DISTINCT wrapper that the
-- modifier-guard catches. Per W3C 1.1 §16.2 these are explicitly
-- prohibited on CONSTRUCT.
SELECT _check_error(
  'h-rejects-distinct-modifier',
  $$SELECT * FROM pgrdf.construct(
    'CONSTRUCT { <http://example.com/s> <http://example.com/p> "x" } '
    'WHERE { { SELECT DISTINCT ?s WHERE { ?s ?p ?o } } }')$$,
  $$pgrdf.construct: DISTINCT / ORDER BY / GROUP BY / aggregates not supported$$
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
