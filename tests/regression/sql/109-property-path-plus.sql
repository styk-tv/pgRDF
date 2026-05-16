-- 109-property-path-plus.sql
--
-- Phase E group E2 (slices 45-42) — SPARQL property-path `+`
-- (one-or-more), the depth guard, and the path.rs carve. Builds on
-- E1's foundation (108-property-path-inverse): the translator now
-- lowers `+` to the LLD v0.4 §7.2 `WITH RECURSIVE walk(src, dst,
-- depth)` CTE (as a derived FROM relation, so it composes with the
-- existing BGP machinery for free), enforces `pgrdf.path_max_depth`
-- as a hard cap (truncate, never error), and bumps
-- `pgrdf.stats().path_depth_truncations` when the cap actually cuts a
-- continuable path. `*` / `?` (E3), `|` (E4), negated sets, and a
-- `+` with a nested-recursive inner stay preview-panicking with
-- their STABLE rollout prefixes.
--
-- Invariants locked by this file (all expected values hand-computed;
-- never ACCEPT=1 baselined):
--
--   A. Chain traversal (§7.3) — a length-10 `sub` chain. `?x sub+
--      <c11>` resolves all 10 ancestors c1..c10; `<c1> sub+ ?y`
--      resolves all 10 descendants c2..c11.
--   B. `+` is NON-reflexive — a node with no outgoing `sub` is not
--      its own ancestor/descendant; c11 (chain tail) is NOT in the
--      ancestor set of itself. (`*` would add the reflexive pair —
--      E3, preview-panics here; see invariant J.)
--   C. Cycle safety — a 3-cycle a→b→c→a. `<a> rel+ ?o` terminates
--      and `UNION` dedups to exactly {a,b,c} (no infinite loop).
--   D. Depth-guard truncation — `SET pgrdf.path_max_depth = 3` over
--      the length-10 chain returns only depths 1..3 (c2,c3,c4) and
--      `path_depth_truncations` > 0 afterwards; `shmem_reset()`
--      returns it to 0; a traversal completing UNDER the cap leaves
--      the counter at 0 (no false truncation).
--   E. `^p+` / `(^p)+` inverse-of-plus = forward `p+` with
--      subject/object swapped (equivalence assertion).
--   F. `p+` under `GRAPH <iri>` — scoped; other graphs excluded.
--   G. `p+` under `GRAPH ?g` — ?g binds to the named graph (W3C
--      SPARQL 1.1 §13.3 / slice-55: the default graph never binds ?g).
--   H. `p+` composed with a plain triple pattern in one BGP (join
--      across the recursive-CTE relation and a normal pattern).
--   I. `pgrdf.construct` inherits `p+` (shared BGP walker).
--   J. `*` / `?` still preview-panic (E3); `(a|b)+` and a nested-
--      recursive inner panic with the E4 prefix; substring locks only.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — captures SQLERRM from a wrapped EXECUTE and asserts the
-- expected substring is present. Same shape as 108 / 93.
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
--   c1 sub c2 . c2 sub c3 . … c10 sub c11   (10 edges, 11 nodes)
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:c1 ex:sub ex:c2 . ex:c2 ex:sub ex:c3 . ex:c3 ex:sub ex:c4 .
   ex:c4 ex:sub ex:c5 . ex:c5 ex:sub ex:c6 . ex:c6 ex:sub ex:c7 .
   ex:c7 ex:sub ex:c8 . ex:c8 ex:sub ex:c9 . ex:c9 ex:sub ex:c10 .
   ex:c10 ex:sub ex:c11 .',
  0
);

-- ─── Invariant A: chain traversal (§7.3) ─────────────────────────
-- `?x ex:sub+ ex:c11` — every transitive ancestor of c11 = c1..c10.
-- 10 rows, ordered. (c11 itself excluded — invariant B, non-reflexive.)
SELECT (s.j->>'x') AS ancestor
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?x WHERE { ?x ex:sub+ ex:c11 } ORDER BY ?x'
) AS s(j);

SELECT count(*)::bigint AS ancestor_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?x WHERE { ?x ex:sub+ ex:c11 }'
);

-- `ex:c1 ex:sub+ ?y` — every transitive descendant of c1 = c2..c11.
-- 10 rows, ordered.
SELECT (s.j->>'y') AS descendant
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?y WHERE { ex:c1 ex:sub+ ?y } ORDER BY ?y'
) AS s(j);

SELECT count(*)::bigint AS descendant_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?y WHERE { ex:c1 ex:sub+ ?y }'
);

-- ─── Invariant B: `+` is non-reflexive ───────────────────────────
-- The chain is acyclic, so NO node is its own transitive successor:
-- `ASK { ex:c1 ex:sub+ ex:c1 }` is false — `+` never yields the
-- reflexive pair on an acyclic graph. (`*` WOULD add it — that
-- lands in E3 and preview-panics; see invariant J. We do NOT
-- exercise `*` here.)
SELECT (s.j->>'_ask') AS c1_reflexive_ask
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   ASK { ex:c1 ex:sub+ ex:c1 }'
) AS s(j);

-- c11 also not in its own ancestor set via the variable form.
SELECT bool_or((s.j->>'x') = 'http://example.org/c11') AS c11_is_own_ancestor
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?x WHERE { ?x ex:sub+ ex:c11 }'
) AS s(j);

-- ─── Invariant C: cycle safety ───────────────────────────────────
-- Seed a 3-cycle in its own graph so it doesn't pollute the chain.
SELECT pgrdf.add_graph('http://example.org/cyc');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:a ex:rel ex:b . ex:b ex:rel ex:c . ex:c ex:rel ex:a .',
  pgrdf.graph_id('http://example.org/cyc')
);

-- `GRAPH <cyc> { ex:a ex:rel+ ?o }` — the walk follows a→b→c→a.
-- `UNION` (not UNION ALL) dedups the revisited pairs so it
-- TERMINATES; every node is reachable from a ⇒ {a,b,c}. 3 rows.
SELECT (s.j->>'o') AS reached
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/cyc> { ex:a ex:rel+ ?o } }
   ORDER BY ?o'
) AS s(j);

SELECT count(*)::bigint AS cycle_reached_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/cyc> { ex:a ex:rel+ ?o } }'
);

-- ─── Invariant D: depth-guard truncation ─────────────────────────
-- First confirm a traversal that COMPLETES under the (default 64)
-- cap leaves the counter at 0 — no false truncation. Reset first.
SELECT pgrdf.shmem_reset();
SELECT count(*)::bigint AS full_chain_descendants FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:c1 ex:sub+ ?o }'
);
SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint AS trunc_under_cap;

-- Now cap depth at 3. The length-10 chain from c1 yields only
-- depths 1..3 → c2 (d1), c3 (d2), c4 (d3). 3 rows; the row at
-- depth==3 (c1→c4) has a continuable ex:sub edge (c4→c5) so the
-- guard truncates and the stat moves off 0.
SET pgrdf.path_max_depth = 3;
SELECT (s.j->>'o') AS bounded
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:c1 ex:sub+ ?o } ORDER BY ?o'
) AS s(j);

SELECT count(*)::bigint AS bounded_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:c1 ex:sub+ ?o }'
);

SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint > 0 AS truncation_bumped;

-- shmem_reset zeroes it back.
SELECT pgrdf.shmem_reset();
SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint AS trunc_after_reset;
RESET pgrdf.path_max_depth;

-- ─── Invariant E: `^p+` / `(^p)+` inverse-of-plus ────────────────
-- `^(ex:sub+)` and `(^ex:sub)+` both = the forward `ex:sub+` with
-- subject/object swapped (inverse of a transitive closure = the
-- transitive closure of the inverse). Rather than hand-listing all
-- 55 transitive pairs of the length-10 chain, assert EQUIVALENCE:
-- the ordered `a|b` fingerprint of the forward query must equal the
-- ordered fingerprint of `?b ^(sub+) ?a` and of `?b (^sub)+ ?a`
-- (both re-ordered by ?a ?b so the same pairs line up). The chain is
-- acyclic so the closure is the strict triangle — 55 pairs.
SELECT
  (SELECT string_agg((j->>'a') || '|' || (j->>'b'), ',')
     FROM pgrdf.sparql(
       'PREFIX ex: <http://example.org/>
        SELECT ?a ?b WHERE { ?a ex:sub+ ?b } ORDER BY ?a ?b') AS s(j))
  =
  (SELECT string_agg((j->>'a') || '|' || (j->>'b'), ',')
     FROM pgrdf.sparql(
       'PREFIX ex: <http://example.org/>
        SELECT ?a ?b WHERE { ?b ^(ex:sub+) ?a } ORDER BY ?a ?b') AS s(j))
  AS fwd_eq_inv_paren;

SELECT
  (SELECT string_agg((j->>'a') || '|' || (j->>'b'), ',')
     FROM pgrdf.sparql(
       'PREFIX ex: <http://example.org/>
        SELECT ?a ?b WHERE { ?a ex:sub+ ?b } ORDER BY ?a ?b') AS s(j))
  =
  (SELECT string_agg((j->>'a') || '|' || (j->>'b'), ',')
     FROM pgrdf.sparql(
       'PREFIX ex: <http://example.org/>
        SELECT ?a ?b WHERE { ?b (^ex:sub)+ ?a } ORDER BY ?a ?b') AS s(j))
  AS fwd_eq_inv_inner;

-- Absolute cardinality of the forward closure: a strict length-10
-- chain has 10+9+…+1 = 55 transitive pairs.
SELECT count(*)::bigint AS forward_closure_pairs FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?a ?b WHERE { ?a ex:sub+ ?b }'
);

-- ─── Invariant F: `p+` under GRAPH <iri> ─────────────────────────
-- A named graph with its own short chain; the default-graph chain
-- (10 edges) must NOT bleed in.
SELECT pgrdf.add_graph('http://example.org/gB');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:n1 ex:sub ex:n2 . ex:n2 ex:sub ex:n3 .',
  pgrdf.graph_id('http://example.org/gB')
);

-- `GRAPH <gB> { ex:n1 ex:sub+ ?o }` = {n2, n3} ONLY. 2 rows.
SELECT (s.j->>'o') AS in_graph_b
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/gB> { ex:n1 ex:sub+ ?o } }
   ORDER BY ?o'
) AS s(j);

-- The gB chain's n1 has no path to the default-graph c-nodes.
SELECT count(*)::bigint AS gb_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/gB> { ex:n1 ex:sub+ ?o } }'
);

-- ─── Invariant G: `p+` under GRAPH ?g ────────────────────────────
-- `GRAPH ?g { ex:n1 ex:sub+ ?o }` — ?g binds to gB (the default
-- graph never binds ?g per W3C §13.3 / slice-55). 2 solutions, ?g =
-- the gB IRI for both.
SELECT count(*)::bigint AS graphvar_rows FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?o WHERE { GRAPH ?g { ex:n1 ex:sub+ ?o } }'
);

SELECT bool_and((s.j->>'g') = 'http://example.org/gB') AS all_bound_to_gb
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?o WHERE { GRAPH ?g { ex:n1 ex:sub+ ?o } }'
) AS s(j);

-- ─── Invariant H: `p+` composed with a plain triple in one BGP ───
-- Tag two chain nodes with a label; join the recursive-CTE relation
-- to a plain `ex:label` pattern. `{ ?x ex:sub+ ex:c11 . ?x ex:label
-- ?l }` — only c3 and c7 carry labels, so 2 solutions.
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:c3 ex:label "three" . ex:c7 ex:label "seven" .',
  0
);

SELECT (s.j->>'x') || '|' || (s.j->>'l') AS joined
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?x ?l WHERE { ?x ex:sub+ ex:c11 . ?x ex:label ?l }
   ORDER BY ?x'
) AS s(j);

-- ─── Invariant I: pgrdf.construct inherits `p+` ──────────────────
-- `CONSTRUCT { ?x ex:ancestorOf ex:c11 } WHERE { ?x ex:sub+ ex:c11 }`
-- — the shared BGP walker means construct gets `+` for free. 10
-- ancestors ⇒ 10 constructed rows.
SELECT count(*)::bigint AS constructed_count FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?x ex:ancestorOf ex:c11 } WHERE { ?x ex:sub+ ex:c11 }'
);

-- c1 is a transitive ancestor of c11 ⇒ the template row
-- (ex:c1 ex:ancestorOf ex:c11) must appear.
SELECT bool_or(
  (c.j->'subject'->>'value')   = 'http://example.org/c1'
  AND (c.j->'predicate'->>'value') = 'http://example.org/ancestorOf'
  AND (c.j->'object'->>'value')    = 'http://example.org/c11'
) AS has_c1_ancestorof_c11
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?x ex:ancestorOf ex:c11 } WHERE { ?x ex:sub+ ex:c11 }'
) AS c(j);

-- ─── Invariant J: `*` / `?` / `|` / nested-recursive panics ──────
-- Substring match on the STABLE prefix only (the slice-number tail
-- shifts as the countdown advances). Note `(a|b)+` / `(p*)+` reach
-- the *nested-recursive* E4 message (the `+` classifier sees a
-- non-predicate inner box) — distinct from a pure top-level `(a|b)`
-- which gets the gated-stretch E4 message.
SELECT _check_error(
  'zero-or-more-still-E3',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:sub* ?o }')$$,
  $$lands in Phase E group E3$$
);
SELECT _check_error(
  'zero-or-one-still-E3',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:sub? ?o }')$$,
  $$lands in Phase E group E3$$
);
-- Pure top-level alternation `(a|b)` → the gated-stretch E4 message.
SELECT _check_error(
  'alternation-still-E4-gated',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:sub|ex:rel) ?o }')$$,
  $$gated stretch goal (Phase E group E4)$$
);
-- A `+` whose inner box is itself recursive/alternation/sequence —
-- `(p*)+`, `(a|b)+`, `(p1/p2)+` — is the nested-recursive E4 case.
SELECT _check_error(
  'nested-star-plus-E4',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:sub*)+ ?o }')$$,
  $$nested recursive property path$$
);
SELECT _check_error(
  'nested-alternation-plus-E4',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:sub|ex:rel)+ ?o }')$$,
  $$nested recursive property path$$
);
SELECT _check_error(
  'negated-set-still-out-of-scope',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s !(ex:sub) ?o }')$$,
  $$negated property sets are out of scope for v0.4$$
);

-- ─── sparql_parse analysis: `+` is now EXECUTABLE (not flagged) ───
-- E2 makes `?s ex:sub+ ?o` executable, so it lowers into the bgp
-- shape and is NOT flagged `unsupported_algebra` (parse-time). `*`
-- (E3) is still flagged.
SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:sub+ ?o }'
  )->'unsupported_algebra'
) AS plus_unsupported_count;

SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:sub* ?o }'
  )->'unsupported_algebra'
) AS star_unsupported_count;

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
