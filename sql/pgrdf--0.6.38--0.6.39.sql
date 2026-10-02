-- pgRDF 0.6.38 -> 0.6.39 — graph ids are never reused (#150).
--
-- * New sequence _pgrdf_graph_id_seq: add_graph(iri) allocates from it
--   instead of MAX(graph_id) + 1, so the id of a dropped graph is never
--   handed out again. It starts above every id in use, bound in
--   _pgrdf_graphs or present as a partition, and is registered for
--   pg_dump like _pgrdf_graphs so a restore keeps the mark. Existing ids
--   are unchanged.
--
-- Behaviour changes carried by the .so (no DDL):
-- * every partition creation advances the sequence past its id, so an
--   explicitly bound id is never allocated later either.
-- * built on pgrx 0.19.3 (#148).

CREATE SEQUENCE IF NOT EXISTS pgrdf._pgrdf_graph_id_seq MINVALUE 1 START 1;
SELECT pg_catalog.pg_extension_config_dump('pgrdf._pgrdf_graph_id_seq', '');
SELECT pg_catalog.setval('pgrdf._pgrdf_graph_id_seq', GREATEST(m, 1), m >= 1)
  FROM (SELECT GREATEST(
          (SELECT COALESCE(max(graph_id), 0) FROM pgrdf._pgrdf_graphs),
          (SELECT COALESCE(max(substring(c.relname FROM '^_pgrdf_quads_g([0-9]+)$')::bigint), 0)
             FROM pg_catalog.pg_class c
             JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = 'pgrdf' AND c.relname ~ '^_pgrdf_quads_g[0-9]+$')) AS m) s;
