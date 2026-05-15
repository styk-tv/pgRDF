-- 27-update-insert-data / setup.sql
--
-- Phase C slice 77 — W3C-shape conformance for the §3.1.1 INSERT DATA
-- form. No data.ttl: an INSERT DATA test by definition LOADS the
-- triples, so the harness's default `data.ttl + parse_turtle` path
-- would pre-stage state we're trying to verify the UPDATE itself
-- lands. We pre-allocate the named graph `http://example.org/g1` via
-- pgrdf.add_graph so the INSERT DATA's GRAPH clause has a partition
-- to land into without auto-allocation noise interfering with the
-- assertion shape.

SELECT pgrdf.add_graph('http://example.org/g1');
