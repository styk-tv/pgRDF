# 26-graph-var-groupby

W3C SPARQL 1.1 §13.3 (`GRAPH ?g { ... }`) composed with §11 (group-
by + aggregates) and §15.1 (`ORDER BY ?g`). Confirms that:

1. `?g` projects as the **IRI** (not the integer `graph_id`).
2. `GROUP BY ?g` keys on the IRI value — one group per distinct
   named graph in the dataset.
3. `COUNT(*)` over the inner BGP `?s ?p ?o` returns the triple count
   per group.
4. `ORDER BY ?g` returns rows in IRI-lexicographic order — the
   harness's bag-equivalence sort would mask order errors otherwise,
   but the spec-mandated ORDER BY semantics still need to hold; the
   query's authored order matches the lex-sorted output here, so
   both invariants are visible.

Provenance:

- W3C SPARQL 1.1 Query §13.3 (named-graph clause, variable form).
- W3C SPARQL 1.1 Query §11 (aggregates + GROUP BY).
- W3C SPARQL 1.1 Query §15.1 (ORDER BY).
- LLD v0.4 §3.3 acceptance criterion 2:
  > `SELECT ?g (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } }
  > GROUP BY ?g` groups by IRI; `?g` projects as a `NamedNode` JSONB
  > term, not as an integer.
- Landed-in **slice 113** (variable-form `GRAPH ?g` translation).
  Authored at slice 111; first green once both 111 + 113 are on
  main.

The fixture's `setup.sql` populates:

```
http://example.org/g1  →  ex:a ex:p "p1", ex:b ex:p "p2", ex:c ex:p "p3"   (3 triples)
http://example.org/g2  →  ex:d ex:p "p4", ex:e ex:p "p5"                    (2 triples)
```

Hand-computed expected output:

```jsonl
{"g": "http://example.org/g1", "n": "3"}
{"g": "http://example.org/g2", "n": "2"}
```

(The `"n"` value is captured as a JSON string here because
`pgrdf.sparql` materialises numeric solution terms via the term
shaper, which emits `xsd:integer` as a quoted lexical form — this
matches the convention test 07-aggregates-count uses for its
`COUNT(?v)` output.)
