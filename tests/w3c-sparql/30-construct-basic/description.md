# 30-construct-basic

W3C SPARQL 1.1 Query §16.2.1 — the canonical CONSTRUCT example. A
template with a blank node `_:v` joining two triples per solution:

```
CONSTRUCT { ?s vcard:N _:v . _:v vcard:givenName ?gname }
WHERE     { ?s foaf:name ?gname }
```

Exercises three template surfaces together — a variable (`?s`,
`?gname`), a blank node (`_:v`, fresh-per-solution, shared across the
two template triples within the solution per W3C §16.2's blank-node
scoping), and a multi-triple template.

Provenance:

- W3C SPARQL 1.1 Query §16.2.1 (the worked CONSTRUCT example).
- LLD v0.4 §6 — variable + blank-node + multi-triple template rows
  (slices 58 / 57 / 56).
- Engine-side invariants locked by
  `tests/regression/sql/101-construct-variable-templates.sql`,
  `102-construct-blank-node-templates.sql`,
  `103-construct-multi-triple-templates.sql`. This W3C-shape fixture
  cross-checks the same surface through the conformance harness on
  the canonical spec example.

## Single-solution seed — deterministic blank-node label

The seed has exactly ONE `foaf:name` triple (`ex:alice "Alice"`).
CONSTRUCT mints a fresh blank-node label per solution; with a single
solution the emitter mints exactly one label (`b1_1`), so the
bag-equivalent (sorted-line) diff against `expected.jsonl` is stable
across runs. A multi-solution seed would mint per-solution labels
(`b1_1`, `b2_1`, …) whose assignment tracks the unordered solution
sequence — not a stable target for a hand-authored expected file.
The within-solution `_:v` join (object of triple 1 == subject of
triple 2) is the load-bearing invariant and is fully observable with
one solution.

Hand-computed expected output (W3C §16.2.1 semantics; sorted
lexicographically by the runner):

```jsonl
{"object": {"type": "bnode", "value": "b1_1"}, "subject": {"type": "iri", "value": "http://example.org/alice"}, "predicate": {"type": "iri", "value": "http://www.w3.org/2001/vcard-rdf/3.0#N"}}
{"object": {"type": "literal", "value": "Alice", "datatype": "http://www.w3.org/2001/XMLSchema#string"}, "subject": {"type": "bnode", "value": "b1_1"}, "predicate": {"type": "iri", "value": "http://www.w3.org/2001/vcard-rdf/3.0#givenName"}}
```

## Why `kind: construct`

CONSTRUCT emits triple rows (`{subject,predicate,object}`), not
solution bindings. `pgrdf.sparql` only translates SELECT / ASK (a
CONSTRUCT through it panics `sparql: query form not supported yet`),
so this fixture carries a `kind` file containing `construct`, which
routes `query.rq` through `pgrdf.construct` instead. See
`run.sh` header (Phase D slice 51).
