# 21-numeric-filter

W3C SPARQL 1.1 §17.3 — numeric ordering operators on typed
literals. Filter keeps subjects where `xsd:integer` age is in
`[25, 42)`. Alice (30) and Bob (25) pass; Carol (42) fails the
upper bound; Dan (17) fails the lower bound.

Hand-computed expected output:

```jsonl
{"n": "Alice"}
{"n": "Bob"}
```

This test also exercises the **numeric-aware translator path**
(`expr_to_numeric_sql` in `src/query/executor.rs`): variables
resolved through a sub-SELECT on `_pgrdf_dictionary` cast to
NUMERIC iff the row's `datatype_iri_id` is in the xsd-numeric set.
Non-numeric rows yield NULL on the comparison and are dropped
per SPARQL "type error → unbound" semantics.
