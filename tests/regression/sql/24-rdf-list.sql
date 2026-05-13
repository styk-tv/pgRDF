-- 24-rdf-list — Turtle list syntax `( 1 2 3 )` desugars to a chain of
-- rdf:first / rdf:rest triples ending in rdf:nil. Seven triples
-- total. All assertions are scoped to graph 240.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

SELECT pgrdf.load_turtle('/fixtures/regression/rdf-list.ttl', 240) AS n_loaded;
SELECT pgrdf.count_quads(240) AS n_in_graph;

-- The 3 rdf:first triples each point at one of 1, 2, 3.
SELECT count(*)::int AS rdf_first_triples
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary p ON q.predicate_id = p.id
 WHERE q.graph_id = 240
   AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#first';

-- 3 rdf:rest triples — two intermediate, one pointing at rdf:nil.
SELECT count(*)::int AS rdf_rest_triples
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary p ON q.predicate_id = p.id
 WHERE q.graph_id = 240
   AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#rest';

-- Exactly one rdf:rest triple has rdf:nil as its object.
SELECT count(*)::int AS rdf_rest_nil_triples
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary p ON q.predicate_id = p.id
  JOIN pgrdf._pgrdf_dictionary o ON q.object_id    = o.id
 WHERE q.graph_id = 240
   AND p.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#rest'
   AND o.lexical_value = 'http://www.w3.org/1999/02/22-rdf-syntax-ns#nil';

-- Three blank nodes appear as list cells (one per list element).
-- Each is the subject of an rdf:first + rdf:rest pair; the rdf:nil
-- terminator only appears as an OBJECT, not as a cell.
SELECT count(DISTINCT b.id)::int AS list_blank_cells
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary b ON b.term_type = 2 AND q.subject_id = b.id
 WHERE q.graph_id = 240;

ROLLBACK;
