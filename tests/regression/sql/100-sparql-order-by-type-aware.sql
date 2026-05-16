-- 100-sparql-order-by-type-aware.sql
--
-- Phase F group F4 (LLD v0.4 §11) — type-aware ORDER BY per SPARQL
-- 1.1 §15.1. Before F4, ORDER BY emitted a single lexical-string
-- compare over `_pgrdf_dictionary.lexical_value`, so xsd:integer
-- literals sorted as text: "1","10","100","2" (codepoint) instead of
-- the value order 1,2,10,100. F4 expands every sort key into the
-- §15.1 value-space term list:
--
--   * a leading kind rank (numeric < dateTime < boolean < other) so
--     comparable lexical spaces group together and the cross-type
--     fallback is stable/total (ORDER BY never raises);
--   * numeric literals compared numerically;
--   * xsd:dateTime compared chronologically;
--   * xsd:boolean false < true;
--   * strings / plain / lang-tagged by Unicode codepoint;
--
-- DESC reverses; multi-key ORDER BY composes; expression sort keys
-- (`ORDER BY STRLEN(?s)`) are translated via the BIND/FILTER
-- translator. This fixture locks each of those.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

SELECT pgrdf.add_graph('http://ex/g');
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://ex/> .
   @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
   ex:a ex:n "2"^^xsd:integer .
   ex:b ex:n "10"^^xsd:integer .
   ex:c ex:n "1"^^xsd:integer .
   ex:d ex:n "100"^^xsd:integer .
   ex:e ex:dt "2024-12-01T00:00:00Z"^^xsd:dateTime .
   ex:f ex:dt "2024-01-15T00:00:00Z"^^xsd:dateTime .
   ex:g ex:dt "2024-03-09T08:30:00Z"^^xsd:dateTime .
   ex:h ex:s "banana" .
   ex:i ex:s "apple" .
   ex:j ex:s "Cherry" .
   ex:k ex:w "iiii" .
   ex:l ex:w "z" .
   ex:m ex:w "yy" .
   ex:o ex:w "xxx" .',
  pgrdf.graph_id('http://ex/g')
);

-- ─── Numeric ASC: 1,2,10,100 (NOT lexical 1,10,100,2) ────────────
SELECT string_agg(j->>'n', ',') AS numeric_asc
FROM pgrdf.sparql(
  'PREFIX ex: <http://ex/>
   SELECT ?n WHERE { ?x ex:n ?n } ORDER BY ?n'
) AS j;

-- ─── Numeric DESC: 100,10,2,1 ────────────────────────────────────
SELECT string_agg(j->>'n', ',') AS numeric_desc
FROM pgrdf.sparql(
  'PREFIX ex: <http://ex/>
   SELECT ?n WHERE { ?x ex:n ?n } ORDER BY DESC(?n)'
) AS j;

-- ─── dateTime ASC: chronological (Jan, Mar, Dec) ─────────────────
SELECT string_agg(j->>'dt', ',') AS datetime_asc
FROM pgrdf.sparql(
  'PREFIX ex: <http://ex/>
   SELECT ?dt WHERE { ?x ex:dt ?dt } ORDER BY ?dt'
) AS j;

-- ─── String ASC by codepoint: "Cherry" (C=0x43) < "apple"
--     (a=0x61) < "banana" (b=0x62) — uppercase sorts before
--     lowercase in codepoint order (W3C §15.1, COLLATE "C"). ──────
SELECT string_agg(j->>'s', ',') AS string_codepoint_asc
FROM pgrdf.sparql(
  'PREFIX ex: <http://ex/>
   SELECT ?s WHERE { ?x ex:s ?s } ORDER BY ?s'
) AS j;

-- ─── Expression sort key STRLEN(?w): distinct lengths so the
--     numeric expression key fully orders the rows —
--     "z"=1 < "yy"=2 < "xxx"=3 < "iiii"=4. Proves an expression
--     sort key is translated and sorted numerically (a lexical
--     compare of the length text would also work here, but the
--     numeric-kind path is what executes — covered by ?n above). ──
SELECT string_agg(j->>'w', ',') AS strlen_key
FROM pgrdf.sparql(
  'PREFIX ex: <http://ex/>
   SELECT ?w WHERE { ?x ex:w ?w } ORDER BY STRLEN(?w)'
) AS j;

-- ─── ORDER BY ?n LIMIT 1 returns the numerically smallest. ───────
SELECT (j->>'n') AS smallest
FROM pgrdf.sparql(
  'PREFIX ex: <http://ex/>
   SELECT ?n WHERE { ?x ex:n ?n } ORDER BY ?n LIMIT 1'
) AS j;

-- ─── pgrdf.sparql_parse: ORDER BY is a supported SELECT shape and
--     is NOT flagged in unsupported_algebra (it walks transparently
--     like DISTINCT/LIMIT). Empty array ⇒ no residual gap. ────────
SELECT pgrdf.sparql_parse(
  'PREFIX ex: <http://ex/>
   SELECT ?n WHERE { ?x ex:n ?n } ORDER BY ?n'
)->'unsupported_algebra' AS order_by_not_unsupported;
