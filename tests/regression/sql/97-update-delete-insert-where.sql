-- 97-update-delete-insert-where.sql
--
-- Phase C slice 80 — SPARQL UPDATE `DELETE { … } INSERT { … } WHERE
-- { … }` (combined modify form). Both halves resolve against the SAME
-- WHERE solutions snapshot: the pattern is evaluated exactly once, the
-- DELETE template instantiates and removes per-row, then the INSERT
-- template instantiates and adds per-row — all inside the same UDF
-- call (one Postgres transaction → natural atomicity). Per W3C SPARQL
-- 1.1 Update §3.1.3 the DELETE conceptually precedes the INSERT,
-- which matters when the templates overlap on subject/predicate
-- (e.g. a "draft" → "approved" status flip): without DELETE-first,
-- the INSERT would either duplicate the row (and the WHERE NOT EXISTS
-- guard then drops it) or trip a churn loop. The `_update` summary's
-- `form` field reports `"DELETE_INSERT_WHERE"` (distinct from the
-- pure-INSERT-WHERE / pure-DELETE-WHERE halves so callers can route
-- on which variant ran).
--
-- Strategy: share the slice 81/82 WHERE-walk machinery —
-- `parse_select(pattern)` + `build_from_and_where` once, projecting
-- the UNION of template-referenced variables from BOTH halves as
-- BIGINT dict ids. Rust iterates the binding rows via SPI and per
-- row instantiates: (a) the DELETE template through
-- `instantiate_ground_template_quad` (lookup-only — missing terms
-- skip the row), then (b) the INSERT template through
-- `instantiate_template_quad` (interning — fresh dict rows allocate
-- on demand). DELETE counter uses the `WITH d AS (DELETE …
-- RETURNING 1) SELECT count(*)` idiom from slice 83/81 (actual rows
-- removed); INSERT counter is per-attempt as slice 82 (the
-- `WHERE NOT EXISTS` guard silently dedupes but the attempt count
-- surfaces for audit-trail callers).
--
-- Invariants locked by this file:
--
--   1. Status-flip — two `ex:status "draft"` rows DELETE/INSERT
--      WHERE flipped to `ex:status "approved"`. Counters: 2 deletes
--      + 2 inserts. Post-state: 0 draft rows, 2 approved rows.
--   2. Idempotent termination — re-running the same DELETE/INSERT
--      WHERE on the already-flipped state matches 0 rows in the
--      WHERE; the counters are 0/0 and the table is unchanged.
--   3. Multi-template — DELETE { ?x ex:tag "old" } INSERT
--      { ?x ex:tag "new" . ?x ex:updated "true" } WHERE
--      { ?x ex:tag "old" }. Two seeded rows ⇒ 2 deletes +
--      4 inserts (2 solutions × 2 insert-template quads).
--   4. Zero-match no-op — WHERE matches nothing ⇒ both counters
--      are 0, no error.
--   5. Round-trip post-state — SELECT against each predicate
--      confirms the table state matches the counter trail.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Seed: two `ex:status "draft"` rows ──────────────────────────
SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_inserted
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { '
  '  ex:alice ex:status "draft" . '
  '  ex:bob   ex:status "draft" '
  '}'
) AS s(j);

-- ─── Invariant 1: status-flip — 2 deletes + 2 inserts ────────────
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted,
  (j->'_update'->'graphs_touched')                   AS graphs
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'DELETE { ?x ex:status "draft" } '
  'INSERT { ?x ex:status "approved" } '
  'WHERE  { ?x ex:status "draft" }'
) AS s(j);

-- ─── Invariant 5: post-state — 0 drafts, 2 approveds ─────────────
SELECT count(*)::bigint AS drafts_remaining
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?x WHERE { ?x ex:status "draft" }'
  ) AS s(j);

SELECT count(*)::bigint AS approved_after_flip
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?x WHERE { ?x ex:status "approved" }'
  ) AS s(j);

-- ─── Invariant 2: idempotent termination ─────────────────────────
-- Re-issue the same DELETE/INSERT WHERE — the WHERE now matches
-- nothing (no draft rows remain), so both counters report 0 and the
-- table is unchanged.
SELECT
  (j->'_update'->>'form')                            AS form_reissue,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_reissue,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_reissue
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'DELETE { ?x ex:status "draft" } '
  'INSERT { ?x ex:status "approved" } '
  'WHERE  { ?x ex:status "draft" }'
) AS s(j);

-- ─── Invariant 3: multi-template — 2 solutions × 2 inserts ───────
-- Seed two `ex:tag "old"` rows then issue a DELETE+INSERT WHERE
-- with a 1-quad DELETE template and a 2-quad INSERT template.
-- Expected counters: 2 deletes, 4 inserts.
SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_tags_inserted
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { ex:a ex:tag "old" . ex:b ex:tag "old" }'
) AS s(j);

SELECT
  (j->'_update'->>'form')                            AS form_multi,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_multi,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_multi
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'DELETE { ?x ex:tag "old" } '
  'INSERT { ?x ex:tag "new" . ?x ex:updated "true" } '
  'WHERE  { ?x ex:tag "old" }'
) AS s(j);

-- ─── Invariant 5 (continued): post-state — 0 old, 2 new, 2 updated
SELECT count(*)::bigint AS old_tags_remaining
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?x WHERE { ?x ex:tag "old" }'
  ) AS s(j);

SELECT count(*)::bigint AS new_tags_after_multi
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?x WHERE { ?x ex:tag "new" }'
  ) AS s(j);

SELECT count(*)::bigint AS updated_after_multi
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?x WHERE { ?x ex:updated "true" }'
  ) AS s(j);

-- ─── Invariant 4: zero-match no-op against an unrelated WHERE ────
-- The `foaf:name` predicate was never seeded; the WHERE returns no
-- solutions, both counters are 0, and no error fires.
SELECT
  (j->'_update'->>'form')                            AS form_nomatch,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_nomatch,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_nomatch
FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/> '
  'PREFIX ex:   <http://example.org/> '
  'DELETE { ?x ex:name ?n } '
  'INSERT { ?x ex:fullname ?n } '
  'WHERE  { ?x foaf:name ?n }'
) AS s(j);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
