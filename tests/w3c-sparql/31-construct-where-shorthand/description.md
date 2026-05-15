# 31-construct-where-shorthand

W3C SPARQL 1.1 Query §16.2.4 — the CONSTRUCT WHERE shorthand. When
the template and the WHERE pattern are identical, the query MAY be
abbreviated:

```
CONSTRUCT WHERE { ?s ?p ?o }
```

is exactly equivalent to the explicit form

```
CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }
```

The shorthand is restricted by the spec to a single basic graph
pattern (no GRAPH, no OPTIONAL, no blank nodes in the template). This
fixture locks the shorthand-equals-explicit equivalence at the
W3C-shape (conformance harness) level — the engine-side equivalence
is locked in `tests/regression/sql/105-construct-where-shorthand.sql`.

Provenance:

- W3C SPARQL 1.1 Query §16.2.4 (CONSTRUCT WHERE shorthand).
- LLD v0.4 §6 — WHERE-shorthand row (slice 54).

## Seed + hand-computed expected

Two default-graph triples:

```
ex:alice ex:knows ex:bob
ex:alice ex:age   30          (xsd:integer per Turtle bare-integer rule)
```

`CONSTRUCT WHERE { ?s ?p ?o }` reflects every matched triple back
out verbatim — two solutions, one template triple each, two emitted
rows. Hand-computed (sorted lexicographically by the runner):

```jsonl
{"object": {"type": "iri", "value": "http://example.org/bob"}, "subject": {"type": "iri", "value": "http://example.org/alice"}, "predicate": {"type": "iri", "value": "http://example.org/knows"}}
{"object": {"type": "literal", "value": "30", "datatype": "http://www.w3.org/2001/XMLSchema#integer"}, "subject": {"type": "iri", "value": "http://example.org/alice"}, "predicate": {"type": "iri", "value": "http://example.org/age"}}
```

This expected output was independently verified to be byte-identical
between the shorthand `CONSTRUCT WHERE { ?s ?p ?o }` and the explicit
`CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }` forms, which is the
§16.2.4 equivalence this fixture exists to lock.

## Why `kind: construct`

See `run.sh` header (Phase D slice 51) — CONSTRUCT routes through
`pgrdf.construct`, not `pgrdf.sparql`.
