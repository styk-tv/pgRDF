-- 71-shacl-real.sql
--
-- LLD §9 acceptance — real SHACL validation exercises sh:NodeShape +
-- sh:property + sh:datatype, and surfaces violations on focus nodes
-- whose data is missing required properties. Lands in v0.4 via the
-- shacl 0.3.x + patched-reasonable unblock (ERRATA.v0.4 E-011).

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- Data graph: Alice has a name but lacks the required ex:age. Bob
-- has both.
SELECT pgrdf.add_graph(8971);
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a foaf:Person ;
         foaf:name "Alice" .
ex:bob a foaf:Person ;
       foaf:name "Bob" ;
       ex:age "30"^^xsd:integer .
', 8971);

-- Shapes graph: PersonShape requires both foaf:name and ex:age.
SELECT pgrdf.add_graph(8972);
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass foaf:Person ;
    sh:property [
        sh:path foaf:name ;
        sh:minCount 1 ;
        sh:datatype xsd:string ;
    ] ;
    sh:property [
        sh:path ex:age ;
        sh:minCount 1 ;
        sh:datatype xsd:integer ;
    ] .
', 8972);

-- Validate. Project stable fields. Bob conforms; Alice doesn't.
WITH v AS (SELECT pgrdf.validate(8971, 8972) AS r)
SELECT
    (r->>'conforms')::boolean             AS conforms,
    jsonb_array_length(r->'results') > 0  AS has_results,
    (r->>'data_graph_id')::bigint         AS data_g,
    (r->>'shapes_graph_id')::bigint       AS shapes_g
FROM v;

-- Alice MUST appear as a focus node in the violations list.
SELECT bool_or(res->>'focusNode' = 'http://example.org/alice') AS alice_flagged
FROM jsonb_array_elements((SELECT pgrdf.validate(8971, 8972)->'results')) AS res;

-- Bob MUST NOT appear (he conforms).
SELECT bool_and(res->>'focusNode' <> 'http://example.org/bob') AS bob_not_flagged
FROM jsonb_array_elements((SELECT pgrdf.validate(8971, 8972)->'results')) AS res;

-- Every violation MUST declare a sh:Violation severity (default
-- when shapes don't override it).
SELECT bool_and(res->>'resultSeverity' = 'sh:Violation') AS all_violations
FROM jsonb_array_elements((SELECT pgrdf.validate(8971, 8972)->'results')) AS res;

-- The W3C-shape report carries the required per-result fields. We
-- only assert the focusNode-bearing rows; some engines also emit
-- "general" results without a focus node (none expected here, but
-- the shape check is still informative).
SELECT bool_and(res ? 'focusNode')
   AND bool_and(res ? 'resultSeverity')
   AND bool_and(res ? 'sourceConstraintComponent') AS shape_fields_present
FROM jsonb_array_elements((SELECT pgrdf.validate(8971, 8972)->'results')) AS res;

-- Cleanup.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
