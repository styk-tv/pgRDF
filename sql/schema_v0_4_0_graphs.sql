-- pgRDF schema v0.4 — named-graph IRI mapping.
-- Loaded into the install SQL via `extension_sql_file!` in src/lib.rs.
--
-- Maps the integer `graph_id` (existing LIST partition key on
-- `_pgrdf_quads`, see `sql/schema_v0_2_0.sql`) to the user-visible
-- graph IRI used by SPARQL `GRAPH { … }` and the IRI-keyed UDF
-- overloads landing in subsequent slices. Reference:
-- SPEC.pgRDF.LLD.v0.4 §3.1.
--
-- Schema landed by countdown slice 120; UDF surface lands in slices
-- 118-115; SPARQL `GRAPH { … }` surface lands in slices 111-110.
--
-- The default partition (`graph_id = 0`) carries a synthetic IRI so
-- the catch-all bucket has a queryable name. New graphs allocated
-- via `pgrdf.add_graph(id BIGINT)` get synthetic IRIs of the form
-- `urn:pgrdf:graph:{id}` once slice 117 wires the binding; the
-- explicit IRI surface (`pgrdf.add_graph(iri TEXT)` etc.) lands in
-- slice 118.

CREATE TABLE IF NOT EXISTS _pgrdf_graphs (
    graph_id BIGINT PRIMARY KEY,
    iri      TEXT   NOT NULL UNIQUE
);

INSERT INTO _pgrdf_graphs (graph_id, iri)
     VALUES (0, 'urn:pgrdf:graph:0')
ON CONFLICT (graph_id) DO NOTHING;

-- Register `_pgrdf_graphs` as a user-data table so `pg_dump` includes
-- its rows in the dump (rather than treating it as extension-managed
-- DDL and re-running this install SQL on restore, which would drop
-- every user-bound IRI mapping).
--
-- #152: the filter leaves out the seed row (graph_id 0). A restore runs
-- CREATE EXTENSION first, which inserts that row, and then COPYs the
-- dumped rows; with the seed row in the dump the COPY hit the primary key
-- and aborted, and every graph lost its registry row. The seed row is
-- never dumped because every install creates it.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graphs', 'WHERE graph_id <> 0');

-- #35: `_pgrdf_dictionary` is also an extension member, but was UNregistered —
-- so a plain `pg_dump` skipped its row data and a restore rebuilt quads that
-- point at an EMPTY dictionary (silent corruption: the runtime `add_graph`
-- quad partitions are standalone tables and dump normally, so only the dict
-- was lost). Register it the same way. O(1) — appends the table OID to the
-- extension's `extconfig` array, no data movement / no rewrite of the dict
-- heap. The dict table is created in `schema_v0_2_0` (loaded first via
-- `extension_sql_file!`), so its OID is resolvable here. Round-trip covered by
-- `tests/upgrade/dump-roundtrip.sh`.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_dictionary', '');

-- #152: the default partition is created by this script, so it is an
-- extension member and `pg_dump` skipped its rows: every triple of the
-- default graph (graph_id 0), and of any graph id without its own
-- partition, was missing after a restore. Graph partitions created at run
-- time are ordinary tables and were always dumped.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_quads_default', '');

-- #152: the dictionary's rows were dumped (#35) but its id allocator was
-- not. A restored database handed out dictionary ids from 1 again, so the
-- first load that introduced a new term failed on the primary key
-- (23505). Registering the sequence makes the dump carry its position.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_dictionary_id_seq', '');

-- #150 (0.6.39): graph ids come from a sequence, so an id is never handed
-- out twice. `add_graph(iri)` used to allocate MAX(graph_id) + 1, which gave
-- the id of a dropped highest-numbered graph to the next graph created. Every
-- partition creation advances the sequence past its id (explicitly bound ids
-- included), so the sequence is a high-water mark that never goes down.
-- Registered for pg_dump like `_pgrdf_graphs`, so a restore keeps the mark.
CREATE SEQUENCE IF NOT EXISTS _pgrdf_graph_id_seq MINVALUE 1 START 1;
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graph_id_seq', '');
