-- 93-update-insert-data.sql
--
-- Phase C slice 84 — SPARQL UPDATE foundation + INSERT DATA. Opens
-- LLD v0.4 §4 (the UPDATE surface) with the simplest form: a static
-- triple block with no WHERE clause. The executor's entry point
-- `pgrdf.sparql(q)` now detects UPDATE queries via a try-parse-then-
-- fallback strategy — `parse_query` first (the v0.3 SELECT/ASK path,
-- unchanged), then `parse_update` if that fails. UPDATE forms return
-- a single summary row of shape `{"_update": {…}}` per LLD v0.4 §4.2,
-- paralleling the v0.3 `_ask` sentinel for ASK queries.
--
-- Invariants locked by this file:
--
--   1. Default-graph INSERT DATA — one triple lands in `_pgrdf_quads`
--      with `graph_id = 0`, the summary row reports
--      `triples_inserted = 1`, `triples_deleted = 0`,
--      `graphs_touched = ["DEFAULT"]`, `form = "INSERT_DATA"`.
--   2. Named-graph INSERT DATA — `GRAPH <iri> { … }` auto-allocates
--      a fresh `graph_id` via `pgrdf.add_graph(iri)` (slice 118),
--      creates the partition, and lands the triple there. The
--      `graphs_touched` array carries the IRI, not the synthetic
--      seed `urn:pgrdf:graph:<N>`.
--   3. Multi-triple INSERT DATA — a single statement carrying N
--      triples reports `triples_inserted = N` and all N rows are
--      present.
--   4. Idempotency — issuing the same INSERT DATA twice does NOT
--      duplicate rows (set-semantics per LLD v0.4 §4); the second
--      call still reports `triples_inserted = 1` (we count attempted
--      inserts, not net row delta), but the underlying table stays
--      at one row via `ON CONFLICT DO NOTHING`.
--   5. Round-trip — a triple inserted via INSERT DATA is queryable
--      by a subsequent `SELECT ?s ?p ?o WHERE { ?s ?p ?o }` — the
--      dictionary internment + partition router stay consistent
--      between the UPDATE and the SELECT side.
--   6. `sparql_parse` integration — INSERT DATA reports
--      `form: "UPDATE"`, per-op `op: "InsertData"`, and an empty
--      `unsupported_algebra`.
--
-- Negative paths — the per-form follow-up slices (83 / 82-77 / 71 /
-- 70 / 69) panic with a stable "lands in slice NN" prefix until they
-- ship. We lock the prefix so downstream tooling can route on
-- partial-translatability without depending on the volatile tail.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — captures SQLERRM from a wrapped EXECUTE and asserts the
-- expected substring is present. Same shape as `81-error-paths.sql`
-- and `88-drop-graph.sql`.
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

-- ─── Invariant 1: default-graph INSERT DATA ──────────────────────
-- The triple lands in `_pgrdf_quads_g0` and the `_update` summary
-- reports the correct counts. We dig into the JSONB rather than
-- printing it whole because `elapsed_ms` is non-deterministic.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted,
  (j->'_update'->'graphs_touched')                   AS graphs,
  ((j->'_update'->>'elapsed_ms')::numeric >= 0)      AS elapsed_nonneg
FROM pgrdf.sparql(
  'INSERT DATA { <http://example.org/s1> <http://example.org/p1> <http://example.org/o1> }'
) AS s(j);

SELECT count(*)::bigint AS rows_in_default
  FROM pgrdf._pgrdf_quads WHERE graph_id = 0;

-- ─── Invariant 2: named-graph INSERT DATA ────────────────────────
-- The IRI is auto-allocated, partition created, triple lands there.
-- We pull the graph_id back via `pgrdf.graph_id(iri)` (slice 116)
-- to verify the binding survived.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->'graphs_touched')                   AS graphs
FROM pgrdf.sparql(
  'INSERT DATA { GRAPH <http://example.org/g1> { '
  '  <http://example.org/s2> <http://example.org/p2> <http://example.org/o2> '
  '} }'
) AS s(j);

SELECT pgrdf.graph_id('http://example.org/g1') IS NOT NULL AS g1_bound;
SELECT count(*)::bigint AS rows_in_g1
  FROM pgrdf._pgrdf_quads
 WHERE graph_id = pgrdf.graph_id('http://example.org/g1');

-- ─── Invariant 3: multi-triple INSERT DATA ───────────────────────
-- Three triples in one block. All three are reported and all three
-- are observable in `_pgrdf_quads`.
SELECT
  (j->'_update'->>'triples_inserted')::bigint        AS inserted
FROM pgrdf.sparql(
  'INSERT DATA { '
  '  <http://example.org/m1> <http://example.org/p> <http://example.org/v1> . '
  '  <http://example.org/m1> <http://example.org/p> <http://example.org/v2> . '
  '  <http://example.org/m1> <http://example.org/p> <http://example.org/v3> . '
  '}'
) AS s(j);

SELECT count(*)::bigint AS m1_count
  FROM pgrdf.sparql(
    'SELECT ?o WHERE { <http://example.org/m1> <http://example.org/p> ?o }'
  ) AS s(j);

-- ─── Invariant 4: idempotency on repeat ──────────────────────────
-- The same triple inserted twice does not double-count in the table
-- (ON CONFLICT DO NOTHING). The summary row still claims
-- `triples_inserted = 1` (attempted, not net delta) — we lock that
-- explicit semantic so callers know what the counter means.
SELECT
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_first
FROM pgrdf.sparql(
  'INSERT DATA { <http://example.org/dup> <http://example.org/p> <http://example.org/o> }'
) AS s(j);

SELECT
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_second
FROM pgrdf.sparql(
  'INSERT DATA { <http://example.org/dup> <http://example.org/p> <http://example.org/o> }'
) AS s(j);

SELECT count(*)::bigint AS dup_rows
  FROM pgrdf.sparql(
    'SELECT ?o WHERE { <http://example.org/dup> <http://example.org/p> ?o }'
  ) AS s(j);

-- ─── Invariant 5: round-trip — typed literal payload ─────────────
-- Mix in a typed literal so the dictionary internment path (datatype
-- IRI in `_pgrdf_dictionary.datatype_iri_id`) gets exercised. Pull
-- the value back via a SELECT to verify both sides see the same row.
SELECT
  (j->'_update'->>'triples_inserted')::bigint        AS inserted
FROM pgrdf.sparql(
  'INSERT DATA { '
  '  <http://example.org/lit> <http://example.org/n> '
  '    "42"^^<http://www.w3.org/2001/XMLSchema#integer> '
  '}'
) AS s(j);

SELECT (j->>'v') AS round_tripped_value
  FROM pgrdf.sparql(
    'SELECT ?v WHERE { <http://example.org/lit> <http://example.org/n> ?v }'
  ) AS s(j);

-- ─── Invariant 6: sparql_parse integration ───────────────────────
-- `pgrdf.sparql_parse(q)` reports `form: "UPDATE"` for any UPDATE
-- query and does NOT flag it as `unsupported_algebra` — that array
-- is reserved for genuinely-out-of-scope shapes (e.g. LOAD).
SELECT pgrdf.sparql_parse(
  'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }'
)->>'form' AS parse_form;

SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }'
  )->'operations'
) AS parse_op_count;

SELECT
  pgrdf.sparql_parse(
    'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }'
  )->'operations'->0->>'op' AS parse_op_kind;

SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }'
  )->'unsupported_algebra'
) AS parse_unsupported_count;

-- ─── Negative paths: per-form panics ─────────────────────────────
-- Each unimplemented variant panics with its documented "lands in
-- slice NN" prefix so callers can preview the rollout schedule.

SELECT _check_error(
  'update-delete-data-lands-83',
  $$SELECT * FROM pgrdf.sparql('DELETE DATA { <http://x/a> <http://x/b> <http://x/c> }')$$,
  $$UPDATE form 'DELETE DATA' lands in slice 83$$
);

SELECT _check_error(
  'update-delete-insert-where-lands-82-77',
  $$SELECT * FROM pgrdf.sparql(
      'DELETE { ?s ?p ?o } INSERT { ?s ?p "new" } WHERE { ?s ?p ?o }'
    )$$,
  $$UPDATE form 'DELETE/INSERT WHERE' lands$$
);

SELECT _check_error(
  'update-clear-graph-lands-71',
  $$SELECT * FROM pgrdf.sparql('CLEAR GRAPH <http://example.org/g1>')$$,
  $$UPDATE form 'CLEAR GRAPH' lands in slice 71$$
);

SELECT _check_error(
  'update-create-graph-lands-70',
  $$SELECT * FROM pgrdf.sparql('CREATE GRAPH <http://example.org/gnew>')$$,
  $$UPDATE form 'CREATE GRAPH' lands in slice 70$$
);

SELECT _check_error(
  'update-drop-graph-lands-69',
  $$SELECT * FROM pgrdf.sparql('DROP GRAPH <http://example.org/g1>')$$,
  $$UPDATE form 'DROP GRAPH' lands in slice 69$$
);

-- Malformed UPDATE — neither parse_query nor parse_update accepts
-- it. The error message must carry the stable `sparql: parse error:`
-- prefix (slice #63 contract); we route the query-side error
-- through, not the update-side, because the prefix is the user-
-- facing surface.
SELECT _check_error(
  'update-malformed-still-uses-query-parse-error',
  $$SELECT * FROM pgrdf.sparql('INSERT BUT NOT VALID')$$,
  $$sparql: parse error:$$
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
