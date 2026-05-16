# 05 — Validation

> **Status: real SHACL Core (v0.4) + the `mode` argument and the
> W3C SHACL Core manifest gate (v0.5).** The SQL surface
> `pgrdf.validate(data BIGINT, shapes BIGINT, mode TEXT DEFAULT
> 'native') → JSONB` is a real W3C-shape SHACL Core validator
> backed by the rudof project's `shacl 0.3.x` crate. The v0.3 stub
> is gone. The v0.4 upstream unblock is tracked in
> [`specs/ERRATA.v0.4.md`](../specs/ERRATA.v0.4.md) **E-011**; the
> v0.5 `mode`-arg scope is tracked in
> [`specs/ERRATA.v0.5.md`](../specs/ERRATA.v0.5.md) **E-012**
> (SHACL-SPARQL upstream stub) and **E-013** (Core manifest gate
> invariant + one excluded fixture).

## Surface

```sql
SELECT pgrdf.validate(data_graph_id, shapes_graph_id [, mode]);
--   data_graph_id   — graph containing the assertions to validate
--   shapes_graph_id — graph containing the SHACL shapes (sh:NodeShape /
--                     sh:PropertyShape) those assertions must satisfy
--   mode            — TEXT DEFAULT 'native'; one of {'native','sparql'}.
--                     'native' is the Rust-native SHACL Core engine
--                     (the v0.4 surface, unchanged — the 2-arg form
--                     defaults here). 'sparql' is wired but the
--                     upstream shacl 0.3.1 SparqlEngine is a stub
--                     (ERRATA.v0.5 E-012): it returns a deterministic
--                     structured "unavailable" report (conforms:null
--                     + an error naming the gap), never a panic.
--                     An unknown mode errors `validate: unknown mode`.
-- Returns JSONB:
--   {
--     "conforms":        <bool|null>,
--     "results":         [ ValidationResult, ... ],
--     "data_graph_id":   <i64>,
--     "shapes_graph_id": <i64>,
--     "data_triples":    <i64>,
--     "shapes_triples":  <i64>,
--     "mode":            "native|sparql",
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
follow the same rule. Under `mode => 'sparql'`, `conforms` is
`null` and an `error` field names the upstream gap (E-012) — the
engine is not invoked.

### The `mode` argument (v0.5)

`pgrdf.validate(data, shapes, mode TEXT DEFAULT 'native')`:

- `'native'` — the Rust-native SHACL Core engine. The default; the
  2-arg `pgrdf.validate(d, s)` form is byte-identical to v0.4.
- `'sparql'` — the SHACL-SPARQL mode. `shacl 0.3.1` has **no**
  SHACL-SPARQL constraint component and its `SparqlEngine` is an
  upstream stub (`unimplemented!()` in every target-resolution
  method). pgRDF does **not** invoke the broken engine (a panic the
  SQL caller cannot act on); `'sparql'` returns a clean,
  deterministic structured report: `conforms:null`, empty
  `results`, `mode:"sparql"`, and an `error` naming the gap +
  ERRATA.v0.5 **E-012**. Forward-compatible: the day a rudof
  release ships the engine, one guard is deleted and the existing
  `&validation_mode` dispatch routes `'sparql'` through with no
  signature change.
- Any other value → `validate: unknown mode "<x>" (supported:
  'native', 'sparql')`, raised **before** any work (no silent
  fallback — mirrors `materialize: unknown profile`).

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
   &validation_mode)` runs the requested engine. `'native'` is the
   in-process Rust engine. `'sparql'` short-circuits to the E-012
   structured report **before** this step (the upstream engine is a
   stub) — steps 5-6 are skipped for `'sparql'`.
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
- `'native'` engine (in-process). The `'sparql'` mode argument
  ships (v0.5) but the upstream `shacl 0.3.1` SPARQL engine is a
  stub (E-012) — `'sparql'` returns the deterministic structured
  report, not a validation. SHACL-SPARQL (`sh:sparql`/`sh:select`)
  constraint components are not parsed by `shacl 0.3.1` at all.
- Validation against materialised graphs works via the rehydrate's
  `is_inferred` inclusivity: `pgrdf.materialize` then
  `pgrdf.validate` validates the entailed closure (regression
  `122-shacl-modes.sql` §E + the W3C SHACL Core gate below).

## W3C SHACL Core manifest gate (v0.5)

`just test-shacl-manifest` runs a vendored, hermetic subset of the
W3C `data-shapes-test-suite` SHACL **Core** tests
(`tests/w3c-shacl/`, structured like `tests/w3c-sparql/`), wired
into CI on every PG major (14-17) as a real gate. The vendored Core
suite is a genuine **full-pass — 24 / 24** on the W3C `sh:conforms`
invariant; per ERRATA.v0.5 **E-013** the gate compares `conforms`
(not the violation *count*, which drifts ±1 from pgRDF's
blank-node-relabelling dictionary rehydrate — a serialization
artifact that does not flip conformance). One W3C Core fixture
(`prop-nodeKind-001`) is documented-excluded for an upstream
`sh:nodeKind` multi-value bug (E-013) and carried to Phase H+I for
the final v0.5.0. `just test-shacl-manifest --sparql` asserts the
E-012 known state (`conforms:null` for every fixture — the upstream
SparqlEngine stub).

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
- [`tests/regression/sql/122-shacl-modes.sql`](../tests/regression/sql/122-shacl-modes.sql) —
  v0.5 §5: mode field + default; unknown-mode error; `'native'`
  ignores a silently-dropped `sh:sparql` block while still flagging
  the Core violation; `'sparql'` structured report; §5.3 #2
  materialised-graph entailment (RDFS profile).
- [`tests/w3c-shacl/`](../tests/w3c-shacl/) — the W3C SHACL Core
  manifest gate (24/24, `conforms` invariant; E-013).

Plus seven `#[pg_test]` integration tests in
`src/validation/shacl.rs::tests` (conforming, violations, unknown
graphs, mode-field-default, unknown-mode-errors, sparql-mode
structured-unavailable, materialised-graph-entailed).

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
- Spec: [`specs/SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md) §9
  (real SHACL Core) +
  [`specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md`](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md)
  §5 / §6 (the `mode` arg + the W3C SHACL Core gate)
- ERRATA: [`E-011`](../specs/ERRATA.v0.4.md) — fork patch +
  unblock vehicle.
- ERRATA: [`E-012`](../specs/ERRATA.v0.5.md) — `shacl 0.3.1`
  SHACL-SPARQL mode is an upstream stub; the `mode` arg ships
  forward-compatible.
- ERRATA: [`E-013`](../specs/ERRATA.v0.5.md) — the W3C SHACL Core
  gate `sh:conforms` invariant + the one excluded fixture.
- ERRATA: [`E-009`](../specs/ERRATA.v0.2.md) — original
  dep-block, now resolved.
