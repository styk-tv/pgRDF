-- 96-update-delete-where.sql
--
-- Phase C slice 81 — SPARQL UPDATE `DELETE { template } WHERE { pattern }`
-- pattern-driven removal. Sibling of slice 82's INSERT WHERE
-- (`tests/regression/sql/95-update-insert-where.sql`). For each
-- solution row of the WHERE pattern the template's variables
-- substitute and the resulting concrete quads are removed from
-- `_pgrdf_quads` via the same lookup-only dictionary path slice 83
-- installed for DELETE DATA (LLD v0.4 §4.1, row `DELETE { template }
-- WHERE { pattern }`). The `_update` summary's `form` field reports
-- `"DELETE_WHERE"` to discriminate from `DELETE_DATA` — callers can
-- route post-hoc on which UPDATE variant ran.
--
-- Strategy mirrors slice 82: the WHERE pattern goes through the v0.3
-- `parse_select` walker (sharing BGP/FILTER/OPTIONAL/MINUS algebra
-- with SELECT), emits a custom SQL that returns the template-
-- referenced variables' dict ids (BIGINT, not lexical text) one row
-- per solution, and Rust then materialises each template ground quad
-- per row. Per-row template-quad DELETE uses the same
-- `WITH d AS (DELETE … RETURNING 1) SELECT count(*)` idiom slice 83
-- installed for DELETE DATA, so `triples_deleted` counts ACTUAL rows
-- removed (not template instantiations attempted) — important
-- distinction from INSERT WHERE's "attempted insert" counter.
--
-- spargebra models DELETE templates as `Vec<GroundQuadPattern>`
-- (rather than `Vec<QuadPattern>` for INSERT) — the type bakes the
-- W3C SPARQL 1.1 §4.1.2 rule "blank nodes are not allowed in the
-- DELETE clause" into the AST. The implementation surfaces this in
-- the helper-pair `collect_ground_template_vars` /
-- `instantiate_ground_template_quad` — see `src/query/executor.rs`
-- (slice 81 block).
--
-- Invariants locked by this file:
--
--   1. Happy path with FILTER — four `rdf:type ex:Person` rows
--      seeded, DELETE WHERE narrowed by `FILTER(?x = ex:carol)`
--      removes exactly one; the summary reports
--      `form = "DELETE_WHERE"` and `triples_deleted = 1`.
--   2. Broad DELETE WHERE — the remaining three `rdf:type ex:Person`
--      rows fall to a single un-filtered DELETE WHERE; counter
--      reports 3.
--   3. Zero-match no-op — DELETE WHERE against a pattern that
--      returns no solutions reports `triples_deleted = 0` and the
--      quads table is unchanged. Never errors (spec-correct
--      "remove if exists", per LLD v0.4 §4 set-semantic contract).
--   4. Round-trip post-state — SELECT against the type-of-Person
--      pattern confirms the table state matches the counter trail.
--   5. Set-semantics on re-issue — issuing the broad DELETE WHERE
--      twice deletes 3 rows on the first call, 0 on the second
--      (idempotent termination, mirrors slice 83 DELETE DATA's
--      "second call deletes zero rows" lock).
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Seed four `rdf:type ex:Person` triples via INSERT DATA (slice 84).
SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_inserted
FROM pgrdf.sparql(
  'PREFIX ex:  <http://example.org/> '
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
  'INSERT DATA { '
  '  ex:alice rdf:type ex:Person . '
  '  ex:bob   rdf:type ex:Person . '
  '  ex:carol rdf:type ex:Person . '
  '  ex:dave  rdf:type ex:Person '
  '}'
) AS s(j);

-- ─── Invariant 1: filtered DELETE WHERE removes exactly one ──────
-- `DELETE { ?x rdf:type ex:Person } WHERE { ?x rdf:type ex:Person
-- FILTER(?x = ex:carol) }` — only carol's row matches.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted,
  (j->'_update'->'graphs_touched')                   AS graphs
FROM pgrdf.sparql(
  'PREFIX ex:  <http://example.org/> '
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
  'DELETE { ?x rdf:type ex:Person } '
  'WHERE  { ?x rdf:type ex:Person FILTER(?x = ex:carol) }'
) AS s(j);

-- ─── Invariant 4: post-state — three persons remain ──────────────
SELECT count(*)::bigint AS remaining_persons
  FROM pgrdf.sparql(
    'PREFIX ex:  <http://example.org/> '
    'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
    'SELECT ?x WHERE { ?x rdf:type ex:Person }'
  ) AS s(j);

-- ─── Invariant 2: broad DELETE WHERE removes the rest ────────────
-- No FILTER — every `rdf:type ex:Person` solution row falls.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted
FROM pgrdf.sparql(
  'PREFIX ex:  <http://example.org/> '
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
  'DELETE { ?x rdf:type ex:Person } WHERE { ?x rdf:type ex:Person }'
) AS s(j);

-- ─── Invariant 5: set-semantics on re-issue — second call zero ───
SELECT (j->'_update'->>'triples_deleted')::bigint AS reissue_deleted
FROM pgrdf.sparql(
  'PREFIX ex:  <http://example.org/> '
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
  'DELETE { ?x rdf:type ex:Person } WHERE { ?x rdf:type ex:Person }'
) AS s(j);

-- ─── Invariant 4 (continued): all persons gone ───────────────────
SELECT count(*)::bigint AS persons_after_broad_delete
  FROM pgrdf.sparql(
    'PREFIX ex:  <http://example.org/> '
    'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
    'SELECT ?x WHERE { ?x rdf:type ex:Person }'
  ) AS s(j);

-- ─── Invariant 3: zero-match no-op against an unrelated pattern ──
-- The `foaf:name` predicate was never seeded; the WHERE returns no
-- solutions, the counter is 0, and no error fires.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted
FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/> '
  'PREFIX ex:   <http://example.org/> '
  'DELETE { ?x ex:name ?n } WHERE { ?x foaf:name ?n }'
) AS s(j);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
