-- 123-dictionary-lexical-contract.sql
--
-- TF-5 — per CX-002 EVAL recommendation
-- (_WIP/RESPONSE.pgRDF.v0.5.0-by-CX-002.md). Locks the **dictionary
-- surface** against lexical drift: every term shape pgRDF stores
-- (URI / blank node / plain literal / typed literal / lang-tagged
-- literal) must round-trip EXACT lexical bytes through parse_turtle
-- → dictionary → pgrdf.sparql, including the pathological cases
-- that cheap "trim whitespace" or "uppercase scheme" implementations
-- would silently mangle.
--
-- TF-4 / TG-4 (`124-end-to-end-lexical-rehydration.sql`) covers the
-- full **pipeline** (parse_turtle → CONSTRUCT → materialize → ...);
-- this file is the narrower **dictionary contract** — does a single
-- ingest + read-back preserve every byte we promised to preserve?
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
--        `pgrdf.sparql(SELECT ?o ...)`
--   I-2  the (term_type, datatype, lang) tuple stored in the
--        dictionary matches the input form (no normalization)
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
  $ttl$, 'urn:test/tf5/dict-lexical-contract');
END $$;

-- ─── A — http:// IRI object ─────────────────────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s1 ex:eqIri ?o } }
  $q$) -> 'o' ->> 'value')
  = 'http://example.org/alice'
  AS a_http_iri;

-- ─── B — urn: IRI object ───────────────────────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s2 ex:eqIri ?o } }
  $q$) -> 'o' ->> 'value')
  = 'urn:example:bob'
  AS b_urn_iri;

-- ─── C — custom-scheme IRI with fragment ───────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s3 ex:eqIri ?o } }
  $q$) -> 'o' ->> 'value')
  = 'ckp://Task#001'
  AS c_custom_scheme_iri;

-- ─── D — percent-encoded IRI ───────────────────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s4 ex:eqIri ?o } }
  $q$) -> 'o' ->> 'value')
  = 'http://example.org/path%20with%20spaces'
  AS d_percent_encoded_iri;

-- ─── E — plain literal (no datatype tag in the input) ──────────
-- Turtle promotes a bare "..." to xsd:string per W3C §2.5.
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s5 ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = 'Alice'
  AS e_plain_literal;

-- ─── F — typed literal "0030"^^xsd:integer (NO zero-strip) ─────
-- Invariant I-3: pgRDF preserves the AS-WRITTEN lexical.
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s6 ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = '0030'
  AS f_integer_no_zero_strip;

-- Datatype IRI on the same term must be exactly xsd:integer.
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s6 ex:lex ?o } }
  $q$) -> 'o' ->> 'datatype')
  = 'http://www.w3.org/2001/XMLSchema#integer'
  AS f_integer_datatype;

-- ─── G — typed literal boolean ─────────────────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s7 ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = 'true'
  AS g_boolean_value;

-- ─── H — dateTime with timezone (verbatim) ─────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s8 ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = '2026-05-29T12:00:00Z'
  AS h_datetime_tz_verbatim;

-- ─── I — lang-tagged literal, single subtag ────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s9 ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = 'Hello'
  AS i_lang_value;

SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:s9 ex:lex ?o } }
  $q$) -> 'o' ->> 'language')
  = 'en'
  AS i_lang_tag;

-- ─── J — lang-tagged literal, region subtag ────────────────────
-- Invariant I-4: subtag case-as-written should round-trip.
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sA ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = 'Salut'
  AS j_region_value;

SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sA ex:lex ?o } }
  $q$) -> 'o' ->> 'language')
  = 'fr-CA'
  AS j_region_tag;

-- ─── K — literal with embedded quote ───────────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sB ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = 'say "hi"'
  AS k_embedded_quote;

-- ─── L — literal with embedded tab ─────────────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sC ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = E'tab\there'
  AS l_embedded_tab;

-- ─── M — Unicode literal ───────────────────────────────────────
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sD ex:lex ?o } }
  $q$) -> 'o' ->> 'value')
  = 'üñîçødé'
  AS m_unicode;

-- ─── N — blank-node object lexical round-trip ──────────────────
-- The blank-node label that pgRDF stores will NOT be the input
-- `note0` (Turtle scopes blanks to the file; the parser is free to
-- relabel). We assert the type is "bnode" and that the value is a
-- valid blank-node-shaped string (starts with `_:`-stripped form is
-- not what we expose; we just require any non-empty string and a
-- known type sentinel — see pgrdf.sparql contract for the {"type":
-- "bnode"} shape).
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?o WHERE { GRAPH <urn:test/tf5/dict-lexical-contract> { ex:sE ex:hasNote ?o } }
  $q$) -> 'o' ->> 'type')
  = 'bnode'
  AS n_bnode_type;

-- The blank node carries its annotation; locking the followthrough
-- lexical guarantees the blank survived dict + roundtrip.
SELECT
  (pgrdf.sparql($q$
    PREFIX ex: <http://example.org/>
    SELECT ?lex WHERE {
      GRAPH <urn:test/tf5/dict-lexical-contract> {
        ex:sE ex:hasNote ?b . ?b ex:lex ?lex
      }
    }
  $q$) -> 'lex' ->> 'value')
  = 'blank-node anchored'
  AS n_bnode_followthrough;

ROLLBACK;
