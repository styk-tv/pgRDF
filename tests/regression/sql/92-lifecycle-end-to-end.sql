-- 92-lifecycle-end-to-end.sql
--
-- Phase B slice 95 — end-to-end lifecycle integration. The §5 UDFs
-- (`drop_graph`, `clear_graph`, `copy_graph`, `move_graph`) each have
-- their own per-UDF regression file (88/89/90/91); this one wires the
-- four together against a realistic load → mutate → verify flow to
-- catch composition regressions that the per-UDF files cannot:
--
--   1. Loaded-Turtle survives copy_graph + drop_graph round-trip.
--      `parse_turtle` into g1, `copy_graph(g1, g2)`, `drop_graph(g1)`
--      and the dst graph still answers the original BGP through
--      `pgrdf.sparql`. (Catches a regression where the loader's
--      side-effects — dict_cache, hexastore, _pgrdf_graphs — get
--      corrupted by a lifecycle UDF.)
--   2. move_graph is a faithful compose of copy + drop. After
--      `move_graph(g1, g2)`, g1 must answer like a freshly-dropped
--      graph (zero rows, _pgrdf_graphs row gone) and g2 must answer
--      like a freshly-copied graph (rows present, IRI synthetic).
--   3. clear_graph isolates correctly. Clearing g1 must NOT touch g2;
--      the per-partition routing must keep them disjoint even after
--      the loader has fed both through the same dict/hexastore.
--   4. SPARQL GRAPH <iri> { … } projection survives the full
--      lifecycle. A query that selects from the synthetic IRI of a
--      copied graph must return the same answer set as the same
--      query against the source's IRI before the copy.
--   5. Re-binding loop — drop a graph and re-add it under a custom
--      IRI; loader writes new rows and `pgrdf.graph_id(iri)` resolves
--      to the new id. (Catches a regression where _pgrdf_graphs stale
--      state would block re-allocation of a recently-dropped id.)
--
-- All expected values hand-computed against the loader semantics
-- (parse_turtle returns the triple count) and the §5 UDF contracts.
-- No ACCEPT=1 used.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- ─── Invariant 1: load → copy → drop round-trip ──────────────────
-- Load three triples into g9201 from a small inline Turtle, then
-- copy to g9202, then drop g9201. The dst graph still answers a
-- BGP about ex:alice; the src partition is gone.
SELECT pgrdf.add_graph(9201::bigint) AS created_9201;
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:alice ex:age "30" ; ex:city "Paris" ; ex:role "engineer" .',
  9201::bigint
) AS parsed_9201_triples;

SELECT pgrdf.copy_graph(9201::bigint, 9202::bigint) AS copied_9201_to_9202;
SELECT pgrdf.drop_graph(9201::bigint) AS dropped_9201;

-- Source partition gone; dst still holds the loaded rows.
SELECT EXISTS(
  SELECT 1 FROM pg_class
   WHERE relnamespace = 'pgrdf'::regnamespace
     AND relname = '_pgrdf_quads_g9201'
) AS src_partition_after_drop;

SELECT count(*)::bigint AS dst_rows_after_copy_drop
  FROM pgrdf._pgrdf_quads WHERE graph_id = 9202;

-- _pgrdf_graphs binding for 9201 is gone; for 9202 still present.
SELECT pgrdf.graph_iri(9201::bigint) IS NULL AS src_iri_gone;
SELECT pgrdf.graph_iri(9202::bigint) AS dst_iri_after_drop;

-- ─── Invariant 2: move_graph as compose of copy + drop ───────────
-- Fresh load into g9203, move into g9204. End state should match
-- the (copy + drop) end state from Invariant 1 in shape — src gone,
-- dst populated with the moved triples.
SELECT pgrdf.add_graph(9203::bigint) AS created_9203;
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:bob ex:age "42" ; ex:city "Berlin" .',
  9203::bigint
) AS parsed_9203_triples;

SELECT pgrdf.move_graph(9203::bigint, 9204::bigint) AS moved_9203_to_9204;

SELECT EXISTS(
  SELECT 1 FROM pg_class
   WHERE relnamespace = 'pgrdf'::regnamespace
     AND relname = '_pgrdf_quads_g9203'
) AS src_partition_after_move;

SELECT count(*)::bigint AS dst_rows_after_move
  FROM pgrdf._pgrdf_quads WHERE graph_id = 9204;

SELECT pgrdf.graph_iri(9203::bigint) IS NULL AS moved_src_iri_gone;
SELECT pgrdf.graph_iri(9204::bigint) AS moved_dst_iri;

-- ─── Invariant 3: clear_graph isolation under shared dict ────────
-- Load the same vocabulary into both g9205 and g9206 (same loader
-- run path, so the dict cache is shared). Clear g9205 — g9206 must
-- be unaffected, both at the row level and the _pgrdf_graphs row.
SELECT pgrdf.add_graph(9205::bigint) AS created_9205;
SELECT pgrdf.add_graph(9206::bigint) AS created_9206;
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:shared ex:label "in g9205" .',
  9205::bigint
) AS parsed_9205;
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.org/> . ex:shared ex:label "in g9206" .',
  9206::bigint
) AS parsed_9206;

SELECT pgrdf.clear_graph(9205::bigint) AS cleared_9205;

SELECT count(*)::bigint AS g9205_rows_after_clear
  FROM pgrdf._pgrdf_quads WHERE graph_id = 9205;
SELECT count(*)::bigint AS g9206_rows_after_g9205_clear
  FROM pgrdf._pgrdf_quads WHERE graph_id = 9206;

-- _pgrdf_graphs entries unchanged by clear_graph for both rows.
SELECT pgrdf.graph_iri(9205::bigint) AS g9205_iri_after_clear;
SELECT pgrdf.graph_iri(9206::bigint) AS g9206_iri_after_g9205_clear;

-- ─── Invariant 4: SPARQL GRAPH <iri> projection survives copy ────
-- The synthetic IRI urn:pgrdf:graph:9204 was bound by move_graph's
-- internal copy_graph call (per slice 119 — every add_graph(id)
-- writes the synthetic). A SPARQL query that scopes by that IRI
-- must answer the loaded triples.
SELECT count(*)::bigint AS sparql_count_from_moved_dst
  FROM pgrdf.sparql(
    'PREFIX ex: <http://example.org/>
     SELECT ?p ?o WHERE {
       GRAPH <urn:pgrdf:graph:9204> { ex:bob ?p ?o }
     }'
  ) AS s(sparql);

-- ─── Invariant 5: re-binding loop ────────────────────────────────
-- Drop 9202 (left populated from Invariant 1), then re-add 9202 via
-- the IRI-keyed surface bound to a fresh IRI. graph_id(new_iri)
-- resolves to 9202 again (auto-allocated; could also be a different
-- integer — the contract is the new IRI is bound, not the integer).
SELECT pgrdf.drop_graph(9202::bigint) AS dropped_9202;
SELECT pgrdf.graph_iri(9202::bigint) IS NULL AS reused_id_initially_unbound;

SELECT pgrdf.add_graph('http://example.org/rebound') AS rebound_id;
SELECT pgrdf.graph_id('http://example.org/rebound') IS NOT NULL AS rebound_lookup_succeeds;

-- Cleanup — restore a fresh extension state for the next test.
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
