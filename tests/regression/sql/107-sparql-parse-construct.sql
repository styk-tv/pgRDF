-- 107-sparql-parse-construct — Phase D slice 52: pgrdf.sparql_parse
-- reports a structured CONSTRUCT shape (template + where_shape +
-- shorthand + unsupported_algebra). Locks the field contract from
-- SPEC.pgRDF.LLD.v0.4 §6.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

-- A. Simple constant template (no variables, no blank nodes). All
-- template positions are constants → has_constants_only is true and
-- has_variables / has_blank_nodes are false. shorthand is false (the
-- query uses the explicit `CONSTRUCT { ... } WHERE { ... }` form).
SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } WHERE { ?s ?p ?o }'
)->>'form') AS a_form;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } WHERE { ?s ?p ?o }'
)->'template'->>'triple_count')::int AS a_tmpl_triples;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } WHERE { ?s ?p ?o }'
)->'template'->>'has_constants_only')::bool AS a_constants_only;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } WHERE { ?s ?p ?o }'
)->'template'->>'has_variables')::bool AS a_has_variables;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } WHERE { ?s ?p ?o }'
)->'where_shape'->>'kind') AS a_where_kind;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } WHERE { ?s ?p ?o }'
)->>'shorthand')::bool AS a_shorthand;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } WHERE { ?s ?p ?o }'
)->'unsupported_algebra')::text AS a_unsupported;

-- B. Variable template, multi-triple WHERE. The constant literal
-- "src" in template position is NOT a variable; only ?s and ?o are
-- in `template.variables` (sorted alphabetically: ?o, ?s).
SELECT (pgrdf.sparql_parse(
  'PREFIX ex: <http://example.org/> '
  'CONSTRUCT { ?s ex:tag ?o . ?s ex:from "src" } '
  'WHERE { ?s ex:p ?o . ?s ex:p2 ?o2 }'
)->'template'->>'triple_count')::int AS b_tmpl_triples;

SELECT (pgrdf.sparql_parse(
  'PREFIX ex: <http://example.org/> '
  'CONSTRUCT { ?s ex:tag ?o . ?s ex:from "src" } '
  'WHERE { ?s ex:p ?o . ?s ex:p2 ?o2 }'
)->'template'->>'has_variables')::bool AS b_has_variables;

SELECT (pgrdf.sparql_parse(
  'PREFIX ex: <http://example.org/> '
  'CONSTRUCT { ?s ex:tag ?o . ?s ex:from "src" } '
  'WHERE { ?s ex:p ?o . ?s ex:p2 ?o2 }'
)->'template'->'variables')::text AS b_tmpl_vars;

SELECT (pgrdf.sparql_parse(
  'PREFIX ex: <http://example.org/> '
  'CONSTRUCT { ?s ex:tag ?o . ?s ex:from "src" } '
  'WHERE { ?s ex:p ?o . ?s ex:p2 ?o2 }'
)->'where_shape'->>'kind') AS b_where_kind;

SELECT (pgrdf.sparql_parse(
  'PREFIX ex: <http://example.org/> '
  'CONSTRUCT { ?s ex:tag ?o . ?s ex:from "src" } '
  'WHERE { ?s ex:p ?o . ?s ex:p2 ?o2 }'
)->'where_shape'->>'triple_count')::int AS b_where_triples;

SELECT (pgrdf.sparql_parse(
  'PREFIX ex: <http://example.org/> '
  'CONSTRUCT { ?s ex:tag ?o . ?s ex:from "src" } '
  'WHERE { ?s ex:p ?o . ?s ex:p2 ?o2 }'
)->'where_shape'->'variables')::text AS b_where_vars;

-- C. Blank-node template. `_:b` in the template surfaces as
-- has_blank_nodes: true. has_constants_only is false (a bnode
-- doesn't count as a "constant" for the round-trip-ingest decision
-- — fresh per-solution labels are minted at execute time per W3C
-- SPARQL 1.1 §16.2).
SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { _:b <http://example.org/type> ?t } WHERE { ?s ?p ?t }'
)->'template'->>'has_blank_nodes')::bool AS c_has_blank;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { _:b <http://example.org/type> ?t } WHERE { ?s ?p ?t }'
)->'template'->>'has_constants_only')::bool AS c_constants_only;

-- D. GRAPH literal WHERE. The literal IRI surfaces in
-- where_shape.named_graphs_used.
SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { ?s <http://example.org/src> ?o } '
  'WHERE { GRAPH <http://example.org/g1> { ?s <http://example.org/p> ?o } }'
)->'where_shape'->>'kind') AS d_where_kind;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { ?s <http://example.org/src> ?o } '
  'WHERE { GRAPH <http://example.org/g1> { ?s <http://example.org/p> ?o } }'
)->'where_shape'->'named_graphs_used')::text AS d_named_graphs;

-- E. GRAPH variable WHERE. The variable surfaces with the `?`
-- prefix in named_graphs_used.
SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { ?s <http://example.org/from> ?g } '
  'WHERE { GRAPH ?g { ?s <http://example.org/p> ?o } }'
)->'where_shape'->>'kind') AS e_where_kind;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { ?s <http://example.org/from> ?g } '
  'WHERE { GRAPH ?g { ?s <http://example.org/p> ?o } }'
)->'where_shape'->'named_graphs_used')::text AS e_named_graphs;

-- F. Shorthand form. `CONSTRUCT WHERE { ... }` is equivalent to
-- `CONSTRUCT { ... } WHERE { ... }` per W3C SPARQL 1.1 §16.2.4.
-- The `shorthand` flag is true; spargebra populates template from
-- the BGP at parse time so template/where triple counts match.
SELECT (pgrdf.sparql_parse(
  'CONSTRUCT WHERE { ?s ?p ?o }'
)->>'shorthand')::bool AS f_shorthand;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT WHERE { ?s ?p ?o }'
)->'template'->>'triple_count')::int AS f_tmpl_triples;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT WHERE { ?s ?p ?o }'
)->'where_shape'->>'triple_count')::int AS f_where_triples;

-- G. Unsupported algebra detection — DISTINCT inside a CONSTRUCT
-- sub-SELECT. `pgrdf.construct` will reject this at execute time
-- per LLD §6.2; `sparql_parse` surfaces it ahead of time as the
-- "Distinct" tag in `unsupported_algebra`.
SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } '
  'WHERE { { SELECT DISTINCT ?s WHERE { ?s ?p ?o } } }'
)->>'form') AS g_form;

SELECT (pgrdf.sparql_parse(
  'CONSTRUCT { <http://example.org/s> <http://example.org/p> "x" } '
  'WHERE { { SELECT DISTINCT ?s WHERE { ?s ?p ?o } } }'
)->'unsupported_algebra')::text AS g_unsupported;

-- H. SELECT shape NOT regressed. The slice-52 work only touched the
-- CONSTRUCT branch; SELECT still carries `form: "SELECT"` (the
-- existing slice-30 contract).
SELECT (pgrdf.sparql_parse(
  'SELECT ?s WHERE { ?s ?p ?o }'
)->>'form') AS h_form;

-- I. UPDATE shape NOT regressed. INSERT DATA still parses to
-- `form: "UPDATE"` with the Phase C slice-74 per-op enrichment intact.
SELECT (pgrdf.sparql_parse(
  'INSERT DATA { <http://example.org/a> <http://example.org/b> <http://example.org/c> }'
)->>'form') AS i_form;
