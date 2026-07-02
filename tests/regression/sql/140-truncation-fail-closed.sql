-- 140-truncation-fail-closed.sql — #14 fail-closed truncation reporting
--
-- Locks the pgrdf.on_path_truncation policy surface and the carve boundary
-- report: (1) the default 'warn' emits a client-visible WARNING when a
-- property-path walk is cut at pgrdf.path_max_depth (a partial result is
-- never silent); (2) 'error' fails the query with the stable prefix instead
-- of returning a truncated result; (3) 'count' restores the pre-#14 silent
-- counter-only behaviour; (4) the neighbourhood carve reports its
-- unexpanded boundary via NOTICE, with count semantics unchanged
-- (companion to 137/138/139).
--
-- ZERO-FOOTPRINT by construction: the 31+-era tests inherit the
-- lexically-previous test's leftover state into their `SELECT ?s ?p ?o`
-- counts, and this file sorts right before them — so everything here runs
-- inside BEGIN…ROLLBACK (no DROP EXTENSION, no shmem_reset, no residue),
-- and every assertion is scoped to this file's own unique terms/graphs.

SET client_min_messages = NOTICE;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

SELECT pgrdf.add_graph(91400);
SELECT pgrdf.parse_turtle('@prefix ex: <http://pgrdf-140.test/> . ex:c1 ex:sub ex:c2 . ex:c2 ex:sub ex:c3 . ex:c3 ex:sub ex:c4 . ex:c4 ex:sub ex:c5 .', 91400);

-- ── (1) default policy is 'warn': depth-limited rows + a WARNING ──────────
SHOW pgrdf.on_path_truncation;
SET pgrdf.path_max_depth = 2;
SELECT 'warn_rows_2: ' || (count(*) = 2) FROM pgrdf.sparql('PREFIX ex: <http://pgrdf-140.test/> SELECT ?o WHERE { ex:c1 ex:sub+ ?o }');

-- ── (2) 'error' fails closed (trapped: ON_ERROR_STOP would halt the file) ──
SET pgrdf.on_path_truncation = 'error';
CREATE OR REPLACE FUNCTION pg_temp._trunc_raises() RETURNS TEXT LANGUAGE plpgsql AS $$
DECLARE msg TEXT;
BEGIN
  BEGIN
    PERFORM count(*) FROM pgrdf.sparql('PREFIX ex: <http://pgrdf-140.test/> SELECT ?o WHERE { ex:c1 ex:sub+ ?o }');
    RETURN 'error_mode_fails_closed: !!! unexpected success !!!';
  EXCEPTION WHEN OTHERS THEN msg := SQLERRM;
  END;
  IF position('forbids partial results' IN msg) > 0 THEN RETURN 'error_mode_fails_closed: t';
  ELSE RETURN 'error_mode_fails_closed: f (' || left(msg, 80) || ')'; END IF;
END $$;
SELECT pg_temp._trunc_raises();

-- an under-cap walk must NOT error even in 'error' mode (no false positive)
SET pgrdf.path_max_depth = 64;
SELECT 'error_mode_undercap_ok_4: ' || (count(*) = 4) FROM pgrdf.sparql('PREFIX ex: <http://pgrdf-140.test/> SELECT ?o WHERE { ex:c1 ex:sub+ ?o }');

-- ── (3) 'count' is the silent pre-#14 behaviour, opt-in ───────────────────
SET pgrdf.path_max_depth = 2;
SET pgrdf.on_path_truncation = 'count';
SELECT 'count_rows_2: ' || (count(*) = 2) FROM pgrdf.sparql('PREFIX ex: <http://pgrdf-140.test/> SELECT ?o WHERE { ex:c1 ex:sub+ ?o }');
SET pgrdf.path_max_depth = 64;
SET pgrdf.on_path_truncation = 'warn';

-- ── (4) carve boundary report: 1-hop ball from a over a—b—c ───────────────
-- nodes {a,b} extract BOTH edges (b→c rides its subject); boundary = {c},
-- reported via NOTICE. The 2-hop carve closes the component: same count,
-- boundary 0, NO notice.
SELECT pgrdf.add_graph(91410);
SELECT pgrdf.parse_turtle('@prefix ex: <http://pgrdf-140.test/> . ex:a ex:p ex:b . ex:b ex:p ex:c .', 91410);
SELECT 'carve_1hop_2: ' || (pgrdf.carve_graph(91410, ARRAY['http://pgrdf-140.test/a'], 91411, 1) = 2);
SELECT 'carve_2hop_2: ' || (pgrdf.carve_graph(91410, ARRAY['http://pgrdf-140.test/a'], 91412, 2) = 2);

ROLLBACK;
