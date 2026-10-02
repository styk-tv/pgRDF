-- 141-path-walk-interrupts.sql — SPEC.pgRDF.v0.6.37 §3.2 (#138): the
-- breadth-first property-path walk is cancellable and memory-bounded.
--
-- A pgrx test cannot pin a statement_timeout cancel (the whole test is one
-- statement), so it lives here. A 20,000-node chain under
-- pgrdf.path_max_depth = 1024 walked UNBOUND is 1,024 expansion levels over
-- ~20,000 starts each — seconds of work — so a 200 ms statement_timeout can
-- only land promptly if the walk polls for interrupts between levels
-- (57014). The pair budget (pgrdf.path_max_pairs) refuses 54000 before the
-- closure exhausts backend memory. Both errors are caught in a plpgsql
-- block so ON_ERROR_STOP keeps the script going.
--
-- Invariants:
--   A. a bounded walk from the head at the default cap (64) returns 64
--      nodes and STATES the cut (WARNING; last_call_stats = 1)
--   B. an unbound walk past pgrdf.path_max_pairs refuses 54000 with the cure
--   C. an unbound walk under a 200 ms statement_timeout cancels (57014)
--   D. afterwards the session is healthy: the bounded walk answers again

CREATE FUNCTION pg_temp.walk_try(q text) RETURNS text LANGUAGE plpgsql AS $$
DECLARE st text; h text;
BEGIN
  EXECUTE q; RETURN 'ok';
EXCEPTION
  -- OTHERS never matches query_canceled (PostgreSQL excludes it), so the
  -- cancel is trapped by name: that IS invariant C.
  WHEN query_canceled THEN RETURN '57014|cancelled';
  WHEN OTHERS THEN
    GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, h = PG_EXCEPTION_HINT;
    RETURN st || '|' || coalesce(h, '');
END $$;

SELECT pgrdf.add_graph('urn:reg:walk:chain') AS g \gset
SELECT pgrdf.parse_turtle(
  (SELECT string_agg(format('<urn:w:n%s> <urn:w:p> <urn:w:n%s> .', i, i + 1), E'\n')
     FROM generate_series(0, 19999) i),
  :g);

-- A. bounded walk, default cap: 64 nodes, cut stated
SELECT count(*) FROM pgrdf.sparql(
  'SELECT ?o WHERE { GRAPH <urn:reg:walk:chain> { <urn:w:n0> <urn:w:p>+ ?o } }');
SELECT pgrdf.last_call_stats()->>'path_depth_truncations';

-- B. the pair budget refuses 54000 before memory does
SET pgrdf.path_max_pairs = 1000;
SELECT pg_temp.walk_try(
  'SELECT count(*) FROM pgrdf.sparql(''SELECT ?s ?o WHERE { GRAPH <urn:reg:walk:chain> { ?s <urn:w:p>+ ?o } }'')');
RESET pgrdf.path_max_pairs;

-- C. a 200 ms statement_timeout cancels the 1,024-level unbound walk
SET pgrdf.path_max_depth = 1024;
SET pgrdf.path_max_pairs = 2000000000;
SET statement_timeout = '200ms';
SELECT pg_temp.walk_try(
  'SELECT count(*) FROM pgrdf.sparql(''SELECT ?s ?o WHERE { GRAPH <urn:reg:walk:chain> { ?s <urn:w:p>+ ?o } }'')');
RESET statement_timeout;
RESET pgrdf.path_max_pairs;
RESET pgrdf.path_max_depth;

-- D. the session is healthy afterwards
SELECT count(*) FROM pgrdf.sparql(
  'SELECT ?o WHERE { GRAPH <urn:reg:walk:chain> { <urn:w:n0> <urn:w:p>+ ?o } }');

SELECT pgrdf.drop_graph(:g, true) > 0;
