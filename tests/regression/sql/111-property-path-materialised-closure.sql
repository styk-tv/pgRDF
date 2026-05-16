-- 111-property-path-materialised-closure.sql
--
-- Phase E group E4 (slices 37-35) — the materialised-closure no-CTE
-- fallback (LLD v0.4 §7.2 v0.4 heuristic / §7.3 acceptance). Builds
-- on E2's `+` recursive CTE (109) and E3's `*` (110): when the graph
-- has been materialised under a profile that already entails the
-- closure of the path's predicate, a recursive CTE is wasted work —
-- every transitive pair is already a direct `is_inferred = TRUE`
-- edge. The translator detects this per-query (NOT cached) and emits
-- a DIRECT match instead of the `WITH RECURSIVE` CTE.
--
-- v0.4 heuristic: for `+`/`*` over a SINGLE predicate that is one of
-- the well-known transitive predicates (rdfs:subClassOf,
-- rdfs:subPropertyOf, owl:sameAs), if `_pgrdf_quads` carries any
-- `is_inferred = TRUE` row for that predicate in the active scope,
-- the recursive CTE is elided. `pgrdf.materialize(graph_id)` (the
-- v0.3 OWL-RL UDF, unchanged) is what writes the `is_inferred = TRUE`
-- closure rows.
--
-- §7.3 acceptance: the SAME `?c rdfs:subClassOf* <Top>` query,
-- post-`pgrdf.materialize`, emits NO recursive CTE in the executed
-- plan — verified here by scraping `EXPLAIN (FORMAT JSON)` of the
-- translated SQL (exposed via the `pgrdf.sparql_sql` debug hook) for
-- the ABSENCE of `CTE Scan`. The user-visible result set must be
-- IDENTICAL before and after materialize (the optimisation is
-- semantics-preserving).
--
-- Invariants locked by this file (all expected values hand-computed;
-- never ACCEPT=1 baselined):
--
--   A. Pre-materialise: `?c rdfs:subClassOf* <Top>` over a length-5
--      chain resolves all 5 subclasses + <Top> itself (reflexive)
--      = 6 rows. The translated SQL CONTAINS a recursive CTE
--      (`WITH RECURSIVE` / `CTE Scan` present in the plan).
--   B. Post-materialise: the IDENTICAL query returns the IDENTICAL
--      6-row set, AND the translated SQL has NO `WITH RECURSIVE`
--      and the executed plan has NO `CTE Scan` (§7.3 acceptance).
--   C. `+` (non-reflexive) likewise: `<Bottom> rdfs:subClassOf+ ?c`
--      pre-materialise = recursive CTE, 5 ancestors; post-materialise
--      = no CTE, SAME 5 ancestors.
--   D. The fallback is well-known-predicate-gated: a `*` over a
--      NON-well-known predicate (`ex:plainLink`) with inferred rows
--      present still emits the recursive CTE (the heuristic only
--      recognises subClassOf / subPropertyOf / sameAs).
--   E. The fallback is is_inferred-gated: before any materialize,
--      `rdfs:subClassOf*` over a graph with NO inferred rows uses
--      the CTE (only a materialised closure elides it).
--   F. `?` and `^` are unaffected (no recursion to elide) — a
--      `rdfs:subClassOf?` never carried a CTE, materialised or not.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper: does the EXECUTED plan of the SQL `pgrdf.sparql` would run
-- for `q` contain a `CTE Scan` node? We get the translated SQL via
-- the `pgrdf.sparql_sql` debug hook (dict ids inlined, so it is
-- self-contained + safely EXPLAIN-able), EXPLAIN it as JSON, and
-- substring-probe the plan text for `"Node Type": "CTE Scan"`. A
-- recursive property-path CTE always surfaces as a CTE Scan in the
-- executed plan; the materialised-closure fallback emits a plain
-- scan instead.
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

-- ── Seed: a length-5 subClassOf chain in graph 11100. ────────────
--   c1 ⊏ c2 ⊏ c3 ⊏ c4 ⊏ c5 ⊏ Top   (5 subClassOf edges, 6 classes)
-- Plus an OWL-RL trigger: a typed individual so materialize has a
-- reason to run the subClassOf transitive rule. Also a parallel
-- `ex:plainLink` chain (NOT a well-known transitive predicate) for
-- invariant D.
SELECT pgrdf.add_graph(11100);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.org/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:c1 rdfs:subClassOf ex:c2 .
ex:c2 rdfs:subClassOf ex:c3 .
ex:c3 rdfs:subClassOf ex:c4 .
ex:c4 rdfs:subClassOf ex:c5 .
ex:c5 rdfs:subClassOf ex:Top .
ex:anInstance rdf:type ex:c1 .
ex:n1 ex:plainLink ex:n2 .
ex:n2 ex:plainLink ex:n3 .
ex:n3 ex:plainLink ex:n4 .
', 11100);

-- ─── Invariant E: no inferred rows yet ⇒ the CTE is used ─────────
-- Before any materialize the graph has zero is_inferred rows, so
-- `rdfs:subClassOf*` MUST translate to the recursive CTE. Probed
-- UNSCOPED (slice-112: scans all graphs; only graph 11100 has any
-- subClassOf edges here, so the answer is unambiguous).
SELECT _plan_has_cte_scan(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
) AS pre_materialise_uses_cte;

-- ─── Invariant A: pre-materialise result + recursive CTE ─────────
-- `?c rdfs:subClassOf* ex:Top` — every transitive subclass of Top
-- (c1..c5) PLUS Top itself (object bound ⇒ (Top,Top) reflexive,
-- W3C §9.3) = 6 rows.
SELECT (s.j->>'c') AS subclass_pre
FROM pgrdf.sparql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top } ORDER BY ?c'
) AS s(j);

SELECT count(*)::bigint AS subclass_pre_count FROM pgrdf.sparql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
);

-- ─── Materialise the OWL-RL closure (writes is_inferred rows) ────
-- Strip the non-deterministic elapsed_ms before asserting; assert
-- that the subClassOf transitive closure produced inferred rows.
SELECT (j->>'inferred_triples_written')::int >= 4 AS materialise_wrote_inferred
FROM (SELECT pgrdf.materialize(11100) - 'elapsed_ms' AS j) m;

-- Confirm is_inferred rows for rdfs:subClassOf are now present
-- (the detection probe's precondition).
SELECT EXISTS(
  SELECT 1
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary d ON d.id = q.predicate_id
  WHERE d.lexical_value = 'http://www.w3.org/2000/01/rdf-schema#subClassOf'
    AND q.is_inferred
) AS subclassof_closure_materialised;

-- ─── Invariant B: post-materialise — NO CTE, IDENTICAL result ────
-- §7.3 acceptance: the SAME query now emits NO recursive CTE.
SELECT _plan_has_cte_scan(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
) AS post_materialise_uses_cte;

-- The translated SQL itself must no longer carry WITH RECURSIVE.
SELECT position('WITH RECURSIVE' IN pgrdf.sparql_sql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
)) = 0 AS post_materialise_sql_has_no_with_recursive;

-- Result set IDENTICAL to pre-materialise (semantics preserved).
-- The materialised closure makes every transitive pair a direct
-- edge, so direct ∪ identity (= the `?`-shaped fallback relation)
-- yields exactly the same 6 classes (c1..c5 + Top).
SELECT (s.j->>'c') AS subclass_post
FROM pgrdf.sparql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top } ORDER BY ?c'
) AS s(j);

SELECT count(*)::bigint AS subclass_post_count FROM pgrdf.sparql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ?c rdfs:subClassOf* ex:Top }'
);

-- ─── Invariant C: `+` (non-reflexive) likewise ──────────────────
-- `ex:c1 rdfs:subClassOf+ ?c` — every strict transitive superclass
-- of c1 = {c2,c3,c4,c5,Top} = 5 (non-reflexive: c1 itself excluded).
-- Pre-materialise the SQL carries the recursive CTE, post-materialise
-- it is the direct (non-reflexive) step over the materialised
-- closure — SAME 5-row answer either way.
SELECT _plan_has_cte_scan(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ex:c1 rdfs:subClassOf+ ?c }'
) AS plus_post_materialise_uses_cte;

SELECT count(*)::bigint AS plus_post_count FROM pgrdf.sparql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ex:c1 rdfs:subClassOf+ ?c }'
);

SELECT (s.j->>'c') AS superclass_post
FROM pgrdf.sparql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ex:c1 rdfs:subClassOf+ ?c } ORDER BY ?c'
) AS s(j);

-- ─── Invariant D: non-well-known predicate keeps the CTE ─────────
-- `ex:plainLink` is NOT one of the three well-known transitive
-- predicates. Even with inferred rows present in the graph (the
-- materialize above also entailed nothing for ex:plainLink, but the
-- gate is predicate-identity, not row-count), `?n ex:plainLink* ?x`
-- MUST still translate to the recursive CTE — the heuristic only
-- recognises subClassOf / subPropertyOf / sameAs.
SELECT _plan_has_cte_scan(
  'PREFIX ex: <http://example.org/>
   SELECT ?x WHERE { ex:n1 ex:plainLink* ?x }'
) AS plainlink_star_still_uses_cte;

-- ─── Invariant F: `?` / `^` unaffected (never had a CTE) ─────────
-- `rdfs:subClassOf?` is non-recursive (direct ∪ identity) — it never
-- carried a CTE, so the materialised-closure fallback is a no-op for
-- it (the fallback only ever elides a `+`/`*` recursive CTE). Confirm
-- NO CTE. Note the result is `ex:c1 rdfs:subClassOf? ?c` = identity
-- (c1) ∪ every DIRECT `subClassOf` edge from c1 — and POST-MATERIALISE
-- the OWL-RL closure made c1 directly subClassOf its full ancestor
-- chain (c2,c3,c4,c5,Top), so `?` here yields 6 (c1 + 5 now-direct
-- edges). This is W3C-correct: `?` is "the term, or one direct edge",
-- and materialize genuinely added those direct edges. The point of
-- this invariant is the NO-CTE shape, not the cardinality.
SELECT _plan_has_cte_scan(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ex:c1 rdfs:subClassOf? ?c }'
) AS opt_uses_cte;

SELECT count(*)::bigint AS opt_count FROM pgrdf.sparql(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ex:c1 rdfs:subClassOf? ?c }'
);

-- `^(rdfs:subClassOf)` (inverse, E1 lower-to-triple) has no CTE
-- either — it is a plain swapped triple, materialised or not.
SELECT _plan_has_cte_scan(
  'PREFIX rdfs: <http://www.w3.org/2000/01/rdf-schema#>
   PREFIX ex: <http://example.org/>
   SELECT ?c WHERE { ex:c2 ^rdfs:subClassOf ?c }'
) AS inverse_uses_cte;

DROP FUNCTION _plan_has_cte_scan(TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
