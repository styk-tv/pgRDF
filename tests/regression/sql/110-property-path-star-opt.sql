-- 110-property-path-star-opt.sql
--
-- Phase E group E3 (slices 41-38) — SPARQL property-path `*`
-- (zero-or-more) and `?` (zero-or-one). Builds on E2's `+` recursive
-- CTE (109-property-path-plus): `*` is the E2 cycle-safe recursive
-- `+` walk UNION the W3C SPARQL 1.1 §9.3 zero-length node-set; `?`
-- is the single direct edge UNION the SAME zero-length node-set
-- (non-recursive, no depth guard). The §7.2 spec sketch
-- ("union with `SELECT ?s ?s`") is refined here to the precise W3C
-- §9.3 ZeroLengthPath set — exactly as E2 refined §7.2's bare-UNION
-- cycle sketch to Postgres's CYCLE clause.
--
-- W3C §9.3 zero-length-path semantics (the subtle part):
--   * BOUND endpoint (`<x> p* ?o` etc.) → the self-pair (x,x) holds
--     UNCONDITIONALLY, even if x is not a node of the active graph.
--   * UNBOUND endpoint (`?s p* ?o`) → reflexive pairs (n,n) for every
--     node n that is a term of the active graph in subject OR object
--     position. Scoped to the active GRAPH (a node only in another
--     graph is NOT in the identity set of the scoped query).
--   `?` shares the SAME zero-length set (W3C ZeroLengthPath is shared
--   between `*` and `?`).
--
-- Invariants locked by this file (all expected values hand-computed;
-- never ACCEPT=1 baselined):
--
--   A. `*` full chain + reflexive (§7.3) — length-10 `sub` chain
--      c1..c11. `?x sub* <c11>` = c1..c11 (11 rows — c11 reflexive-
--      includes itself). `<c1> sub* ?o` = c1..c11 (11 — c1 includes
--      itself).
--   B. `*` both-bound reflexive. `ASK { <c5> sub* <c5> }` true
--      (zero-length); `ASK { <c1> sub* <c11> }` true (transitive);
--      `ASK { <c11> sub* <c1> }` false.
--   C. `*` both-var = identity ∪ p+. Tiny graph a→b→c (nodes
--      {a,b,c}): `?s sub* ?o` = {(a,a),(b,b),(c,c)} ∪
--      {(a,b),(b,c),(a,c)} = 6 rows exactly.
--   D. `*` bound-subject identity even when isolated. <lone> is NOT
--      seeded. `ASK { <lone> sub* <lone> }` → true; `<lone> sub* ?o`
--      → just <lone> (1 row).
--   E. `?` direct ∪ identity. `<a> sub? ?o` = {a, b} (2 rows);
--      `ASK { <a> sub? <a> }` true; `ASK { <a> sub? <b> }` true;
--      `ASK { <a> sub? <c> }` false; `?s sub? ?o` over {a,b,c} =
--      identity {(a,a),(b,b),(c,c)} ∪ direct {(a,b),(b,c)} = 5 rows.
--   F. `*` is cycle-safe. a→b→c→a. `<a> rel* ?o` terminates →
--      {a,b,c} (3 rows; reflexive a + cycle). Stat stays 0 (a cycle
--      is NOT a truncation — E2 CYCLE-clause discipline inherited).
--   G. `*` depth-guard. `SET pgrdf.path_max_depth=3` over the
--      length-10 chain: `<c1> sub* ?o` → c1 (reflexive) + c2,c3,c4
--      (depths 1-3) = 4 rows; `path_depth_truncations` > 0;
--      shmem_reset() → 0; an under-cap `*` traversal leaves it 0.
--   H. `^(p*)` / `(^p)*` inverse composition = forward `p*` with
--      endpoints swapped (equivalence assertion). Cardinality is
--      computed over the UNSCOPED all-graphs set (slice-112) = 72.
--   I. `GRAPH <iri>` and `GRAPH ?g` scoping — the zero-length
--      node-set is scoped to that graph's nodes (a node only in
--      another graph is NOT in the scoped identity set). Locked
--      explicitly for both `*` and `?`.
--   J. BGP join + `pgrdf.construct` inheritance. `p*` beside a plain
--      pattern (join); `CONSTRUCT { ?x ex:reaches ex:c11 } WHERE
--      { ?x sub* ex:c11 }` emits 11 rows.
--   K. `|` / nested-recursive / negated still preview-panic with
--      stable prefixes (E4 not shipped); substring locks only.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — captures SQLERRM from a wrapped EXECUTE and asserts the
-- expected substring is present. Same shape as 108 / 109 / 93.
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

-- Default-graph seed: a length-10 subClassOf-style chain.
--   c1 sub c2 . … c10 sub c11   (10 edges, 11 nodes c1..c11)
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:c1 ex:sub ex:c2 . ex:c2 ex:sub ex:c3 . ex:c3 ex:sub ex:c4 .
   ex:c4 ex:sub ex:c5 . ex:c5 ex:sub ex:c6 . ex:c6 ex:sub ex:c7 .
   ex:c7 ex:sub ex:c8 . ex:c8 ex:sub ex:c9 . ex:c9 ex:sub ex:c10 .
   ex:c10 ex:sub ex:c11 .',
  0
);

-- ─── Invariant A: `*` full chain + reflexive (§7.3) ──────────────
-- `?x ex:sub* ex:c11` — every transitive ancestor of c11 (c1..c10)
-- PLUS c11 itself (W3C §9.3: object is bound ⇒ (c11,c11) holds
-- unconditionally). 11 rows, ordered.
SELECT (s.j->>'x') AS star_to_c11
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?x WHERE { ?x ex:sub* ex:c11 } ORDER BY ?x'
) AS s(j);

SELECT count(*)::bigint AS star_to_c11_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?x WHERE { ?x ex:sub* ex:c11 }'
);

-- `ex:c1 ex:sub* ?o` — every transitive descendant of c1 (c2..c11)
-- PLUS c1 itself (subject bound ⇒ (c1,c1) unconditional). 11 rows.
SELECT (s.j->>'o') AS star_from_c1
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:c1 ex:sub* ?o } ORDER BY ?o'
) AS s(j);

SELECT count(*)::bigint AS star_from_c1_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:c1 ex:sub* ?o }'
);

-- ─── Invariant B: `*` both-bound reflexive ───────────────────────
-- <c5> sub* <c5> — true via the zero-length path (reflexive).
SELECT (s.j->>'_ask') AS c5_star_self
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> ASK { ex:c5 ex:sub* ex:c5 }'
) AS s(j);
-- <c1> sub* <c11> — true via the length-10 transitive chain.
SELECT (s.j->>'_ask') AS c1_star_c11
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> ASK { ex:c1 ex:sub* ex:c11 }'
) AS s(j);
-- <c11> sub* <c1> — false (chain is one-directional, c11 has no
-- sub edge and c11 != c1).
SELECT (s.j->>'_ask') AS c11_star_c1
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> ASK { ex:c11 ex:sub* ex:c1 }'
) AS s(j);

-- ─── Invariant C: `*` both-var = identity ∪ p+ ───────────────────
-- Tiny acyclic graph a→b→c in its own graph (nodes {a,b,c}).
-- `?s sub* ?o` = identity {(a,a),(b,b),(c,c)} ∪ p+ {(a,b),(b,c),
-- (a,c)} = 6 pairs exactly. Lock the full ordered set.
SELECT pgrdf.add_graph('http://example.org/tiny');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:a ex:sub ex:b . ex:b ex:sub ex:c .',
  pgrdf.graph_id('http://example.org/tiny')
);

SELECT (s.j->>'s') || '|' || (s.j->>'o') AS tiny_star_pair
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { GRAPH <http://example.org/tiny> { ?s ex:sub* ?o } }
   ORDER BY ?s ?o'
) AS s(j);

SELECT count(*)::bigint AS tiny_star_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { GRAPH <http://example.org/tiny> { ?s ex:sub* ?o } }'
);

-- ─── Invariant D: `*` bound-subject identity even when isolated ──
-- <lone> is never seeded anywhere. W3C §9.3: a bound term's
-- zero-length path to itself holds UNCONDITIONALLY.
SELECT (s.j->>'_ask') AS lone_star_self
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> ASK { ex:lone ex:sub* ex:lone }'
) AS s(j);
-- `<lone> sub* ?o` → just <lone> (1 row — only the zero-length pair).
SELECT (s.j->>'o') AS lone_star_o
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:lone ex:sub* ?o } ORDER BY ?o'
) AS s(j);
SELECT count(*)::bigint AS lone_star_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:lone ex:sub* ?o }'
);

-- ─── Invariant E: `?` direct ∪ identity ──────────────────────────
-- Use the tiny graph (a→b→c). `<a> sub? ?o` = identity {a} ∪ direct
-- {b} = {a, b}, 2 rows.
SELECT (s.j->>'o') AS a_opt_o
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/tiny> { ex:a ex:sub? ?o } }
   ORDER BY ?o'
) AS s(j);
SELECT count(*)::bigint AS a_opt_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/tiny> { ex:a ex:sub? ?o } }'
);
-- <a> sub? <a> → true (identity); <a> sub? <b> → true (direct);
-- <a> sub? <c> → false (no direct a→c edge, c != a).
SELECT (s.j->>'_ask') AS a_opt_a FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   ASK { GRAPH <http://example.org/tiny> { ex:a ex:sub? ex:a } }'
) AS s(j);
SELECT (s.j->>'_ask') AS a_opt_b FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   ASK { GRAPH <http://example.org/tiny> { ex:a ex:sub? ex:b } }'
) AS s(j);
SELECT (s.j->>'_ask') AS a_opt_c FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   ASK { GRAPH <http://example.org/tiny> { ex:a ex:sub? ex:c } }'
) AS s(j);
-- `?s sub? ?o` over {a,b,c} = identity {(a,a),(b,b),(c,c)} ∪ direct
-- {(a,b),(b,c)} = 5 pairs exactly.
SELECT (s.j->>'s') || '|' || (s.j->>'o') AS tiny_opt_pair
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { GRAPH <http://example.org/tiny> { ?s ex:sub? ?o } }
   ORDER BY ?s ?o'
) AS s(j);
SELECT count(*)::bigint AS tiny_opt_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { GRAPH <http://example.org/tiny> { ?s ex:sub? ?o } }'
);

-- ─── Invariant F: `*` is cycle-safe ──────────────────────────────
-- 3-cycle a→b→c→a in its own graph. `<a> rel* ?o` terminates;
-- reflexive a + the cycle ⇒ {a,b,c}, 3 rows. The CYCLE clause stops
-- a revisited pair BEFORE the depth cap so a cycle is NOT counted as
-- a truncation (stat stays 0). Reset the stat first to assert it.
SELECT pgrdf.add_graph('http://example.org/cyc');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:a ex:rel ex:b . ex:b ex:rel ex:c . ex:c ex:rel ex:a .',
  pgrdf.graph_id('http://example.org/cyc')
);
SELECT pgrdf.shmem_reset();
SELECT (s.j->>'o') AS cyc_star_reached
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/cyc> { ex:a ex:rel* ?o } }
   ORDER BY ?o'
) AS s(j);
SELECT count(*)::bigint AS cyc_star_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/cyc> { ex:a ex:rel* ?o } }'
);
SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint AS cyc_trunc_zero;

-- ─── Invariant G: `*` depth-guard ────────────────────────────────
-- First: an under-cap `*` traversal leaves the counter at 0.
SELECT pgrdf.shmem_reset();
SELECT count(*)::bigint AS star_under_cap_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:c1 ex:sub* ?o }'
);
SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint AS star_trunc_under_cap;
-- Cap depth at 3 over the length-10 chain. `<c1> sub* ?o` = c1
-- (reflexive, zero-length — NOT subject to the depth cap) + c2,c3,c4
-- (transitive depths 1,2,3) = 4 rows. The depth-3 row (c1→c4) has a
-- continuable ex:sub edge so the guard truncates ⇒ stat > 0.
SET pgrdf.path_max_depth = 3;
SELECT (s.j->>'o') AS star_bounded
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:c1 ex:sub* ?o } ORDER BY ?o'
) AS s(j);
SELECT count(*)::bigint AS star_bounded_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:c1 ex:sub* ?o }'
);
SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint > 0 AS star_truncation_bumped;
SELECT pgrdf.shmem_reset();
SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint AS star_trunc_after_reset;
RESET pgrdf.path_max_depth;

-- ─── Invariant H: `^(p*)` / `(^p)*` inverse composition ──────────
-- Inverse of a reflexive-transitive closure = the same closure over
-- the inverse edge. Assert EQUIVALENCE: the ordered a|b fingerprint
-- of forward `?a sub* ?b` equals that of `?b ^(sub*) ?a` and of
-- `?b (^sub)* ?a` (both re-ordered by ?a ?b so pairs line up).
-- Cardinality note: this query is UNSCOPED, and pgRDF's slice-112
-- semantic is that an unscoped pattern scans ALL graphs (default +
-- named). By this point the named graphs `tiny` (a,b,c via sub) and
-- `cyc` (a,b,c via rel) also exist. So the transitive `sub` part =
-- the length-10 c-chain's 55 pairs + tiny's 3 (a→b,b→c,a→c) = 58,
-- and the W3C §9.3 zero-length node-set = subject∪object over ALL
-- graphs = {c1..c11} ∪ {a,b,c} = 14 reflexive pairs. 58 + 14 = 72.
SELECT
  (SELECT string_agg((j->>'a') || '|' || (j->>'b'), ',')
     FROM pgrdf.sparql(
       'PREFIX ex: <http://example.org/>
        SELECT ?a ?b WHERE { ?a ex:sub* ?b } ORDER BY ?a ?b') AS s(j))
  =
  (SELECT string_agg((j->>'a') || '|' || (j->>'b'), ',')
     FROM pgrdf.sparql(
       'PREFIX ex: <http://example.org/>
        SELECT ?a ?b WHERE { ?b ^(ex:sub*) ?a } ORDER BY ?a ?b') AS s(j))
  AS star_fwd_eq_inv_paren;

SELECT
  (SELECT string_agg((j->>'a') || '|' || (j->>'b'), ',')
     FROM pgrdf.sparql(
       'PREFIX ex: <http://example.org/>
        SELECT ?a ?b WHERE { ?a ex:sub* ?b } ORDER BY ?a ?b') AS s(j))
  =
  (SELECT string_agg((j->>'a') || '|' || (j->>'b'), ',')
     FROM pgrdf.sparql(
       'PREFIX ex: <http://example.org/>
        SELECT ?a ?b WHERE { ?b (^ex:sub)* ?a } ORDER BY ?a ?b') AS s(j))
  AS star_fwd_eq_inv_inner;

-- Absolute cardinality: 55 transitive + 11 reflexive = 66.
SELECT count(*)::bigint AS star_closure_pairs FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?a ?b WHERE { ?a ex:sub* ?b }'
);

-- ─── Invariant I: GRAPH scoping of the zero-length node-set ──────
-- gB carries n1→n2→n3 (nodes {n1,n2,n3}); the default chain
-- (c1..c11) must NOT contribute to gB's identity set.
SELECT pgrdf.add_graph('http://example.org/gB');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:n1 ex:sub ex:n2 . ex:n2 ex:sub ex:n3 .',
  pgrdf.graph_id('http://example.org/gB')
);
-- `GRAPH <gB> { ?s ex:sub* ?o }` = identity {(n1,n1),(n2,n2),
-- (n3,n3)} ∪ p+ {(n1,n2),(n2,n3),(n1,n3)} = 6 pairs. The c-nodes
-- are NOT in the identity set (scoped to gB's nodes).
SELECT count(*)::bigint AS gb_star_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { GRAPH <http://example.org/gB> { ?s ex:sub* ?o } }'
);
SELECT bool_or((s.j->>'s') LIKE '%/c%') AS gb_star_has_cnode
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { GRAPH <http://example.org/gB> { ?s ex:sub* ?o } }'
) AS s(j);
-- `GRAPH ?g { ?s ex:sub* ?o }` — ?g binds only to NAMED graphs (W3C
-- §13.3). The W3C §9.3 zero-length node-set under `GRAPH ?g` is
-- every node of each named graph (predicate-agnostic — it is the
-- graph's term set, not just `sub`-touched nodes). Named graphs:
--   * tiny  (a→b→c via sub): identity {a,b,c}=3 ∪ sub+ {(a,b),
--            (b,c),(a,c)}=3 → 6
--   * gB    (n1→n2→n3 via sub): identity 3 ∪ sub+ 3 → 6
--   * cyc   (a→b→c via REL, no sub): identity {a,b,c}=3 ∪ sub+ 0 → 3
-- Total = 6 + 6 + 3 = 15 rows. The default graph never binds ?g.
SELECT count(*)::bigint AS graphvar_star_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:sub* ?o } }'
);
SELECT bool_or((s.j->>'g') LIKE '%defaultGraph%' OR (s.j->>'g') IS NULL)
       AS graphvar_star_binds_default
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:sub* ?o } }'
) AS s(j);
-- `?` under GRAPH ?g (same predicate-agnostic node-set rule):
--   tiny identity{3}∪direct sub{2}=5; gB identity{3}∪direct{2}=5;
--   cyc identity{3}∪direct sub{0}=3  ⇒ 5+5+3 = 13. No default bind.
SELECT count(*)::bigint AS graphvar_opt_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ex:sub? ?o } }'
);

-- ─── Invariant J: BGP join + pgrdf.construct inheritance ─────────
-- Tag c3 and c7 with a label; `{ ?x ex:sub* ex:c11 . ?x ex:label
-- ?l }` — of c1..c11 (the 11 `*`-reachers of c11) only c3 and c7
-- carry labels ⇒ 2 solutions.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:c3 ex:label "three" . ex:c7 ex:label "seven" .',
  0
);
SELECT (s.j->>'x') || '|' || (s.j->>'l') AS star_joined
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?x ?l WHERE { ?x ex:sub* ex:c11 . ?x ex:label ?l }
   ORDER BY ?x'
) AS s(j);
-- CONSTRUCT inherits `*` via the shared BGP walker. c1..c11 reach
-- c11 (10 transitive + c11 reflexive) ⇒ 11 constructed rows.
SELECT count(*)::bigint AS star_constructed_count FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?x ex:reaches ex:c11 } WHERE { ?x ex:sub* ex:c11 }'
);
-- c11 itself is among the reachers (reflexive) ⇒ the template row
-- (ex:c11 ex:reaches ex:c11) must appear.
SELECT bool_or(
  (c.j->'subject'->>'value')       = 'http://example.org/c11'
  AND (c.j->'predicate'->>'value') = 'http://example.org/reaches'
  AND (c.j->'object'->>'value')    = 'http://example.org/c11'
) AS construct_has_c11_reflexive
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?x ex:reaches ex:c11 } WHERE { ?x ex:sub* ex:c11 }'
) AS c(j);

-- ─── Invariant K: `|` / nested-recursive / negated still panic ───
-- Substring match on the STABLE prefix only (slice tail shifts).
-- Pure top-level `(a|b)` → the gated-stretch E4 message.
SELECT _check_error(
  'alternation-still-E4-gated',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:sub|ex:rel) ?o }')$$,
  $$gated stretch goal (Phase E group E4)$$
);
-- `(p*)+`, `(a|b)*`, `(p1/p2)?` — a recursive/optional op whose
-- inner box is not a plain (optionally inverted) predicate ⇒ the
-- nested-recursive E4 message.
SELECT _check_error(
  'nested-star-plus-E4',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:sub*)+ ?o }')$$,
  $$nested recursive property path$$
);
SELECT _check_error(
  'nested-alternation-star-E4',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:sub|ex:rel)* ?o }')$$,
  $$nested recursive property path$$
);
SELECT _check_error(
  'negated-set-still-out-of-scope',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s !(ex:sub) ?o }')$$,
  $$negated property sets are out of scope for v0.4$$
);

-- ─── sparql_parse analysis: `*` and `?` are now EXECUTABLE ───────
-- E3 makes `?s ex:sub* ?o` / `?s ex:sub? ?o` executable, so they
-- lower into the bgp shape and are NOT flagged unsupported_algebra
-- (parse-time). A pure top-level `(a|b)` (E4) is still flagged.
SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:sub* ?o }'
  )->'unsupported_algebra'
) AS star_unsupported_count;
SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:sub? ?o }'
  )->'unsupported_algebra'
) AS opt_unsupported_count;
SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:sub|ex:rel) ?o }'
  )->'unsupported_algebra'
) AS alt_unsupported_count;

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
