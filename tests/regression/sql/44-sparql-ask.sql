-- 44-sparql-ask — ASK queries yield a single-row JSONB with
-- {"_ask": "true"} or {"_ask": "false"}.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

SELECT pgrdf.parse_turtle(
  '@prefix ex:   <http://example.com/> .
   @prefix foaf: <http://xmlns.com/foaf/0.1/> .
   ex:alice foaf:name "Alice" ; foaf:age 30 .
   ex:bob   foaf:name "Bob"   ; foaf:age 25 .',
  440
);

-- 1. ASK that should match.
SELECT (sparql->>'_ask')::text AS ask_match_true
  FROM pgrdf.sparql('ASK { ?s ?p ?o }') AS sparql;

-- 2. ASK that should NOT match.
SELECT (sparql->>'_ask')::text AS ask_match_false
  FROM pgrdf.sparql(
    'ASK { <http://example.com/missing> <http://example.com/nope> <http://example.com/zzz> }'
  ) AS sparql;

-- 3. ASK with FILTER passing.
SELECT (sparql->>'_ask')::text AS ask_filter_passes
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     ASK { ?s foaf:age ?a FILTER(?a > 20) }'
  ) AS sparql;

-- 4. ASK with FILTER failing.
SELECT (sparql->>'_ask')::text AS ask_filter_fails
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     ASK { ?s foaf:age ?a FILTER(?a > 100) }'
  ) AS sparql;

-- 5. ASK with OPTIONAL (the mandatory BGP must match; optional may
-- or may not). Mandatory matches → true regardless of optional.
SELECT (sparql->>'_ask')::text AS ask_with_optional
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     ASK { ?s foaf:name ?n OPTIONAL { ?s foaf:mbox ?m } }'
  ) AS sparql;

-- 6. ASK with UNION (either branch matching makes it true).
SELECT (sparql->>'_ask')::text AS ask_with_union
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     ASK { { ?s foaf:name ?n } UNION { ?s foaf:mbox ?m } }'
  ) AS sparql;

ROLLBACK;
