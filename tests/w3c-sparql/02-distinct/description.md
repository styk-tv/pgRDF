# 02-distinct

W3C SPARQL 1.1 §15.4 — `SELECT DISTINCT` reduces the multiset
of solutions to a set. Three input subjects share two distinct
names (`Alice` x2, `Bob` x1); the result has exactly two rows.

Hand-computed expected output:

```jsonl
{"n": "Alice"}
{"n": "Bob"}
```
