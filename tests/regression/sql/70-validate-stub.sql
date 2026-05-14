-- 70-validate-stub.sql
--
-- Phase 5 v0.3 — pgrdf.validate(data, shapes) → JSONB ships as a STUB.
-- See specs/ERRATA.v0.2.md E-009 for the upstream-dep block. This
-- regression locks in the SQL surface so downstream tooling can wire
-- against it today.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- Data graph: one triple.
SELECT pgrdf.add_graph(8500);
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.com/> .
ex:a ex:p ex:b .
', 8500);

-- Shapes graph: two SHACL-shape triples.
SELECT pgrdf.add_graph(8501);
SELECT pgrdf.parse_turtle('
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.com/> .
ex:Shape a sh:NodeShape .
ex:Shape sh:targetClass ex:Thing .
', 8501);

-- The stub returns a fixed-shape JSONB. Lock in the field set.
SELECT (j->>'status')                = 'stub'      AS status_stub,
       (j->>'data_graph_id')::int    = 8500        AS data_id_echo,
       (j->>'shapes_graph_id')::int  = 8501        AS shapes_id_echo,
       (j->>'data_triples')::int     = 1           AS data_triples_1,
       (j->>'shapes_triples')::int   = 2           AS shapes_triples_2,
       (j->'conforms')               = 'null'::jsonb  AS conforms_is_null,
       jsonb_typeof(j->'results')    = 'array'     AS results_is_array,
       j ? 'reason'                                AS has_reason
  FROM (SELECT pgrdf.validate(8500, 8501) AS j) s;

-- Unknown graphs return zero counts; stub is still well-formed.
SELECT (j->>'status')                = 'stub'  AS unknown_still_stub,
       (j->>'data_triples')::int     = 0       AS no_data_triples,
       (j->>'shapes_triples')::int   = 0       AS no_shapes_triples
  FROM (SELECT pgrdf.validate(99990, 99991) AS j) s;

-- Cleanup.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
