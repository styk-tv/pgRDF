-- 117-materialize-rdfs.sql
--
-- Phase G group G1 (slices 21-18, SPEC.pgRDF.LLD.v0.5-FUTURE §3) —
-- the reasoning-profile selector on `pgrdf.materialize`. v0.5 adds
--
--   pgrdf.materialize(graph_id BIGINT, profile TEXT DEFAULT 'owl-rl')
--
-- with profiles `'owl-rl'` (default, the v0.3/v0.4 surface unchanged)
-- and `'rdfs'` (the RDFS entailment-rule subset only). The spec
-- names this sibling fixture "63-materialize-rdfs.sql"; the actual
-- regression numbering has moved on (116 is the highest pre-G1 slot),
-- so this lands as 117 — the real next free slot.
--
-- §3 implementation route (documented in the commit + LLD): the
-- patched `styk-tv/reasonable` fork exposes only a fused OWL-RL
-- fixpoint (no upstream RDFS-only rule selection), so route 2 — a
-- pgRDF-internal RDFS forward-chain — is used, implemented as a
-- *strict, sound, complete* RDFS rule engine (rdfs2/3/5/7/9/11),
-- NOT a lossy filter. Because the RDFS rules are a strict subset of
-- the OWL 2 RL rules `reasonable` runs, the §3.1 acceptance holds by
-- construction.
--
-- Invariants locked by this file (all expected values hand-computed
-- from the RDFS entailment rules; never ACCEPT=1 baselined):
--
--   Seed (graph 11700), 7 base triples:
--     ex:Engineer  rdfs:subClassOf    ex:Person
--     ex:Person    rdfs:subClassOf    ex:Agent
--     ex:hasParent rdfs:subPropertyOf ex:hasRelative
--     ex:hasParent rdfs:domain        ex:Person
--     ex:hasParent rdfs:range         ex:Person
--     ex:alice     rdf:type           ex:Engineer
--     ex:alice     ex:hasParent       ex:carol
--
--   RDFS closure (the 6 productive rules), hand-derived:
--     rdfs11: Engineer ⊑ Person, Person ⊑ Agent  ⇒ Engineer ⊑ Agent
--     rdfs9 : alice a Engineer + Engineer ⊑ Person ⇒ alice a Person
--             + Engineer ⊑ Agent (rdfs11)         ⇒ alice a Agent
--     rdfs7 : alice hasParent carol + hasParent ⊑ hasRelative
--                                                 ⇒ alice hasRelative carol
--     rdfs2 : hasParent rdfs:domain Person        ⇒ alice a Person (dup)
--     rdfs3 : hasParent rdfs:range  Person        ⇒ carol a Person
--             then rdfs9: carol a Person + Person ⊑ Agent
--                                                 ⇒ carol a Agent
--   = exactly 6 NEW inferred triples under 'rdfs':
--     (1) ex:Engineer rdfs:subClassOf ex:Agent
--     (2) ex:alice    rdf:type        ex:Person
--     (3) ex:alice    rdf:type        ex:Agent
--     (4) ex:alice    ex:hasRelative  ex:carol
--     (5) ex:carol    rdf:type        ex:Person
--     (6) ex:carol    rdf:type        ex:Agent
--
--   A. §3.1 #1 — count(rdfs) ≤ count(owl-rl) on the same input
--      (non-strict subset). rdfs writes exactly 6; owl-rl writes
--      ≥ 6 (reasonable also emits axiomatic OWL/RDFS triples).
--   B. §3.1 #2 — RDFS-axiom agreement: every one of the 6 rdfs
--      entailments is ALSO present under owl-rl (subClassOf
--      transitivity, type propagation, domain/range, subProperty
--      application). Specific triples locked.
--   C. The bare `pgrdf.materialize(g)` default-arg form is
--      byte-identical to `pgrdf.materialize(g,'owl-rl')` (no v0.4
--      regression) — same inferred count.
--   D. JSONB carries a `profile` field = the requested profile
--      ('rdfs', 'owl-rl', and the default-arg form → 'owl-rl').
--   E. §3.1 #3 — an unknown profile string errors with the stable
--      prefix `materialize: unknown profile`; no silent fallback.
--      The reserved future 'owl-rl-ext' is treated as unknown.
--   F. Compose with the v0.4 §7 materialised-closure no-CTE
--      path-detection: a `rdfs:subClassOf*` query over an
--      rdfs-materialised graph emits NO recursive CTE (the
--      heuristic gates on is_inferred subClassOf rows, which the
--      rdfs profile produces just as owl-rl does). EXPLAIN-scrape
--      via `pgrdf.sparql_sql`, mirroring 111's approach.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — re-declared locally so each pg_regress file stays
-- self-contained (same pattern as 88 / 100 / 111).
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

-- EXPLAIN-scrape helper (verbatim from 111) for invariant F.
CREATE OR REPLACE FUNCTION _plan_has_cte_scan(q TEXT)
RETURNS BOOLEAN
LANGUAGE plpgsql AS $$
DECLARE
  inner_sql TEXT;
  plan_json TEXT;
BEGIN
  SELECT pgrdf.sparql_sql(q) INTO inner_sql;
  EXECUTE 'EXPLAIN (FORMAT JSON) ' || inner_sql INTO plan_json;
  RETURN position('"CTE Scan"' IN plan_json) > 0;
END
$$;

-- ─── Seed: the same 7 base triples into two graphs ──────────────
-- 11700 → materialised under 'rdfs'; 11701 → under 'owl-rl'.
-- Identical input so the subset comparison is apples-to-apples.
SELECT pgrdf.add_graph(11700);
SELECT pgrdf.add_graph(11701);

SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.org/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Engineer  rdfs:subClassOf    ex:Person .
ex:Person    rdfs:subClassOf    ex:Agent .
ex:hasParent rdfs:subPropertyOf ex:hasRelative .
ex:hasParent rdfs:domain        ex:Person .
ex:hasParent rdfs:range         ex:Person .
ex:alice     rdf:type           ex:Engineer .
ex:alice     ex:hasParent       ex:carol .
', 11700);

SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.org/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Engineer  rdfs:subClassOf    ex:Person .
ex:Person    rdfs:subClassOf    ex:Agent .
ex:hasParent rdfs:subPropertyOf ex:hasRelative .
ex:hasParent rdfs:domain        ex:Person .
ex:hasParent rdfs:range         ex:Person .
ex:alice     rdf:type           ex:Engineer .
ex:alice     ex:hasParent       ex:carol .
', 11701);

-- ─── Materialise both, capture inferred counts + profile field ──
SELECT (j->>'base_triples')::int = 7 AS rdfs_base_7,
       (j->>'inferred_triples_written')::int AS rdfs_written,
       (j->>'profile') AS rdfs_profile
  FROM (SELECT pgrdf.materialize(11700, 'rdfs') AS j) s \gset

SELECT (j->>'base_triples')::int = 7 AS owl_base_7,
       (j->>'inferred_triples_written')::int AS owl_written,
       (j->>'profile') AS owl_profile
  FROM (SELECT pgrdf.materialize(11701, 'owl-rl') AS j) s \gset

-- ─── Invariant D: profile field reflects the request ────────────
SELECT :'rdfs_profile' = 'rdfs'   AS rdfs_profile_field_ok;
SELECT :'owl_profile'  = 'owl-rl' AS owl_profile_field_ok;

-- ─── Invariant A: exact rdfs count = 6, and rdfs ≤ owl-rl ────────
SELECT :rdfs_written = 6 AS rdfs_exact_six;
SELECT :rdfs_written <= :owl_written AS rdfs_subset_of_owl;

-- ─── Invariant B: RDFS-axiom agreement ──────────────────────────
-- Every one of the 6 hand-derived rdfs entailments must be a
-- TRUE-is_inferred row under BOTH profiles. Check all 6 in each
-- graph; the result is the count of the 6 present (must be 6).
SELECT count(*) = 6 AS rdfs_has_all_six
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
 WHERE q.graph_id = 11700 AND q.is_inferred = TRUE
   AND (
     (s.lexical_value='http://example.org/Engineer' AND p.lexical_value='http://www.w3.org/2000/01/rdf-schema#subClassOf' AND o.lexical_value='http://example.org/Agent')
  OR (s.lexical_value='http://example.org/alice'    AND p.lexical_value='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'   AND o.lexical_value='http://example.org/Person')
  OR (s.lexical_value='http://example.org/alice'    AND p.lexical_value='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'   AND o.lexical_value='http://example.org/Agent')
  OR (s.lexical_value='http://example.org/alice'    AND p.lexical_value='http://example.org/hasRelative'                    AND o.lexical_value='http://example.org/carol')
  OR (s.lexical_value='http://example.org/carol'    AND p.lexical_value='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'   AND o.lexical_value='http://example.org/Person')
  OR (s.lexical_value='http://example.org/carol'    AND p.lexical_value='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'   AND o.lexical_value='http://example.org/Agent')
   );

-- The SAME 6 must also be entailed under owl-rl (agreement).
SELECT count(*) = 6 AS owl_has_all_six
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
 WHERE q.graph_id = 11701 AND q.is_inferred = TRUE
   AND (
     (s.lexical_value='http://example.org/Engineer' AND p.lexical_value='http://www.w3.org/2000/01/rdf-schema#subClassOf' AND o.lexical_value='http://example.org/Agent')
  OR (s.lexical_value='http://example.org/alice'    AND p.lexical_value='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'   AND o.lexical_value='http://example.org/Person')
  OR (s.lexical_value='http://example.org/alice'    AND p.lexical_value='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'   AND o.lexical_value='http://example.org/Agent')
  OR (s.lexical_value='http://example.org/alice'    AND p.lexical_value='http://example.org/hasRelative'                    AND o.lexical_value='http://example.org/carol')
  OR (s.lexical_value='http://example.org/carol'    AND p.lexical_value='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'   AND o.lexical_value='http://example.org/Person')
  OR (s.lexical_value='http://example.org/carol'    AND p.lexical_value='http://www.w3.org/1999/02/22-rdf-syntax-ns#type'   AND o.lexical_value='http://example.org/Agent')
   );

-- No spurious rdfs:Resource-style universal typing under 'rdfs'
-- (the engine restricts to the 6 productive rules — this is what
-- keeps it a true subset of owl-rl).
SELECT NOT EXISTS (
  SELECT 1 FROM pgrdf._pgrdf_quads q
   JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
  WHERE q.graph_id = 11700 AND q.is_inferred = TRUE
    AND o.lexical_value = 'http://www.w3.org/2000/01/rdf-schema#Resource'
) AS rdfs_no_universal_resource_typing;

-- ─── Invariant C: default-arg ≡ owl-rl (no v0.4 regression) ──────
SELECT pgrdf.add_graph(11702);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.org/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Engineer  rdfs:subClassOf    ex:Person .
ex:Person    rdfs:subClassOf    ex:Agent .
ex:hasParent rdfs:subPropertyOf ex:hasRelative .
ex:hasParent rdfs:domain        ex:Person .
ex:hasParent rdfs:range         ex:Person .
ex:alice     rdf:type           ex:Engineer .
ex:alice     ex:hasParent       ex:carol .
', 11702);

SELECT (j->>'inferred_triples_written')::int AS bare_written,
       (j->>'profile') AS bare_profile
  FROM (SELECT pgrdf.materialize(11702) AS j) s \gset

SELECT :bare_written = :owl_written AS bare_equals_owl_rl;
SELECT :'bare_profile' = 'owl-rl' AS bare_profile_owl_rl;

-- ─── Invariant E: unknown profile errors (no silent fallback) ───
SELECT pgrdf.add_graph(11703);
SELECT _check_error(
  'unknown_profile_bogus',
  'SELECT pgrdf.materialize(11703::bigint, ''bogus'')',
  'materialize: unknown profile'
);
SELECT _check_error(
  'unknown_profile_owl_rl_ext',
  'SELECT pgrdf.materialize(11703::bigint, ''owl-rl-ext'')',
  'materialize: unknown profile'
);
-- The failed unknown-profile call must NOT have written inferred
-- rows (validated before the idempotency wipe / any side effect).
SELECT count(*)::bigint = 0 AS unknown_profile_no_side_effect
  FROM pgrdf._pgrdf_quads WHERE graph_id = 11703 AND is_inferred = TRUE;

-- ─── Invariant F: rdfs profile composes with v0.4 §7 no-CTE ──────
-- Pre-materialise (graph 11710), `rdfs:subClassOf*` uses the
-- recursive CTE (no inferred rows yet).
SELECT pgrdf.add_graph(11710);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.org/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:c1 rdfs:subClassOf ex:c2 .
ex:c2 rdfs:subClassOf ex:c3 .
ex:c3 rdfs:subClassOf ex:Top .
ex:anInstance rdf:type ex:c1 .
', 11710);

SELECT _plan_has_cte_scan(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
) AS rdfs_pre_materialise_uses_cte;

-- Materialise under 'rdfs' — rdfs11 writes the transitive
-- subClassOf closure as is_inferred rows.
SELECT (j->>'inferred_triples_written')::int >= 3 AS rdfs_materialise_wrote_inferred
  FROM (SELECT pgrdf.materialize(11710, 'rdfs') AS j) m;

SELECT EXISTS(
  SELECT 1 FROM pgrdf._pgrdf_quads q
   JOIN pgrdf._pgrdf_dictionary d ON d.id = q.predicate_id
  WHERE d.lexical_value = 'http://www.w3.org/2000/01/rdf-schema#subClassOf'
    AND q.is_inferred AND q.graph_id = 11710
) AS rdfs_subclassof_closure_materialised;

-- Post-materialise: the SAME query now emits NO recursive CTE
-- (§7.3 acceptance composes with the rdfs profile).
SELECT _plan_has_cte_scan(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
) AS rdfs_post_materialise_uses_cte;

SELECT position('WITH RECURSIVE' IN pgrdf.sparql_sql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
)) = 0 AS rdfs_post_materialise_no_with_recursive;

-- Result set: c1,c2,c3 + Top (object bound ⇒ reflexive) = 4.
SELECT count(*)::bigint AS rdfs_subclass_post_count FROM pgrdf.sparql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
);

DROP FUNCTION _plan_has_cte_scan(TEXT);
DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
