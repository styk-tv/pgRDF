# 10-limit-offset

W3C SPARQL 1.1 §15.3 — `LIMIT` / §15.2 `OFFSET`. Skip the first
two solutions by `ORDER BY ?s` (so `i1`, `i2` skipped) then take 2
(`i3`, `i4`).

Hand-computed expected output:

```jsonl
{"s": "http://example.com/i3"}
{"s": "http://example.com/i4"}
```
