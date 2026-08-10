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

-- Unknown graphs: the "no shapes ⇒ no failures ⇒ conforms" report this
-- used to lock is the #83 defect — "nothing was checked" and
-- "everything passed" are different facts. #103 (0.6.26) tightened it
-- again: the strict refusal now RAISES, because the in-band
-- conforms:null was fail-open at call sites (NOT(conforms)::bool over
-- null is NULL, which WHERE drops). Pin the raise, then pin the
-- lenient opt-out's zero-count echo shape.
DO $$ BEGIN
  PERFORM pgrdf.validate(99990, 99991);
  PERFORM set_config('regress70.err', 'NO-ERROR', false);
EXCEPTION WHEN OTHERS THEN
  PERFORM set_config('regress70.err', SQLERRM, false);
END $$;
SELECT current_setting('regress70.err', true)
       LIKE 'validate: shapes graph 99991 declares no SHACL target%'
       AS unknown_graphs_strict_raises;

SELECT (j->>'conforms')::boolean             = TRUE          AS unknown_conforms_lenient,
       (j->>'data_triples')::int             = 0             AS no_data_triples,
       (j->>'shapes_triples')::int           = 0             AS no_shapes_triples
  FROM (SELECT pgrdf.validate(99990, 99991, 'native', false) AS j) s;

-- Cleanup.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
