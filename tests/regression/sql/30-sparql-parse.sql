-- 30-sparql-parse — pgrdf.sparql_parse returns a stable JSONB shape
-- for SELECT queries. Empirically verifies spargebra integration on
-- a handful of representative queries; the BGP-to-SQL translator
-- (step 5) consumes the same JSONB shape.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

-- 1. Generic 3-var BGP.
SELECT (pgrdf.sparql_parse('SELECT ?s ?p ?o WHERE { ?s ?p ?o }')->>'form') AS form;

SELECT (pgrdf.sparql_parse('SELECT ?s ?p ?o WHERE { ?s ?p ?o }')->'variables')::text AS vars;

SELECT (pgrdf.sparql_parse('SELECT ?s ?p ?o WHERE { ?s ?p ?o }')->>'bgp_pattern_count')::int AS n_patterns;

-- 2. BGP with a bound predicate.
SELECT (
  pgrdf.sparql_parse(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?p ?n WHERE { ?p foaf:name ?n }'
  )->'bgp_patterns'->0->'p'->>'iri'
) AS p_iri;

-- 3. Two-pattern BGP — count comes through correctly.
SELECT (
  pgrdf.sparql_parse(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?p ?n ?m WHERE { ?p foaf:name ?n . ?p foaf:mbox ?m }'
  )->>'bgp_pattern_count'
)::int AS n_patterns_two;

-- 4. Filter is now translatable by pgrdf.sparql, so the parser
-- walks into it instead of flagging it. unsupported_algebra is empty
-- for a FILTERed BGP.
SELECT (
  pgrdf.sparql_parse('SELECT ?s WHERE { ?s ?p ?o FILTER(isIRI(?o)) }')->'unsupported_algebra'
)::text AS unsupported_after_filter;

-- 4b. OPTIONAL is now supported — parser walks through it. Both
-- the mandatory BGP triple and the OPTIONAL's triple are counted.
SELECT (
  pgrdf.sparql_parse(
    'SELECT ?s ?n WHERE { ?s ?p ?o OPTIONAL { ?s <http://x/n> ?n } }'
  )->>'bgp_pattern_count'
)::int AS bgp_count_with_optional;

-- 4c. UNION is now supported. Both branches' BGPs are visible.
SELECT (
  pgrdf.sparql_parse(
    'SELECT ?s WHERE { { ?s <http://x/a> ?o } UNION { ?s <http://x/b> ?o } }'
  )->>'bgp_pattern_count'
)::int AS bgp_count_union;

-- 4d. MINUS is now supported. Both arms' BGPs are visible.
SELECT (
  pgrdf.sparql_parse(
    'SELECT ?s WHERE { ?s ?p ?o MINUS { ?s <http://x/a> ?b } }'
  )->>'bgp_pattern_count'
)::int AS bgp_count_minus;

-- 4e. Property-path executability tracks the Phase E rollout: E1
-- (bare/`^`), E2 (`+`), and E3 (`*`/`?`) are NOT flagged (they
-- lower into the bgp shape). Alternation `|` is the still-deferred
-- E4 gated stretch, so it still surfaces in `unsupported_algebra`
-- with the "recursive/alternation" tag. Note: simple sequence
-- paths (<a>/<b>) are desugared by spargebra into BGP chains and
-- don't appear as Path nodes.
SELECT (
  pgrdf.sparql_parse(
    'SELECT ?s ?o WHERE { ?s (<http://x/a>|<http://x/b>) ?o }'
  )->'unsupported_algebra'
)::text AS unsupported_path;

-- 5. CONSTRUCT — Phase D slice 52 enriched the JSONB shape: `form` is
-- still `"CONSTRUCT"` but the `supported: false` placeholder is gone,
-- replaced by `template` + `where_shape` + `shorthand` +
-- `unsupported_algebra`. See 107-sparql-parse-construct for full
-- coverage; here we just lock the `form` field.
SELECT (
  pgrdf.sparql_parse('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }')->>'form'
) AS form_construct;

SELECT (
  pgrdf.sparql_parse('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }')->'template'->>'triple_count'
)::int AS construct_template_triples;

-- 6. Literal object (typed) round-trips through the JSONB shape.
SELECT (
  pgrdf.sparql_parse(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?p WHERE { ?p foaf:age 42 }'
  )->'bgp_patterns'->0->'o'->>'literal'
) AS literal_object_value;
