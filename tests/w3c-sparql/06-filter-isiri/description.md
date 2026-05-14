# 06-filter-isiri

W3C SPARQL 1.1 §17.4.2.1 — `isIRI(t)` returns true iff the term
is a NamedNode. The filter keeps only the `ex:alice foaf:knows ex:bob`
triple; the two foaf:name triples have literal objects and are
filtered out.

Hand-computed expected output:

```jsonl
{"s": "http://example.com/alice", "o": "http://example.com/bob"}
```
