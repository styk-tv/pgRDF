-- 94-update-delete-data.sql
--
-- Phase C slice 83 — SPARQL UPDATE `DELETE DATA { … }`. Companion to
-- 93's INSERT DATA: ground quads only, no WHERE clause, no variables.
-- The executor's `execute_update` dispatcher now routes
-- `GraphUpdateOperation::DeleteData` through a dictionary-lookup-only
-- path (no interning — missing terms imply the quad cannot exist, so
-- the operation is a spec-correct no-op rather than an error).
--
-- Invariants locked by this file:
--
--   1. Default-graph DELETE DATA — seed three triples via INSERT DATA,
--      delete one, the `_update` summary reports
--      `triples_deleted = 1`, `triples_inserted = 0`,
--      `graphs_touched = ["DEFAULT"]`, `form = "DELETE_DATA"`. A
--      follow-up SELECT shows the other two triples survive.
--   2. No-op on missing-term — DELETE DATA referencing IRIs not yet
--      in `_pgrdf_dictionary` returns `triples_deleted = 0` without
--      erroring. Same for a quad whose terms are individually
--      present but never appeared in that combination.
--   3. Named-graph scope — `DELETE DATA { GRAPH <iri> { … } }`
--      removes the matching quad from that partition only; a
--      same-shape quad in the default graph is NOT touched. The
--      `graphs_touched` array carries the IRI.
--   4. Round-trip — a triple deleted via DELETE DATA is no longer
--      visible to a follow-up `SELECT ?s ?p ?o WHERE { ?s ?p ?o }`.
--   5. Idempotency on repeat — deleting the same triple twice
--      reports `triples_deleted = 1` the first time and
--      `triples_deleted = 0` the second time (the second call falls
--      through the dictionary lookup but finds nothing to remove).
--   6. Typed-literal payload — DELETE DATA can target a literal-
--      bearing triple (datatype IRI lookup composes with the value
--      lookup through `lookup_ground_term_id`).
--
-- Negative paths — the remaining unimplemented UPDATE forms still
-- panic with "lands in slice NN" prefixes; we sample one
-- (DELETE/INSERT WHERE) to confirm slice 83 didn't accidentally
-- broaden the dispatch.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

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

-- ─── Invariant 1: default-graph DELETE DATA ──────────────────────
-- Seed three triples then delete the middle one. The summary row
-- and the underlying table both reflect the removal.
SELECT (j->'_update'->>'triples_inserted')::bigint AS seeded
FROM pgrdf.sparql(
  'INSERT DATA { '
  '  <http://example.org/a> <http://example.org/p> <http://example.org/v1> . '
  '  <http://example.org/a> <http://example.org/p> <http://example.org/v2> . '
  '  <http://example.org/a> <http://example.org/p> <http://example.org/v3> . '
  '}'
) AS s(j);

SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted,
  (j->'_update'->'graphs_touched')                   AS graphs,
  ((j->'_update'->>'elapsed_ms')::numeric >= 0)      AS elapsed_nonneg
FROM pgrdf.sparql(
  'DELETE DATA { <http://example.org/a> <http://example.org/p> <http://example.org/v2> }'
) AS s(j);

SELECT count(*)::bigint AS rows_remaining
  FROM pgrdf._pgrdf_quads WHERE graph_id = 0;

SELECT count(*)::bigint AS via_select
  FROM pgrdf.sparql(
    'SELECT ?o WHERE { <http://example.org/a> <http://example.org/p> ?o }'
  ) AS s(j);

-- ─── Invariant 2: no-op on missing terms ─────────────────────────
-- Subject + predicate + object all absent from the dictionary. The
-- dictionary lookups return None; the quad cannot exist; the form
-- is a no-op rather than an error.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted
FROM pgrdf.sparql(
  'DELETE DATA { <http://example.org/never> <http://example.org/seen> <http://example.org/before> }'
) AS s(j);

-- Partial-presence case: s and p exist (from above), o does not.
-- Still a no-op because the FULL quad isn't in `_pgrdf_quads`. Use
-- a fresh-IRI object so the partial-presence is preserved across
-- dictionary cache state.
SELECT
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_partial
FROM pgrdf.sparql(
  'DELETE DATA { <http://example.org/a> <http://example.org/p> <http://example.org/never-bound> }'
) AS s(j);

-- ─── Invariant 3: named-graph scope ──────────────────────────────
-- Same-shape triple in two graphs; DELETE DATA scoped to the named
-- graph removes only that copy.
SELECT (j->'_update'->>'triples_inserted')::bigint AS seeded_g
FROM pgrdf.sparql(
  'INSERT DATA { GRAPH <http://example.org/g1> { '
  '  <http://example.org/a> <http://example.org/p> <http://example.org/v1> '
  '} }'
) AS s(j);

SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted,
  (j->'_update'->'graphs_touched')                   AS graphs
FROM pgrdf.sparql(
  'DELETE DATA { GRAPH <http://example.org/g1> { '
  '  <http://example.org/a> <http://example.org/p> <http://example.org/v1> '
  '} }'
) AS s(j);

-- Default-graph copy of the v1 quad must still be present.
SELECT count(*)::bigint AS v1_in_default
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = 0
   AND object_id = (SELECT id FROM pgrdf._pgrdf_dictionary
                     WHERE lexical_value = 'http://example.org/v1' LIMIT 1);

-- Named-graph partition is empty for that subject.
SELECT count(*)::bigint AS rows_in_g1
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = pgrdf.graph_id('http://example.org/g1');

-- ─── Invariant 4: round-trip — SELECT no longer sees the triple ──
-- We already deleted <…/v2> in invariant 1; double-check that the
-- public SELECT path doesn't surface it either (no stale plan-cache,
-- no orphan partition row).
SELECT count(*)::bigint AS v2_via_select
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.org/p> <http://example.org/v2> }'
  ) AS s(j);

-- ─── Invariant 5: idempotency on repeat ──────────────────────────
-- Deleting the same triple a second time is a clean 0-no-op.
SELECT
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_again
FROM pgrdf.sparql(
  'DELETE DATA { <http://example.org/a> <http://example.org/p> <http://example.org/v2> }'
) AS s(j);

-- ─── Invariant 6: typed-literal payload ──────────────────────────
-- Insert + delete a typed literal — the dictionary path with a
-- datatype_iri_id binding round-trips through DELETE DATA.
SELECT (j->'_update'->>'triples_inserted')::bigint AS seeded_lit
FROM pgrdf.sparql(
  'INSERT DATA { '
  '  <http://example.org/lit> <http://example.org/n> '
  '    "42"^^<http://www.w3.org/2001/XMLSchema#integer> '
  '}'
) AS s(j);

SELECT
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_lit
FROM pgrdf.sparql(
  'DELETE DATA { '
  '  <http://example.org/lit> <http://example.org/n> '
  '    "42"^^<http://www.w3.org/2001/XMLSchema#integer> '
  '}'
) AS s(j);

SELECT count(*)::bigint AS lit_remaining
  FROM pgrdf.sparql(
    'SELECT ?v WHERE { <http://example.org/lit> <http://example.org/n> ?v }'
  ) AS s(j);

-- ─── Slice 80 smoke: DELETE/INSERT WHERE no longer panics ────────
-- The combined modify form shipped in slice 80; we keep a smoke
-- assertion that the dispatcher returns a well-formed _update row.
-- The dedicated regression lives in `97-update-delete-insert-where.sql`.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted
FROM pgrdf.sparql(
  'PREFIX zzz: <http://example.org/unbound/> '
  'DELETE { ?s zzz:p ?o } INSERT { ?s zzz:p "x" } WHERE { ?s zzz:p ?o }'
) AS s(j);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
