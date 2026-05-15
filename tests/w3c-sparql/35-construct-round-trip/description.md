# 35-construct-round-trip

LLD v0.4 §6.3 round-trip acceptance criterion at W3C-shape level:
`pgrdf.construct(q)` followed by re-ingesting the rows via the
slice-53 pair (`pgrdf.put_construct_rows`) produces the same graph
state (modulo non-user-visible dictionary id reshuffles).

Pipeline (constant predicate/subject preserved; IRI + language-tagged
+ typed-literal objects exercised):

```
seed src graph
  → CONSTRUCT { ?s ?p ?o } WHERE { GRAPH <src> { ?s ?p ?o } }
  → pgrdf.put_construct_rows(..., dst)          [in setup.sql]
  → CONSTRUCT { ?s ?p ?o } WHERE { GRAPH <dst> { ?s ?p ?o } }   [query.rq]
```

Provenance:

- W3C SPARQL 1.1 Query §16.2 (CONSTRUCT graph result).
- LLD v0.4 §6.3 acceptance criterion (round-trip preservation).
- Engine-side invariants A–J (bidirectional EXCEPT equivalence,
  typed/lang preservation, bnode within-solution joining,
  idempotency, empty-set, single-row primitive, reject paths):
  `tests/regression/sql/106-construct-round-trip.sql`.

## Harness limitation

The W3C runner executes exactly ONE `query.rq` through
`pgrdf.construct`; it has no two-query "assert dst == src" shape. So
the construct + re-ingest leg runs in `setup.sql` and `query.rq`
re-queries the destination graph. `expected.jsonl` is the
**re-queried destination state**, hand-computed to equal the source
seed reflected through CONSTRUCT — if the round-trip lost a term,
dropped the `xsd:integer` datatype, or mangled the `@en` tag, the
re-queried rows would diverge from this expected file and the
bag-equivalent diff would fail. The stronger bidirectional
set-equivalence + idempotency assertions live in the engine-side
regression `106-construct-round-trip.sql` (this is the conformance
cross-check, not a replacement for it).

## Seed + hand-computed expected

Source graph `http://example.org/src`:

```
ex:a ex:p ex:b                 (IRI object)
ex:b ex:q "hello"@en           (language-tagged literal)
ex:c ex:r "7"^^xsd:integer     (typed literal)
```

After construct → put_construct_rows → re-query of `dst`, the three
triples come back intact (sorted lexicographically by the runner):

```jsonl
{"object": {"type": "iri", "value": "http://example.org/b"}, "subject": {"type": "iri", "value": "http://example.org/a"}, "predicate": {"type": "iri", "value": "http://example.org/p"}}
{"object": {"type": "literal", "value": "hello", "datatype": "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString", "language": "en"}, "subject": {"type": "iri", "value": "http://example.org/b"}, "predicate": {"type": "iri", "value": "http://example.org/q"}}
{"object": {"type": "literal", "value": "7", "datatype": "http://www.w3.org/2001/XMLSchema#integer"}, "subject": {"type": "iri", "value": "http://example.org/c"}, "predicate": {"type": "iri", "value": "http://example.org/r"}}
```

## Why `kind: construct` + `setup.sql`

CONSTRUCT routes through `pgrdf.construct` (see `run.sh` header,
Phase D slice 51). `setup.sql` is the sole input source — the
round-trip leg needs multiple statements the single-`query.rq` path
cannot carry.
