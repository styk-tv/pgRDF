-- 36-sparql-optional — OPTIONAL { … } translates to LEFT JOIN.
-- Today's restriction: each OPTIONAL block holds a single triple
-- pattern. Multiple OPTIONALs chain. OPTIONAL FILTER lands in the
-- LEFT JOIN's ON clause so unmatched rows still survive.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture (graph 360): foaf-shaped data with varying completeness.
--   alice: name "Alice", mbox <mailto:a@x>, age 30
--   bob:   name "Bob"   (no mbox, no age)
--   carol: name "Carol", mbox <mailto:c@x>  (no age)
--   dave:  name "Dave",                       age 17  (under-18)
-- 4 persons. Mandatory part is `?s foaf:name ?n` → 4 rows.
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:mbox <mailto:a@x> ; foaf:age 30 .
   ex:bob   foaf:name "Bob"   .
   ex:carol foaf:name "Carol" ; foaf:mbox <mailto:c@x> .
   ex:dave  foaf:name "Dave"  ; foaf:age 17 .',
  360
);

-- 1. Simple OPTIONAL — LEFT JOIN keeps Bob and Dave too (their
-- ?m is NULL). 4 rows total.
SELECT count(*)::int AS optional_rows
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?m
       WHERE { ?s foaf:name ?n
               OPTIONAL { ?s foaf:mbox ?m } }'
  );

-- 2. Count rows where ?m IS NULL (= LEFT JOIN miss). Bob + Dave.
SELECT count(*)::int AS optional_unbound_m
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?m
       WHERE { ?s foaf:name ?n
               OPTIONAL { ?s foaf:mbox ?m } }'
  ) WHERE sparql->'m' = 'null'::jsonb;

-- 3. Count rows where ?m IS NOT NULL — Alice + Carol.
SELECT count(*)::int AS optional_bound_m
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?m
       WHERE { ?s foaf:name ?n
               OPTIONAL { ?s foaf:mbox ?m } }'
  ) WHERE sparql->'m' != 'null'::jsonb;

-- 4. OPTIONAL with inner FILTER `?age >= 18` — Dave's age is 17, so
-- the filter rejects his row inside the LEFT JOIN, but he still
-- comes back with ?a = NULL. Total rows = 4 (one per person).
SELECT count(*)::int AS optional_inner_filter_rows
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?a
       WHERE { ?s foaf:name ?n
               OPTIONAL { ?s foaf:age ?a FILTER(?a >= 18) } }'
  );

-- 5. Same query — count rows where ?a IS NOT NULL.  Only Alice (30).
SELECT count(*)::int AS optional_filter_bound_a
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?a
       WHERE { ?s foaf:name ?n
               OPTIONAL { ?s foaf:age ?a FILTER(?a >= 18) } }'
  ) WHERE sparql->'a' != 'null'::jsonb;

-- 6. Multiple chained OPTIONALs — each becomes its own LEFT JOIN.
-- 4 persons survive, all have ?n; ?m and ?a vary by person.
SELECT count(*)::int AS chained_optionals
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?m ?a
       WHERE { ?s foaf:name ?n
               OPTIONAL { ?s foaf:mbox ?m }
               OPTIONAL { ?s foaf:age  ?a } }'
  );

-- 7. Outer FILTER(BOUND(?m)) after the OPTIONAL prunes Bob + Dave.
SELECT count(*)::int AS optional_with_outer_bound
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s
       WHERE { ?s foaf:name ?n
               OPTIONAL { ?s foaf:mbox ?m }
               FILTER(BOUND(?m)) }'
  );

-- 8. OPTIONAL + ORDER BY ?n — alphabetical first row's ?s value.
SELECT (sparql->>'s')::text AS first_by_name
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?m
       WHERE { ?s foaf:name ?n
               OPTIONAL { ?s foaf:mbox ?m } }
     ORDER BY ?n LIMIT 1'
  ) AS sparql;

ROLLBACK;
