-- 108-property-path-inverse.sql
--
-- Phase E group E1 (slices 49-46) — SPARQL property-path foundation +
-- the `^` inverse operator. Opens LLD v0.4 §7. The translator now
-- recognises `GraphPattern::Path` in the single shared WHERE walker
-- (`walk_select_scoped`), so property paths reach SELECT / ASK /
-- CONSTRUCT / INSERT-WHERE / DELETE-WHERE at once. E1 lowers the
-- non-recursive surface to an ordinary triple:
--
--   * bare predicate `p`            → `?s p ?o`
--   * inverse `^p`                  → `?o p ?s`  (LLD v0.4 §7.2/§7.3)
--   * nested `^(^p)`                → `?s p ?o`  (parity fold;
--     `^^` is reserved for typed literals in the W3C grammar)
--
-- `+` (and `^(p+)`) GRADUATED in Phase E group E2 — they now
-- execute (full coverage in 109-property-path-plus.sql); this file
-- only sanity-checks that they no longer panic. `*` / `?` (E3),
-- alternation `|` (E4) and negated property sets (out of scope)
-- remain NOT executable — they panic with a STABLE rollout-preview
-- prefix so downstream tooling can preview the schedule without
-- depending on the slice-number tail.
--
-- Invariants locked by this file:
--
--   A. `^` round-trip equivalence — `?s ^p ?o` ≡ `?o p ?s` over the
--      same graph (the §7.3 acceptance criterion).
--   B. `^` with a constant subject — `<x> ^p ?o` returns the things
--      that point AT <x> via p.
--   C. `^(^p)` double-inverse = the plain predicate `p`.
--   D. `^p` composed with a plain triple pattern in the same BGP.
--   E. `^p` under `GRAPH <iri>` — scoped; default-graph rows excluded.
--   F. `^p` under `GRAPH ?g` — ?g binds to the named graph; default
--      graph never binds ?g (W3C SPARQL 1.1 §13.3, slice-55 lock).
--   G. `pgrdf.construct` inherits `^` (shared BGP walker → CONSTRUCT
--      gets path support for free).
--   H. `pgrdf.path_max_depth` GUC present + bounded (default 64,
--      range 1..1024; out-of-range SET rejected).
--   I. `path_depth_truncations` present in `pgrdf.stats()`, value 0
--      after `pgrdf.shmem_reset()` (no `+` query truncates here;
--      depth-guard accounting is exercised in 109 invariant D).
--   J. `*` / `?` / `|` / negated sets preview-panic with their
--      stable prefixes; `+` and `^(p+)` GRADUATED in E2 (they now
--      execute — full coverage in 109); `*` (E3) stays panicking.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — captures SQLERRM from a wrapped EXECUTE and asserts the
-- expected substring is present. Same shape as 93-update-insert-data.
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

-- Default-graph seed: a small `knows` graph.
--   alice knows bob
--   alice knows carol
--   bob   knows carol
--   dave  knows alice
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:alice ex:knows ex:bob .
   ex:alice ex:knows ex:carol .
   ex:bob   ex:knows ex:carol .
   ex:dave  ex:knows ex:alice .',
  0
);

-- ─── Invariant A: `^` round-trip equivalence (§7.3) ──────────────
-- The forward query `?o ex:knows ?s` and the inverse query
-- `?s ^ex:knows ?o` must return the IDENTICAL solution set. We emit
-- both as ordered `s|o` text and diff them by stacking: 4 forward
-- rows, then 4 inverse rows — the two ordered blocks must match.
SELECT (s.j->>'s') || '|' || (s.j->>'o') AS forward_pair
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { ?o ex:knows ?s } ORDER BY ?s ?o'
) AS s(j);

SELECT (s.j->>'s') || '|' || (s.j->>'o') AS inverse_pair
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { ?s ^ex:knows ?o } ORDER BY ?s ?o'
) AS s(j);

-- Count parity: same cardinality (4 triples ⇒ 4 inverse solutions).
SELECT count(*)::bigint AS inverse_row_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { ?s ^ex:knows ?o }'
);

-- ─── Invariant B: `^` with a constant subject ────────────────────
-- `ex:carol ^ex:knows ?o` ≡ `?o ex:knows ex:carol` — who points at
-- carol via knows? alice and bob. Ordered.
SELECT (s.j->>'o') AS points_at_carol
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { ex:carol ^ex:knows ?o } ORDER BY ?o'
) AS s(j);

-- ─── Invariant C: `^(^p)` double-inverse = plain `p` ─────────────
-- The W3C SPARQL grammar reserves `^^` for typed-literal datatypes,
-- so a double inverse is written with explicit parentheses:
-- `^(^ex:knows)`. spargebra yields `Reverse(Reverse(NamedNode))`;
-- the parity fold collapses it back to the plain predicate, so
-- `?s ^(^ex:knows) ?o` must equal `?s ex:knows ?o` (same set).
SELECT (s.j->>'s') || '|' || (s.j->>'o') AS plain_pair
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { ?s ex:knows ?o } ORDER BY ?s ?o'
) AS s(j);

SELECT (s.j->>'s') || '|' || (s.j->>'o') AS double_inverse_pair
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?s ?o WHERE { ?s ^(^ex:knows) ?o } ORDER BY ?s ?o'
) AS s(j);

-- ─── Invariant D: `^p` composed with a plain triple in one BGP ───
-- A distinct predicate `ex:age` avoids self-join ambiguity. Seed
-- ages so the join target is unambiguous:
--   alice age 30 ; bob age 25 ; carol age 40
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:alice ex:age "30" .
   ex:bob   ex:age "25" .
   ex:carol ex:age "40" .',
  0
);

-- `{ ?person ^ex:knows ?known . ?known ex:age ?age }`:
--   ?person ^knows ?known  ≡  ?known knows ?person
--     (so ?known is a knower, ?person is who they know)
--   ?known ex:age ?age
-- Pairs (?known knows ?person), with ?known's age:
--   alice(30) knows bob   → person=bob,   known=alice, age=30
--   alice(30) knows carol → person=carol, known=alice, age=30
--   bob(25)   knows carol → person=carol, known=bob,   age=25
--   dave(no age) knows alice → dropped (dave has no age triple)
-- Expected (?person|?known|?age), ordered by person, known:
--   bob|alice|30
--   carol|alice|30
--   carol|bob|25
SELECT (s.j->>'person') || '|' || (s.j->>'known') || '|' || (s.j->>'age') AS joined
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?person ?known ?age
   WHERE { ?person ^ex:knows ?known . ?known ex:age ?age }
   ORDER BY ?person ?known'
) AS s(j);

-- ─── Invariant E: `^p` under GRAPH <iri> ─────────────────────────
-- A named graph with its own knows triples; the default-graph rows
-- above must NOT bleed in.
SELECT pgrdf.add_graph('http://example.org/gA');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> .
   ex:p1 ex:knows ex:q1 .
   ex:p2 ex:knows ex:q1 .',
  pgrdf.graph_id('http://example.org/gA')
);

-- `GRAPH <gA> { ex:q1 ^ex:knows ?o }` ≡ who (in gA) points at q1:
-- p1 and p2. The default graph has no `?x knows ex:q1`, so excluded.
SELECT (s.j->>'o') AS in_graph_a
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?o WHERE { GRAPH <http://example.org/gA> { ex:q1 ^ex:knows ?o } }
   ORDER BY ?o'
) AS s(j);

-- ─── Invariant F: `^p` under GRAPH ?g ────────────────────────────
-- `GRAPH ?g { ?s ^ex:knows ?o }` — ?g binds to gA only (the default
-- graph never binds ?g per W3C §13.3 / slice-55). gA has 2 triples ⇒
-- 2 inverse solutions, ?g = the gA IRI for both.
SELECT count(*)::bigint AS graphvar_rows FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ^ex:knows ?o } }'
);

SELECT bool_and((s.j->>'g') = 'http://example.org/gA') AS all_bound_to_ga
FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/>
   SELECT ?g ?s ?o WHERE { GRAPH ?g { ?s ^ex:knows ?o } }'
) AS s(j);

-- ─── Invariant G: pgrdf.construct inherits `^` ───────────────────
-- CONSTRUCT with an explicit template and a `^` WHERE pattern. The
-- shared BGP walker means construct gets path support for free.
-- `CONSTRUCT { ?o ex:knownBy ?s } WHERE { ?s ^ex:knows ?o }`:
--   ?s ^knows ?o ≡ ?o knows ?s, so each emitted triple is
--   (?o ex:knownBy ?s).
--
-- An unscoped BGP in pgRDF scans EVERY partition (the established
-- slice-112 semantic: scope `None` = all graphs, default + named),
-- so the inverse path matches knows triples across the default graph
-- (4) AND graph gA seeded above (2: p1→q1, p2→q1) ⇒ 6 constructed
-- rows. The robust invariant is that `^knows` and forward `knows`
-- yield the SAME cardinality (the path composes identically to a
-- plain triple through the shared walker) — assert equality so the
-- test is insensitive to the absolute number.
SELECT
  (SELECT count(*) FROM pgrdf.construct(
     'PREFIX ex: <http://example.org/>
      CONSTRUCT { ?o ex:knownBy ?s } WHERE { ?s ^ex:knows ?o }'))
  =
  (SELECT count(*) FROM pgrdf.construct(
     'PREFIX ex: <http://example.org/>
      CONSTRUCT { ?o ex:knownBy ?s } WHERE { ?o ex:knows ?s }'))
  AS construct_inverse_matches_forward_cardinality;

SELECT count(*)::bigint AS constructed_count FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?o ex:knownBy ?s } WHERE { ?s ^ex:knows ?o }'
);

-- alice knows bob ⇒ inverse binds ?s=bob,?o=alice ⇒ template row
-- (?o ex:knownBy ?s) = (alice ex:knownBy bob). Assert it appears.
SELECT bool_or(
  (c.j->'subject'->>'value')   = 'http://example.org/alice'
  AND (c.j->'predicate'->>'value') = 'http://example.org/knownBy'
  AND (c.j->'object'->>'value')    = 'http://example.org/bob'
) AS has_alice_knownby_bob
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?o ex:knownBy ?s } WHERE { ?s ^ex:knows ?o }'
) AS c(j);

-- ─── Invariant H: pgrdf.path_max_depth GUC ───────────────────────
-- Registered, default 64, Userset-settable within 1..1024, rejects
-- out-of-range.
SELECT current_setting('pgrdf.path_max_depth') AS default_depth;

SET pgrdf.path_max_depth = 1024;
SELECT current_setting('pgrdf.path_max_depth') AS max_depth;

SET pgrdf.path_max_depth = 1;
SELECT current_setting('pgrdf.path_max_depth') AS min_depth;

RESET pgrdf.path_max_depth;
SELECT current_setting('pgrdf.path_max_depth') AS reset_depth;

-- Out-of-range below the min (0) and above the max (2000) are
-- rejected by the GUC bounds.
SELECT _check_error(
  'guc-below-min-rejected',
  $$SET pgrdf.path_max_depth = 0$$,
  $$pgrdf.path_max_depth$$
);
SELECT _check_error(
  'guc-above-max-rejected',
  $$SET pgrdf.path_max_depth = 2000$$,
  $$pgrdf.path_max_depth$$
);

-- ─── Invariant I: path_depth_truncations in stats ────────────────
-- E1 scaffold: present, 0 after a fresh shmem_reset (E2 increments).
SELECT pgrdf.shmem_reset();
SELECT (pgrdf.stats() ? 'path_depth_truncations') AS key_present;
SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint AS truncations;

-- ─── Invariant J: recursive ops graduated; gated/negated panic ───
-- `+` (E2) and `*`/`?` (E3) are all executable now; only `|` (E4
-- gated) and negated sets (out of scope) still preview-panic.
-- Substring match on the STABLE prefix only for the panic asserts.
SELECT (count(*) > 0) AS one_or_more_executes FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:knows+ ?o }'
);
-- `*` GRADUATED in Phase E group E3 — `?s ex:knows* ?o` no longer
-- preview-panics; it executes (reflexive transitive closure; full
-- coverage lives in 110-property-path-star-opt.sql). It binds at
-- least the reflexive node-set so a positive row count.
SELECT (count(*) > 0) AS zero_or_more_executes FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:knows* ?o }'
);
-- `?` GRADUATED in E3 too — direct ∪ identity, non-recursive.
SELECT (count(*) > 0) AS zero_or_one_executes FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:knows? ?o }'
);
-- E4: `(ex:knows|ex:likes)` EXECUTES — the seed has 4 `ex:knows`
-- edges and zero `ex:likes`, so the alternation (union of the two
-- per-predicate scans) yields exactly 4 solutions.
SELECT count(*)::bigint AS alternation_executes_count FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:knows|ex:likes) ?o }'
);
-- The §7.1-permitted gated remainder still preview-panics: an
-- alternation arm that is itself a sequence (`ex:knows/ex:knows`).
SELECT _check_error(
  'alternation-sequence-arm-gated',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s (ex:knows/ex:knows|ex:likes) ?o }')$$,
  $$nested recursive property path$$
);
SELECT _check_error(
  'negated-set-out-of-scope-panic',
  $$SELECT * FROM pgrdf.sparql('PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s !(ex:knows) ?o }')$$,
  $$negated property sets are out of scope for v0.4$$
);

-- `^(ex:knows+)` (inverse of one-or-more, E2) and `^(ex:knows*)`
-- (inverse of zero-or-more, E3) both execute — the `^` wrapper
-- composes with the recursive operators (inverse of a closure =
-- closure of the inverse). Full coverage: 109 invariant E, 110
-- invariant H.
SELECT (count(*) >= 0) AS reverse_of_plus_executes FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ^(ex:knows+) ?o }'
);
SELECT (count(*) >= 0) AS reverse_of_star_executes FROM pgrdf.sparql(
  'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ^(ex:knows*) ?o }'
);

-- ─── sparql_parse analysis: E1+E2+E3 path NOT flagged unsupported ─
-- `?s ^ex:knows ?o` lowers to a BGP triple — parse reports it in the
-- bgp shape and does NOT flag `unsupported_algebra`. `?s ex:knows+
-- ?o` (E2) and `?s ex:knows* ?o` (E3) are executable too, so they
-- are ALSO not flagged (parse-time analysis mirrors execution).
SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ^ex:knows ?o }'
  )->'unsupported_algebra'
) AS inverse_unsupported_count;

SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:knows+ ?o }'
  )->'unsupported_algebra'
) AS plus_unsupported_count;

SELECT jsonb_array_length(
  pgrdf.sparql_parse(
    'PREFIX ex: <http://example.org/> SELECT ?s ?o WHERE { ?s ex:knows* ?o }'
  )->'unsupported_algebra'
) AS star_unsupported_count;

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
