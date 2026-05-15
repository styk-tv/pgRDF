-- 98-update-graph-scoped.sql
--
-- Phase C slice 79 — SPARQL UPDATE graph-scoped variants:
--
--   (a) `INSERT DATA { GRAPH <g> { … } }` lands quads in graph <g>
--       (already routed by slice 84's `resolve_or_allocate_graph` on
--       the per-quad `graph_name` — locked here as a regression).
--   (b) `DELETE DATA { GRAPH <g> { … } }` removes quads from graph <g>
--       only (slice 83 — same locking rationale).
--   (c) `INSERT { GRAPH <g2> { ?x ex:tag ?t } } WHERE { GRAPH <g1>
--       { ?x ex:p ?o } }` — cross-graph copy from g1 to g2.
--   (d) `DELETE { GRAPH <g> { ?s ?p ?o } } WHERE { GRAPH <g>
--       { ?s ?p ?o } }` — scoped wipe (DELETE WHERE).
--   (e) `WITH <g> INSERT { ?x ex:tag "t" } WHERE { ?x ex:p ?o }` —
--       spargebra desugars WITH into per-quad `graph_name = <g>` on
--       every default-graph template triple AND a `using:
--       Some(QueryDataset { default: [<g>], named: None })` sentinel
--       on the operation. Slice 79 lifts that IRI out and wraps the
--       WHERE pattern in `GraphPattern::Graph` so its BGP triples
--       inherit the scope.
--   (f) `WITH <g> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o }` — analogous
--       for the DELETE WHERE half (same desugar machinery).
--   (g) `WITH <g> DELETE { ?s ?p "old" } INSERT { ?s ?p "new" } WHERE
--       { ?s ?p "old" }` — combined DELETE+INSERT WHERE under WITH.
--
-- W3C ground: SPARQL 1.1 Update §3.1.3 + §4.1. Spec semantics for the
-- WHERE-side WITH come from §3.1.3 paragraph 3: "If a USING clause is
-- not provided and the WITH clause is provided, the default graph used
-- to evaluate the WHERE clause will be the graph from the WITH clause."
--
-- Invariants locked by this file:
--
--   1. INSERT DATA / DELETE DATA scoped to GRAPH <g> route quads into
--      g's partition; graph-scoped SELECT confirms isolation from the
--      default graph.
--   2. INSERT WHERE with explicit GRAPH in both template and WHERE
--      can copy bindings ACROSS graphs (g1 → g2).
--   3. DELETE WHERE with explicit GRAPH in template + WHERE removes
--      only from the named graph; the default graph is untouched.
--   4. WITH <g> scopes BOTH the WHERE pattern AND the template, even
--      when neither the WHERE nor the template carries an explicit
--      `GRAPH <g>` block — the per-quad graph_name injection
--      (template side) and the pattern wrapping (WHERE side) align.
--   5. WITH <g> DELETE+INSERT WHERE preserves the slice-80 atomic
--      modify semantics under the graph scope.
--   6. The `_update` summary's `graphs_touched` array includes the
--      named graph (not "DEFAULT") when WITH / GRAPH constrain the
--      operation to it.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Seed: distinct triples in g1, g2, and the default graph ─────
-- We seed via INSERT DATA so the GRAPH-in-data path is also exercised
-- as a side effect (invariant 1 first half).
SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_g1_inserted
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { '
  '  GRAPH <http://example.org/g1> { '
  '    ex:alice ex:p "in-g1-a" . '
  '    ex:bob   ex:p "in-g1-b" '
  '  } '
  '}'
) AS s(j);

SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_g2_inserted
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { '
  '  GRAPH <http://example.org/g2> { '
  '    ex:carol ex:p "in-g2-c" '
  '  } '
  '}'
) AS s(j);

SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_default_inserted
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { ex:dan ex:p "in-default-d" }'
) AS s(j);

-- ─── Invariant 1: GRAPH in INSERT DATA isolates to named partition ──
-- Count triples per graph via graph-scoped SELECT (slice 112).
SELECT count(*)::bigint AS rows_in_g1
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s ?o WHERE { GRAPH <http://example.org/g1> { ?s ex:p ?o } }'
  ) AS s(j);

SELECT count(*)::bigint AS rows_in_g2
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s ?o WHERE { GRAPH <http://example.org/g2> { ?s ex:p ?o } }'
  ) AS s(j);

SELECT count(*)::bigint AS rows_in_default
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s ?o WHERE { ?s ex:p ?o }'
  ) AS s(j);

-- ─── Invariant 1b: graphs_touched reports the named graph IRI ─────
-- INSERT DATA { GRAPH <g> { … } } surfaces <g> in graphs_touched (NOT
-- "DEFAULT"). The seeds above wrote it; verify on a fresh op.
SELECT (j->'_update'->'graphs_touched') AS gt_after_g1_extra
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { GRAPH <http://example.org/g1> { ex:extra ex:p "x" } }'
) AS s(j);

-- ─── Invariant 2: cross-graph INSERT WHERE — g1 → g2 ──────────────
-- 2 solutions in g1 (alice, bob) × 1 template quad → 2 inserts.
-- All land in g2 (not g1, not the default).
SELECT
  (j->'_update'->>'form')                            AS form_cross,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_cross,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_cross,
  (j->'_update'->'graphs_touched')                   AS graphs_cross
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT { GRAPH <http://example.org/g2> { ?x ex:tag "t" } } '
  'WHERE  { GRAPH <http://example.org/g1> { ?x ex:p ?o } }'
) AS s(j);

-- Post-state — g2 now has its original 1 ex:p + 2 new ex:tag rows.
SELECT count(*)::bigint AS tag_rows_in_g2
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g2> { ?s ex:tag "t" } }'
  ) AS s(j);

-- And g1 picked up zero ex:tag rows — the template's GRAPH <g2> wins.
SELECT count(*)::bigint AS tag_rows_in_g1
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g1> { ?s ex:tag "t" } }'
  ) AS s(j);

-- ─── Invariant 3: DELETE WHERE scoped to GRAPH <g1> ───────────────
-- Wipe g1's ex:p rows. The default graph's ex:p row stays.
SELECT
  (j->'_update'->>'form')                            AS form_del_scoped,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_del_scoped,
  (j->'_update'->'graphs_touched')                   AS graphs_del_scoped
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'DELETE { GRAPH <http://example.org/g1> { ?s ex:p ?o } } '
  'WHERE  { GRAPH <http://example.org/g1> { ?s ex:p ?o } }'
) AS s(j);

SELECT count(*)::bigint AS p_rows_in_g1_after_del
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g1> { ?s ex:p ?o } }'
  ) AS s(j);

SELECT count(*)::bigint AS p_rows_in_default_after_del
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { ?s ex:p ?o }'
  ) AS s(j);

-- ─── Invariant 4a: WITH <g> INSERT WHERE — both halves scoped ─────
-- Setup: re-seed g1 (we wiped it in inv 3).
SELECT (j->'_update'->>'triples_inserted')::bigint AS reseed_g1
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { '
  '  GRAPH <http://example.org/g1> { '
  '    ex:alice ex:p "in-g1-a" . '
  '    ex:bob   ex:p "in-g1-b" '
  '  } '
  '}'
) AS s(j);

-- `WITH <g1>` scopes both WHERE and INSERT to g1. The default-graph
-- has its own ex:p row (ex:dan); WITH must keep it OUT of the WHERE
-- solutions. THE PROOF: without WITH the WHERE `?x ex:p ?o` would
-- match ALL 4 globals (2 g1 + 1 g2 + 1 default — bare-BGP semantics
-- per slice 114). With WITH wrapping the WHERE in `GRAPH <g1> { … }`
-- the match shrinks to exactly the 2 g1 rows ⇒ `inserted = 2`.
-- The `graphs_touched` array reports `["http://example.org/g1"]`
-- (NOT `["DEFAULT"]`) because the template's per-quad graph_name
-- was injected to g1 by spargebra at parse time.
SELECT
  (j->'_update'->>'form')                            AS form_with_insert,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_with_insert,
  (j->'_update'->'graphs_touched')                   AS graphs_with_insert
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'WITH <http://example.org/g1> '
  'INSERT { ?x ex:tag "with-t" } '
  'WHERE  { ?x ex:p ?o }'
) AS s(j);

-- Post-state — g1 has 2 new ex:tag "with-t" rows.
SELECT count(*)::bigint AS with_tag_rows_in_g1
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g1> { ?s ex:tag "with-t" } }'
  ) AS s(j);

-- Cross-check via direct partition inspection: the default partition
-- (graph_id = 0) holds EXACTLY ONE row — the `ex:dan ex:p "in-default-d"`
-- seed. None of the WITH-inserted ex:tag rows leaked into it. This
-- locks the scope: WITH directed every inserted quad into g1's
-- partition, not the catch-all default. (Bare-BGP SPARQL SELECTs scan
-- every partition, so a SPARQL count of `?s ex:tag "with-t"` would
-- include the g1 rows; the partition probe sidesteps that.)
SELECT count(*)::bigint AS rows_in_default_partition
  FROM pgrdf._pgrdf_quads_default
 WHERE graph_id = 0;

-- ─── Invariant 4b: WITH <g> DELETE WHERE — both halves scoped ─────
-- Delete the ex:tag "with-t" rows we just created in g1. The
-- default-graph remains untouched (we never wrote a "with-t" tag
-- there anyway, but invariant locks the scope).
SELECT
  (j->'_update'->>'form')                            AS form_with_delete,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_with_delete,
  (j->'_update'->'graphs_touched')                   AS graphs_with_delete
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'WITH <http://example.org/g1> '
  'DELETE { ?s ex:tag "with-t" } '
  'WHERE  { ?s ex:tag "with-t" }'
) AS s(j);

SELECT count(*)::bigint AS with_tag_rows_in_g1_after_del
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g1> { ?s ex:tag "with-t" } }'
  ) AS s(j);

-- ─── Invariant 5: WITH <g> DELETE+INSERT WHERE — atomic modify ────
-- Setup: re-seed g1 with a status field, then flip draft → approved.
SELECT (j->'_update'->>'triples_inserted')::bigint AS reseed_status
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { '
  '  GRAPH <http://example.org/g1> { '
  '    ex:alice ex:status "draft" . '
  '    ex:bob   ex:status "draft" '
  '  } '
  '}'
) AS s(j);

-- ALSO seed a draft in the default graph — WITH should leave it alone.
SELECT (j->'_update'->>'triples_inserted')::bigint AS reseed_default_draft
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { ex:default_user ex:status "draft" }'
) AS s(j);

SELECT
  (j->'_update'->>'form')                            AS form_with_modify,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_with_modify,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_with_modify,
  (j->'_update'->'graphs_touched')                   AS graphs_with_modify
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'WITH <http://example.org/g1> '
  'DELETE { ?x ex:status "draft" } '
  'INSERT { ?x ex:status "approved" } '
  'WHERE  { ?x ex:status "draft" }'
) AS s(j);

-- Post-state: g1 has 2 approved, 0 draft; the default graph still
-- has its 1 draft row untouched.
SELECT count(*)::bigint AS approved_in_g1
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g1> { ?s ex:status "approved" } }'
  ) AS s(j);

SELECT count(*)::bigint AS draft_in_g1
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g1> { ?s ex:status "draft" } }'
  ) AS s(j);

SELECT count(*)::bigint AS draft_in_default_after_with
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { ?s ex:status "draft" }'
  ) AS s(j);

-- ─── Invariant 1c: DELETE DATA with GRAPH — scoped removal ────────
-- Pluck one row out of g2 via DELETE DATA { GRAPH <g2> { … } } and
-- confirm partition isolation.
SELECT
  (j->'_update'->>'form')                            AS form_del_data_scoped,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_del_data_scoped,
  (j->'_update'->'graphs_touched')                   AS graphs_del_data_scoped
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'DELETE DATA { GRAPH <http://example.org/g2> { ex:carol ex:p "in-g2-c" } }'
) AS s(j);

SELECT count(*)::bigint AS p_rows_in_g2_after_del_data
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { GRAPH <http://example.org/g2> { ?s ex:p ?o } }'
  ) AS s(j);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
