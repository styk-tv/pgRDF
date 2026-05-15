# 34-construct-typed-literals

RDF 1.1 §3.3 literal term shaping through a CONSTRUCT variable
template. A WHERE-bound `?v` resolved through the dictionary into the
structured-term shape (LLD v0.4 §6.1) must preserve:

- a typed literal's datatype IRI verbatim (`xsd:integer`), and
- a language-tagged literal's `language` tag PLUS the implicit
  `rdf:langString` datatype (RDF 1.1 §3.3 — every lang-tagged literal
  has datatype `rdf:langString`).

```
CONSTRUCT { ?s ex:copied ?v }
WHERE     { ?s ?p ?v }
```

Provenance:

- RDF 1.1 Concepts §3.3 (typed + language-tagged literals).
- W3C SPARQL 1.1 Query §16.2 (CONSTRUCT term substitution).
- LLD v0.4 §6.1 — structured-term object cell (`type`, `value`,
  `datatype`, `language`). Engine-side term-shaper invariants:
  `tests/regression/sql/101-construct-variable-templates.sql` and
  the round-trip preservation in `106-construct-round-trip.sql`
  invariants B (typed) and C (lang-tagged).

## Seed + hand-computed expected

Two default-graph triples on the same subject:

```
ex:widget ex:weight "42"^^xsd:integer
ex:widget ex:label  "Le Widget"@fr
```

`CONSTRUCT { ?s ex:copied ?v }` rebinds the predicate to `ex:copied`
and passes each object through unchanged. Two solutions → two rows.
Hand-computed (sorted lexicographically by the runner):

```jsonl
{"object": {"type": "literal", "value": "42", "datatype": "http://www.w3.org/2001/XMLSchema#integer"}, "subject": {"type": "iri", "value": "http://example.org/widget"}, "predicate": {"type": "iri", "value": "http://example.org/copied"}}
{"object": {"type": "literal", "value": "Le Widget", "datatype": "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString", "language": "fr"}, "subject": {"type": "iri", "value": "http://example.org/widget"}, "predicate": {"type": "iri", "value": "http://example.org/copied"}}
```

Note the lang-tagged row carries BOTH `language: "fr"` AND
`datatype: rdf:langString` — this is the RDF 1.1 §3.3 contract the
construct emitter obeys (cross-checked by
`106-construct-round-trip.sql` invariant C).

## Why `kind: construct`

See `run.sh` header (Phase D slice 51).
