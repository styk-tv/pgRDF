# 14-filter-regex

W3C SPARQL 1.1 §17.4.3.14 — `REGEX(?v, "pattern")`. Pattern
`^A` matches names starting with `A` — Alice, Andrew, Anya. Bob
is filtered out.

Hand-computed expected output:

```jsonl
{"n": "Alice"}
{"n": "Andrew"}
{"n": "Anya"}
```
