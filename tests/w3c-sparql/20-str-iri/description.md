# 20-str-iri

W3C SPARQL 1.1 §17.4.2.5 — `STR(t)` returns the lexical form of
the term. For a NamedNode the lexical form is the IRI string. Used
here to compare an IRI-bound variable against a literal IRI as a
string.

Hand-computed expected output:

```jsonl
{"s": "http://example.com/alice", "hp": "http://alice.example.com/"}
```
