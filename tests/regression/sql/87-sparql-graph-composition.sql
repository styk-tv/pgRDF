-- 87-sparql-graph-composition.sql
--
-- Phase A slice 112 — SPARQL `GRAPH { … }` composition with the
-- other surface operators (OPTIONAL, UNION, MINUS).
--
-- Slices 114 / 113 shipped `GRAPH <iri> { … }` and `GRAPH ?g { … }`
-- for the single-BGP case (one graph constraint applied uniformly
-- to the whole single-branch BGP + OPTIONAL + MINUS bundle).
--
-- Slice 112 lifts that simplifying assumption. The executor now
-- carries an `Option<GraphScope>` PER triple pattern (not per
-- ParsedSelect), so a GRAPH block inside an OPTIONAL / UNION
-- branch / MINUS body scopes ONLY those contained triples; the
-- outer query's BGP keeps its own scope (or `None` = scan every
-- partition). Distinct GRAPH blocks within one query get distinct
-- `scope_id`s; the SQL builder emits one INNER JOIN to
-- `_pgrdf_graphs g{scope_id}` per Variable scope that has a
-- mandatory triple, and one LEFT JOIN for scopes born inside an
-- OPTIONAL — so an unmatched OPTIONAL still leaves `?g` NULL on
-- the outer row.
--
-- Coverage shapes locked here:
--
--   1. `GRAPH <g1> { ?s ex:p ?o } OPTIONAL { GRAPH <g2> { ?s ex:q ?v } }`
--      — outer BGP scoped to g1, OPTIONAL scoped to g2. Unmatched
--      OPTIONAL leaves ?v NULL but the outer row survives.
--   2. `{ GRAPH <g1> { ?s ex:p ?o } } UNION { GRAPH <g2> { ?s ex:p ?o } }`
--      — each branch independent graph scope. Rows come from both
--      graphs, with no cross-graph stitches.
--   3. `?s ex:p ?o MINUS { GRAPH <g1> { ?s ex:q ?o2 } }` — MINUS
--      body scoped to g1; only g1's exclusions apply.
--   4. `GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } }` — OPTIONAL
--      inherits the outer GRAPH ?g scope; both triples must share
--      the same graph_id.
--   5. `GRAPH <g1> { ?s ex:p ?o MINUS { ?s ex:q ?o2 } }` — MINUS
--      inherits the outer GRAPH <g1> scope.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- Three named graphs with overlapping subjects.
-- g1: ex:alice ex:p "p1-alice" ; ex:q "q1-alice"
-- g2: ex:alice ex:p "p2-alice" ; ex:q "q2-alice"
-- g3: ex:bob   ex:p "p3-bob"
-- The default graph (graph_id 0) carries ex:alice and ex:bob for the
-- bare-BGP shape used in cases (3) and (4)'s outer MINUS.
SELECT pgrdf.add_graph('http://example.org/g1');
SELECT pgrdf.add_graph('http://example.org/g2');
SELECT pgrdf.add_graph('http://example.org/g3');

SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:alice ex:p "p1-alice" .
   ex:alice ex:q "q1-alice" .',
  pgrdf.graph_id('http://example.org/g1')
);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:alice ex:p "p2-alice" .
   ex:alice ex:q "q2-alice" .',
  pgrdf.graph_id('http://example.org/g2')
);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:bob ex:p "p3-bob" .',
  pgrdf.graph_id('http://example.org/g3')
);
-- Default graph (id 0) carries ex:alice and ex:bob each with one
-- ex:p but no ex:q. Used by the MINUS / outer-bare-BGP shapes.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:alice ex:p "p-default-alice" .
   ex:bob   ex:p "p-default-bob"   .',
  0
);

-- ─── Case 1: GRAPH <g1> wraps outer; OPTIONAL wraps GRAPH <g2> ────
-- Outer BGP scoped to g1 → one row (alice, p1-alice).
-- OPTIONAL { GRAPH <g2> { ?s ex:q ?v } } binds ?v to "q2-alice"
-- because alice's ?s value bound by g1 also appears in g2 via the
-- shared subject IRI. The OPTIONAL did match.
SELECT count(*) AS case1_row_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o ?v WHERE {
     GRAPH <http://example.org/g1> { ?s ex:p ?o }
     OPTIONAL { GRAPH <http://example.org/g2> { ?s ex:q ?v } }
   }'
);

SELECT bool_and(
  (s.j->>'s') = 'http://example.org/alice'
  AND (s.j->>'o') = 'p1-alice'
  AND (s.j->>'v') = 'q2-alice'
) AS case1_optional_match
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o ?v WHERE {
     GRAPH <http://example.org/g1> { ?s ex:p ?o }
     OPTIONAL { GRAPH <http://example.org/g2> { ?s ex:q ?v } }
   }'
) AS s(j);

-- Same query against g3 in the outer (bob, only ?p) — OPTIONAL on
-- g2 returns nothing for bob, so ?v is NULL but the outer row
-- survives. Confirms the LEFT-JOIN semantics of the OPTIONAL +
-- nested GRAPH compose correctly.
SELECT count(*) AS case1b_row_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o ?v WHERE {
     GRAPH <http://example.org/g3> { ?s ex:p ?o }
     OPTIONAL { GRAPH <http://example.org/g2> { ?s ex:q ?v } }
   }'
);

SELECT bool_and(
  (s.j->>'s') = 'http://example.org/bob'
  AND (s.j->>'o') = 'p3-bob'
  AND (s.j->>'v') IS NULL
) AS case1b_optional_unbound
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o ?v WHERE {
     GRAPH <http://example.org/g3> { ?s ex:p ?o }
     OPTIONAL { GRAPH <http://example.org/g2> { ?s ex:q ?v } }
   }'
) AS s(j);

-- ─── Case 2: { GRAPH <g1> { … } } UNION { GRAPH <g2> { … } } ─────
-- Each branch independently scoped; UNION ALL across the two.
-- g1 has one ex:p (p1-alice); g2 has one ex:p (p2-alice) → 2 rows.
SELECT count(*) AS case2_union_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE {
     { GRAPH <http://example.org/g1> { ?s ex:p ?o } }
     UNION
     { GRAPH <http://example.org/g2> { ?s ex:p ?o } }
   }'
);

-- Both branches surface exactly the expected pair (per-graph).
SELECT count(*) AS case2_pairs_matched FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE {
     { GRAPH <http://example.org/g1> { ?s ex:p ?o } }
     UNION
     { GRAPH <http://example.org/g2> { ?s ex:p ?o } }
   }'
) AS s(j)
WHERE (s.j->>'o') IN ('p1-alice', 'p2-alice');

-- ─── Case 3: outer bare-BGP MINUS GRAPH <g1> { … } ──────────────
-- Outer matches every ex:p across every partition (alice@g1,
-- alice@g2, bob@g3, alice@default, bob@default — 5 rows).
-- MINUS { GRAPH <g1> { ?s ex:q ?o2 } } subtracts rows whose ?s
-- is bound in g1's ex:q — that's alice (q1-alice). So all alice
-- rows drop, leaving only bob's two rows (g3 + default).
SELECT count(*) AS case3_after_minus FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE {
     ?s ex:p ?o
     MINUS { GRAPH <http://example.org/g1> { ?s ex:q ?o2 } }
   }'
);

-- No alice rows survive the MINUS.
SELECT count(*) AS case3_no_alice_rows FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE {
     ?s ex:p ?o
     MINUS { GRAPH <http://example.org/g1> { ?s ex:q ?o2 } }
   }'
) AS s(j)
WHERE (s.j->>'s') = 'http://example.org/alice';

-- ─── Case 4: GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } } ─────
-- Both triples MUST come from the same graph (the OPTIONAL inherits
-- the outer GRAPH ?g scope; slice 112's per-pattern scope propagates
-- into the OPTIONAL's triple). g1 has ex:p and ex:q both for alice
-- → 1 row, ?v = "q1-alice". g2 likewise → 1 row, ?v = "q2-alice".
-- g3 has only ex:p for bob → 1 row, ?v NULL (OPTIONAL unmatched).
SELECT count(*) AS case4_row_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?o ?v WHERE {
     GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } }
   }'
);

-- The g1 row has its ex:q from the SAME graph (q1, not q2).
SELECT count(*) AS case4_g1_q1_pairing FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?o ?v WHERE {
     GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } }
   }'
) AS s(j)
WHERE (s.j->>'g') = 'http://example.org/g1' AND (s.j->>'v') = 'q1-alice';

-- The g3 row's OPTIONAL is unmatched (bob has no ex:q in g3).
SELECT count(*) AS case4_g3_v_unbound FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?o ?v WHERE {
     GRAPH ?g { ?s ex:p ?o OPTIONAL { ?s ex:q ?v } }
   }'
) AS s(j)
WHERE (s.j->>'g') = 'http://example.org/g3' AND (s.j->>'v') IS NULL;

-- ─── Case 5: GRAPH <g1> { ?s ex:p ?o MINUS { ?s ex:q ?o2 } } ─────
-- MINUS inherits the outer GRAPH <g1> scope: every alias inside the
-- MINUS NOT EXISTS subquery is constrained to graph_id = g1. Alice
-- in g1 has both ex:p and ex:q, so MINUS subtracts her; result is
-- empty.
SELECT count(*) AS case5_minus_inherits_outer FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE {
     GRAPH <http://example.org/g1> { ?s ex:p ?o MINUS { ?s ex:q ?o2 } }
   }'
);

-- Sanity: WITHOUT the MINUS, GRAPH <g1> { ?s ex:p ?o } returns the
-- 1 alice-row from g1. Confirms the outer scope is otherwise correct.
SELECT count(*) AS case5_outer_only FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE {
     GRAPH <http://example.org/g1> { ?s ex:p ?o }
   }'
);

-- Cleanup so the next regression file starts from a clean slate.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
