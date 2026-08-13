-- 73-filter-union-fail-closed.sql
--
-- #114: group-level constructs alongside a UNION must REFUSE, never
-- silently drop. Before v0.6.30 every UNION assembly path consumed
-- only branch-local state — a group-level FILTER / VALUES / triple /
-- OPTIONAL / MINUS was discarded wholesale and the result set WIDENED
-- with no signal (measured live: a graph-membership filter, a NOT
-- EXISTS, and a plain equality filter all ignored; a downstream
-- security gate ran eleven versions on the widened answers). This
-- case pins the fail-closed contract: (A) the three measured shapes
-- refuse with the stable #114 message; (B) each refusal increments
-- stats().filter_clauses_dropped (shmem survives the abort); (C) the
-- same restriction written branch-locally still APPLIES — the correct
-- form keeps working.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph('urn:g:c73') > 0 AS gid_ok;
SELECT j->'_update'->>'triples_inserted' AS inserted
  FROM pgrdf.sparql('INSERT DATA { GRAPH <urn:g:c73> {
    <urn:t:x> a <urn:t:A> . <urn:t:y> a <urn:t:B> . } }') AS s(j);

-- Baseline: counter starts at zero after shmem_reset.
SELECT (pgrdf.stats()->>'filter_clauses_dropped')::bigint AS ctr_baseline;

-- ─── A1: plain equality group filter over UNION → refuses ─────────
DO $$
BEGIN
  PERFORM * FROM pgrdf.sparql(
    'SELECT ?c WHERE { { ?c a <urn:t:A> } UNION { ?c a <urn:t:B> }
       FILTER(?c = <urn:t:x>) }');
  RAISE EXCEPTION 'a1_widened_silently';
EXCEPTION WHEN OTHERS THEN
  IF SQLERRM LIKE '%pgRDF#114%' AND SQLERRM LIKE '%FILTER%' THEN
    RAISE NOTICE 'a1_group_filter_refuses: OK';
  ELSE
    RAISE;
  END IF;
END $$;

-- ─── A2: ASK + graph-membership filter over UNION → refuses ───────
DO $$
BEGIN
  PERFORM * FROM pgrdf.sparql(
    'ASK WHERE { GRAPH ?g { { ?s a <urn:t:A> } UNION { ?s a <urn:t:B> } }
       FILTER(?g IN (<urn:g:no-such-graph>)) }');
  RAISE EXCEPTION 'a2_widened_silently';
EXCEPTION WHEN OTHERS THEN
  IF SQLERRM LIKE '%pgRDF#114%' AND SQLERRM LIKE '%ASK path%' THEN
    RAISE NOTICE 'a2_ask_graph_filter_refuses: OK';
  ELSE
    RAISE;
  END IF;
END $$;

-- ─── A3: FILTER NOT EXISTS next to UNION → refuses ────────────────
DO $$
BEGIN
  PERFORM * FROM pgrdf.sparql(
    'SELECT ?c WHERE { { ?c a <urn:t:A> } UNION { ?c a <urn:t:B> }
       FILTER NOT EXISTS { ?sh <urn:p:targets> ?c } }');
  RAISE EXCEPTION 'a3_widened_silently';
EXCEPTION WHEN OTHERS THEN
  IF SQLERRM LIKE '%pgRDF#114%' THEN
    RAISE NOTICE 'a3_not_exists_refuses: OK';
  ELSE
    RAISE;
  END IF;
END $$;

-- ─── B: every refusal counted (3 probes → counter = 3) ────────────
SELECT (pgrdf.stats()->>'filter_clauses_dropped')::bigint AS ctr_after;

-- ─── C: branch-local restriction still applies (the correct form) ─
SELECT j->>'c' AS only_x FROM pgrdf.sparql(
  'SELECT ?c WHERE {
     { ?c a <urn:t:A> FILTER(?c = <urn:t:x>) }
     UNION
     { ?c a <urn:t:B> FILTER(?c = <urn:t:x>) } }') AS s(j);

DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
