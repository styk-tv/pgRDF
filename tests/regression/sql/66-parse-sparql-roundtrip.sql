-- 66-parse-sparql-roundtrip.sql
--
-- Edge-case correctness lock for the END-TO-END round-trip from
-- `pgrdf.parse_turtle` ingest through `pgrdf.sparql` query: every
-- triple the parser saw MUST be observable through the SPARQL
-- executor across all four object-term kinds plus the blank-node
-- subject case.
--
-- Sibling to `61-materialize-then-sparql.sql` (which locks the
-- materialize→sparql edge) — this one locks the parse→sparql edge.
-- Together they pin both ends of the storage layer's visibility
-- contract to the SPARQL surface.
--
-- Coverage shapes:
--   1. IRI subject + IRI predicate + IRI object         (foaf:knows)
--   2. IRI + IRI + plain literal                        (foaf:name)
--   3. IRI + IRI + typed literal (xsd:integer)          (ex:age)
--   4. IRI + IRI + language-tagged literal (@en)        (ex:bio)
--   5. Blank-node subject keyed by sibling property     ([ ... ])
--
-- Each shape projects a single boolean (`bool_and(EXISTS …)`) so the
-- expected output stays `t`-flat across upstream churn in datatype
-- URI normalisation, lang-tag echo policy, or blank-node id format.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

SELECT pgrdf.add_graph(9957);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
ex:alice a foaf:Person ;
         foaf:name "Alice" ;
         foaf:knows ex:bob ;
         ex:age "30"^^xsd:integer ;
         ex:bio "Engineer"@en .
[ a foaf:Person ; foaf:name "Anon" ] .
', 9957);

-- 1. IRI object — `ex:alice foaf:knows ex:bob` round-trips with
-- the bob IRI as the lexical projection of ?o.
SELECT bool_and(EXISTS (
  SELECT 1 FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?o WHERE { <http://example.org/alice> foaf:knows ?o }'
  ) AS sparql
   WHERE sparql->>'o' = 'http://example.org/bob'
)) AS r1_iri_object FROM (VALUES (1)) v(_);

-- 2. Plain literal — `foaf:name "Alice"` round-trips with "Alice"
-- as the lexical projection of ?n.
SELECT bool_and(EXISTS (
  SELECT 1 FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?n WHERE { <http://example.org/alice> foaf:name ?n }'
  ) AS sparql
   WHERE sparql->>'n' = 'Alice'
)) AS r2_plain_literal FROM (VALUES (1)) v(_);

-- 3. Typed literal — `ex:age "30"^^xsd:integer` round-trips with
-- "30" as the lexical projection of ?a. The datatype URI is NOT
-- pinned (the SPARQL projection emits the lexical only); the
-- typed-literal storage contract is locked separately by
-- `21-typed-literals.sql`.
SELECT bool_and(EXISTS (
  SELECT 1 FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/>
     SELECT ?a WHERE { ex:alice ex:age ?a }'
  ) AS sparql
   WHERE sparql->>'a' = '30'
)) AS r3_typed_literal FROM (VALUES (1)) v(_);

-- 4. Language-tagged literal — `ex:bio "Engineer"@en` round-trips
-- with "Engineer" as the lexical projection of ?b. The lang tag is
-- NOT pinned by this slice (the SPARQL projection emits the lexical
-- only); the storage-side lang-tag contract is locked separately by
-- `22-lang-tags.sql`.
SELECT bool_and(EXISTS (
  SELECT 1 FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/>
     SELECT ?b WHERE { ex:alice ex:bio ?b }'
  ) AS sparql
   WHERE sparql->>'b' = 'Engineer'
)) AS r4_lang_literal FROM (VALUES (1)) v(_);

-- 5. Blank-node subject — the anonymous `[ a foaf:Person ;
-- foaf:name "Anon" ]` is keyed via a sibling property pattern:
-- `?s foaf:name "Anon" . ?s foaf:name ?n` resolves ?s through the
-- literal anchor, then projects the same property as ?n. The bnode
-- id format is parser-allocated and NOT pinned; this slice locks
-- that the bnode is *queryable* via a join on sibling property
-- shapes — the most subtle parse→sparql round-trip case.
SELECT bool_and(EXISTS (
  SELECT 1 FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?n WHERE { ?s foaf:name "Anon" . ?s foaf:name ?n }'
  ) AS sparql
   WHERE sparql->>'n' = 'Anon'
)) AS r5_bnode_subject FROM (VALUES (1)) v(_);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
