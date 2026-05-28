-- 124-end-to-end-lexical-rehydration.sql
--
-- TG-4 / TF-4 (shared, single source-of-truth file): regression
-- locking the dictionary rehydrate path against drift in term
-- lexicals across the full pgRDF pipeline. Per CX-002 EVAL
-- recommendation (_WIP/RESPONSE.pgRDF.v0.5.0-by-CX-002.md): assert
-- EXACT lexical values at every stage rather than row counts.
-- Catches dictionary-encode + rehydrate drift end-to-end.
--
-- Pipeline:
--   parse_turtle → CONSTRUCT (GRAPH-scoped) → materialize(RDFS)
--               → CONSTRUCT → validate(SHACL Native)
--               → put_construct_rows → dictionary de-dup check
--
-- Term-shape coverage:
--   A. IRI http://     — http://example.org/alice
--   B. IRI urn:        — urn:example:bob
--   C. IRI custom+frag — ckp://Task#001 (covers the pgCK case)
--   D. Plain literal   — "Alice"           (xsd:string implicit)
--   E. xsd:integer     — "30"^^xsd:integer (lexical "30", no padding)
--   F. xsd:dateTime    — timezone preserved
--   G. Lang-tagged     — "Hi"@en           (rdf:langString + lang)
--
-- Invariants:
--   I-1 — CONSTRUCT projects byte-identical lexicals for every shape.
--   I-2 — Lang-tagged literal carries datatype = rdf:langString
--         AND language = "en"; typed-literal datatype IRI exact.
--   I-3 — Materialize(RDFS) preserves IRI lexicals byte-identical
--         in the entailed triples.
--   I-4 — SHACL focusNode lexical byte-identical to the data-graph
--         IRI it pinpoints (custom-scheme IRIs included).
--   I-5 — CONSTRUCT round-trip (put_construct_rows) preserves every
--         term shape byte-identical; dictionary de-dups against the
--         identical (term_type, lexical_value, datatype_iri_id,
--         language_tag) tuple.
--
-- GRAPH scoping: every CONSTRUCT carries `GRAPH <urn:test/tg4/…> { … }`
-- so the test is isolated against any pre-existing data in the shared
-- compose database. The matching graph IRIs are registered via
-- `pgrdf.add_graph(iri)` to enable the SPARQL GRAPH <iri> form.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- ─── Bind IRIs to test graphs (clean isolation) ────────────────
DO $$
DECLARE
  g_data   bigint;
  g_shapes bigint;
  g_round  bigint;
BEGIN
  g_data   := pgrdf.add_graph('urn:test/tg4/data');
  g_shapes := pgrdf.add_graph('urn:test/tg4/shapes');
  g_round  := pgrdf.add_graph('urn:test/tg4/roundtrip');
  PERFORM pgrdf.clear_graph(g_data);
  PERFORM pgrdf.clear_graph(g_shapes);
  PERFORM pgrdf.clear_graph(g_round);

  -- Seed the data graph with rich term shapes
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .
     @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
     ex:alice ex:knows <urn:example:bob> ;
              ex:relatedTo <ckp://Task#001> ;
              ex:name "Alice" ;
              ex:age "30"^^xsd:integer ;
              ex:lastSeen "2026-05-28T10:00:00Z"^^xsd:dateTime ;
              ex:greeting "Hi"@en .',
    g_data);
END $$;

-- ─── I-1 — CONSTRUCT lexicals exact (one cell per SELECT) ──────

-- A. http subject IRI
SELECT (j->'subject'->>'value') AS a1_subject_iri
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?s ex:knows ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ?s ex:knows ?o } }'
) AS s(j);

-- A. http predicate IRI
SELECT (j->'predicate'->>'value') AS a2_predicate_iri
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?s ex:knows ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ?s ex:knows ?o } }'
) AS s(j);

-- B. urn: object IRI
SELECT (j->'object'->>'value') AS b1_urn_iri
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?s ex:knows ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ?s ex:knows ?o } }'
) AS s(j);

-- B. object type = iri
SELECT (j->'object'->>'type') AS b2_object_type
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?s ex:knows ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ?s ex:knows ?o } }'
) AS s(j);

-- C. custom-scheme IRI with fragment (the pgCK shape)
SELECT (j->'object'->>'value') AS c1_custom_iri
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ?s ex:relatedTo ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ?s ex:relatedTo ?o } }'
) AS s(j);

-- D. plain literal lexical
SELECT (j->'object'->>'value') AS d1_plain_lexical
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:name ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:name ?o } }'
) AS s(j);

-- D. plain literal datatype (implicit xsd:string)
SELECT (j->'object'->>'datatype') AS d2_plain_datatype
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:name ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:name ?o } }'
) AS s(j);

-- E. typed integer lexical (no padding)
SELECT (j->'object'->>'value') AS e1_integer_lexical
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:age ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:age ?o } }'
) AS s(j);

-- E. typed integer datatype
SELECT (j->'object'->>'datatype') AS e2_integer_datatype
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:age ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:age ?o } }'
) AS s(j);

-- F. dateTime lexical with timezone
SELECT (j->'object'->>'value') AS f1_datetime_lexical
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:lastSeen ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:lastSeen ?o } }'
) AS s(j);

-- F. dateTime datatype
SELECT (j->'object'->>'datatype') AS f2_datetime_datatype
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:lastSeen ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:lastSeen ?o } }'
) AS s(j);

-- G. lang-tagged literal lexical
SELECT (j->'object'->>'value') AS g1_lang_lexical
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:greeting ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:greeting ?o } }'
) AS s(j);

-- G. lang-tagged datatype = rdf:langString
SELECT (j->'object'->>'datatype') AS g2_lang_datatype
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:greeting ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:greeting ?o } }'
) AS s(j);

-- G. lang-tagged language tag
SELECT (j->'object'->>'language') AS g3_lang_tag
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:greeting ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:greeting ?o } }'
) AS s(j);

-- ─── I-3 — materialize(RDFS) preserves IRI lexicals exact ─────
-- Add rdfs7 rule: ex:knows ⊑ ex:relatedTo. After materialize, the
-- entailed triple ex:alice ex:relatedTo urn:example:bob carries
-- urn:example:bob byte-identical.
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .
     @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
     ex:knows rdfs:subPropertyOf ex:relatedTo .',
    pgrdf.graph_id('urn:test/tg4/data'));
END $$;

SELECT 'i3_materialized' AS i3_step
FROM (
  SELECT pgrdf.materialize(pgrdf.graph_id('urn:test/tg4/data'), 'rdfs')
) _m;

-- Total ex:relatedTo objects from alice = 2 (original ckp + entailed urn)
SELECT count(*)::int AS i3_total_relatedto
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:relatedTo ?o }
   WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:relatedTo ?o } }'
) AS s(j);

-- Entailed urn IRI surfaces byte-identical
SELECT count(*)::int AS i3_entailed_urn
FROM (
  SELECT j FROM pgrdf.construct(
    'PREFIX ex: <http://example.org/>
     CONSTRUCT { ex:alice ex:relatedTo ?o }
     WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:relatedTo ?o } }'
  ) AS s(j)
) q
WHERE (j->'object'->>'value') = 'urn:example:bob';

-- Original ckp IRI still present byte-identical
SELECT count(*)::int AS i3_original_ckp
FROM (
  SELECT j FROM pgrdf.construct(
    'PREFIX ex: <http://example.org/>
     CONSTRUCT { ex:alice ex:relatedTo ?o }
     WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ex:relatedTo ?o } }'
  ) AS s(j)
) q
WHERE (j->'object'->>'value') = 'ckp://Task#001';

-- ─── I-4 — SHACL focusNode preserves IRI lexicals ─────────────
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .
     @prefix sh: <http://www.w3.org/ns/shacl#> .
     @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
     ex:KnowsSubjectShape a sh:NodeShape ;
       sh:targetSubjectsOf ex:knows ;
       sh:property [ sh:path ex:age ;
                     sh:minCount 1 ;
                     sh:datatype xsd:integer ] .',
    pgrdf.graph_id('urn:test/tg4/shapes'));
END $$;

-- Alice has ex:age → conforms (validate scopes by graph_id implicitly)
SELECT (pgrdf.validate(
  pgrdf.graph_id('urn:test/tg4/data'),
  pgrdf.graph_id('urn:test/tg4/shapes')
) ->> 'conforms') AS i4_alice_conforms;

-- Add a second subject of ex:knows with a custom-scheme IRI and no age.
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle(
    '@prefix ex: <http://example.org/> .
     <ckp://Person#charlie> ex:knows ex:dave .',
    pgrdf.graph_id('urn:test/tg4/data'));
END $$;

SELECT (pgrdf.validate(
  pgrdf.graph_id('urn:test/tg4/data'),
  pgrdf.graph_id('urn:test/tg4/shapes')
) ->> 'conforms') AS i4_post_conforms;

SELECT count(*)::int AS i4_charlie_focus
FROM jsonb_array_elements(
  pgrdf.validate(
    pgrdf.graph_id('urn:test/tg4/data'),
    pgrdf.graph_id('urn:test/tg4/shapes')
  ) -> 'results'
) r
WHERE r ->> 'focusNode' = 'ckp://Person#charlie';

-- ─── I-5 — CONSTRUCT round-trip preserves lexicals exact ──────
DO $$
BEGIN
  PERFORM pgrdf.put_construct_rows(
    array(
      SELECT j FROM pgrdf.construct(
        'PREFIX ex: <http://example.org/>
         CONSTRUCT { ex:alice ?p ?o }
         WHERE    { GRAPH <urn:test/tg4/data> { ex:alice ?p ?o } }'
      ) j
    ),
    pgrdf.graph_id('urn:test/tg4/roundtrip')
  );
END $$;

-- Round-trip: read back the integer lexical from the new graph.
-- Must be byte-identical to the source ("30" + xsd:integer).
SELECT (j->'object'->>'value') AS i5e_integer_lexical_rt
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:age ?o }
   WHERE    { GRAPH <urn:test/tg4/roundtrip> { ex:alice ex:age ?o } }'
) AS s(j);

SELECT (j->'object'->>'datatype') AS i5e_integer_datatype_rt
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:age ?o }
   WHERE    { GRAPH <urn:test/tg4/roundtrip> { ex:alice ex:age ?o } }'
) AS s(j);

-- Round-trip: lang-tagged literal preserves BOTH datatype and language
SELECT (j->'object'->>'datatype') AS i5g_lang_datatype_rt
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:greeting ?o }
   WHERE    { GRAPH <urn:test/tg4/roundtrip> { ex:alice ex:greeting ?o } }'
) AS s(j);

SELECT (j->'object'->>'language') AS i5g_lang_tag_rt
FROM pgrdf.construct(
  'PREFIX ex: <http://example.org/>
   CONSTRUCT { ex:alice ex:greeting ?o }
   WHERE    { GRAPH <urn:test/tg4/roundtrip> { ex:alice ex:greeting ?o } }'
) AS s(j);

-- Dictionary de-dup: exactly one row per distinct term across graphs.
-- The round-trip MUST NOT create near-duplicates from encoding drift.

SELECT count(*)::int AS i5_dict_alice
FROM pgrdf._pgrdf_dictionary
WHERE lexical_value = 'http://example.org/alice'
  AND term_type = 1;  -- term_type::URI

SELECT count(*)::int AS i5_dict_ckp_task
FROM pgrdf._pgrdf_dictionary
WHERE lexical_value = 'ckp://Task#001'
  AND term_type = 1;

SELECT count(*)::int AS i5_dict_hi_en
FROM pgrdf._pgrdf_dictionary
WHERE lexical_value = 'Hi'
  AND term_type = 3  -- term_type::LITERAL
  AND language_tag = 'en';

ROLLBACK;
