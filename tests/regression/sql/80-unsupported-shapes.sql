-- 80-unsupported-shapes.sql
--
-- Regression signals for SPARQL shapes pgRDF does not yet support.
-- The contract: every query below MUST fail with a recognisable,
-- stable error-message substring — NOT silently succeed with wrong
-- results, NOT panic with an opaque trace.
--
-- These shapes are tracked as v0.4 SPARQL-surface work (see
-- specs/SPEC.pgRDF.LLD.v0.3.md §3 deferred list). The point of this
-- file is to *lock the failure mode in*: if we accidentally start
-- producing wrong results (a translator bug), the baseline diff
-- fires; if we genuinely add support, this file gets updated as
-- part of the same commit.
--
-- Each gap is checked through plpgsql `BEGIN ... EXCEPTION ... END;`
-- so the captured output is a clean boolean (`t` = expected
-- substring present in SQLERRM). The exact SQLERRM contents — which
-- can include unstable algebra dumps, synthetic variable hashes,
-- and base_iri / dataset internals from spargebra — are not pinned;
-- only the stable error prefix our translator emits IS.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

SELECT pgrdf.add_graph(9990);
SELECT pgrdf.parse_turtle('
@prefix ex:  <http://example.com/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
ex:a foaf:name "Alice" .
ex:a foaf:age  30 .
ex:b foaf:name "Bob" .
ex:b foaf:age  25 .

ex:i1 ex:cat "books" ; ex:price 10 .
ex:i2 ex:cat "books" ; ex:price 12 .
ex:i3 ex:cat "books" ; ex:price 18 .
', 9990);

-- The check helper:
--   * Runs `query` inside a try/catch.
--   * If the query SUCCEEDED, emits `unexpected success` (which
--     fires the diff against the baseline).
--   * If the query failed AND SQLERRM contains the stable
--     `expected_fragment` substring, emits `t`.
--   * If the query failed but the SQLERRM message changed shape,
--     emits `f` plus the new SQLERRM (so the diff carries
--     diagnostics).
CREATE OR REPLACE FUNCTION _check_gap(label TEXT, query TEXT, expected_fragment TEXT)
RETURNS TEXT
LANGUAGE plpgsql AS $$
DECLARE
  msg TEXT;
BEGIN
  BEGIN
    PERFORM 1 FROM pgrdf.sparql(query);
    RETURN format('%s: !!! unexpected success !!!', label);
  EXCEPTION WHEN OTHERS THEN
    msg := SQLERRM;
  END;
  IF position(expected_fragment IN msg) > 0 THEN
    RETURN format('%s: t', label);
  ELSE
    RETURN format('%s: f (got: %s)', label, left(msg, 80));
  END IF;
END
$$;

-- Gap 1 (inline `HAVING(SUM(?v) > c)`) was originally locked here
-- as a known translator failure. The fix landed alongside this file
-- update: AggregateSpec now carries a `synth_aliases` vec that
-- preserves spargebra's internal synthetic variable name even after
-- Extend renames `output_var` to the user-facing AS-alias. The
-- HAVING-filter migration + translator both consult both names.
-- Positive coverage now lives at
-- `tests/w3c-sparql/22-having-inline-aggregate/`.

-- ─── Gap 2: multi-triple OPTIONAL ────────────────────────────────
-- W3C §6 allows OPTIONAL blocks with N>=1 triples. pgRDF handles
-- only single-triple OPTIONAL; multi-triple needs a derived-table
-- refactor inside the LEFT JOIN.
SELECT _check_gap(
  'gap-2 multi-triple OPTIONAL',
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?a WHERE {
     ?s foaf:name ?n
     OPTIONAL { ?s foaf:age ?a . ?s foaf:name ?n2 }
   }',
  'sparql: OPTIONAL today only supports a single triple pattern'
);

-- ─── Gap 3: VALUES inline data block ─────────────────────────────
-- W3C §10.2 — pgRDF parses but does not translate.
SELECT _check_gap(
  'gap-3 VALUES inline data',
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n WHERE { ?s foaf:name ?n . VALUES ?n { "Alice" "Bob" } }',
  'sparql: unsupported algebra'
);

-- ─── Gap 4 (RETIRED): GRAPH ?g { … } variable form ──────────────
-- W3C §13.3 — both literal-IRI (`GRAPH <iri> { … }`, slice 114) and
-- variable (`GRAPH ?g { … }`, slice 113) forms are now supported.
-- The variable form JOINs `_pgrdf_graphs` to project ?g as the
-- IRI string. Positive coverage:
--   * `tests/regression/sql/78-sparql-graph-literal-iri.sql`
--   * `tests/regression/sql/79-sparql-graph-variable.sql`
-- This gap entry is intentionally removed — the executor no longer
-- panics on the variable form, so a `_check_gap` here would emit
-- `!!! unexpected success !!!`.

-- ─── Gap 5: CONSTRUCT query form ─────────────────────────────────
-- W3C §16.2 — CONSTRUCT returns an RDF graph; pgrdf.sparql returns
-- SETOF JSONB, so CONSTRUCT would need a separate UDF surface.
SELECT _check_gap(
  'gap-5 CONSTRUCT',
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   CONSTRUCT { ?s <http://example.com/named> ?n } WHERE { ?s foaf:name ?n }',
  'sparql: query form not supported yet'
);

-- ─── Gap 6: DESCRIBE query form ──────────────────────────────────
-- W3C §16.4 — same output-shape constraint as CONSTRUCT.
SELECT _check_gap(
  'gap-6 DESCRIBE',
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   DESCRIBE ?s WHERE { ?s foaf:name "Alice" }',
  'sparql: query form not supported yet'
);

-- ─── Gap 7: Property path with `|` (alternation) ─────────────────
-- W3C §9.1 — Phase E E1 (bare/`^`), E2 (`+`), and E3 (`*`/`?`) are
-- all executable now (see 108/109/110-property-path-*.sql).
-- Alternation `|` is the gated E4 stretch goal and is still NOT
-- executable; it panics with a STABLE rollout-preview prefix so
-- downstream tooling can preview the schedule. We match on the
-- stable prefix substring only.
SELECT _check_gap(
  'gap-7 property path alternation',
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?o WHERE { ?s (foaf:knows|foaf:member) ?o }',
  'gated stretch goal (Phase E group E4)'
);

-- ─── Gap 8: aggregates over UNION ────────────────────────────────
-- The executor explicitly panics; aggregate output construction
-- needs to happen after the union derived table, which isn't
-- wired today.
SELECT _check_gap(
  'gap-8 aggregates over UNION',
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT (COUNT(?n) AS ?c) WHERE {
     { ?s foaf:name ?n } UNION { ?s foaf:age ?n }
   }',
  'sparql: aggregates on top of UNION not supported yet'
);

DROP FUNCTION _check_gap(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
