# 05-minus-no-shared

W3C SPARQL 1.1 §8.3.2 — `MINUS` with **no shared variables** is
a no-op: every base solution survives. Even though `ex:place` IS
a `ex:Place` in the data, `?other` ≠ `?s`, so the MINUS predicate
trivially evaluates to false and nothing is removed.

(pgRDF's translator detects this case during parse — `?s` and
`?other` share no variable — and elides the MINUS entirely from
the generated SQL. The result must still be both rows.)

Hand-computed expected output:

```jsonl
{"n": "Alice"}
{"n": "Bob"}
```
