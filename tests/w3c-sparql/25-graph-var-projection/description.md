# 25-graph-var-projection

W3C SPARQL 1.1 §13.3 — `GRAPH ?g { ... }` variable form. The graph
slot is a variable, so the BGP is matched against EVERY named graph
in the dataset and `?g` is bound to each graph's IRI (NOT its
integer `graph_id`).

Provenance:

- W3C SPARQL 1.1 Query §13.3 (named-graph clause, variable form).
- LLD v0.4 §3.3 — `GRAPH ?g { … }` row.
- Landed-in **slice 113** (`feat(sparql): GRAPH ?g { ... } variable
  form translation`). Slice 113 runs in parallel with this slice
  (111) in a sibling worktree; this fixture is authored at slice 111
  but **first goes green when both 111 + 113 merge to main**. Until
  113 lands, the executor panics on this query with the stable
  `sparql: GRAPH ?g { ... } (variable form) not yet supported — see
  slice 113` prefix (locked by
  `tests/regression/sql/80-unsupported-shapes.sql` gap-4).

The fixture's `setup.sql` mirrors test 24 — two named graphs with
one `ex:name` triple each:

```
http://example.org/g1  →  ex:alice ex:name "Alice in g1"
http://example.org/g2  →  ex:bob   ex:name "Bob in g2"
```

The query projects both `?g` and `?name`. Spec-correct result: two
rows, one per graph, with `?g` bound to the **IRI** (`NamedNode`-
shaped JSONB term, materialised as a JSON string in this fixture's
shaped output).

Hand-computed expected output (lexicographically sorted by the
harness):

```jsonl
{"g": "http://example.org/g1", "name": "Alice in g1"}
{"g": "http://example.org/g2", "name": "Bob in g2"}
```

A regression that incorrectly projected the integer `graph_id`
instead of the IRI would surface rows like `{"g": "1", ...}` and
fail the diff. A regression that scoped only one graph would surface
a single row.

## Parallel-slice scheduling note

This fixture's authoring slice (111) and its dependency slice (113)
are in flight simultaneously. The parent agent cherry-picks both
into main; running `bash tests/w3c-sparql/run.sh 25-graph-var-projection`
before slice 113 lands will FAIL with the slice-113 stable error
prefix — that's the expected pre-merge state.
