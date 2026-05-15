# 24-graph-named-iri

W3C SPARQL 1.1 §13.3 — `GRAPH <iri> { ... }` literal-IRI form. The
query scopes a BGP to a single named graph identified by its absolute
IRI; only triples in that graph are surfaced.

Provenance:

- W3C SPARQL 1.1 Query §13.3 (named-graph clause).
- LLD v0.4 §3.3 — `GRAPH <iri> { … }` row.
- Landed-in slice 114 (`feat(sparql): GRAPH <iri> { ... } literal-form
  translation`). Slice 114's `tests/regression/sql/78-...` already
  locks the SQL-side scoping invariants; this W3C-shape fixture
  cross-checks the same surface through the conformance harness.

The fixture's `setup.sql` populates **two** named graphs:

```
http://example.org/g1  →  ex:alice ex:name "Alice in g1"
http://example.org/g2  →  ex:bob   ex:name "Bob in g2"
```

The query asks for `ex:name` values inside `<http://example.org/g1>`
only. Spec-correct result: exactly the one row binding `?name` to
`"Alice in g1"`. A regression that incorrectly scans both partitions
would surface `"Bob in g2"` as well — failing the bag-equivalence
diff against `expected.jsonl`.

Hand-computed expected output:

```jsonl
{"name": "Alice in g1"}
```

## Why a `setup.sql` and not `data.ttl`

The default harness path loads `data.ttl` into a single hashed graph
id derived from the test name (`graph_id_for()` in `run.sh`). That
path cannot express the multi-graph fixtures §13.3 conformance needs.
Slice 111 extended `run.sh` to:

1. Accept either `data.ttl` or `setup.sql` (or both) as the input
   source — a test dir is now valid if EITHER exists.
2. Run `setup.sql` (when present) after `CREATE EXTENSION pgrdf` and
   before any `data.ttl` parse.
3. Skip the default `add_graph + parse_turtle` step entirely when
   `data.ttl` is missing or empty.

Tests 01–23 remain identical: they have a non-empty `data.ttl` and no
`setup.sql`, so the v0.3 single-graph path is unchanged for them.
