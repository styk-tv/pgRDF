# 17-lang-tag

W3C SPARQL 1.1 §17.4.2.4 — `LANG(?lit)`. Returns the language
tag of a literal as an unmarked string. Pure data RDF preserves
the tag through the dictionary.

Hand-computed expected output:

```jsonl
{"t": "Bonjour", "lang": "fr"}
{"t": "Hello",   "lang": "en"}
{"t": "Hola",    "lang": "es"}
```
