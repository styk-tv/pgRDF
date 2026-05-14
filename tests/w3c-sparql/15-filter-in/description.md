# 15-filter-in

W3C SPARQL 1.1 §17.4.1.9 — `FILTER(?v IN (a, b, c))`. Three
documents pass the filter: two reports + one draft. Memos drop.

Hand-computed expected output:

```jsonl
{"s": "http://example.com/doc1", "t": "report"}
{"s": "http://example.com/doc3", "t": "report"}
{"s": "http://example.com/doc4", "t": "draft"}
```
