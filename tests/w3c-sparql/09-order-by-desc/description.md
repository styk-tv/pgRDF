# 09-order-by-desc

W3C SPARQL 1.1 §15.1 — `ORDER BY DESC(?v)`. Four names sorted
descending: Diana > Charlie > Bob > Alice (lexicographic).

**Order matters here** — the harness sorts both sides
lexicographically before diff, so it tests the SET of returned rows
not the order. For a true ordering test you'd inspect the raw
output stream. This case is still useful as a "doesn't drop or
duplicate any row" check; the per-row content is what's verified.

Hand-computed expected output (post-sort):

```jsonl
{"n": "Alice"}
{"n": "Bob"}
{"n": "Charlie"}
{"n": "Diana"}
```
