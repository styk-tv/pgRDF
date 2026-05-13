-- 23-blank-nodes — the `[]` Turtle syntax desugars into one blank
-- node and three triples sharing it. Scoped to graph 230 throughout.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

SELECT pgrdf.load_turtle('/fixtures/regression/blank-nodes.ttl', 230) AS n_loaded;
SELECT pgrdf.count_quads(230) AS n_in_graph;

-- Exactly one distinct blank node is referenced anywhere in graph 230.
SELECT count(DISTINCT d.id)::int AS distinct_blank_nodes_in_graph
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary d
    ON d.term_type = 2
   AND (q.subject_id = d.id OR q.object_id = d.id)
 WHERE q.graph_id = 230;

-- That blank node appears in three triples: once as object
-- (ex:alice foaf:knows _:b1), twice as subject (_:b1 foaf:name "Bob",
-- _:b1 foaf:age 30).
SELECT count(*)::int AS triples_touching_blank_node
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary d
    ON d.term_type = 2
   AND (q.subject_id = d.id OR q.object_id = d.id)
 WHERE q.graph_id = 230;

ROLLBACK;
