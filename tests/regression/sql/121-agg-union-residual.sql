-- 121-agg-union-residual.sql
--
-- Phase G group G2 (slices 15-14, SPEC.pgRDF.LLD.v0.5-FUTURE §8) —
-- the six aggregates-over-UNION residual refinements. v0.4 §11
-- (Phase F group F2) shipped aggregates over UNION via a derived-
-- table refactor but left SIX cases as STABLE PANICS (never wrong
-- answers). v0.5 §8 lifts them. This fixture has one labelled
-- section per case, each with the expected aggregate output hand-
-- computed from SQL + SPARQL 1.1 semantics, and asserts the F2
-- panic is GONE for cases 1–3 (the exact query that used to panic
-- now returns the correct aggregate).
--
--   Case 1  GROUP BY a `GRAPH ?g`-scope-only var across a UNION.
--   Case 2  computed BIND expr as a triple join key.
--   Case 3  BIND var in a CONSTRUCT template output position.
--   Case 4  aggregates over nested UNION-of-UNION (UNION in JOIN).
--   Case 5  HAVING over UNION-derived aggregates, cross-branch ref.
--   Case 6  GROUP_CONCAT(DISTINCT … ; SEPARATOR='…') over UNION.
--
-- All expected values hand-computed; never ACCEPT=1.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Shared default-graph fixture.
--   books: i1 price 10, i2 price 12, i3 price 18  (cat "books")
--   tools: t1 price 5,  t2 price 25               (cat "tools")
--   arithmetic-join fixture: ex:row ex:base 1 ; ex:hit ex:tag 2 ;
--                            ex:miss ex:tag 3   (object-position
--                            join key — a computed numeric value is
--                            an RDF literal, valid only as an object)
--   construct fixture:       ex:o ex:x 3 ; ex:y 4
--   nested-union fixture:    ex:n1 ex:a 1 ; ex:tag "k" ;
--                            ex:n2 ex:b 2 ; ex:tag "k" ; ex:n3 ex:c 3
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> .
     ex:i1 ex:cat "books" ; ex:price 10 .
     ex:i2 ex:cat "books" ; ex:price 12 .
     ex:i3 ex:cat "books" ; ex:price 18 .
     ex:t1 ex:cat "tools" ; ex:price 5 .
     ex:t2 ex:cat "tools" ; ex:price 25 .
     ex:row ex:base 1 .
     ex:hit  ex:tag 2 .
     ex:miss ex:tag 3 .
     ex:o ex:x 3 ; ex:y 4 .
     ex:n1 ex:a 1 ; ex:tag "k" .
     ex:n2 ex:b 2 ; ex:tag "k" .
     ex:n3 ex:c 3 .',
    0);
  -- Two named graphs for case 1.
  PERFORM pgrdf.add_graph('http://example.com/ga');
  PERFORM pgrdf.add_graph('http://example.com/gb');
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> . ex:p1 ex:v 1 . ex:p2 ex:v 2 .',
    pgrdf.graph_id('http://example.com/ga'));
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.com/> . ex:p3 ex:v 3 .',
    pgrdf.graph_id('http://example.com/gb'));
END $$;

-- ─── Case 1 — GROUP BY a GRAPH ?g-scope-only var over UNION ──────
-- The union doubles each branch (same pattern twice): ga has 2
-- quads → 4 rows; gb has 1 → 2 rows. Group key ?g is consistent
-- across branches (no panic, no split groups). 2 groups; ga's
-- count = 4, gb's = 2.
SELECT count(*)::int AS c1_groups
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?g (COUNT(?o) AS ?n) WHERE {
       { GRAPH ?g { ?s ex:v ?o } } UNION { GRAPH ?g { ?s ex:v ?o } } }
     GROUP BY ?g') AS s(sparql);
SELECT (sparql->>'n')::int AS c1_ga_count
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?g (COUNT(?o) AS ?n) WHERE {
       { GRAPH ?g { ?s ex:v ?o } } UNION { GRAPH ?g { ?s ex:v ?o } } }
     GROUP BY ?g') AS s(sparql)
  WHERE sparql->>'g' = 'http://example.com/ga';

-- ─── Case 2 — computed BIND expr as a triple join key ───────────
-- ex:row ex:base 1 → ?a=1, BIND(?a+1 AS ?k)=2; ?x ex:tag ?k
-- correlates ?k (object position — a computed numeric is an RDF
-- literal, valid only as an object) ONLY to ex:hit ex:tag 2 →
-- ?x = ex:hit, exactly 1 row. (F2 left ?k an unconstrained scan
-- matching BOTH ex:hit and ex:miss; v0.5 correlates it.)
SELECT (sparql->>'x')::text AS c2_hit
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x WHERE {
       ex:row ex:base ?a . BIND(?a + 1 AS ?k) . ?x ex:tag ?k }') AS s(sparql);
SELECT count(*)::int AS c2_rows
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?x WHERE {
       ex:row ex:base ?a . BIND(?a + 1 AS ?k) . ?x ex:tag ?k }');

-- ─── Case 3 — BIND var in a CONSTRUCT template output position ──
-- ex:o ex:x 3 ; ex:y 4 → BIND(?x+?y AS ?sum)=7. The constructed
-- object literal value = "7", datatype xsd:integer. (F2 raised
-- "unbound template variable ?sum".)
SELECT (row->'object'->>'value') AS c3_sum
  FROM pgrdf.construct(
    'PREFIX ex: <http://example.com/>
     CONSTRUCT { ?s ex:total ?sum } WHERE {
       ?s ex:x ?x . ?s ex:y ?y . BIND(?x + ?y AS ?sum) }') AS c(row);
SELECT (row->'object'->>'datatype') AS c3_datatype
  FROM pgrdf.construct(
    'PREFIX ex: <http://example.com/>
     CONSTRUCT { ?s ex:total ?sum } WHERE {
       ?s ex:x ?x . ?s ex:y ?y . BIND(?x + ?y AS ?sum) }') AS c(row);

-- ─── Case 4 — aggregates over nested UNION-of-UNION (in JOIN) ────
-- Branch L = { { {?x ex:a ?v} UNION {?x ex:b ?v} } . ?x ex:tag ?t }
--   → n1 (a=1,tag k) + n2 (b=2,tag k) = 2 rows.
-- Branch R = { ?x ex:c ?v } → n3 = 1 row.
-- COUNT(*) over the whole = 3. (F2 panicked on the inner UNION.)
SELECT (sparql->>'n')::int AS c4_count
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (COUNT(*) AS ?n) WHERE {
       { { { ?x ex:a ?v } UNION { ?x ex:b ?v } } . ?x ex:tag ?t }
       UNION { ?x ex:c ?v } }') AS s(sparql);

-- ─── Case 5 — HAVING over UNION-derived aggregates ──────────────
-- GROUP BY ?c over the union; keep groups with COUNT(?p) > 2.
-- books = 3 (kept), tools = 2 (dropped) → 1 surviving group "books".
SELECT count(*)::int AS c5_surviving
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?c (COUNT(?p) AS ?n) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }
     GROUP BY ?c HAVING(COUNT(?p) > 2)');
SELECT (sparql->>'c')::text AS c5_group
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT ?c (COUNT(?p) AS ?n) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }
     GROUP BY ?c HAVING(COUNT(?p) > 2)') AS s(sparql);

-- ─── Case 6 — GROUP_CONCAT(DISTINCT … ; SEPARATOR='…') / UNION ──
-- Categories across the union: books×3, tools×2 → DISTINCT =
-- {books, tools}. Concat with separator "|" is order-undefined but
-- length-invariant: "books|tools" (or reversed) = 11 chars (NOT
-- the 29-char non-distinct "books|books|books|tools|tools").
SELECT length(sparql->>'g') AS c6_distinct_len
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.com/>
     SELECT (GROUP_CONCAT(DISTINCT ?c ; SEPARATOR = "|") AS ?g) WHERE {
       { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
       UNION { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }')
  AS s(sparql);

-- §8.1 acceptance — sparql_parse must NOT flag these residuals as
-- unsupported (the parser always walked Group/Union; the gate was
-- the executor panic, now lifted). Lock the empty array for the
-- case-1 GRAPH-scope-over-UNION query.
SELECT pgrdf.sparql_parse(
  'PREFIX ex: <http://example.com/>
   SELECT ?g (COUNT(?o) AS ?n) WHERE {
     { GRAPH ?g { ?s ex:v ?o } } UNION { GRAPH ?g { ?s ex:v ?o } } }
   GROUP BY ?g')->'unsupported_algebra' AS acc_unsupported;

ROLLBACK;
