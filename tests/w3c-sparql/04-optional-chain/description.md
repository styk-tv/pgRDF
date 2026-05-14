# 04-optional-chain

W3C SPARQL 1.1 §6 — `OPTIONAL` patterns. The base BGP binds
`?n` for every person. The OPTIONAL block extends each solution
with `?m` where available; rows where the OPTIONAL fails to match
keep `?m` unbound (null).

Hand-computed expected output:

```jsonl
{"n": "Alice", "m": "mailto:alice@example.com"}
{"n": "Bob",   "m": null}
{"n": "Carol", "m": "mailto:carol@example.com"}
```
