-- 39-sparql-aggregates — COUNT, COUNT(DISTINCT), SUM, AVG, MIN, MAX
-- plus GROUP BY. Aggregate output values come back as JSON strings.
-- SUM/AVG are numeric-aware (non-numeric literals contribute NULL).
-- MIN/MAX are lexicographic on the term's lexical_value.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture (graph 390): 9 triples.
--   alice/bob/carol have foaf:name + foaf:age.
--   dave has foaf:name only.
--   alice/bob also have foaf:mbox.
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:age 30 ; foaf:mbox <mailto:a@x> .
   ex:bob   foaf:name "Bob"   ; foaf:age 25 ; foaf:mbox <mailto:b@x> .
   ex:carol foaf:name "Carol" ; foaf:age 40 .
   ex:dave  foaf:name "Dave"  .',
  390
);

-- 1. COUNT(*) — total solutions for ?s ?p ?o (all 9 triples).
SELECT (sparql->>'n')::text AS count_all
  FROM pgrdf.sparql(
    'SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }'
  ) AS sparql;

-- 2. COUNT(?o) — same as COUNT(*) here since every triple has a
-- bound object.
SELECT (sparql->>'n')::text AS count_o
  FROM pgrdf.sparql(
    'SELECT (COUNT(?o) AS ?n) WHERE { ?s ?p ?o }'
  ) AS sparql;

-- 3. COUNT(DISTINCT ?s) — 4 distinct subjects (alice, bob, carol, dave).
SELECT (sparql->>'n')::text AS count_distinct_s
  FROM pgrdf.sparql(
    'SELECT (COUNT(DISTINCT ?s) AS ?n) WHERE { ?s ?p ?o }'
  ) AS sparql;

-- 4. SUM(?age) — 30 + 25 + 40 = 95. Non-numeric literals (the names)
-- aren't matched by `?s foaf:age ?age` so don't contribute.
SELECT (sparql->>'total')::numeric AS sum_age
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT (SUM(?age) AS ?total) WHERE { ?s foaf:age ?age }'
  ) AS sparql;

-- 5. AVG(?age) — 95 / 3 ≈ 31.67. Compare as numeric to absorb the
-- Postgres NUMERIC formatting.
SELECT round((sparql->>'mean')::numeric, 2) AS avg_age
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT (AVG(?age) AS ?mean) WHERE { ?s foaf:age ?age }'
  ) AS sparql;

-- 6. MIN(?n) — alphabetic min among Alice, Bob, Carol, Dave → "Alice".
SELECT (sparql->>'lo')::text AS min_name
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT (MIN(?n) AS ?lo) WHERE { ?s foaf:name ?n }'
  ) AS sparql;

-- 7. MAX(?n) — alphabetic max → "Dave".
SELECT (sparql->>'hi')::text AS max_name
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT (MAX(?n) AS ?hi) WHERE { ?s foaf:name ?n }'
  ) AS sparql;

-- 8. GROUP BY ?p with COUNT(?o) — number of triples per predicate.
-- Triples per predicate: foaf:name → 4, foaf:age → 3, foaf:mbox → 2.
-- 3 rows.
SELECT count(*)::int AS group_by_predicate
  FROM pgrdf.sparql(
    'SELECT ?p (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?p'
  );

-- 9. Multiple aggregates in one row: COUNT + SUM over the age data.
SELECT (sparql->>'n')::int     AS age_count,
       (sparql->>'total')::int AS age_sum
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT (COUNT(?age) AS ?n) (SUM(?age) AS ?total)
       WHERE { ?s foaf:age ?age }'
  ) AS sparql;

-- 10. Aggregate + ORDER BY on the aggregate output, then LIMIT.
-- Per-predicate counts in descending order; LIMIT 1 → foaf:name (4).
SELECT (sparql->>'p')::text AS top_predicate
  FROM pgrdf.sparql(
    'SELECT ?p (COUNT(?o) AS ?n)
       WHERE { ?s ?p ?o }
     GROUP BY ?p
     ORDER BY DESC(?n) LIMIT 1'
  ) AS sparql;

ROLLBACK;
