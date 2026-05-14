# 22-having-inline-aggregate

W3C SPARQL 1.1 §11.5 — `HAVING` with an **inline aggregate
expression** (`HAVING(SUM(?p) > 15)`), as opposed to the
HAVING-by-alias form covered by test `08-aggregates-having`.

Companion to test 08:
- `08` uses `HAVING(?total > 15)` — references the aggregate's
  AS-alias (`?total`).
- `22` uses `HAVING(SUM(?p) > 15)` — re-states the aggregate
  inline.

Both are valid SPARQL 1.1 forms; spargebra synthesises a fresh
intermediate variable for the inline form's reference. pgRDF's
translator keeps the synthetic name on `AggregateSpec.synth_aliases`
so the HAVING filter can match the aggregate by either the
user-facing alias OR the original synth name.

Hand-computed expected output (after ORDER BY ?cat):

```jsonl
{"cat": "books", "total": "40"}
{"cat": "tools", "total": "25"}
```

food (sum=10) is filtered out.
