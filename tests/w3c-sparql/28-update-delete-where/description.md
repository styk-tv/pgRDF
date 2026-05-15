# 28-update-delete-where

W3C SPARQL 1.1 Update §3.1.3 — `DELETE WHERE { … }` form. A
pattern-driven delete: every solution of the WHERE clause (here, a
single BGP) yields a concrete triple to remove. This is the
shorthand for `DELETE { ?s ex:age ?o } WHERE { ?s ex:age ?o }` — the
template equals the pattern.

Provenance:

- W3C SPARQL 1.1 Update §3.1.3 (DELETE WHERE shorthand).
- LLD v0.4 §4.2 — `_update` summary row with `form: "DELETE_WHERE"`.
- Landed-in slice 81 (`feat(sparql): UPDATE DELETE WHERE pattern-
  driven`). Slice 81's `tests/regression/sql/96-update-delete-
  where.sql` locks the SQL-side invariants (per-graph router, BGP
  solution iteration, dictionary id reuse); this W3C-shape fixture
  cross-checks the same surface via `pgrdf.sparql()`.

The fixture seeds four triples in the default graph (two with
`ex:age`, two with `ex:name`). The query deletes the two `ex:age`
rows; the `_update` summary reports
`triples_deleted = 2, triples_inserted = 0, graphs_touched = ["DEFAULT"]`.

Hand-computed expected output:

```jsonl
{"_update": {"form": "DELETE_WHERE", "elapsed_ms": 0, "graphs_touched": ["DEFAULT"], "triples_deleted": 2, "triples_inserted": 0}}
```

## Why a `setup.sql` and not `data.ttl`

DELETE WHERE is observable only against pre-existing state. The
`setup.sql` path is the cleanest way to surface the seeded rows
without ambiguity over which graph_id the harness's default seed
would have used (the test name → graph_id hash is fine for SELECT
queries but adds noise to UPDATE assertions where the touched-graphs
list is part of the contract).
