-- 41-sparql-expressions — expression richness in FILTER.
-- Covers arithmetic (+, -, *, /, unary -), STRLEN, CONTAINS,
-- STRSTARTS, STRENDS, UCASE, LCASE, LANG, DATATYPE.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture (graph 410): numeric attrs + string/typed/lang literals.
SELECT pgrdf.parse_turtle(
  '@prefix ex:  <http://example.com/> .
   @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
   ex:a ex:price 10 ; ex:tax 2  ; ex:label "Hello"      .
   ex:b ex:price 20 ; ex:tax 4  ; ex:label "World"      .
   ex:c ex:price 50 ; ex:tax 12 ; ex:label "HELLO WORLD".
   ex:d ex:price 100; ex:tax 25 ; ex:label "Bonjour"@fr .
   ex:e ex:price 0  ; ex:tax 0  ; ex:label "42"^^xsd:integer .',
  410
);

-- 1. Arithmetic: ?p + ?t > 25. b → 24 (drops), c → 62, d → 125. 2 rows.
SELECT count(*)::int AS sum_gt_25
  FROM pgrdf.sparql(
    'SELECT ?s WHERE {
       ?s <http://example.com/price> ?p .
       ?s <http://example.com/tax>   ?t
       FILTER(?p + ?t > 25) }'
  );

-- 2. Multiplication: ?p * ?t > 100. a:20, b:80, c:600, d:2500, e:0. 2 rows (c, d).
SELECT count(*)::int AS product_gt_100
  FROM pgrdf.sparql(
    'SELECT ?s WHERE {
       ?s <http://example.com/price> ?p .
       ?s <http://example.com/tax>   ?t
       FILTER(?p * ?t > 100) }'
  );

-- 3. Division: ?p / ?t < 5. a:5(no), b:5(no), c:50/12≈4.17(yes),
-- d:4(yes), e:0/0=NULL(drop). 2 rows.
SELECT count(*)::int AS ratio_lt_5
  FROM pgrdf.sparql(
    'SELECT ?s WHERE {
       ?s <http://example.com/price> ?p .
       ?s <http://example.com/tax>   ?t
       FILTER(?p / ?t < 5) }'
  );

-- 4. Unary minus: -?t > -10 → ?t < 10. a,b,e qualify (2,4,0). 3 rows.
SELECT count(*)::int AS neg_t
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/tax> ?t FILTER(-?t > -10) }'
  );

-- 5. STRLEN: labels of length > 6 → "HELLO WORLD" (11). 1 row.
-- Wait — "Bonjour"@fr is 7 chars too. So 2 rows.
SELECT count(*)::int AS long_labels
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(STRLEN(?l) > 6) }'
  );

-- 6. CONTAINS (case-sensitive): "Hello" appears in "Hello", "HELLO WORLD".
-- Case-sensitive: only literal "Hello" (which contains "Hello"). "HELLO WORLD" doesn't.
SELECT count(*)::int AS contains_Hello
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(CONTAINS(?l, "Hello")) }'
  );

-- 7. CONTAINS(UCASE(?l), "HELLO") — case-insensitive: both "Hello" and "HELLO WORLD".
SELECT count(*)::int AS contains_ucase
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(CONTAINS(UCASE(?l), "HELLO")) }'
  );

-- 8. STRSTARTS: starts with "H" → "Hello", "HELLO WORLD". 2 rows.
SELECT count(*)::int AS starts_H
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(STRSTARTS(?l, "H")) }'
  );

-- 9. STRENDS: ends with "WORLD" → only "HELLO WORLD". 1 row.
SELECT count(*)::int AS ends_WORLD
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(STRENDS(?l, "WORLD")) }'
  );

-- 10. LANG: only "Bonjour"@fr. 1 row.
SELECT count(*)::int AS lang_fr
  FROM pgrdf.sparql(
    'SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(LANG(?l) = "fr") }'
  );

-- 11. DATATYPE: only "42"^^xsd:integer. 1 row.
SELECT count(*)::int AS datatype_int
  FROM pgrdf.sparql(
    'PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
     SELECT ?s WHERE { ?s <http://example.com/label> ?l FILTER(DATATYPE(?l) = xsd:integer) }'
  );

ROLLBACK;
