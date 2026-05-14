# 03-union-disjoint

W3C SPARQL 1.1 §18.2.4 — `UNION` produces the multiset union of
its branch solutions. Variables bound by one branch but not the
other appear as **unbound (null)** in the rows contributed by the
other branch.

Hand-computed expected output:

```jsonl
{"s": "http://example.com/alice", "n": "Alice",                       "m": null}
{"s": "http://example.com/bob",   "n": null,    "m": "mailto:bob@example.com"}
```
