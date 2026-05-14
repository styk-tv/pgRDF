# 07-aggregates-count

W3C SPARQL 1.1 §11 — `COUNT(?o)` aggregate with `GROUP BY ?s`.
Friend-count per subject. Three subjects with 2/1/2 outgoing knows
edges.

Hand-computed expected output (ORDER BY ?s gives a stable order):

```jsonl
{"s": "http://example.com/alice", "friends": "2"}
{"s": "http://example.com/bob",   "friends": "1"}
{"s": "http://example.com/carol", "friends": "2"}
```

Note: aggregate output is stringified as the value comes back via
JSONB; that's the pgRDF convention documented in `docs/03-query.md`.
