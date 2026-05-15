# 27-update-insert-data

W3C SPARQL 1.1 Update §3.1.1 — `INSERT DATA` form. The simplest UPDATE:
a static ground-triple block, no `WHERE` clause, lands rows into the
target graph(s). This fixture covers both surfaces in a single query:

1. A bare triple goes into the **default graph** (`graph_id = 0`).
2. A triple wrapped in `GRAPH <iri> { … }` lands into the **named
   graph** identified by that IRI (pre-allocated in `setup.sql` via
   `pgrdf.add_graph`).

The harness invokes `pgrdf.sparql(<query>)`. For UPDATE forms the
function returns a single summary row of shape
`{"_update": { form, elapsed_ms, graphs_touched, triples_deleted, triples_inserted }}`
per LLD v0.4 §4.2. `elapsed_ms` is non-deterministic; `run.sh`
normalises it to `0` before diffing.

Provenance:

- W3C SPARQL 1.1 Update §3.1.1 (INSERT DATA).
- LLD v0.4 §4.2 — `_update` summary row shape.
- Landed-in slice 84 (`feat(sparql): UPDATE foundation + INSERT DATA`).
  Slice 84's `tests/regression/sql/93-update-insert-data.sql` locks
  the SQL-side invariants (table-level row landing, idempotency,
  named-graph auto-allocation); this W3C-shape fixture exercises the
  same surface through the conformance harness.

The fixture's `setup.sql` pre-allocates `http://example.org/g1` so the
query's `GRAPH` clause has a partition ready and the test does not
race against the auto-allocate path (which is locked separately in
the regression file).

Hand-computed expected output (one `_update` summary row, two triples
inserted, both `DEFAULT` and the named graph touched):

```jsonl
{"_update": {"form": "INSERT_DATA", "elapsed_ms": 0, "graphs_touched": ["DEFAULT", "http://example.org/g1"], "triples_deleted": 0, "triples_inserted": 2}}
```

## Why a `setup.sql` and not `data.ttl`

UPDATE-form fixtures populate state via the query itself. The default
`data.ttl + parse_turtle` path would pre-stage rows the query is
trying to LAND, blurring whether the UPDATE actually worked or merely
re-touched pre-existing rows. The slice-77 fixture set keeps the
data.ttl path absent and uses `setup.sql` only for the necessary
preconditions (here: the IRI binding for `g1`).
