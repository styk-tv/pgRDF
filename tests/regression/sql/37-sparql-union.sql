-- 37-sparql-union — UNION { … } combines branches with UNION ALL.
--
-- Each branch becomes a complete sub-SELECT. Variables not bound
-- by a branch are emitted as NULL::TEXT so every branch's row
-- shape matches. Outer DISTINCT / ORDER BY / LIMIT / OFFSET wrap
-- the union via a derived table.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Fixture (graph 370):
--   alice: foaf:name "Alice", foaf:nick "Ali"
--   bob:   foaf:name "Bob"   (no nick, no mbox)
--   carol: foaf:nick "C"     (no name, but has mbox)
--          + foaf:mbox <mailto:c@x>
--   dave:  foaf:mbox <mailto:d@x>  (no name, no nick)
-- 6 triples total.
SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:nick "Ali" .
   ex:bob   foaf:name "Bob"   .
   ex:carol foaf:nick "C"     ; foaf:mbox <mailto:c@x> .
   ex:dave  foaf:mbox <mailto:d@x> .',
  370
);

-- 1. UNION over same projected var ?n (name OR nick). 4 rows:
-- Alice's name + Alice's nick + Bob's name + Carol's nick.
SELECT count(*)::int AS union_basic
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n WHERE { { ?s foaf:name ?n } UNION { ?s foaf:nick ?n } }'
  );

-- 2. SELECT DISTINCT — Alice doesn't have name == nick (different
-- literals), so all 4 rows survive DISTINCT here.
SELECT count(*)::int AS union_distinct
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT DISTINCT ?s ?n
       WHERE { { ?s foaf:name ?n } UNION { ?s foaf:nick ?n } }'
  );

-- 3. UNION where branches bind DIFFERENT variables. The branch
-- that doesn't bind a var emits NULL::TEXT for it.
--   name branch → alice, bob (2 rows; ?m = NULL)
--   mbox branch → carol, dave (2 rows; ?n = NULL)
-- Total: 4 rows.
SELECT count(*)::int AS union_diff_vars
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?m
       WHERE { { ?s foaf:name ?n } UNION { ?s foaf:mbox ?m } }'
  );

-- 4. Count rows where ?n IS NULL → the mbox branch's 2 rows.
SELECT count(*)::int AS union_n_null
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?m
       WHERE { { ?s foaf:name ?n } UNION { ?s foaf:mbox ?m } }'
  ) WHERE sparql->'n' = 'null'::jsonb;

-- 5. Count rows where ?m IS NULL → the name branch's 2 rows.
SELECT count(*)::int AS union_m_null
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n ?m
       WHERE { { ?s foaf:name ?n } UNION { ?s foaf:mbox ?m } }'
  ) WHERE sparql->'m' = 'null'::jsonb;

-- 6. Three-way chained UNION. name + nick + mbox branches each
-- contribute their own subjects → 1 + 2 + 1 + 2 + 2 = 8 -- nope,
-- recount: name (alice, bob = 2), nick (alice, carol = 2),
-- mbox (carol, dave = 2). Total: 6 rows.
SELECT count(*)::int AS union_three_branches
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?o
       WHERE { { ?s foaf:name ?o }
               UNION
               { ?s foaf:nick ?o }
               UNION
               { ?s foaf:mbox ?o } }'
  );

-- 7. UNION + ORDER BY ?n ASC LIMIT 1 → alphabetically-first
-- among Alice, Ali, Bob, C, mailto:c@x, mailto:d@x is "Ali"
-- (mailto:... starts with lowercase m which sorts after capital
-- letters; "Ali" < "Alice" < "Bob" < "C" < "mailto:...").
SELECT (sparql->>'o')::text AS first_in_union
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?o
       WHERE { { ?s foaf:name ?o }
               UNION
               { ?s foaf:nick ?o }
               UNION
               { ?s foaf:mbox ?o } }
     ORDER BY ?o LIMIT 1'
  ) AS sparql;

-- 8. UNION + LIMIT 2 — first 2 rows from the union.
SELECT count(*)::int AS union_limited
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n WHERE { { ?s foaf:name ?n } UNION { ?s foaf:nick ?n } } LIMIT 2'
  );

-- 9. UNION + FILTER on a branch. The filter is local to its
-- branch — only the right branch keeps rows that match.
SELECT count(*)::int AS union_filtered_branch
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?n
       WHERE { { ?s foaf:name ?n }
               UNION
               { ?s foaf:nick ?n FILTER(?n != "Ali") } }'
  );

ROLLBACK;
