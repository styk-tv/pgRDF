-- 114-bind-downstream.sql
--
-- Phase F group F2 (slices 30-27, LLD v0.4 §11) — BIND output
-- downstream. The v0.3 limitation was that `BIND(expr AS ?v)` was
-- PROJECTION-ONLY: a textually-later FILTER or BGP that referenced
-- `?v` could not see it. F2 fixes this with an AST substitution
-- pass — every reference to a BIND-introduced variable in a later
-- FILTER, triple slot (join key), or chained BIND is rewritten to
-- the bound expression BEFORE the structural walk, so the existing
-- anchors-driven translator resolves it with no new surface.
-- W3C SPARQL 1.1 §18.2.5: a BIND adds a binding that is in scope for
-- everything textually after it in the group.
--
-- Invariants (all expected values hand-computed; never ACCEPT=1):
--
--   A. BIND then FILTER on the bound var — FILTER(?s > 10) where
--      ?s = ?a + ?b.
--   B. BIND then the bound var used as a join key in a later triple
--      pattern.
--   C. Chained BIND — ?c derived from ?b derived from ?a.
--   D. BIND referencing an unbound var → that solution has the BIND
--      var unbound (NULL), not an error (W3C §18.2.5).
--   E. BIND projection-only still works (no regression of v0.3).
--   F. BIND composed with F1 OPTIONAL (bound only when the optional
--      matched) and under GRAPH scoping.
--   G. pgrdf.construct + SPARQL UPDATE WHERE inherit downstream BIND.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Default-graph fixture (id 0). Numeric objects are xsd:integer.
--   a ex:x 3 ; ex:y 8 ; ex:size "M"
--   b ex:x 5 ; ex:y 5
--   c ex:x 1 ; ex:y 2
--   d ex:x 9            (no ex:y → for the unbound-var case)
-- Plus a join-target dataset: ex:key3 ex:label "three" so a BIND
-- that produces ex:key3 (case B) can join into a later triple.
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:a ex:x 3 ; ex:y 8 ; ex:size "M" .
     ex:b ex:x 5 ; ex:y 5 .
     ex:c ex:x 1 ; ex:y 2 .
     ex:d ex:x 9 .
     ex:key3 ex:label "three" .',
    0);
  -- Named graph for the F (GRAPH-scoped) + G (UPDATE) cases —
  -- distinct subjects so a scoped query is unambiguous.
  PERFORM pgrdf.add_graph('http://example.com/g/b');
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:ga ex:x 4 ; ex:y 6 .
     ex:gb ex:x 7 ; ex:y 1 .',
    pgrdf.graph_id('http://example.com/g/b'));
END $$;

-- A. BIND then FILTER on the bound var. ?sum = ?x + ?y:
--   a: 3+8=11, b: 5+5=10, c: 1+2=3, d: ?y unbound → ?sum unbound.
-- FILTER(?sum > 10) keeps only a (11). = 1 row.
SELECT count(*)::int AS a_bind_filter_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?sum
       WHERE { ?s ex:x ?x . ?s ex:y ?y
               BIND(?x + ?y AS ?sum)
               FILTER(?sum > 10) }'
  );
SELECT (sparql->>'sum')::text AS a_bind_filter_sum
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?sum
       WHERE { ?s ex:x ?x . ?s ex:y ?y
               BIND(?x + ?y AS ?sum)
               FILTER(?sum > 10) }'
  ) AS sparql;

-- A2. FILTER on a BIND var with a string function downstream.
-- ?up = UCASE(?sz); only a has ex:size ("M" → "M"). FILTER(?up = "M")
-- keeps the 1 row.
SELECT count(*)::int AS a2_bind_filter_str
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?up
       WHERE { ?s ex:size ?sz
               BIND(UCASE(?sz) AS ?up)
               FILTER(?up = "M") }'
  );

-- B. BIND then the bound var as a join key in a later triple.
-- BIND(ex:key3 AS ?k) then ?k ex:label ?lab — ?k is an IRI alias
-- substituted into the triple subject; ex:key3 ex:label "three"
-- exists → the right side yields exactly 1 binding (?lab="three").
-- No shared variable with `?s ex:x ?o` → cross product. Unscoped
-- BGP scans ALL graphs (slice-112): ex:x subjects = a,b,c,d
-- (default) + ga,gb (named g/b) = 6. 6 × 1 = 6 rows, lab "three".
SELECT count(*)::int AS b_bind_join_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?lab
       WHERE { ?s ex:x ?o
               BIND(ex:key3 AS ?k)
               ?k ex:label ?lab }'
  );
SELECT DISTINCT (sparql->>'lab')::text AS b_bind_join_label
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?lab
       WHERE { ?s ex:x ?o
               BIND(ex:key3 AS ?k)
               ?k ex:label ?lab }'
  ) AS sparql;

-- B2. Variable-alias BIND as a join key: BIND(?o AS ?j) substitutes
-- ?j → ?o, so `?s2 ex:y ?j` becomes `?s2 ex:y ?o` — a join on the
-- shared ?o between `?s ex:x ?o` and `?s2 ex:y ?o`. Unscoped BGP
-- scans ALL graphs (slice-112). ex:x objects: a=3 b=5 c=1 d=9
-- ga=4 gb=7. ex:y objects: a=8 b=5 c=2 ga=6 gb=1. Pairs where
-- ?s.x == ?s2.y:
--   ?o=5 (s=b) ↔ ?s2 with ex:y=5 → b   → 1 pair
--   ?o=1 (s=c) ↔ ?s2 with ex:y=1 → gb  → 1 pair
-- (3,9,4,7 have no matching ex:y) = 2 rows.
SELECT count(*)::int AS b2_var_join_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?s2
       WHERE { ?s ex:x ?o
               BIND(?o AS ?j)
               ?s2 ex:y ?j }'
  );

-- C. Chained BIND — ?b2 = ?x + 1, ?c = ?b2 * 2. For a (x=3):
-- b2=4, c=8. Project ?c, ORDER BY ?s LIMIT 1 → a's ?c = 8.
SELECT (sparql->>'c')::text AS c_chained_bind
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?c
       WHERE { ?s ex:x ?x
               BIND(?x + 1 AS ?b2)
               BIND(?b2 * 2 AS ?c) }
     ORDER BY ?s LIMIT 1'
  ) AS sparql;
-- C2. Chained BIND feeding a FILTER. Same chain; FILTER(?c >= 16)
-- → c = (x+1)*2 >= 16 → x >= 7. ex:x across all graphs: 3,5,1,9,4,7.
-- x in {9,7} → 2 rows (d:9, gb:7).
SELECT count(*)::int AS c2_chained_filter
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?c
       WHERE { ?s ex:x ?x
               BIND(?x + 1 AS ?b2)
               BIND(?b2 * 2 AS ?c)
               FILTER(?c >= 16) }'
  );

-- D. BIND referencing an unbound var → unbound (NULL), not error.
-- ?missing is never bound by any triple; BIND(?missing AS ?z)
-- yields NULL for every row. The query MUST succeed (NOT raise).
-- Restrict ?s to subjects with ex:size so the row set is
-- deterministic (only a); ?z is NULL.
SELECT (sparql->'z' = 'null'::jsonb) AS d_unbound_is_null
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?z
       WHERE { ?s ex:size ?sz
               BIND(?missing AS ?z) }'
  ) AS sparql;

-- E. BIND projection-only still works (no v0.3 regression). The
-- bound value appears in the projection exactly as before; no
-- downstream consumer. ?dbl = ?x * 2 for a (x=3) → 6.
SELECT (sparql->>'dbl')::text AS e_projection_only
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?dbl
       WHERE { ?s ex:x ?x BIND(?x * 2 AS ?dbl) }
     ORDER BY ?s LIMIT 1'
  ) AS sparql;

-- F. BIND over an F1 OPTIONAL var, then FILTER. OPTIONAL binds ?sz
-- only for a (ex:size "M"). BIND(?sz AS ?tag); FILTER(BOUND(?tag) =
-- ... ) — instead test the spec-correct outcome directly: ?tag is
-- bound (= "M") only for a, NULL for b/c/d. Count rows where ?tag is
-- non-null. Restrict ?s to {a,b} via VALUES (F1) so the answer is
-- exactly 1 (a).
SELECT count(*)::int AS f_bind_over_optional
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?tag
       WHERE { ?s ex:x ?x
               OPTIONAL { ?s ex:size ?sz }
               BIND(?sz AS ?tag)
               VALUES (?s) { (ex:a) (ex:b) } }'
  ) WHERE sparql->'tag' != 'null'::jsonb;

-- F2. Downstream BIND under GRAPH scoping. Scoped to g/b
-- (subjects ga,gb). BIND(?x + ?y AS ?sum); ga:4+6=10, gb:7+1=8.
-- FILTER(?sum > 9) keeps ga only → 1 row, ?sum = 10.
SELECT count(*)::int AS f2_bind_graph_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?sum
       WHERE { GRAPH <http://example.com/g/b> {
                 ?s ex:x ?x . ?s ex:y ?y
                 BIND(?x + ?y AS ?sum)
                 FILTER(?sum > 9) } }'
  );
SELECT (sparql->>'sum')::text AS f2_bind_graph_sum
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s ?sum
       WHERE { GRAPH <http://example.com/g/b> {
                 ?s ex:x ?x . ?s ex:y ?y
                 BIND(?x + ?y AS ?sum)
                 FILTER(?sum > 9) } }'
  ) AS sparql;

-- G. pgrdf.construct inherits downstream BIND in its WHERE. The
-- WHERE's FILTER references a BIND var (?sum = ?x+?y); the template
-- emits only WHERE-bound BGP vars (?s, ?x). The substitution pass
-- runs inside pgrdf.construct's parse_select, so the FILTER gate
-- composes. Unscoped WHERE scans ALL graphs; subjects with both
-- ex:x+ex:y: a(11) b(10) c(3) ga(10) gb(8). FILTER(?sum > 10) →
-- only a → 1 ex:flagged triple.
-- (A BIND var used directly in the CONSTRUCT *template* output
-- position — vs in the WHERE — is a documented v0.5-FUTURE §8 item:
-- the construct emitter projects dict ids, not computed lexical
-- values. F2's construct inheritance is the WHERE-side guarantee.)
SELECT count(*)::int AS g_construct_downstream_bind
  FROM pgrdf.construct(
    'PREFIX ex: <http://example.com/>
     CONSTRUCT { ?s ex:flagged ?x }
       WHERE { ?s ex:x ?x . ?s ex:y ?y
               BIND(?x + ?y AS ?sum)
               FILTER(?sum > 10) }'
  ) AS t(j)
  WHERE j->'predicate'->>'value' = 'http://example.com/flagged';

-- G2. SPARQL UPDATE INSERT ... WHERE inherits downstream BIND.
-- Flag named-graph subjects whose ?x+?y > 9: ga(10) yes, gb(8) no.
-- → 1 flag inserted.
SELECT (j->'_update'->>'form') || '|'
       || (j->'_update'->>'triples_inserted') AS g2_update_summary
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     INSERT { GRAPH <http://example.com/g/b> { ?s ex:hot true } }
       WHERE { GRAPH <http://example.com/g/b> {
                 ?s ex:x ?x . ?s ex:y ?y
                 BIND(?x + ?y AS ?sum)
                 FILTER(?sum > 9) } }'
  ) AS j;
SELECT count(*)::int AS g2_update_flags
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?s WHERE { GRAPH <http://example.com/g/b> {
       ?s <http://example.com/hot> true } }'
  );

-- O (shared with 115). LLD §11 acceptance: pgrdf.sparql_parse no
-- longer flags BIND-downstream in `unsupported_algebra` (it never
-- had a dedicated tag — the parser always walked through Extend —
-- but lock the empty array so a future regression that re-adds a
-- BIND-downstream flag is caught). DESCRIBE now ships (Phase F
-- group F3, LLD §11): form="DESCRIBE", NOT flagged unsupported.
-- OPTIONAL/VALUES still unflagged (F1 not regressed).
SELECT pgrdf.sparql_parse(
  'PREFIX ex: <http://example.com/>
   SELECT ?s ?sum WHERE { ?s ex:x ?x . ?s ex:y ?y
     BIND(?x + ?y AS ?sum) FILTER(?sum > 10) }'
)->'unsupported_algebra' AS o_bind_downstream_unsupported;
SELECT pgrdf.sparql_parse('DESCRIBE <http://example.com/a>')->>'form'
  AS o_describe_form;
SELECT pgrdf.sparql_parse('DESCRIBE <http://example.com/a>')
  ->'unsupported_algebra' AS o_describe_unsupported;

ROLLBACK;
