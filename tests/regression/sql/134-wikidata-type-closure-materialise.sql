-- 134-wikidata-type-closure-materialise.sql — K-7 (TYPE-CLOSURE spec §8):
-- instance-of + subclass-of closure for a Wikidata-shaped graph via
-- owl:TransitiveProperty + pgrdf.materialize — the carve's primary,
-- depth-cap-free closure path.
--
-- The Wikidata pattern "all X" is `?s wdt:P31/wdt:P279* <X>` — it must follow
-- the subclass tree or it under-collects: a Slicer that anchors on a DIRECT
-- `?s wdt:P31 <X>` (or `?t wdt:P279 <X>`) silently drops entities typed to a
-- SUBCLASS of <X>. §8's fix needs no recursive walk: declare the subclass
-- property transitive (`wdt:P279 a owl:TransitiveProperty`), run
-- pgrdf.materialize (OWL 2 RL prp-trp), and every transitive subclass pair
-- becomes a direct `is_inferred = TRUE` edge — so a PLAIN `?t wdt:P279 <X>`
-- resolves the full closure with no recursive CTE and no path_max_depth
-- involvement. (The shipped 111-property-path-materialised-closure.sql proves
-- the same elision for the well-known rdfs:subClassOf; this locks it for the
-- non-well-known wdt:P279 via the owl:TransitiveProperty declaration.)
--
-- Real IRIs: wdt:P279 / wdt:P31 are the actual Wikidata direct-property IRIs;
-- the hierarchy anchors on the real fact `public university (Q875538) ⊑
-- university (Q3918)`. The deeper class (Q9999001) and the instances
-- (Q9999101/02/03) are illustrative test entities in the wd: namespace.
-- All expected values are hand-computed AND confirmed on a live pgrdf v0.6.14
-- instance; never ACCEPT=1 baselined blind.
--
-- Class hierarchy:  Q9999001 ⊑ Q875538 (public university) ⊑ Q3918 (university)
-- Instances (P31):  Q9999101→Q9999001 ,  Q9999102→Q875538 ,  Q9999103→Q3918
--
-- Invariants (all LIVE-verified):
--   A. PRE-materialise PLAIN `?x wdt:P279 wd:Q3918` = {Q875538} only — the
--      deeper subclass (Q9999001) is omitted (the under-collection bug).
--   B. PRE type-closure `?s wdt:P31 ?t . ?t wdt:P279 wd:Q3918` = {Q9999102}
--      only — the instance one subclass down (Q9999101) is missed.
--   C. materialize writes the transitive subclass edge Q9999001→Q3918 as
--      is_inferred = TRUE (inferred_triples_written >= 1).
--   D. POST-materialise PLAIN `?x wdt:P279 wd:Q3918` = {Q875538, Q9999001} —
--      the full subclass closure, now as direct edges.
--   E. POST type-closure = {Q9999101, Q9999102} — the subclass-typed instance
--      is recovered. (Q9999103, typed DIRECTLY to Q3918, is matched by the
--      reflexive/direct arm the Slicer's include_subclasses toggle adds in
--      K-6 — out of scope here; K-7 proves the subclass-closure mechanism.)

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph(13400);
SELECT pgrdf.parse_turtle('
@prefix wd:  <http://www.wikidata.org/entity/> .
@prefix wdt: <http://www.wikidata.org/prop/direct/> .
@prefix owl: <http://www.w3.org/2002/07/owl#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .

# declare the subclass property transitive (the K-7 enabler)
wdt:P279 rdf:type owl:TransitiveProperty .

# class hierarchy (Q875538 ⊑ Q3918 is a real Wikidata fact)
wd:Q9999001 wdt:P279 wd:Q875538 .
wd:Q875538  wdt:P279 wd:Q3918 .

# instances, typed at three depths
wd:Q9999101 wdt:P31 wd:Q9999001 .
wd:Q9999102 wdt:P31 wd:Q875538 .
wd:Q9999103 wdt:P31 wd:Q3918 .
', 13400);

-- ─── Invariant A: PRE-materialise PLAIN subclass query — direct only ──────
SELECT (s.j->>'x') AS pre_subclass
FROM pgrdf.sparql(
  'PREFIX wdt: <http://www.wikidata.org/prop/direct/>
   PREFIX wd:  <http://www.wikidata.org/entity/>
   SELECT ?x WHERE { ?x wdt:P279 wd:Q3918 } ORDER BY ?x'
) AS s(j);

SELECT count(*)::bigint AS pre_subclass_count FROM pgrdf.sparql(
  'PREFIX wdt: <http://www.wikidata.org/prop/direct/>
   PREFIX wd:  <http://www.wikidata.org/entity/>
   SELECT ?x WHERE { ?x wdt:P279 wd:Q3918 }'
);

-- ─── Invariant B: PRE-materialise type-closure — subclass-typed missed ────
SELECT (s.j->>'s') AS pre_typed
FROM pgrdf.sparql(
  'PREFIX wdt: <http://www.wikidata.org/prop/direct/>
   PREFIX wd:  <http://www.wikidata.org/entity/>
   SELECT ?s WHERE { ?s wdt:P31 ?t . ?t wdt:P279 wd:Q3918 } ORDER BY ?s'
) AS s(j);

SELECT count(*)::bigint AS pre_typed_count FROM pgrdf.sparql(
  'PREFIX wdt: <http://www.wikidata.org/prop/direct/>
   PREFIX wd:  <http://www.wikidata.org/entity/>
   SELECT ?s WHERE { ?s wdt:P31 ?t . ?t wdt:P279 wd:Q3918 }'
);

-- ─── Invariant C: materialize writes the transitive subclass edge ─────────
SELECT (pgrdf.materialize(13400)->>'inferred_triples_written')::int >= 1
  AS materialise_wrote_inferred;

SELECT q.is_inferred AS transitive_edge_inferred
FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
  JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
  JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
WHERE s.lexical_value = 'http://www.wikidata.org/entity/Q9999001'
  AND p.lexical_value = 'http://www.wikidata.org/prop/direct/P279'
  AND o.lexical_value = 'http://www.wikidata.org/entity/Q3918';

-- ─── Invariant D: POST-materialise PLAIN subclass query — full closure ────
SELECT (s.j->>'x') AS post_subclass
FROM pgrdf.sparql(
  'PREFIX wdt: <http://www.wikidata.org/prop/direct/>
   PREFIX wd:  <http://www.wikidata.org/entity/>
   SELECT ?x WHERE { ?x wdt:P279 wd:Q3918 } ORDER BY ?x'
) AS s(j);

SELECT count(*)::bigint AS post_subclass_count FROM pgrdf.sparql(
  'PREFIX wdt: <http://www.wikidata.org/prop/direct/>
   PREFIX wd:  <http://www.wikidata.org/entity/>
   SELECT ?x WHERE { ?x wdt:P279 wd:Q3918 }'
);

-- ─── Invariant E: POST-materialise type-closure — instance recovered ──────
SELECT (s.j->>'s') AS post_typed
FROM pgrdf.sparql(
  'PREFIX wdt: <http://www.wikidata.org/prop/direct/>
   PREFIX wd:  <http://www.wikidata.org/entity/>
   SELECT ?s WHERE { ?s wdt:P31 ?t . ?t wdt:P279 wd:Q3918 } ORDER BY ?s'
) AS s(j);

SELECT count(*)::bigint AS post_typed_count FROM pgrdf.sparql(
  'PREFIX wdt: <http://www.wikidata.org/prop/direct/>
   PREFIX wd:  <http://www.wikidata.org/entity/>
   SELECT ?s WHERE { ?s wdt:P31 ?t . ?t wdt:P279 wd:Q3918 }'
);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
