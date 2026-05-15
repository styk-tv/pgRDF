-- 70-validate-stub.sql
--
-- Phase 5 v0.4 — pgrdf.validate(data, shapes) → JSONB is now the
-- REAL SHACL Core validator, not the stub it was in v0.3. The file
-- name is retained for diff-friendly history; the body locks in the
-- W3C sh:ValidationReport-shaped JSONB surface. See ERRATA.v0.4
-- E-011 for the upstream unblock that landed this.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- Data graph: one trivially-conforming triple, no shapes target it.
SELECT pgrdf.add_graph(8500);
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.com/> .
ex:a ex:p ex:b .
', 8500);

-- Shapes graph: a NodeShape with a target class that nothing in the
-- data graph instantiates. Vacuously conforming.
SELECT pgrdf.add_graph(8501);
SELECT pgrdf.parse_turtle('
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.com/> .
ex:Shape a sh:NodeShape ;
         sh:targetClass ex:Thing .
', 8501);

-- The real validator returns a W3C-shaped JSONB. Lock in the field set.
SELECT (j->>'conforms')::boolean             = TRUE          AS conforms_true,
       (j->>'data_graph_id')::int            = 8500          AS data_id_echo,
       (j->>'shapes_graph_id')::int          = 8501          AS shapes_id_echo,
       (j->>'data_triples')::int             = 1             AS data_triples_1,
       (j->>'shapes_triples')::int           = 2             AS shapes_triples_2,
       jsonb_typeof(j->'results')            = 'array'       AS results_is_array,
       jsonb_array_length(j->'results')      = 0             AS results_empty,
       jsonb_typeof(j->'elapsed_ms')         = 'number'      AS elapsed_is_number
  FROM (SELECT pgrdf.validate(8500, 8501) AS j) s;

-- Unknown graphs return zero counts; degenerate "no shapes ⇒ no
-- failures ⇒ conforms" report.
SELECT (j->>'conforms')::boolean             = TRUE          AS unknown_conforms,
       (j->>'data_triples')::int             = 0             AS no_data_triples,
       (j->>'shapes_triples')::int           = 0             AS no_shapes_triples
  FROM (SELECT pgrdf.validate(99990, 99991) AS j) s;

-- Cleanup.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
