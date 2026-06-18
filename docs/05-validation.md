# 05 — Validation

> **Status: real SHACL Core (v0.4) + the three-way `mode` argument
> and the W3C SHACL Core manifest gate (v0.5).** The SQL surface
> `pgrdf.validate(data BIGINT, shapes BIGINT, mode TEXT DEFAULT
> 'native') → JSONB` is a real W3C-shape validator returning an
> `sh:ValidationReport`. `mode` selects the engine: `'native'`
> (rudof Core, full-pass), `'sparql'` (rudof `SparqlEngine` — as of
> `shacl 0.3.2` upstream-incomplete, **not recommended**), and
> `'pgrdf'` (pgRDF-native SHACL-SPARQL, the authoritative path for
> `sh:sparql` constraints). The v0.3 stub is gone. The Core manifest
> gate invariant + 25 / 25 full-pass are tracked in
> [`specs/ERRATA.v0.5.md`](../specs/ERRATA.v0.5.md) **E-013**; the
> SHACL-SPARQL correctness gap that makes `'pgrdf'` authoritative is
> tracked in [`specs/ERRATA.v0.6.md`](../specs/ERRATA.v0.6.md)
> **E-014**.

## Surface

```sql
SELECT pgrdf.validate(data_graph_id, shapes_graph_id [, mode]);
--   data_graph_id   — graph containing the assertions to validate
--   shapes_graph_id — graph containing the SHACL shapes (sh:NodeShape /
--                     sh:PropertyShape) those assertions must satisfy
--   mode            — TEXT DEFAULT 'native'; one of
--                     {'native','sparql','pgrdf'}.
--                     'native' is the rudof Core engine (the v0.4
--                     surface, unchanged — the 2-arg form defaults
--                     here). 'sparql' dispatches to rudof's
--                     SparqlEngine, which as of shacl 0.3.2 is
--                     upstream-incomplete (ERRATA.v0.6 E-014) and not
--                     recommended. 'pgrdf' is the pgRDF-native
--                     SHACL-SPARQL engine — the authoritative path for
--                     sh:sparql / sh:select constraints.
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

- `'native'` — the rudof SHACL Core engine. The default; the
  2-arg `pgrdf.validate(d, s)` form is byte-identical to v0.4.
- `'sparql'` — dispatches to rudof's `SparqlEngine`. As of
  `shacl 0.3.2` the engine is functional in places but
  upstream-incomplete: common SHACL-SPARQL topologies silently
  return `conforms:true` with 0 violations even though the
  constraint compiled into the IR (ERRATA.v0.6 **E-014**). **Not
  recommended** — use `'pgrdf'` for SHACL-SPARQL.
- `'pgrdf'` — the pgRDF-native SHACL-SPARQL engine. It evaluates
  each `sh:sparql` / `sh:select` constraint directly against the
  dictionary-indexed hexastore (no N-Triples rehydrate) and returns
  the correct `conforms` verdict on the W3C SHACL-SPARQL suite where
  `'sparql'` returns the wrong one. The authoritative SHACL-SPARQL
  path (ERRATA.v0.6 **E-014**).
- Any other value → `validate: unknown mode "<x>" (supported:
  'native', 'sparql', 'pgrdf')`, raised **before** any work (no
  silent fallback — mirrors `materialize: unknown profile`).

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
   &validation_mode)` runs the requested rudof engine: `'native'`
   (Core) or `'sparql'` (SparqlEngine; upstream-incomplete per
   E-014). The `'pgrdf'` mode takes a separate in-house path that
   evaluates each `sh:sparql` constraint directly against the
   dictionary-indexed hexastore rather than rehydrating into rudof.
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
- Three engines: `'native'` (rudof Core, in-process), `'sparql'`
  (rudof SparqlEngine — upstream-incomplete as of `shacl 0.3.2`,
  E-014, not recommended), and `'pgrdf'` (the pgRDF-native
  SHACL-SPARQL engine, the authoritative path for `sh:sparql` /
  `sh:select` constraints).
- Validation against materialised graphs works via the rehydrate's
  `is_inferred` inclusivity: `pgrdf.materialize` then
  `pgrdf.validate` validates the entailed closure (regression
  `122-shacl-modes.sql` §E + the W3C SHACL Core gate below).

## W3C SHACL Core manifest gate (v0.5)

`just test-shacl-manifest` runs a vendored, hermetic subset of the
W3C `data-shapes-test-suite` SHACL **Core** tests
(`tests/w3c-shacl/`, structured like `tests/w3c-sparql/`), wired
into CI on every PG major (14-17) as a real gate. The vendored Core
suite is a genuine **full-pass — 25 / 25** on the W3C `sh:conforms`
invariant; per ERRATA.v0.5 **E-013** the gate compares `conforms`
(not the violation *count*, which drifts ±1 from pgRDF's
blank-node-relabelling dictionary rehydrate — a serialization
artifact that does not flip conformance). There is no excluded Core
fixture: `prop-nodeKind-001` is graded in `fixtures/core/` and passes
with the W3C-authoritative `conforms:false` result. `just
test-shacl-manifest --sparql` exercises the rudof `'sparql'` engine,
whose divergent SHACL-SPARQL verdicts (vs the authoritative
`'pgrdf'` engine) are tracked as ERRATA.v0.6 **E-014**.

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
  manifest gate (25/25, `conforms` invariant; E-013).

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
  [`specs/SPEC.pgRDF.LLD.v0.5.md`](../specs/SPEC.pgRDF.LLD.v0.5.md)
  §5 / §6 (the `mode` arg + the W3C SHACL Core gate, shipped in v0.5.0)
- ERRATA: [`E-011`](../specs/ERRATA.v0.4.md) — fork patch +
  unblock vehicle.
- ERRATA: [`E-013`](../specs/ERRATA.v0.5.md) — W3C SHACL Core
  manifest gate invariant (25/25 full-pass).
- ERRATA: [`E-014`](../specs/ERRATA.v0.6.md) — the rudof `'sparql'`
  SHACL-SPARQL engine is upstream-incomplete (`shacl 0.3.2`); the
  native `mode => 'pgrdf'` engine is the authoritative path.
- ERRATA: [`E-013`](../specs/ERRATA.v0.5.md) — the W3C SHACL Core
  gate `sh:conforms` invariant + the final 25/25 full-pass.
- ERRATA: [`E-009`](../specs/ERRATA.v0.2.md) — original
  dep-block, now resolved.
