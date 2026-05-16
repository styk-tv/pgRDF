-- 122-shacl-modes.sql
--
-- Phase G group G3 (slices 13-12, SPEC.pgRDF.LLD.v0.5-FUTURE §5) —
-- the SHACL-SPARQL constraint-mode argument + materialised-graph
-- validation. v0.4 §9 shipped `pgrdf.validate(data, shapes) → JSONB`
-- (Native SHACL Core only). v0.5 §5 adds:
--
--   pgrdf.validate(data_graph_id BIGINT, shapes_graph_id BIGINT,
--                  mode TEXT DEFAULT 'native') → JSONB
--
-- with `mode` ∈ {'native','sparql'}; the JSONB gains a `mode` field;
-- an unknown mode errors with prefix `validate: unknown mode`.
--
-- §5.2 / §5.3 acceptance — status per ERRATA.v0.5 E-012:
--
--   `shacl 0.3.1` has NO SHACL-SPARQL constraint component AND its
--   SparqlEngine is an upstream stub (`unimplemented!()` in every
--   target-resolution method — invoking it would panic). So the
--   REALISABLE v0.5 contract, locked here:
--
--     A.  mode field present; default-arg form ⇒ "native"; the v0.4
--         2-arg surface is byte-identical (no regression).
--     B.  unknown mode ⇒ stable `validate: unknown mode` prefix
--         (validated BEFORE any work; no silent fallback).
--     C.  'native' correctly IGNORES a silently-dropped sh:sparql /
--         sh:select block (E-012 Gap 1) while still reporting the
--         Core violation on the same shape.
--     D.  'sparql' returns a DETERMINISTIC structured report
--         (conforms:null + an `error` naming the upstream gap +
--         E-012) — never a panic, never a crash.
--     E.  §5.3 #2 — validation against a `pgrdf.materialize`-d data
--         graph reports violations against ENTAILED triples
--         ('native' mode, the working engine; unaffected by the
--         'sparql' gap). RDFS profile reused from G1.
--
-- All expected values hand-computed; never ACCEPT=1.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

CREATE OR REPLACE FUNCTION _check_error(label TEXT, sql TEXT, expected_fragment TEXT)
RETURNS TEXT AS $fn$
DECLARE
  got TEXT;
BEGIN
  BEGIN
    EXECUTE sql;
    RETURN label || ': NO ERROR (expected ' || expected_fragment || ')';
  EXCEPTION WHEN OTHERS THEN
    got := SQLERRM;
    IF position(expected_fragment IN got) > 0 THEN
      RETURN label || ': OK';
    ELSE
      RETURN label || ': WRONG ERROR <' || got || '>';
    END IF;
  END;
END;
$fn$ LANGUAGE plpgsql;

-- Seed: ex:alice is a foaf:Person with a name but NO ex:age.
-- PersonShape requires ex:age (xsd:integer, minCount 1) — a Core
-- sh:minCount violation — AND carries a sh:sparql/sh:select
-- SPARQLConstraint block that is silently dropped upstream (E-012
-- Gap 1): it must not break the 'native' parse nor change the Core
-- result.
DO $$
BEGIN
  PERFORM pgrdf.add_graph(12201);
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .
     @prefix foaf: <http://xmlns.com/foaf/0.1/> .
     ex:alice a foaf:Person ; foaf:name "Alice" .',
    12201);
  PERFORM pgrdf.add_graph(12202);
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .
     @prefix sh: <http://www.w3.org/ns/shacl#> .
     @prefix foaf: <http://xmlns.com/foaf/0.1/> .
     @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
     ex:PersonShape a sh:NodeShape ;
       sh:targetClass foaf:Person ;
       sh:property [ sh:path ex:age ; sh:minCount 1 ; sh:datatype xsd:integer ] ;
       sh:sparql [ a sh:SPARQLConstraint ;
                   sh:message "needs ex:age (SPARQL)" ;
                   sh:select """SELECT $this WHERE {
                       $this a <http://xmlns.com/foaf/0.1/Person> .
                       FILTER NOT EXISTS { $this <http://example.org/age> ?a } }""" ] .',
    12202);
END $$;

-- ─── A — mode field present; default-arg form ⇒ "native" ────────
-- The 2-arg (v0.4) form defaults mode => 'native'.
SELECT (pgrdf.validate(12201, 12202) ->> 'mode') AS a_default_mode;
-- Explicit 'native' echoes "native".
SELECT (pgrdf.validate(12201, 12202, 'native') ->> 'mode') AS a_native_mode;

-- ─── B — unknown mode ⇒ stable prefix, no side effect ───────────
SELECT _check_error(
  'b_unknown_mode',
  $q$ SELECT pgrdf.validate(12201, 12202, 'endpoint') $q$,
  'validate: unknown mode');

-- ─── C — 'native' ignores the sh:sparql block, still flags Core ──
-- conforms = false (Alice lacks ex:age) and the violation focus
-- node is ex:alice. The sh:select block is a no-op (E-012 Gap 1):
-- it neither breaks the parse nor adds/removes a Core violation.
SELECT (pgrdf.validate(12201, 12202, 'native') ->> 'conforms') AS c_conforms;
SELECT count(*)::int AS c_alice_violation
  FROM jsonb_array_elements(pgrdf.validate(12201, 12202, 'native') -> 'results') r
  WHERE r ->> 'focusNode' = 'http://example.org/alice';

-- ─── D — 'sparql' ⇒ deterministic structured report, no panic ───
-- conforms:null, mode echoed, and an error naming the upstream gap.
SELECT (pgrdf.validate(12201, 12202, 'sparql') -> 'conforms')::text AS d_conforms;
SELECT (pgrdf.validate(12201, 12202, 'sparql') ->> 'mode') AS d_mode;
SELECT (pgrdf.validate(12201, 12202, 'sparql') ->> 'error'
        LIKE '%''sparql'' mode unavailable%E-012%') AS d_error_named;

-- ─── E — §5.3 #2 — validation against a materialised graph ──────
-- ex:fido is typed ex:Dog; AnimalShape targets ex:Animal and
-- requires ex:name (minCount 1). fido is an ex:Animal ONLY via the
-- RDFS rdfs9 entailment (ex:Dog ⊑ ex:Animal). Pre-materialize the
-- shape has no target ⇒ conforms. After `pgrdf.materialize(g,
-- 'rdfs')` the entailed `ex:fido a ex:Animal` makes fido a target;
-- it lacks ex:name ⇒ a violation against an entailment-bound focus.
DO $$
BEGIN
  PERFORM pgrdf.add_graph(12203);
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .
     @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
     ex:Dog rdfs:subClassOf ex:Animal .
     ex:fido a ex:Dog .',
    12203);
  PERFORM pgrdf.add_graph(12204);
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .
     @prefix sh: <http://www.w3.org/ns/shacl#> .
     ex:AnimalShape a sh:NodeShape ;
       sh:targetClass ex:Animal ;
       sh:property [ sh:path ex:name ; sh:minCount 1 ] .',
    12204);
END $$;

-- Pre-materialize: fido is only ex:Dog ⇒ no target ⇒ conforms.
SELECT (pgrdf.validate(12203, 12204) ->> 'conforms') AS e_pre_conforms;

-- Materialise the RDFS closure. rdfs9 derives `ex:fido a ex:Animal`.
SELECT 'materialised' AS e_step
  FROM (SELECT pgrdf.materialize(12203, 'rdfs')) _m;

-- Post-materialize: fido is now an ex:Animal (entailed) and lacks
-- ex:name ⇒ conforms = false, with a violation against ex:fido.
SELECT (pgrdf.validate(12203, 12204) ->> 'conforms') AS e_post_conforms;
SELECT count(*)::int AS e_fido_violation
  FROM jsonb_array_elements(pgrdf.validate(12203, 12204) -> 'results') r
  WHERE r ->> 'focusNode' = 'http://example.org/fido';

ROLLBACK;
