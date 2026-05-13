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

-- 4b. OPTIONAL still flags as unsupported.
SELECT (
  pgrdf.sparql_parse(
    'SELECT ?s ?n WHERE { ?s ?p ?o OPTIONAL { ?s <http://x/n> ?n } }'
  )->'unsupported_algebra'
)::text AS unsupported_optional;

-- 5. CONSTRUCT recognised but flagged out-of-scope.
SELECT (
  pgrdf.sparql_parse('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }')->>'form'
) AS form_construct;

SELECT (
  pgrdf.sparql_parse('CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }')->>'supported'
)::bool AS construct_supported;

-- 6. Literal object (typed) round-trips through the JSONB shape.
SELECT (
  pgrdf.sparql_parse(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?p WHERE { ?p foaf:age 42 }'
  )->'bgp_patterns'->0->'o'->>'literal'
) AS literal_object_value;
