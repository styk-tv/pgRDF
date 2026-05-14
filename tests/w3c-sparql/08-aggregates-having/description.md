# 08-aggregates-having

W3C SPARQL 1.1 §11.5 — `HAVING` filters groups by aggregate
expression. Per-category price totals: books=40, food=10,
tools=25. Filter keeps only groups with SUM > 15 → books + tools.

**Companion test.** `22-having-inline-aggregate` covers the
inline form `HAVING(SUM(?p) > 15)` (without the alias). Both
forms are now supported; the `synth_aliases` track on
`AggregateSpec` lets the HAVING filter migrate by either the
AS-alias OR the synthetic name spargebra emits for the inline
aggregate reference.

Hand-computed expected output:

```jsonl
{"cat": "books", "total": "40"}
{"cat": "tools", "total": "25"}
```
