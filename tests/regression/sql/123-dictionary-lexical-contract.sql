-- 123-dictionary-lexical-contract.sql
--
-- TF-5 — per CX-002 EVAL recommendation
-- (_WIP/RESPONSE.pgRDF.v0.5.0-by-CX-002.md). Locks the **dictionary
-- surface** against lexical drift: every term shape pgRDF stores
-- (URI / blank node / plain literal / typed literal / lang-tagged
-- literal) must round-trip EXACT lexical bytes through parse_turtle
-- → dictionary → pgrdf.construct, including the pathological cases
-- that cheap "trim whitespace" or "uppercase scheme" implementations
-- would silently mangle.
--
-- TF-4 / TG-4 (`124-end-to-end-lexical-rehydration.sql`) covers the
-- full **pipeline** (parse_turtle → CONSTRUCT → materialize → ...);
-- this file is the narrower **dictionary contract** — does a single
-- ingest + read-back preserve every byte we promised to preserve?
--
-- Read-back shape: `pgrdf.construct(q)` returns SETOF jsonb with one
-- row per (solution, template-triple) pair; each row carries
-- `{subject:{type,value}, predicate:{type,value},
--   object:{type,value,datatype?,language?}}` per the contract in
-- src/query/executor.rs:280-286. The SELECT-mode `pgrdf.sparql(q)`
-- only exposes `{varname → scalar-string}` (executor.rs:6453-6459),
-- so it carries the lexical value but NOT the (term_type, datatype,
-- language) metadata the dictionary contract requires asserting.
-- We use `pgrdf.construct(...)` throughout to get the full term shape
-- via `object.{type,value,datatype,language}` in one call.
--
-- Term-shape coverage (positive assertions):
--   A. IRI http://     — http://example.org/alice
--   B. IRI urn:        — urn:example:bob
--   C. IRI custom+frag — ckp://Task#001 (pgCK case)
--   D. IRI percent-enc — http://example.org/path%20with%20spaces
--   E. Plain literal   — "Alice"           (xsd:string)
--   F. xsd:integer     — "0030"            (lexical "0030" — NO zero-strip)
--   G. xsd:boolean     — "true"
--   H. xsd:dateTime    — "2026-05-29T12:00:00Z" (TZ preserved verbatim)
--   I. Lang-tagged     — "Hello"@en        (lang preserved verbatim)
--   J. Lang-tagged     — "Salut"@fr-CA     (region subtag preserved)
--   K. Literal w/ quote — string carrying embedded \"
--   L. Literal w/ tab   — string carrying embedded \\t
--   M. Unicode literal  — "üñîçødé"        (UTF-8 round-trip)
--   N. Blank node       — _:b0
--
-- Invariants:
--   I-1  every shape is recoverable by EXACT lexical bytes via
--        `pgrdf.construct(... ?o ...)` → `object.value`
--   I-2  the (term_type, datatype, language) tuple stored in the
--        dictionary matches the input form (no normalization) and
--        re-emerges via `object.{type,datatype,language}`
--   I-3  zero-padded integers ("0030") are NOT silently stripped
--        to "30" — pgRDF preserves the AS-WRITTEN lexical
--   I-4  lang-tag subtags (`en`, `fr-CA`) round-trip with case
--        preserved AS-WRITTEN by the input Turtle
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- ─── Bind a clean graph for the lexical-contract corpus ────────
DO $$
BEGIN
  PERFORM pgrdf.add_graph('urn:test/tf5/dict-lexical-contract');
END $$;

-- Ingest one triple per shape (subject is always a clean IRI; the
-- coverage interest is the OBJECT term).
DO $$
BEGIN
  PERFORM pgrdf.parse_turtle($ttl$
    @prefix ex:  <http://example.org/> .
    @prefix ckp: <ckp://> .
    @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

    # A: http:// IRI in object position via owl:sameAs
    ex:s1 ex:eqIri <http://example.org/alice> .

    # B: urn: IRI
    ex:s2 ex:eqIri <urn:example:bob> .

    # C: custom-scheme IRI with fragment (pgCK / ckp:// case)
    ex:s3 ex:eqIri <ckp://Task#001> .

    # D: percent-encoded IRI
    ex:s4 ex:eqIri <http://example.org/path%20with%20spaces> .

    # E: plain literal (xsd:string)
    ex:s5 ex:lex "Alice" .

    # F: zero-padded integer — pgRDF MUST NOT strip the leading zeros
    ex:s6 ex:lex "0030"^^xsd:integer .

    # G: boolean literal
    ex:s7 ex:lex "true"^^xsd:boolean .

    # H: dateTime with explicit timezone
    ex:s8 ex:lex "2026-05-29T12:00:00Z"^^xsd:dateTime .

    # I: lang-tagged literal, single subtag
    ex:s9 ex:lex "Hello"@en .

    # J: lang-tagged literal, region subtag
    ex:sA ex:lex "Salut"@fr-CA .

    # K: literal carrying an embedded backslash-quote
    ex:sB ex:lex "say \"hi\"" .

    # L: literal carrying an embedded tab
    ex:sC ex:lex "tab\there" .

    # M: Unicode (UTF-8) literal
    ex:sD ex:lex "üñîçødé" .

    # N: blank-node OBJECT — verify _:label round-trips
    ex:sE ex:hasNote _:note0 .
    _:note0 ex:lex "blank-node anchored" .
  $ttl$, pgrdf.graph_id('urn:test/tf5/dict-lexical-contract'));
END $$;

-- ─── A — http:// IRI object ─────────────────────────────────────
SELECT (j->'object'->>'value') = 'http://example.org/alice' AS a_http_iri
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:eqIri ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s1 ex:eqIri ?o } }
$q$) AS j;

-- ─── B — urn: IRI object ───────────────────────────────────────
SELECT (j->'object'->>'value') = 'urn:example:bob' AS b_urn_iri
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:eqIri ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s2 ex:eqIri ?o } }
$q$) AS j;

-- ─── C — custom-scheme IRI with fragment ───────────────────────
SELECT (j->'object'->>'value') = 'ckp://Task#001' AS c_custom_scheme_iri
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:eqIri ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s3 ex:eqIri ?o } }
$q$) AS j;

-- ─── D — percent-encoded IRI ───────────────────────────────────
SELECT (j->'object'->>'value') = 'http://example.org/path%20with%20spaces' AS d_percent_encoded_iri
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:eqIri ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s4 ex:eqIri ?o } }
$q$) AS j;

-- ─── E — plain literal (Turtle promotes "..." to xsd:string per §2.5)
SELECT (j->'object'->>'value') = 'Alice' AS e_plain_literal
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s5 ex:lex ?o } }
$q$) AS j;

-- ─── F — typed literal "0030"^^xsd:integer (NO zero-strip) ─────
-- Invariant I-3: pgRDF preserves the AS-WRITTEN lexical.
SELECT (j->'object'->>'value') = '0030' AS f_integer_no_zero_strip
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s6 ex:lex ?o } }
$q$) AS j;

-- Datatype IRI on the same term must be exactly xsd:integer.
SELECT (j->'object'->>'datatype') = 'http://www.w3.org/2001/XMLSchema#integer' AS f_integer_datatype
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s6 ex:lex ?o } }
$q$) AS j;

-- ─── G — typed literal boolean ─────────────────────────────────
SELECT (j->'object'->>'value') = 'true' AS g_boolean_value
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s7 ex:lex ?o } }
$q$) AS j;

-- ─── H — dateTime with timezone (verbatim) ─────────────────────
SELECT (j->'object'->>'value') = '2026-05-29T12:00:00Z' AS h_datetime_tz_verbatim
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s8 ex:lex ?o } }
$q$) AS j;

-- ─── I — lang-tagged literal, single subtag ────────────────────
SELECT (j->'object'->>'value') = 'Hello' AS i_lang_value
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s9 ex:lex ?o } }
$q$) AS j;

SELECT (j->'object'->>'language') = 'en' AS i_lang_tag
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s9 ex:lex ?o } }
$q$) AS j;

-- ─── J — lang-tagged literal, region subtag ────────────────────
-- Invariant I-4: subtag case-as-written should round-trip.
SELECT (j->'object'->>'value') = 'Salut' AS j_region_value
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sA ex:lex ?o } }
$q$) AS j;

SELECT (j->'object'->>'language') = 'fr-CA' AS j_region_tag
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sA ex:lex ?o } }
$q$) AS j;

-- ─── K — literal with embedded quote ───────────────────────────
SELECT (j->'object'->>'value') = 'say "hi"' AS k_embedded_quote
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sB ex:lex ?o } }
$q$) AS j;

-- ─── L — literal with embedded tab ─────────────────────────────
SELECT (j->'object'->>'value') = E'tab\there' AS l_embedded_tab
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sC ex:lex ?o } }
$q$) AS j;

-- ─── M — Unicode literal ───────────────────────────────────────
SELECT (j->'object'->>'value') = 'üñîçødé' AS m_unicode
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:lex ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sD ex:lex ?o } }
$q$) AS j;

-- ─── N — blank-node object: type sentinel + followthrough ──────
-- The blank-node label that pgRDF emits will NOT be the input
-- `note0` (Turtle scopes blanks to the file; the parser is free to
-- relabel, and CONSTRUCT may also rename). We assert the type is
-- "bnode" — the dictionary recorded the term shape — and that the
-- followthrough literal anchored to the bnode survives the round
-- trip.
SELECT (j->'object'->>'type') = 'bnode' AS n_bnode_type
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?s ex:hasNote ?o }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sE ex:hasNote ?o } }
$q$) AS j;

SELECT (j->'object'->>'value') = 'blank-node anchored' AS n_bnode_followthrough
FROM pgrdf.construct($q$
  PREFIX ex: <http://example.org/>
  CONSTRUCT { ?b ex:lex ?lex }
  WHERE    { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sE ex:hasNote ?b . ?b ex:lex ?lex } }
$q$) AS j;

ROLLBACK;
