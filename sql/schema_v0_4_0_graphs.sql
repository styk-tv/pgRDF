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
-- every user-bound IRI mapping). LLD v0.4 §3.1 acceptance criterion
-- "pg_dump round-trips the mapping verbatim" depends on this call.
-- The empty filter string includes all rows. Slice 110 verifies the
-- end-to-end round trip via tests/regression/scripts/pg-dump-roundtrip.sh.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graphs', '');
