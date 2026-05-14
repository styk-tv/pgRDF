# 11-bind-concat

W3C SPARQL 1.1 §10.1 — `BIND(expr AS ?v)` adds a synthetic
variable to each solution. Uses `CONCAT(s1, s2, s3)` from §17.4.3.2
to join given + family names into a full-name binding.

Hand-computed expected output:

```jsonl
{"full": "Alice Anderson"}
{"full": "Bob Brown"}
```
