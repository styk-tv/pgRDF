# 29-update-with-graph-scope

W3C SPARQL 1.1 Update §3.1.3 paragraph 3 — `WITH <g>` clause. When
neither the WHERE nor the template carries an explicit `GRAPH <g>`
block, `WITH <g>` selects `<g>` as the default graph for BOTH the
WHERE pattern evaluation AND the template's quad routing. The
spargebra desugar emits a `using: QueryDataset { default: [<g>],
named: None }` sentinel on the operation plus a per-quad
`graph_name = <g>` on every default-graph template triple; the
executor lifts the IRI out, wraps the WHERE pattern in
`GraphPattern::Graph(<g>, …)` for evaluation, and lets the
per-quad routing already carry the template side (slice 79).

Provenance:

- W3C SPARQL 1.1 Update §3.1.3 — WITH clause semantics.
- LLD v0.4 §4.3 — graph-scoped UPDATE variants table.
- Landed-in slice 79 (`feat(sparql): UPDATE graph-scoped variants
  WITH/GRAPH`). Slice 79's `tests/regression/sql/98-update-graph-
  scoped.sql` locks the cross-graph routing matrix; this W3C-shape
  fixture exercises the same surface through the conformance harness.

The fixture seeds three triples in `<http://example.org/store>` via
`setup.sql`, then runs:

```sparql
WITH <http://example.org/store>
INSERT { ?s ex:tagged "yes" }
WHERE  { ?s ex:hasPrice ?p }
```

Hand computation:

1. WHERE evaluates inside `store` → three solutions
   (`?s = item1`, `item2`, `item3`).
2. Template instantiates three triples (one per solution), all of
   shape `?s ex:tagged "yes"`.
3. WITH scopes the template's default-graph quads to `store`, so all
   three rows land in `store`.

Expected `_update` summary:

```jsonl
{"_update": {"form": "INSERT_WHERE", "elapsed_ms": 0, "graphs_touched": ["http://example.org/store"], "triples_deleted": 0, "triples_inserted": 3}}
```

## Why a `setup.sql` and not `data.ttl`

WITH-scoped UPDATE is only observable when the target graph is a
specific named graph (not the harness's auto-allocated one). The
`setup.sql` path lets us materialise `store` with `pgrdf.add_graph`
and then seed via `parse_turtle(content, pgrdf.graph_id(iri))`, so
the IRI/partition binding is stable across runs and the
`graphs_touched` array can be hand-asserted.
