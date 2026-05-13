-- 21-typed-literals — verify oxttl + loader handle XSD datatypes and
-- that datatype IRIs are interned in the dictionary as URIs.
--
-- All assertions are scoped to graph 210 so the test reads the same
-- regardless of what other smoke loads have committed in the session.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

SELECT pgrdf.load_turtle('/fixtures/regression/typed-literals.ttl', 210) AS n_loaded;
SELECT pgrdf.count_quads(210) AS n_in_graph;

-- Distinct datatype IRIs referenced by THIS graph's quads.
-- Five literals -> 5 distinct datatype IRIs (xsd:string implicit on
-- the bare "hello", plus integer, dateTime, decimal, boolean).
SELECT count(DISTINCT dt.id)::int AS distinct_datatypes_in_graph
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary lit ON q.object_id      = lit.id
  JOIN pgrdf._pgrdf_dictionary dt  ON lit.datatype_iri_id = dt.id
 WHERE q.graph_id = 210;

-- "42"^^xsd:integer is one of those literals, with the right datatype.
SELECT count(*)::int AS forty_two_with_int_datatype
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary lit ON q.object_id = lit.id
  JOIN pgrdf._pgrdf_dictionary dt  ON lit.datatype_iri_id = dt.id
 WHERE q.graph_id = 210
   AND lit.lexical_value = '42'
   AND dt.lexical_value  = 'http://www.w3.org/2001/XMLSchema#integer';

-- Second load into a different graph: literals dedup, quads don't.
SELECT pgrdf.load_turtle('/fixtures/regression/typed-literals.ttl', 211) AS n_loaded2;

-- Number of distinct LITERAL terms across the two graphs (210 + 211)
-- equals the number in just one graph — dedup is at the dict layer.
SELECT count(DISTINCT lit.id)::int AS distinct_lits_two_graphs
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary lit ON q.object_id = lit.id
 WHERE q.graph_id IN (210, 211)
   AND lit.term_type = 3;

ROLLBACK;
