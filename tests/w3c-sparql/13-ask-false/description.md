# 13-ask-false

W3C SPARQL 1.1 §16.2 — `ASK` returns false when no solution
exists. The data has only a `foaf:name` triple; no `foaf:knows`
match is possible.

Hand-computed expected output:

```jsonl
{"_ask": "false"}
```
