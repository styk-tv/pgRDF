-- 22-lang-tags — verify language-tagged literals land in the
-- dictionary with distinct language_tag values, and dedup works
-- per (value, lang) pair. Scoped to graph 220 throughout.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

SELECT pgrdf.load_turtle('/fixtures/regression/lang-tags.ttl', 220) AS n_loaded;
SELECT pgrdf.count_quads(220) AS n_in_graph;

-- Four distinct lang-tagged literals are referenced from graph 220.
SELECT count(DISTINCT lit.language_tag)::int AS distinct_langs_in_graph
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary lit ON q.object_id = lit.id
 WHERE q.graph_id = 220 AND lit.language_tag IS NOT NULL;

-- "Hello"@en is interned with the en tag and referenced by graph 220.
SELECT count(*)::int AS hello_en_refs
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary lit ON q.object_id = lit.id
 WHERE q.graph_id = 220
   AND lit.term_type    = 3
   AND lit.lexical_value = 'Hello'
   AND lit.language_tag  = 'en';

-- The untagged "Hello" is a SEPARATE dict entry (NULL language_tag
-- distinct from 'en' under IS-NOT-DISTINCT-FROM dedup).
SELECT count(*)::int AS hello_untagged_refs
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary lit ON q.object_id = lit.id
 WHERE q.graph_id = 220
   AND lit.term_type    = 3
   AND lit.lexical_value = 'Hello'
   AND lit.language_tag IS NULL;

-- Lang-tagged literals carry NO datatype_iri_id (we leave the
-- implicit rdf:langString unwritten per loader.rs object_to_id).
SELECT count(*)::int AS lang_lits_no_dt_in_graph
  FROM pgrdf._pgrdf_quads q
  JOIN pgrdf._pgrdf_dictionary lit ON q.object_id = lit.id
 WHERE q.graph_id = 220
   AND lit.language_tag    IS NOT NULL
   AND lit.datatype_iri_id IS NULL;

ROLLBACK;
