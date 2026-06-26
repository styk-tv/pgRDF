-- 136-staged-multiload-dedup.sql — the staged loader (the DEFAULT path for
-- N-Triples when pgrdf is in shared_preload_libraries) must NOT corrupt a
-- second load into a dictionary already populated by a prior load (issue #8).
--
-- Root cause: the staged DICT phase (src/storage/staged/phases.rs) dedups only
-- WITHIN the staging set and self-assigns fresh ids (row_number() offset from
-- base = MAX(id)); it never checks whether a term ALREADY exists in
-- _pgrdf_dictionary. On a populated dict that re-inserts every term as a
-- byte-identical duplicate row; the RESOLVE join (IS NOT DISTINCT FROM) then
-- multi-matches each of s/p/o, and the quad insert cross-products them — N^3
-- fabricated-but-distinct triples (a 2nd load of an 8-triple file yields
-- 8*2^3 = 64 quads and a doubled dictionary).
--
-- Fix: staged_load_default probes the empty-dict precondition — exactly like
-- its sibling fast-path guards bulk_load_guarded (loader.rs) and
-- streaming_load_guarded (loader.rs) — and falls back to the always-correct
-- combined ingest_dispatch on a populated dict.
--
-- Fixture: multiload-dedup-sample.nt (8 N-Triples: object-URIs, a plain
-- xsd:string literal, @en lang literals, xsd:integer typed literals — so every
-- dict term path is exercised). Two loads of the SAME file into two graphs. A
-- correct loader inserts exactly 8 quads per graph and never fabricates a
-- duplicate dictionary row.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

SELECT pgrdf.add_graph(9001);
SELECT pgrdf.add_graph(9002);

-- ── Load 1 — FRESH dict (the staged fast path is correct here) ───────────
SELECT 'load1_returned_8: ' || (pgrdf.load_turtle('/fixtures/regression/multiload-dedup-sample.nt', 9001) = 8);
SELECT 'g9001_quads_8: ' || (count(*) = 8) FROM pgrdf._pgrdf_quads WHERE graph_id = 9001;

-- ── Load 2 — POPULATED dict, same file, DEFAULT path ─────────────────────
SELECT 'load2_returned_8: ' || (pgrdf.load_turtle('/fixtures/regression/multiload-dedup-sample.nt', 9002) = 8);
-- THE regression: the second load must insert exactly 8 quads, not 8*N^3.
SELECT 'g9002_no_inflation_8: ' || (count(*) = 8) FROM pgrdf._pgrdf_quads WHERE graph_id = 9002;
-- and must NOT fabricate duplicate dictionary rows.
SELECT 'dict_one_row_per_term: ' || (count(*) = count(DISTINCT (term_type, lexical_md5, datatype_iri_id, language_tag))) FROM pgrdf._pgrdf_dictionary;

-- ── Cleanup ──────────────────────────────────────────────────────────────
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
