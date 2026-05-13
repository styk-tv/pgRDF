-- 20-load-turtle — pgrdf.load_turtle on the checked-in 5-triple FOAF
-- fixture. Counts are stable because the fixture is committed.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- Smaller graph id (negative would fail in add_graph); 200 keeps it
-- separate from the 11/12 test ranges and from the 7_001/7_002 pg_test
-- ranges.
SELECT pgrdf.add_graph(200);

SELECT pgrdf.load_turtle('/fixtures/regression/triples-5.ttl', 200) AS n_loaded;
SELECT pgrdf.count_quads(200) AS n_in_graph;

-- The five triples are:
--   ex:alice  rdf:type    foaf:Person
--   ex:alice  foaf:name   "Alice"
--   ex:alice  foaf:mbox   <mailto:alice@example.com>
--   ex:alice  foaf:knows  ex:bob
--   ex:bob    rdf:type    foaf:Person
-- Spot-check the dictionary saw the well-known IRIs as URIs (type=1).
SELECT count(*)::int AS foaf_person_iris
  FROM pgrdf._pgrdf_dictionary
 WHERE term_type = 1 AND lexical_value = 'http://xmlns.com/foaf/0.1/Person';

SELECT count(*)::int AS alice_iris
  FROM pgrdf._pgrdf_dictionary
 WHERE term_type = 1 AND lexical_value = 'http://example.com/alice';

-- And the typed literal "Alice" was stored with a datatype (xsd:string
-- implicitly, since plain strings carry datatype rdf:langString only
-- when they have a lang tag; in oxttl they end up as xsd:string).
SELECT count(*)::int AS literal_alice
  FROM pgrdf._pgrdf_dictionary
 WHERE term_type = 3 AND lexical_value = 'Alice';

ROLLBACK;
