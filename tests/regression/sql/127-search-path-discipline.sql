-- 127-search-path-discipline.sql
--
-- TE-5 — `#[search_path(pgrdf, pg_temp)]` on every #[pg_extern]
-- ensures pgrdf UDFs resolve their own catalog references correctly
-- even when the SESSION search_path doesn't include pgrdf, AND
-- defends against schema-shadow attacks where a malicious schema
-- earlier on the session path defines a relation that would shadow
-- one of pgrdf's internal catalogs (e.g. `_pgrdf_dictionary`).
--
-- pgrx 0.16's `#[search_path(...)]` attribute on `#[pg_extern]`
-- emits `SET search_path = pgrdf, pg_temp` in the CREATE FUNCTION
-- DDL; PostgreSQL applies that SET before each function call,
-- overriding whatever the session had. The attribute is now on all
-- 36 production pg_extern functions across src/.
--
-- Threat scenarios this test exercises:
--   T-1  Session search_path = pg_catalog only (no pgrdf). Calling
--        pgrdf.sparql() must still resolve pgrdf._pgrdf_dictionary
--        / pgrdf._pgrdf_quads internally.
--   T-2  Caller in a non-pgrdf user schema (custom search_path
--        prepended). Same expectation as T-1 — the function SET
--        wins.
--   T-3  Adversarial shadow: caller creates `_pgrdf_dictionary` in
--        their own schema and prepends it to search_path. pgrdf
--        functions MUST keep using pgrdf._pgrdf_dictionary
--        (function-level SET overrides session search_path) and
--        NOT crash or return wrong rows from the shadow table.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- ─── Setup: ingest a known triple so we have something to query. ──
DO $$
BEGIN
  PERFORM pgrdf.add_graph('urn:test/te5/search-path');
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .  ex:s1 ex:p ex:o1 .',
    pgrdf.graph_id('urn:test/te5/search-path')
  );
END $$;

-- ─── T-1 — Session path is pg_catalog only ──────────────────────
SET LOCAL search_path = pg_catalog;

-- The UDF call itself must succeed via fully-qualified name.
SELECT (j->>'s') = 'http://example.org/s1' AS t1_unqualified_session_path
FROM pgrdf.sparql($q$
  PREFIX ex: <http://example.org/>
  SELECT ?s WHERE { GRAPH <urn:test/te5/search-path> { ?s ex:p ?o } }
$q$) AS j;

-- Re-issue using `add_graph` to exercise a SECOND UDF under the
-- restrictive path. add_graph is STRICT + returns bigint; idempotent.
SELECT pgrdf.add_graph('urn:test/te5/search-path') > 0 AS t1_add_graph_returns_id;

-- ─── T-2 — Custom caller schema in front ────────────────────────
RESET search_path;
CREATE SCHEMA te5_caller;
SET LOCAL search_path = te5_caller, pg_catalog;

SELECT (j->>'s') = 'http://example.org/s1' AS t2_custom_caller_schema
FROM pgrdf.sparql($q$
  PREFIX ex: <http://example.org/>
  SELECT ?s WHERE { GRAPH <urn:test/te5/search-path> { ?s ex:p ?o } }
$q$) AS j;

-- ─── T-3 — Adversarial shadow of an internal pgrdf catalog ─────
-- Create a shadow `_pgrdf_dictionary` in te5_caller; if pgrdf's
-- internal SELECT picks this up (because session path put
-- te5_caller first), the wrong rows come back. The function-level
-- SET MUST win.
CREATE TABLE te5_caller._pgrdf_dictionary (
  id BIGINT,
  term_type SMALLINT,
  lexical_value TEXT,
  datatype_iri_id BIGINT,
  language_tag TEXT
);
INSERT INTO te5_caller._pgrdf_dictionary (id, term_type, lexical_value)
VALUES (-1, 99, 'POISONED');

-- Same SPARQL — must still return the real `s1`, not the shadow.
SELECT (j->>'s') = 'http://example.org/s1' AS t3_shadow_does_not_poison
FROM pgrdf.sparql($q$
  PREFIX ex: <http://example.org/>
  SELECT ?s WHERE { GRAPH <urn:test/te5/search-path> { ?s ex:p ?o } }
$q$) AS j;

-- And the shadow table itself MUST still be intact (we didn't
-- accidentally write to it via the function call).
SELECT count(*)::int = 1 AS t3_shadow_table_intact
FROM te5_caller._pgrdf_dictionary;

ROLLBACK;
