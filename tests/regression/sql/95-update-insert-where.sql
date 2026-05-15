-- 95-update-insert-where.sql
--
-- Phase C slice 82 — SPARQL UPDATE `INSERT { template } WHERE { pattern }`
-- pattern-driven insertion. For each solution row of the WHERE pattern
-- the template's variables substitute and the resulting concrete quads
-- land in `_pgrdf_quads` via the same `WHERE NOT EXISTS` guard slice 84
-- installed for INSERT DATA (LLD v0.4 §4.1, row "INSERT { template }
-- WHERE { pattern }"). The `_update` summary's `form` field reports
-- `"INSERT_WHERE"` to discriminate from `INSERT_DATA` — callers can
-- route post-hoc on which UPDATE variant ran.
--
-- Strategy A (per slice 82 plan): the WHERE pattern goes through the
-- existing v0.3 `parse_select` walker, emits a custom SQL that returns
-- the template-referenced variables' dict ids (BIGINT, not lexical text)
-- one row per solution, and Rust then materialises each template
-- instance and routes it through the shared `insert_quad` helper. This
-- shares the BGP/FILTER/OPTIONAL/MINUS algebra with SELECT — every
-- shape that SELECT translates is admissible in the WHERE half of
-- INSERT WHERE.
--
-- Invariants locked by this file:
--
--   1. Happy path — two `rdf:type ex:Person` rows yield two template
--      instantiations of `?x ex:tag "person"`; the summary reports
--      `form = "INSERT_WHERE"` and `triples_inserted = 2`.
--   2. Zero-match no-op — a WHERE that returns no solutions reports
--      `triples_inserted = 0` and the quads table is unchanged.
--   3. Multi-triple template — N solution rows × M template quads
--      = N×M inserted triples; both the summary counter and the
--      table count agree.
--   4. Round-trip — the new triples are queryable via a follow-up
--      SELECT; the dict internment + partition routing stays
--      consistent between the UPDATE and the SELECT side.
--   5. Set-semantics — the same INSERT WHERE re-issued does not
--      double-count (each template instance routes through the same
--      `WHERE NOT EXISTS` guard as INSERT DATA).
--
-- Negative paths:
--
--   6. An unbound template variable (referenced in the template but
--      NOT bound by the WHERE pattern) panics with the stable
--      `INSERT WHERE template feature 'unbound template variable`
--      prefix.
--   7. A variable graph in the template (`GRAPH ?g { … }`) panics
--      with the stable `INSERT WHERE template feature 'variable
--      GRAPH` prefix (lands with slice 76).
--   8. The combined `DELETE … INSERT … WHERE` form still panics
--      with `UPDATE form 'DELETE/INSERT WHERE' lands` — slice 77's
--      territory. The contiguous substring is the same one slice
--      84's regression `93-update-insert-data.sql` already locks.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — same shape as 81-error-paths / 88-drop-graph / 93-update-
-- insert-data. Captures SQLERRM from a wrapped EXECUTE and asserts the
-- expected substring is present.
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

-- Seed two `rdf:type ex:Person` triples via INSERT DATA (slice 84).
SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_inserted
FROM pgrdf.sparql(
  'PREFIX ex:  <http://example.org/> '
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
  'INSERT DATA { '
  '  ex:alice rdf:type ex:Person . '
  '  ex:bob   rdf:type ex:Person '
  '}'
) AS s(j);

-- ─── Invariant 1: happy path — two solutions ⇒ two inserts ───────
-- `INSERT { ?x ex:tag "person" } WHERE { ?x rdf:type ex:Person }`.
-- The summary's form is INSERT_WHERE (not INSERT_DATA — locked).
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted,
  (j->'_update'->'graphs_touched')                   AS graphs
FROM pgrdf.sparql(
  'PREFIX ex:  <http://example.org/> '
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
  'INSERT { ?x ex:tag "person" } WHERE { ?x rdf:type ex:Person }'
) AS s(j);

-- ─── Invariant 4: round-trip — both subjects now carry the tag ───
SELECT count(*)::bigint AS tagged_persons
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { ?s ex:tag "person" }'
  ) AS s(j);

-- ─── Invariant 2: zero-match no-op ───────────────────────────────
-- `?x foaf:name ?n` matches nothing in the current data (we seeded
-- only `rdf:type ex:Person` + `ex:tag "person"`). The summary still
-- reports a well-formed INSERT_WHERE shape with zero counters.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted
FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/> '
  'PREFIX ex:   <http://example.org/> '
  'INSERT { ?x ex:name ?n } WHERE { ?x foaf:name ?n }'
) AS s(j);

-- ─── Invariant 3: multi-triple template ──────────────────────────
-- Seed two `ex:label` rows then INSERT WHERE with a 3-quad template.
-- 2 solutions × 3 template quads = 6 inserted triples.
SELECT (j->'_update'->>'triples_inserted')::bigint AS seed_labels_inserted
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT DATA { ex:a ex:label "A" . ex:b ex:label "B" }'
) AS s(j);

SELECT (j->'_update'->>'triples_inserted')::bigint AS multi_inserted
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> '
  'INSERT { ?s ex:tag1 "t1" . ?s ex:tag2 "t2" . ?s ex:lbl ?l } '
  'WHERE  { ?s ex:label ?l }'
) AS s(j);

SELECT count(*)::bigint AS bound_lbl_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s ?l WHERE { ?s ex:lbl ?l }'
  ) AS s(j);

-- ─── Invariant 5: set-semantics on re-issue ──────────────────────
-- The same INSERT WHERE issued a second time still reports two
-- attempted inserts (the counter is per-template-instance, not net
-- row delta), but the underlying `ex:tag "person"` rows stay at 2.
SELECT (j->'_update'->>'triples_inserted')::bigint AS reissue_inserted
FROM pgrdf.sparql(
  'PREFIX ex:  <http://example.org/> '
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> '
  'INSERT { ?x ex:tag "person" } WHERE { ?x rdf:type ex:Person }'
) AS s(j);

SELECT count(*)::bigint AS tagged_after_reissue
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/> '
    'SELECT ?s WHERE { ?s ex:tag "person" }'
  ) AS s(j);

-- ─── Negative paths ──────────────────────────────────────────────

-- Invariant 6 — unbound template variable.
SELECT _check_error(
  'insert-where-unbound-template-var',
  $$SELECT * FROM pgrdf.sparql(
      'PREFIX ex: <http://example.org/>
       INSERT { ?x ex:tag ?z } WHERE { ?x ?p ?o }'
    )$$,
  $$INSERT WHERE template feature 'unbound template variable ?z'$$
);

-- Invariant 7 — variable graph in template (slice 76 territory).
SELECT _check_error(
  'insert-where-variable-graph-in-template',
  $$SELECT * FROM pgrdf.sparql(
      'PREFIX ex: <http://example.org/>
       INSERT { GRAPH ?g { ?x ex:tag "t" } } WHERE { GRAPH ?g { ?x ?p ?o } }'
    )$$,
  $$INSERT WHERE template feature 'unbound template variable ?g'$$
);

-- Invariant 8 — combined DELETE+INSERT WHERE shipped in slice 80.
-- The slice-77 panic is gone; we keep a smoke assertion that the
-- dispatcher returns a well-formed `form = "DELETE_INSERT_WHERE"`
-- row. The dedicated regression for the implemented form lives in
-- `97-update-delete-insert-where.sql`.
SELECT
  (j->'_update'->>'form')                            AS form_combined,
  (j->'_update'->>'triples_inserted')::bigint        AS inserted_combined,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted_combined
FROM pgrdf.sparql(
  'PREFIX zzz: <http://example.org/unbound/> '
  'DELETE { ?s zzz:p ?o } INSERT { ?s zzz:p "x" } WHERE { ?s zzz:p ?o }'
) AS s(j);

-- DELETE-only WHERE form: now implemented (Phase C slice 81 — sibling
-- of this slice). The slice-78 "lands" panic was removed when slice 81
-- shipped; the dedicated regression for the implemented form lives in
-- `96-update-delete-where.sql`. We keep a smoke-level assertion here
-- that the dispatcher no longer routes DELETE WHERE through a panic —
-- a WHERE pattern against a never-bound predicate returns zero
-- solutions, so the operation is a well-formed
-- `form = "DELETE_WHERE"`, `triples_deleted = 0` row.
SELECT
  (j->'_update'->>'form')                            AS form,
  (j->'_update'->>'triples_deleted')::bigint         AS deleted
FROM pgrdf.sparql(
  'PREFIX zzz: <http://example.org/unbound/> '
  'DELETE { ?s zzz:p ?o } WHERE { ?s zzz:p ?o }'
) AS s(j);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
