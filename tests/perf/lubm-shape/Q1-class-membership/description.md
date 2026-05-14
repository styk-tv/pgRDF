# Q1 — class membership

Pre-inference selectivity check. Counts direct
`rdf:type lubm:GraduateStudent` assertions. The real LUBM Q1 is
similar shape over the full ABox.

Hand-computed expected output (pre-inference):

```jsonl
{"s": "http://www.University0.edu/Alice"}
{"s": "http://www.University1.edu/Eve"}
```

After `pgrdf.materialize`, this query would also return undergraduates'
super-class entailments — but at the surface form here we don't
materialise; that's a separate Q (post-materialize coverage).
