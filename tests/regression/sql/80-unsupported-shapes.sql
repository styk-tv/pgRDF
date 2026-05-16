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

-- ─── Gap 2 (RETIRED): multi-triple OPTIONAL ──────────────────────
-- W3C §6.1 — Phase F group F1 (slices 34-31, LLD v0.4 §11) lifts the
-- v0.3 single-triple restriction. An `OPTIONAL { ?s :p ?n . ?s :q
-- ?ag }` N-triple group now translates to a LATERAL-style derived
-- table inside the LEFT JOIN (all-or-nothing per §6.1). Positive
-- coverage: `tests/regression/sql/112-optional-multi-triple.sql`.
-- This gap entry is intentionally removed — the executor no longer
-- panics on multi-triple OPTIONAL, so a `_check_gap` here would emit
-- `!!! unexpected success !!!`.

-- ─── Gap 3 (RETIRED): VALUES inline data block ───────────────────
-- W3C §10 — Phase F group F1 ships `VALUES`. It translates to a
-- `(VALUES …) AS vN(cols)` derived table joined on the shared
-- variables (constants resolved to dict ids ahead of execution;
-- UNDEF → a NULL cell that places no constraint). Positive
-- coverage: `tests/regression/sql/113-values-inline.sql`. The
-- executor no longer flags VALUES unsupported, so a `_check_gap`
-- here would emit `!!! unexpected success !!!`.

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

-- ─── Gap 6 (RETIRED): DESCRIBE query form ────────────────────────
-- LLD v0.4 §11 — Phase F group F3 (slices 26-24) ships DESCRIBE via
-- the sibling UDF `pgrdf.describe(q TEXT) → SETOF JSONB` (parallel
-- to `pgrdf.construct`; same {subject,predicate,object} structured-
-- term shape). The "description" is the closure of each described
-- resource — every triple with the resource as subject — transitively
-- expanded one hop through blank-node objects per W3C §16.4 (cycle-
-- safe, dedup'd); composes with GRAPH scoping. `DESCRIBE` through
-- `pgrdf.sparql` now gives a clean redirect panic
-- (`sparql: use pgrdf.describe(q) for DESCRIBE queries`) — the
-- generic "query form not supported yet" no longer fires for it, so
-- the old `_check_gap` here would emit `!!! unexpected success !!!`
-- (the panic substring changed) and is removed per this file's
-- self-documented contract ("if we genuinely add support, this file
-- gets updated as part of the same commit"). `pgrdf.sparql_parse`
-- now reports `form:"DESCRIBE"` and no longer flags DESCRIBE in
-- `unsupported_algebra`. Positive coverage:
-- `tests/regression/sql/116-describe.sql`.

-- ─── Gap 7: Property path — the §7.1-gated remainder ─────────────
-- W3C §9.1 — Phase E E1 (bare/`^`), E2 (`+`), E3 (`*`/`?`) AND E4
-- (`|`, incl. `(a|b)+`/`(a|b)*`/`(a|b)?`/`^(a|b)`) are all
-- executable now (see 108/109/110-property-path-*.sql). The ONLY
-- still-NOT-executable property-path form is the §7.1-permitted
-- gated remainder: an alternation arm that is itself a sequence
-- (`foaf:knows/foaf:knows | foaf:member`) — folding it would mean
-- composing a recursive CTE inside an alternation arm (the
-- translator balloon §7.1 explicitly permits gating). It panics
-- with a STABLE rollout-preview prefix; we match on the stable
-- substring only.
SELECT _check_gap(
  'gap-7 property path alternation',
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?o WHERE { ?s (foaf:knows/foaf:knows|foaf:member) ?o }',
  'nested recursive property path'
);

-- ─── Gap 8 (RETIRED): aggregates over UNION ──────────────────────
-- LLD v0.4 §11 — Phase F group F2 (slices 30-27) ships aggregates
-- over a UNION via a derived-table refactor: each branch becomes a
-- sub-SELECT projecting the aggregate/GROUP-BY variables' dict ids
-- into the F1 `vK` derived-column pool, the branches `UNION ALL`
-- into a derived table, and the existing aggregate translator runs
-- over `(<union>) qU` unchanged (COUNT/SUM/AVG/type-aware MIN-MAX/
-- GROUP_CONCAT/SAMPLE, DISTINCT, GROUP BY, HAVING). Positive
-- coverage: `tests/regression/sql/115-aggregate-over-union.sql`.
-- This gap entry is intentionally removed — the executor no longer
-- panics on aggregate-over-UNION, so a `_check_gap` here would emit
-- `!!! unexpected success !!!`.
--
-- (BIND-output-downstream — the other v0.3 SPARQL-surface gap closed
-- in F2 — never had a gap entry here: it produced a "FILTER
-- expression not translatable" / unbound-anchor failure rather than
-- a dedicated stable panic, so there was nothing to lock. F2's AST
-- substitution pass makes BIND vars usable in later FILTER / BGP
-- joins / chained BIND; positive coverage:
-- `tests/regression/sql/114-bind-downstream.sql`.)

DROP FUNCTION _check_gap(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
