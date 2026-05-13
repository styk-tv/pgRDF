# 05 — Validation

`SELECT pgrdf.validate(data_graph BIGINT, shapes_graph BIGINT) → JSONB`
validates `data_graph` against the SHACL shape graph in `shapes_graph`
and returns a W3C `sh:ValidationReport` as JSONB.

## SHACL crate (per ERRATA E-001)

We use `shacl_validation` (crates.io). The v0.2 LLD references
`shacl-rust`, which does not exist as a production-grade crate. The
alternative is `oxirs-shacl`; we'll benchmark both before v0.3 and
pick a default. Until then, validation is plumbed against
`shacl_validation`.

## Output shape

The returned JSONB conforms to the SHACL Core ValidationReport vocab:

```json
{
  "conforms": false,
  "results": [
    {
      "focusNode": "http://example.com/Alice",
      "resultPath": "http://xmlns.com/foaf/0.1/age",
      "resultSeverity": "Violation",
      "sourceConstraintComponent": "MinCountConstraintComponent",
      "sourceShape": "http://example.com/PersonShape",
      "resultMessage": "Property age must have at least 1 value"
    }
  ]
}
```

Consumers can filter with standard Postgres JSONB operators:

```sql
SELECT pgrdf.validate(1, 2) -> 'conforms';
SELECT jsonb_array_elements(pgrdf.validate(1, 2) -> 'results')
WHERE … -> 'resultSeverity' = 'Violation';
```

## v0.2.0 scope

- ✅ SHACL Core node + property shapes
- ✅ Cardinality, value-type, value-range constraints
- ⏳ SHACL-SPARQL constraints — Phase 3
- ⏳ Custom Rust validators registered as `sh:JSConstraint` analogues
  — out of scope for v0 series.
