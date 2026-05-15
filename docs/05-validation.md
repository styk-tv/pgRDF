# 05 — Validation

> **Status: real SHACL Core (v0.4).** The SQL surface
> `pgrdf.validate(data BIGINT, shapes BIGINT) → JSONB` ships as a
> real W3C-shape SHACL Core validator backed by the rudof project's
> `shacl 0.3.x` crate. The v0.3 stub is gone. The upstream unblock
> is tracked in [`specs/ERRATA.v0.4.md`](../specs/ERRATA.v0.4.md)
> **E-011** (which supersedes [`E-009`](../specs/ERRATA.v0.2.md)).

## Surface

```sql
SELECT pgrdf.validate(data_graph_id, shapes_graph_id);
--   data_graph_id   — graph containing the assertions to validate
--   shapes_graph_id — graph containing the SHACL shapes (sh:NodeShape /
--                     sh:PropertyShape) those assertions must satisfy
-- Returns JSONB:
--   {
--     "conforms":        <bool>,
--     "results":         [ ValidationResult, ... ],
--     "data_graph_id":   <i64>,
--     "shapes_graph_id": <i64>,
--     "data_triples":    <i64>,
--     "shapes_triples":  <i64>,
--     "elapsed_ms":      <f64>
--   }
```

Each `ValidationResult` element is:

```json
{
  "focusNode":      "<iri-or-bnode-or-literal-encoded>",
  "resultPath":     "<iri-or-null>",
  "sourceShape":    "<iri-or-bnode-or-null>",
  "resultMessage":  "<string-or-null>",
  "resultSeverity": "sh:Violation|sh:Warning|sh:Info|sh:Trace|sh:Debug",
  "value":          "<term-encoded-or-null>",
  "sourceConstraintComponent": "<iri>"
}
```

`conforms` is `true` iff `results` is empty, mirroring W3C
`sh:conforms`. A degenerate report whose shapes graph names no
targets is vacuously conforming; missing graphs (zero triples)
follow the same rule.

## Example

```sql
-- Data graph: Alice missing required ex:age, Bob complete.
SELECT pgrdf.add_graph(8971);
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:alice a foaf:Person ;
         foaf:name "Alice" .
ex:bob a foaf:Person ;
       foaf:name "Bob" ;
       ex:age "30"^^xsd:integer .
', 8971);

-- Shapes graph: PersonShape requires foaf:name and ex:age.
SELECT pgrdf.add_graph(8972);
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.org/> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass foaf:Person ;
    sh:property [ sh:path foaf:name ; sh:minCount 1 ; sh:datatype xsd:string ] ;
    sh:property [ sh:path ex:age   ; sh:minCount 1 ; sh:datatype xsd:integer ] .
', 8972);

SELECT jsonb_pretty(pgrdf.validate(8971, 8972));
```

Yields:

```json
{
  "conforms": false,
  "results": [{
    "focusNode": "http://example.org/alice",
    "resultPath": "http://example.org/age",
    "sourceShape": "_:b…",
    "resultMessage": "MinCount(1) not satisfied",
    "resultSeverity": "sh:Violation",
    "sourceConstraintComponent": "http://www.w3.org/ns/shacl#MinCountConstraintComponent",
    "value": null
  }],
  "data_graph_id": 8971,
  "shapes_graph_id": 8972,
  "data_triples": 5,
  "shapes_triples": 10,
  "elapsed_ms": 1.68
}
```

Bob is silent because he conforms.

## Pipeline

`src/validation/shacl.rs` walks the same dictionary-join shape as
`pgrdf.materialize`:

1. **Rehydrate** — `_pgrdf_quads` JOIN `_pgrdf_dictionary` for
   both data + shapes graphs; one SPI scan each. Both base and
   inferred rows are included, so `pgrdf.materialize` followed by
   `pgrdf.validate` validates the entailed closure.
2. **Serialise** — `oxttl::NTriplesSerializer` writes each graph
   to N-Triples text in memory.
3. **Parse** — `rudof_rdf::InMemoryGraph::from_str(…, NTriples)`
   re-loads each graph in rudof's shape.
4. **Compile shapes** — `shacl::ShaclDataManager::load(…)`
   compiles the shapes graph into a SHACL `IRSchema`.
5. **Validate** — `GraphValidation::new(data).validate(&schema,
   &ShaclValidationMode::Native)` runs the Native engine.
6. **Shape** — the resulting `ValidationReport.results()` maps to
   the JSONB shape above. Severities normalise to the canonical
   `sh:` constants; literals render Turtle-ish.

Validation is in-process. There is no external SPARQL endpoint,
no external file IO, and the whole pipeline runs inside the
calling Postgres transaction.

## Scope

- SHACL Core — `sh:NodeShape` + `sh:PropertyShape` + the standard
  Core constraint components (cardinality, value-type, value-range,
  string, property-pair, logical, shape-based).
- Native engine (in-process). The `Sparql` engine in `shacl 0.3`
  is wired but not exposed at the SQL boundary today; v0.5 may
  add a third positional arg.
- Validation against materialised graphs works today via the
  rehydrate's `is_inferred` inclusivity (no explicit `materialize`
  call inside `validate`).

### Out of scope (v0.4)

- Custom JavaScript validators (`sh:js`).
- SHACL Advanced Features (`sh:rule`, `sh:targetSubjectsOf`, etc.) —
  follows whatever `shacl 0.3.x` supports upstream; not promised.
- RDF-star / RDF 1.2 quoted triples as focus nodes. The fork's
  `rdf-12` passthrough adds a `TermRef::Triple(_)` arm that
  panics rather than reasons — consistent with `reasonable`'s own
  RDF-star posture.

## Regression coverage

- [`tests/regression/sql/70-validate-stub.sql`](../tests/regression/sql/70-validate-stub.sql) —
  basic shape (vacuously conforming + unknown-graph degenerate
  cases). Filename retained for diff-friendly history.
- [`tests/regression/sql/71-shacl-real.sql`](../tests/regression/sql/71-shacl-real.sql) —
  LLD §9 acceptance: `sh:NodeShape` + `sh:property` + `sh:datatype`
  with a non-conforming focus node (Alice).

Plus three `#[pg_test]` integration tests in
`src/validation/shacl.rs::tests` (conforming, violations, unknown
graphs).

## Unblock vehicle

The v0.4 unblock landed via:

1. **rudof 0.3.1 (2026-05-12)** consolidating `shacl_ast` +
   `shacl_validation` 0.2.x into a single `shacl 0.3.1` crate,
   closing the `iri_s` → `rudof_iri` migration half of
   [E-009](../specs/ERRATA.v0.2.md).
2. **styk-tv/reasonable fork branch `rdf12-passthrough`** adding a
   passthrough `rdf-12` feature + a `TermRef::Triple(_)` arm so
   `shacl 0.3` (which hard-enables `rdf-12` via `rudof_rdf`) can
   coexist with `reasonable` in one workspace. See
   [`specs/ERRATA.v0.4.md`](../specs/ERRATA.v0.4.md) E-011.

The fork is wired via `[patch.crates-io]` in
[`Cargo.toml`](../Cargo.toml). Once `gtfierro/reasonable` merges
the upstream PR, drop the patch and pin the released `reasonable`
version (the `features = ["rdf-12"]` opt-in stays).

## See also

- Implementation: [`src/validation/shacl.rs`](../src/validation/shacl.rs)
- Spec: [`specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`](../specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md) §9
- ERRATA: [`E-011`](../specs/ERRATA.v0.4.md) — fork patch +
  unblock vehicle.
- ERRATA: [`E-009`](../specs/ERRATA.v0.2.md) — original
  dep-block, now resolved.
