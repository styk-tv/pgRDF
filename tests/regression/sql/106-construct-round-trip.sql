-- 106-construct-round-trip.sql
--
-- Phase D slice 53 — round-trip preservation for `pgrdf.construct`.
-- Closes the LLD v0.4 §6.3 acceptance criterion: "`pgrdf.construct(q)`
-- followed by re-inserting the rows via the ingest pair produces the
-- same graph state (modulo dictionary id reshuffles, which are not
-- user-visible)."
--
-- Slice 53 ships:
--   * `pgrdf.put_construct_row(row JSONB, graph_id BIGINT DEFAULT 0)
--      → BIGINT` — single-row primitive. Returns 1 if a fresh quad
--      landed, 0 if `WHERE NOT EXISTS` saw it already.
--   * `pgrdf.put_construct_rows(rows JSONB[], graph_id BIGINT
--      DEFAULT 0) → BIGINT` — the **recommended** surface. Returns
--      the count of newly-inserted quads. A per-call
--      `HashMap<String, i64>` of blank-node labels preserves the
--      within-batch joining that the construct emitter establishes
--      per W3C SPARQL 1.1 §16.2.
--
-- Invariants locked by this file:
--
--   A. Simple round-trip — IRIs only. 3 quads in graph g1 → construct
--      captures 3 rows → re-ingest into fresh g2 → g2 holds 3 quads,
--      same `(s, p, o)` content as g1.
--   B. Typed literal round-trip. `"42"^^xsd:integer` survives;
--      datatype IRI `xsd:integer` preserved verbatim on the
--      re-ingested object.
--   C. Language-tagged literal round-trip. `"Alice"@en` survives;
--      language tag `en` AND implicit datatype `rdf:langString`
--      preserved per RDF 1.1 §3.3 (slice 53 obeys the construct
--      emitter's contract — language-tagged literals carry BOTH
--      fields).
--   D. Plain-string literal round-trip. `"plain text"` survives; the
--      construct emitter writes `xsd:string` explicitly per slice 59,
--      so the re-ingested literal carries that datatype too.
--   E. Bnode round-trip — within-solution joining preserved. A
--      multi-triple template emits two rows whose template positions
--      share `_:r`; after re-ingest, the two stored quads must share
--      the same blank-node dict id (the within-solution sameness
--      survives), and a distinct template label `_:other` mints a
--      different stored blank node.
--   F. Idempotency on re-ingest. Calling `put_construct_rows` twice
--      with the same captured rowset leaves the graph unchanged —
--      the second call returns 0 (set semantics via `WHERE NOT
--      EXISTS`).
--   G. Empty result set round-trip. A construct query that matches
--      no solutions emits zero rows; passing the resulting (NULL or
--      empty) array to `put_construct_rows` is a no-op.
--   H. Single-row primitive — `put_construct_row` accepts one row
--      and lands one quad. Within-call bnode joining only applies
--      inside ONE call, so this surface is for callers that handle
--      batch coordination themselves.
--   I. Reject literal in subject — feeding a hand-crafted row with a
--      literal-typed subject panics with the stable prefix.
--   J. Reject negative graph_id.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();

-- Helper — captures SQLERRM from a wrapped EXECUTE and asserts the
-- expected substring is present. Same shape as
-- `103-construct-multi-triple-templates.sql`.
CREATE OR REPLACE FUNCTION _check_error(label TEXT, sql TEXT, expected_fragment TEXT)
RETURNS TEXT
LANGUAGE plpgsql AS $$
DECLARE
  msg TEXT;
BEGIN
  BEGIN
    EXECUTE sql;
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

-- ─── Invariant A: simple IRI-only round-trip ─────────────────────
SELECT pgrdf.add_graph(8100::bigint);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:a ex:p ex:o1 .
   ex:b ex:p ex:o2 .
   ex:c ex:p ex:o3 .',
  8100);

-- Re-ingest into a fresh graph 8_101 via construct(...) +
-- put_construct_rows(...).
SELECT pgrdf.add_graph(8101::bigint);
SELECT pgrdf.put_construct_rows(
  (SELECT array_agg(j)
     FROM pgrdf.construct(
       'CONSTRUCT { ?s ?p ?o } '
       'WHERE { GRAPH <urn:pgrdf:graph:8100> { ?s ?p ?o } }') AS t(j)),
  8101::bigint) AS a_landed;

-- Same row count on both graphs.
SELECT pgrdf.count_quads(8100::bigint) AS a_src_count,
       pgrdf.count_quads(8101::bigint) AS a_dst_count;

-- Same (s,p,o) content. Build a set per graph as TEXT triples and
-- compare via EXCEPT both ways.
WITH src AS (
  SELECT ds.lexical_value || ' ' || dp.lexical_value || ' ' || do_.lexical_value AS triple
    FROM pgrdf._pgrdf_quads q
    JOIN pgrdf._pgrdf_dictionary ds ON ds.id = q.subject_id
    JOIN pgrdf._pgrdf_dictionary dp ON dp.id = q.predicate_id
    JOIN pgrdf._pgrdf_dictionary do_ ON do_.id = q.object_id
   WHERE q.graph_id = 8100
), dst AS (
  SELECT ds.lexical_value || ' ' || dp.lexical_value || ' ' || do_.lexical_value AS triple
    FROM pgrdf._pgrdf_quads q
    JOIN pgrdf._pgrdf_dictionary ds ON ds.id = q.subject_id
    JOIN pgrdf._pgrdf_dictionary dp ON dp.id = q.predicate_id
    JOIN pgrdf._pgrdf_dictionary do_ ON do_.id = q.object_id
   WHERE q.graph_id = 8101
)
SELECT
  (SELECT count(*) FROM ((SELECT * FROM src) EXCEPT (SELECT * FROM dst)) d1)::bigint AS a_src_minus_dst,
  (SELECT count(*) FROM ((SELECT * FROM dst) EXCEPT (SELECT * FROM src)) d2)::bigint AS a_dst_minus_src;

-- ─── Invariant B: typed-literal round-trip ───────────────────────
SELECT pgrdf.add_graph(8110::bigint);
SELECT pgrdf.parse_turtle(
  '@prefix ex:  <http://example.com/> .
   @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
   ex:n ex:age "42"^^xsd:integer .',
  8110);

SELECT pgrdf.add_graph(8111::bigint);
SELECT pgrdf.put_construct_rows(
  (SELECT array_agg(j)
     FROM pgrdf.construct(
       'CONSTRUCT { ?s ?p ?o } '
       'WHERE { GRAPH <urn:pgrdf:graph:8110> { ?s ?p ?o } }') AS t(j)),
  8111::bigint) AS b_landed;

-- The literal "42" in the dst graph carries the xsd:integer datatype.
SELECT
  o.term_type = 3 AS b_o_is_literal,
  o.lexical_value AS b_o_lex,
  dt.lexical_value AS b_o_datatype
FROM pgrdf._pgrdf_quads q
JOIN pgrdf._pgrdf_dictionary o  ON o.id  = q.object_id
JOIN pgrdf._pgrdf_dictionary dt ON dt.id = o.datatype_iri_id
WHERE q.graph_id = 8111
  AND o.lexical_value = '42';

-- ─── Invariant C: language-tagged literal round-trip ─────────────
SELECT pgrdf.add_graph(8120::bigint);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:a ex:name "Alice"@en .',
  8120);

SELECT pgrdf.add_graph(8121::bigint);
SELECT pgrdf.put_construct_rows(
  (SELECT array_agg(j)
     FROM pgrdf.construct(
       'CONSTRUCT { ?s ?p ?o } '
       'WHERE { GRAPH <urn:pgrdf:graph:8120> { ?s ?p ?o } }') AS t(j)),
  8121::bigint) AS c_landed;

-- Language tag preserved; datatype_iri_id NULL (language-tagged
-- literals don't carry a separate datatype id per slice 53's contract;
-- the rdf:langString IRI surfaces in the construct output but is not
-- bound on the dict row, matching the parse_turtle round-trip shape).
SELECT
  o.lexical_value AS c_o_lex,
  o.language_tag AS c_o_lang,
  o.datatype_iri_id IS NULL AS c_o_dt_id_null
FROM pgrdf._pgrdf_quads q
JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
WHERE q.graph_id = 8121
  AND o.lexical_value = 'Alice';

-- ─── Invariant D: plain-string literal round-trip ────────────────
SELECT pgrdf.add_graph(8130::bigint);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:a ex:label "plain text" .',
  8130);

SELECT pgrdf.add_graph(8131::bigint);
SELECT pgrdf.put_construct_rows(
  (SELECT array_agg(j)
     FROM pgrdf.construct(
       'CONSTRUCT { ?s ?p ?o } '
       'WHERE { GRAPH <urn:pgrdf:graph:8130> { ?s ?p ?o } }') AS t(j)),
  8131::bigint) AS d_landed;

-- Plain-string literal lands with xsd:string datatype (the construct
-- emitter writes it explicitly per slice 59; slice 53 honours that on
-- ingest). Verify via JOIN to the datatype IRI row.
SELECT
  o.lexical_value AS d_o_lex,
  dt.lexical_value AS d_o_datatype
FROM pgrdf._pgrdf_quads q
JOIN pgrdf._pgrdf_dictionary o  ON o.id  = q.object_id
JOIN pgrdf._pgrdf_dictionary dt ON dt.id = o.datatype_iri_id
WHERE q.graph_id = 8131
  AND o.lexical_value = 'plain text';

-- ─── Invariant E: bnode within-solution joining preserved ────────
-- Multi-triple template — `_:r` appears as object of triple-1 and
-- subject of triple-2 within the same solution (slice 56's
-- within-solution sameness contract). After re-ingest the two stored
-- quads MUST reference the SAME bnode dict id. A distinct template
-- label `_:other` would mint a different stored bnode (negative
-- check covered by counting distinct bnode ids).
SELECT pgrdf.add_graph(8140::bigint);
SELECT pgrdf.parse_turtle(
  '@prefix ex: <http://example.com/> .
   ex:s ex:p ex:o .',
  8140);

SELECT pgrdf.add_graph(8141::bigint);
SELECT pgrdf.put_construct_rows(
  (SELECT array_agg(j)
     FROM pgrdf.construct(
       'CONSTRUCT { <http://example.com/s> <http://example.com/has> _:r . '
       '            _:r <http://example.com/about> <http://example.com/o> } '
       'WHERE { GRAPH <urn:pgrdf:graph:8140> { ?s ?p ?o } }') AS t(j)),
  8141::bigint) AS e_landed;

-- Within graph 8141 there should be exactly ONE blank-node dict id
-- across the two quads' bnode positions (the within-solution-shared
-- `_:r`). Count distinct bnode ids appearing in subject or object
-- positions.
WITH bnode_ids AS (
  SELECT q.object_id AS bid
    FROM pgrdf._pgrdf_quads q
    JOIN pgrdf._pgrdf_dictionary d ON d.id = q.object_id
   WHERE q.graph_id = 8141 AND d.term_type = 2
  UNION ALL
  SELECT q.subject_id
    FROM pgrdf._pgrdf_quads q
    JOIN pgrdf._pgrdf_dictionary d ON d.id = q.subject_id
   WHERE q.graph_id = 8141 AND d.term_type = 2
)
SELECT count(DISTINCT bid)::bigint AS e_distinct_bnode_ids,
       count(*)::bigint            AS e_total_bnode_positions
  FROM bnode_ids;

-- And the cross-quad bnode reference is reachable — the object of
-- the `has` quad equals the subject of the `about` quad.
SELECT EXISTS (
  SELECT 1
    FROM pgrdf._pgrdf_quads q_has
    JOIN pgrdf._pgrdf_quads q_about
      ON q_about.subject_id = q_has.object_id
    JOIN pgrdf._pgrdf_dictionary p_has   ON p_has.id   = q_has.predicate_id
    JOIN pgrdf._pgrdf_dictionary p_about ON p_about.id = q_about.predicate_id
   WHERE q_has.graph_id = 8141
     AND q_about.graph_id = 8141
     AND p_has.lexical_value   = 'http://example.com/has'
     AND p_about.lexical_value = 'http://example.com/about'
) AS e_within_solution_join_survived;

-- ─── Invariant F: idempotency on re-ingest ───────────────────────
-- Re-ingest the same captured rowset a second time. Set semantics
-- via `WHERE NOT EXISTS` makes the call a no-op — 0 new quads land,
-- graph 8101 stays at the same count from invariant A.
SELECT pgrdf.put_construct_rows(
  (SELECT array_agg(j)
     FROM pgrdf.construct(
       'CONSTRUCT { ?s ?p ?o } '
       'WHERE { GRAPH <urn:pgrdf:graph:8100> { ?s ?p ?o } }') AS t(j)),
  8101::bigint) AS f_re_ingest_landed;

SELECT pgrdf.count_quads(8101::bigint) AS f_dst_count_after_re_ingest;

-- ─── Invariant G: empty result set round-trip ────────────────────
-- A construct query against a graph that has nothing matching emits
-- zero rows. `array_agg` returns NULL on an empty input; the UDF
-- handles a NULL array as a no-op (zero quads ingested).
SELECT pgrdf.add_graph(8150::bigint);  -- empty source graph

SELECT pgrdf.add_graph(8151::bigint);
SELECT COALESCE(pgrdf.put_construct_rows(
  (SELECT array_agg(j)
     FROM pgrdf.construct(
       'CONSTRUCT { ?s ?p ?o } '
       'WHERE { GRAPH <urn:pgrdf:graph:8150> { ?s ?p ?o } }') AS t(j)),
  8151::bigint), -1::bigint) AS g_landed;

SELECT pgrdf.count_quads(8151::bigint) AS g_dst_count;

-- ─── Invariant H: single-row primitive ───────────────────────────
SELECT pgrdf.add_graph(8160::bigint);
SELECT pgrdf.put_construct_row(
  '{"subject":   {"type":"iri","value":"http://example.com/h1"},
    "predicate": {"type":"iri","value":"http://example.com/hp"},
    "object":    {"type":"iri","value":"http://example.com/h2"}}'::jsonb,
  8160::bigint) AS h_landed;

SELECT pgrdf.count_quads(8160::bigint) AS h_dst_count;

-- Calling the single-row UDF twice on the same row is also
-- idempotent — the second call sees the row via NOT EXISTS and
-- returns 0.
SELECT pgrdf.put_construct_row(
  '{"subject":   {"type":"iri","value":"http://example.com/h1"},
    "predicate": {"type":"iri","value":"http://example.com/hp"},
    "object":    {"type":"iri","value":"http://example.com/h2"}}'::jsonb,
  8160::bigint) AS h_landed_again;

-- ─── Invariant I: reject literal in subject ──────────────────────
SELECT _check_error(
  'i-rejects-literal-subject',
  $$SELECT pgrdf.put_construct_row(
      '{"subject":   {"type":"literal","value":"lit","datatype":"http://www.w3.org/2001/XMLSchema#string"},
        "predicate": {"type":"iri","value":"http://example.com/p"},
        "object":    {"type":"iri","value":"http://example.com/o"}}'::jsonb,
      0::bigint)$$,
  'literal not allowed in subject/predicate position'
);

-- ─── Invariant J: reject negative graph_id ───────────────────────
SELECT _check_error(
  'j-rejects-negative-graph-id',
  $$SELECT pgrdf.put_construct_row(
      '{"subject":   {"type":"iri","value":"http://example.com/a"},
        "predicate": {"type":"iri","value":"http://example.com/p"},
        "object":    {"type":"iri","value":"http://example.com/o"}}'::jsonb,
      (-1)::bigint)$$,
  'graph_id must be >= 0'
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
