# 08-aggregates-having

W3C SPARQL 1.1 §11.5 — `HAVING` filters groups by aggregate
expression. Per-category price totals: books=40, food=10,
tools=25. Filter keeps only groups with SUM > 15 → books + tools.

**Note on translator surface.** pgRDF currently translates HAVING
by **aggregate alias** (`HAVING(?total > 15)` where `?total` is
the projected SUM alias). The inline form
`HAVING(SUM(?p) > 15)` — equally valid SPARQL 1.1 syntax — is
not yet supported and would need additional aggregate-expression
machinery in the translator. Tracked as a v0.4 SPARQL surface
extension.

Hand-computed expected output:

```jsonl
{"cat": "books", "total": "40"}
{"cat": "tools", "total": "25"}
```
