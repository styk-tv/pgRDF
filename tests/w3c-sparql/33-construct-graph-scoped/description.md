# 33-construct-graph-scoped

W3C SPARQL 1.1 Query §13.3 — variable-GRAPH binding inside a
CONSTRUCT's WHERE, with the bound graph IRI flowing into the
template:

```
CONSTRUCT { ?s ex:from ?g }
WHERE     { GRAPH ?g { ?s ?p ?o } }
```

`GRAPH ?g { … }` ranges over the **named** graphs only — the default
graph is NOT a member of the named-graph set per §13.3. So a
default-graph triple in the dataset must NOT contribute a solution.
The bound `?g` (the named-graph IRI) is then projected into the
template's object position.

Provenance:

- W3C SPARQL 1.1 Query §13.3 (GRAPH variable form; named graphs only).
- LLD v0.4 §6 — GRAPH-scoped WHERE for CONSTRUCT (slice 55).
- Engine-side invariants:
  `tests/regression/sql/104-construct-graph-scoped-where.sql`.

## Seed + hand-computed expected

`setup.sql` seeds two named graphs and one default-graph triple:

```
http://example.org/g1  →  ex:alice ex:name "Alice"
http://example.org/g2  →  ex:bob   ex:name "Bob"
DEFAULT (graph 0)      →  ex:carol ex:name "Carol"
```

`GRAPH ?g { ?s ?p ?o }` matches the g1 and g2 triples (binding `?g`
to each graph IRI) but NOT the default-graph `ex:carol` triple
(§13.3 — the default graph is not in the named-graph set). Two
solutions → two template rows. A regression that incorrectly
included the default graph would surface a third row binding
`?s = ex:carol`, failing the bag-equivalent diff. Hand-computed
(sorted lexicographically by the runner):

```jsonl
{"object": {"type": "iri", "value": "http://example.org/g1"}, "subject": {"type": "iri", "value": "http://example.org/alice"}, "predicate": {"type": "iri", "value": "http://example.org/from"}}
{"object": {"type": "iri", "value": "http://example.org/g2"}, "subject": {"type": "iri", "value": "http://example.org/bob"}, "predicate": {"type": "iri", "value": "http://example.org/from"}}
```

## Why a `setup.sql` and not `data.ttl`

The default harness path loads `data.ttl` into ONE hashed graph id —
it cannot express the multi-named-graph + default-graph dataset this
§13.3 scoping fixture needs. `run.sh` (slice 111) accepts `setup.sql`
as the sole input source when `data.ttl` is absent.

## Why `kind: construct`

See `run.sh` header (Phase D slice 51).
