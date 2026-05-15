# 32-construct-constant-template

W3C SPARQL 1.1 Query §16.2 — solution-sequence multiplicity for a
pure constant template. The template carries no variables and no
blank nodes:

```
CONSTRUCT { ex:dataset ex:hasState "loaded" }
WHERE     { ?s ex:p ?o }
```

Per §16.2, "the result is an RDF graph formed by taking each query
solution in the solution sequence, substituting … and combining the
triples into a single RDF graph by set union." The construct UDF
(slice 59) emits one row per (solution, template-triple) pair BEFORE
the set-union dedup that an ingest step would apply — so N solutions
over a constant template emit N identical rows. This fixture locks
that the constant template is emitted once per solution (multiplicity
preserved at the row-emission boundary), matching the slice-59
foundation contract in
`tests/regression/sql/100-construct-foundation.sql` invariant B.

Provenance:

- W3C SPARQL 1.1 Query §16.2 (solution-sequence → graph).
- LLD v0.4 §6 — constant-only template row (slice 59).

## Seed + hand-computed expected

Three default-graph triples share predicate `ex:p`, so the WHERE
`{ ?s ex:p ?o }` yields three solutions. The constant template
`{ ex:dataset ex:hasState "loaded" }` substitutes nothing, so each
solution emits the same triple → three identical rows. The bare
string literal `"loaded"` carries `xsd:string` (slice 59's emitter
writes the datatype explicitly). Hand-computed:

```jsonl
{"object": {"type": "literal", "value": "loaded", "datatype": "http://www.w3.org/2001/XMLSchema#string"}, "subject": {"type": "iri", "value": "http://example.org/dataset"}, "predicate": {"type": "iri", "value": "http://example.org/hasState"}}
{"object": {"type": "literal", "value": "loaded", "datatype": "http://www.w3.org/2001/XMLSchema#string"}, "subject": {"type": "iri", "value": "http://example.org/dataset"}, "predicate": {"type": "iri", "value": "http://example.org/hasState"}}
{"object": {"type": "literal", "value": "loaded", "datatype": "http://www.w3.org/2001/XMLSchema#string"}, "subject": {"type": "iri", "value": "http://example.org/dataset"}, "predicate": {"type": "iri", "value": "http://example.org/hasState"}}
```

Three lines — the runner's bag-equivalent sort keeps all three (it
sorts, it does not dedup), so the multiplicity is observable.

## Why `kind: construct`

See `run.sh` header (Phase D slice 51).
