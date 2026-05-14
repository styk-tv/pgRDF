# 19-bound-after-optional

W3C SPARQL 1.1 §17.4.1.7 — `BOUND(?v)` returns true iff `?v` is
bound in the current solution. Combined with `OPTIONAL`, the
`!BOUND(?m)` filter selects rows where the OPTIONAL block did
NOT match — i.e. people **without** a mbox.

Hand-computed expected output:

```jsonl
{"n": "Bob"}
```
