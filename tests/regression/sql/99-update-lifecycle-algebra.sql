-- 99-update-lifecycle-algebra.sql
--
-- Phase C slice 78 — SPARQL UPDATE lifecycle algebra:
--
--   `DROP GRAPH <iri>`           → pgrdf.drop_graph(id, true)
--   `CLEAR GRAPH <iri>`          → pgrdf.clear_graph(id)
--   `CREATE GRAPH <iri>`         → pgrdf.add_graph(iri TEXT)
--   `CLEAR/DROP DEFAULT`         → pgrdf.clear_graph(0)
--   `CLEAR/DROP ALL`             → iterate every binding (incl. id=0)
--   `CLEAR/DROP NAMED`           → iterate IRI-bound named graphs only
--
-- SPARQL surface ↔ SQL UDF lattice from LLD v0.4 §4.4. The §5 UDFs
-- (slice 96/97/98/99) carry the partition-DDL primitives; the SPARQL
-- dispatcher in `src/query/executor.rs::execute_update` routes through
-- SQL strings so the two front-ends remain consumers of the same
-- partition-level primitives.
--
-- W3C ground: SPARQL 1.1 Update §3.1.3. Key clauses locked here:
--   - DROP / CLEAR with a not-bound IRI panic by default; `SILENT`
--     swallows the error to a no-op.
--   - DROP DEFAULT empties the default graph (the partition stays);
--     pgrdf.drop_graph(0) panics by design (default partition is the
--     catch-all bucket), so the dispatcher routes to clear_graph(0).
--   - CLEAR GRAPH preserves the IRI binding; the partition is still
--     bound after the operation (vs DROP which removes both).
--   - CREATE GRAPH on an already-bound IRI errors unless SILENT.
--
-- Invariants locked by this file:
--
--   1. DROP GRAPH <g1> deletes the partition + binding; subsequent
--      lookups return NULL graph_id, and the `_pgrdf_graphs` row is
--      gone. The `_update.triples_deleted` counter matches the row
--      count that was in g1 before the drop.
--   2. CLEAR GRAPH <g2> empties the partition but PRESERVES the IRI
--      binding (graph_id still resolves; partition still attached
--      but holds zero rows).
--   3. CREATE GRAPH <g3> on an unbound IRI allocates a fresh
--      partition; `_pgrdf_graphs` gets a new row. CREATE doesn't
--      touch row counts (triples_inserted = 0).
--   4. CREATE GRAPH <g3> SILENT on an already-bound IRI is a no-op.
--   5. DROP GRAPH <unbound> (no SILENT) panics with a stable prefix.
--   6. DROP SILENT GRAPH <unbound> is a no-op (zero counters).
--   7. CLEAR DEFAULT empties graph_id=0; the binding stays put.
--   8. CLEAR ALL empties every partition INCLUDING the default; every
--      _pgrdf_graphs row stays put (CLEAR preserves bindings).
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper for the "must panic without SILENT" check — same shape as
-- 81-error-paths.sql's `_check_error` so the diff stays clean.
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

-- ─── Seed: two named graphs (g1, g2) + the default graph ───────────
-- g1 gets 3 triples, g2 gets 2 triples, default gets 1.
SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_g1
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { '
  '  GRAPH <http://example.org/g1> { '
  '    ex:a ex:p "1" . '
  '    ex:b ex:p "2" . '
  '    ex:c ex:p "3" '
  '  } '
  '}'
) AS s(j);

SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_g2
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { '
  '  GRAPH <http://example.org/g2> { '
  '    ex:d ex:p "4" . '
  '    ex:e ex:p "5" '
  '  } '
  '}'
) AS s(j);

SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_default
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { ex:f ex:p "6" }'
) AS s(j);

-- ─── Invariant 1: DROP GRAPH <g1> ─────────────────────────────────
-- Counter must be 3 (the row count before the drop). The partition's
-- _pgrdf_graphs row must be gone; pgrdf.graph_id('…/g1') returns NULL.
SELECT
  (j->'_update'->>'form')                            AS form_drop_g1,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_drop_g1,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_drop_g1,
  (j->'_update'->'graphs_touched')                   AS graphs_drop_g1
FROM pgrdf.sparql('DROP GRAPH <http://example.org/g1>') AS s(j);

SELECT pgrdf.graph_id('http://example.org/g1') AS g1_id_after_drop;
SELECT count(*)::bigint AS g1_rows_in_graphs_after_drop
  FROM pgrdf._pgrdf_graphs WHERE iri = 'http://example.org/g1';

-- ─── Invariant 2: CLEAR GRAPH <g2> ─────────────────────────────────
-- Counter is 2 (the rows truncated). The _pgrdf_graphs row stays;
-- pgrdf.graph_id('…/g2') still resolves to its bigint id.
SELECT
  (j->'_update'->>'form')                            AS form_clear_g2,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_clear_g2,
  (j->'_update'->'graphs_touched')                   AS graphs_clear_g2
FROM pgrdf.sparql('CLEAR GRAPH <http://example.org/g2>') AS s(j);

SELECT (pgrdf.graph_id('http://example.org/g2') IS NOT NULL) AS g2_binding_preserved;
SELECT count(*)::bigint AS g2_rows_after_clear
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g2> { ?s ex:p ?o } }'
  ) AS s(j);

-- ─── Invariant 3: CREATE GRAPH <g3> on an unbound IRI ──────────────
-- Allocates a fresh partition + binding. No row changes.
SELECT
  (j->'_update'->>'form')                            AS form_create_g3,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_create_g3,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_create_g3
FROM pgrdf.sparql('CREATE GRAPH <http://example.org/g3>') AS s(j);

SELECT (pgrdf.graph_id('http://example.org/g3') IS NOT NULL) AS g3_bound_after_create;

-- ─── Invariant 4: CREATE GRAPH <g3> SILENT on already-bound IRI ────
-- No-op. The pre-existing binding is preserved; no second row appears
-- in _pgrdf_graphs.
SELECT
  (j->'_update'->>'form')                            AS form_create_g3_silent,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_create_g3_silent
FROM pgrdf.sparql('CREATE SILENT GRAPH <http://example.org/g3>') AS s(j);

SELECT count(*)::bigint AS g3_binding_rows
  FROM pgrdf._pgrdf_graphs WHERE iri = 'http://example.org/g3';

-- ─── Invariant 5: DROP GRAPH <unbound> without SILENT panics ──────
SELECT _check_error(
  'drop-unbound-no-silent',
  $$SELECT * FROM pgrdf.sparql('DROP GRAPH <http://example.org/never-bound>')$$,
  'DROP GRAPH'
);

-- ─── Invariant 6: DROP SILENT GRAPH <unbound> is a no-op ──────────
SELECT
  (j->'_update'->>'form')                            AS form_drop_silent_unbound,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_drop_silent_unbound
FROM pgrdf.sparql(
  'DROP SILENT GRAPH <http://example.org/never-bound>'
) AS s(j);

-- ─── Invariant 7: CLEAR DEFAULT empties graph_id=0 ────────────────
-- Default partition had 1 triple (ex:f). Counter = 1; SELECT on the
-- default graph returns zero rows afterwards.
SELECT
  (j->'_update'->>'form')                            AS form_clear_default,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_clear_default
FROM pgrdf.sparql('CLEAR DEFAULT') AS s(j);

SELECT count(*)::bigint AS default_rows_after_clear
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { ?s ex:p ?o }'
  ) AS s(j);

-- ─── Invariant 8: CLEAR ALL empties every partition ────────────────
-- Re-seed: at this point g1 is dropped, g2 is empty-bound, g3 is
-- empty-bound, default is empty. Put 2 triples back: one in g3, one
-- in default.
SELECT (j->'_update'->>'triples_inserted')::bigint AS reseed_g3
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { GRAPH <http://example.org/g3> { ex:g ex:p "g3" } }'
) AS s(j);

SELECT (j->'_update'->>'triples_inserted')::bigint AS reseed_default_again
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { ex:h ex:p "default" }'
) AS s(j);

-- CLEAR ALL — counter is 2 (1 in g3 + 1 in default). g2 contributes 0
-- (was already empty). Every binding is preserved (CLEAR, not DROP).
SELECT
  (j->'_update'->>'form')                            AS form_clear_all,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_clear_all
FROM pgrdf.sparql('CLEAR ALL') AS s(j);

-- Post-state: every partition empty; bindings intact (g2 and g3 still
-- resolve to their bigint ids; the default partition row count = 0).
SELECT
  (pgrdf.graph_id('http://example.org/g2') IS NOT NULL) AS g2_still_bound,
  (pgrdf.graph_id('http://example.org/g3') IS NOT NULL) AS g3_still_bound;

SELECT count(*)::bigint AS total_rows_after_clear_all
  FROM pgrdf._pgrdf_quads;

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
