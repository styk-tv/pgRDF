# 06 — Validation recipes (SHACL Core, SHACL-SPARQL, mode selection)

`pgrdf.validate(data_graph_id BIGINT, shapes_graph_id BIGINT, mode TEXT DEFAULT 'native') → JSONB`
checks data against SHACL shapes and returns a W3C-shaped
`sh:ValidationReport`. **The `mode` argument picks the engine**:

| Mode | Engine | Use for | Status |
|---|---|---|---|
| `'native'` (default) | rudof Core engine | SHACL Core constraints (`sh:minCount`, `sh:datatype`, `sh:nodeKind`, `sh:class`, etc.) | ✅ W3C Core full-pass |
| `'sparql'` | rudof SparqlEngine | SHACL-SPARQL (`sh:sparql / sh:select`) — **not recommended; upstream gaps** | ⚠ E-014 |
| `'pgrdf'` | pgRDF-native | SHACL-SPARQL on real data sizes; better correctness than `'sparql'` on the W3C suite | ✅ v0.5.7 |

The 2-arg call `pgrdf.validate(d, s)` defaults `mode => 'native'` and
is behaviourally identical to the v0.4 surface.

## When to use which mode

### `'native'` — every plain SHACL Core shape

```sql
SELECT pgrdf.validate(g_data, g_shapes);
```

If your shapes use only Core constraint components — `sh:minCount`,
`sh:maxCount`, `sh:datatype`, `sh:class`, `sh:nodeKind`,
`sh:pattern`, `sh:minLength`, `sh:in`, `sh:hasValue`, etc. — the
default `'native'` mode is the right choice. It's the rudof Core
engine, hand-implemented per constraint, and pgRDF carries a
full-pass on the vendored W3C SHACL Core conformance suite
(25 / 25, see `tests/w3c-shacl/fixtures/core/`).

### `'pgrdf'` — SHACL-SPARQL constraints

```sql
SELECT pgrdf.validate(g_data, g_shapes, 'pgrdf');
```

The moment your shapes carry a `sh:sparql / sh:select` block — a
SHACL Part 2 SPARQL-based constraint — switch to `'pgrdf'`.
Two reasons:

1. **Correctness.** On the W3C SHACL-SPARQL conformance suite
   (`tests/w3c-shacl/fixtures/sparql/`), pgRDF-native returns the
   right `conforms` verdict where rudof's `'sparql'` mode returns
   the wrong one — see [ERRATA.v0.6 E-014](../specs/ERRATA.v0.6.md).
2. **Performance.** `'pgrdf'` mode evaluates each SPARQL constraint
   directly against the dictionary-indexed hexastore. It does NOT
   serialise the data graph to N-Triples and rehydrate into an
   in-memory copy (which `'sparql'` mode does). At LUBM-10 / 100
   scale this is the difference between O(seconds) and O(many
   minutes).

### `'sparql'` — present but currently NOT the recommended path

```sql
-- Only useful when you specifically want to compare against the
-- rudof SparqlEngine output. NOT the right choice for production.
SELECT pgrdf.validate(g_data, g_shapes, 'sparql');
```

`'sparql'` mode dispatches into rudof's `SparqlEngine`. As of
`shacl 0.3.2`, the engine is functional in places but
upstream-incomplete — common SHACL-SPARQL shape topologies (e.g.
W3C `tests/sparql/node/sparql-001.ttl`) silently return
`conforms=true` with 0 violations even though the constraint is
compiled into the IR. Tracked as ERRATA.v0.6 E-014.

For production SHACL-SPARQL validation, use `'pgrdf'`.

## Worked example — SSN uniqueness (SHACL-SPARQL)

A common SHACL-SPARQL pattern: assert that every `ex:Employee` has
a unique social security number. SHACL Core has no built-in
uniqueness constraint, so this is a textbook SHACL-SPARQL case.

Data graph — three employees, two of whom collide on SSN:

```sql
SELECT pgrdf.add_graph(9001);
SELECT pgrdf.parse_turtle($$
@prefix ex: <http://example.org/> .
ex:alice a ex:Employee ; ex:ssn "123-45-6789" .
ex:bob   a ex:Employee ; ex:ssn "555-12-3456" .
ex:carol a ex:Employee ; ex:ssn "123-45-6789" .
$$, 9001);
```

Shapes graph — `ex:EmployeeShape` carries a SHACL-SPARQL
constraint that returns a row whenever two distinct employees
share an SSN, binding `?this` to the focus node and `?value`
to the conflicting SSN:

```sql
SELECT pgrdf.add_graph(9002);
SELECT pgrdf.parse_turtle($$
@prefix ex: <http://example.org/> .
@prefix sh: <http://www.w3.org/ns/shacl#> .

ex:EmployeeShape a sh:NodeShape ;
  sh:targetClass ex:Employee ;
  sh:sparql [ a sh:SPARQLConstraint ;
              sh:message "SSN must be unique across Employees" ;
              sh:select """SELECT $this ?value WHERE {
                  $this ex:ssn ?value .
                  ?other ex:ssn ?value .
                  FILTER ($this != ?other)
              }""" ] .
$$, 9002);
```

Validate under `'pgrdf'`:

```sql
SELECT pgrdf.validate(9001, 9002, 'pgrdf');
```

Expected JSONB (truncated):

```json
{
  "conforms": false,
  "mode": "pgrdf",
  "results": [
    {
      "focusNode": "http://example.org/alice",
      "sourceConstraintComponent": "http://www.w3.org/ns/shacl#SPARQLConstraintComponent",
      "resultMessage": "SSN must be unique across Employees",
      "resultSeverity": "sh:Violation"
    },
    {
      "focusNode": "http://example.org/carol",
      "sourceConstraintComponent": "http://www.w3.org/ns/shacl#SPARQLConstraintComponent",
      "resultMessage": "SSN must be unique across Employees",
      "resultSeverity": "sh:Violation"
    }
  ],
  "shapes_triples": 6,
  "elapsed_ms": 4.2
}
```

Two violations: alice and carol both fire because each finds an
`?other` Employee with the same SSN.

## Decision matrix

| Your shapes contain… | Recommended mode |
|---|---|
| Only Core constraints (no `sh:sparql`) | `'native'` (or omit the arg) |
| `sh:sparql` / `sh:select` SPARQL-based constraints | `'pgrdf'` |
| A mix of Core + SPARQL-based constraints | `'pgrdf'` — it evaluates SPARQL constraints natively and falls through to Core via the shared IR |
| You want a side-by-side rudof-side comparison | `'sparql'` — diagnostic only |

## What `'pgrdf'` mode evaluates

The pgRDF-native handler intercepts only `IRComponent::BasicSparql`
constraints (the SHACL Part 2 `sh:sparql / sh:select` vocabulary).
For each such constraint it:

1. Resolves the shape's targets against the data graph via direct
   SPI scans of `_pgrdf_quads` + `_pgrdf_dictionary`. Five target
   forms are supported: `sh:targetNode`, `sh:targetClass`,
   `sh:targetSubjectsOf`, `sh:targetObjectsOf`, and implicit
   `rdfs:Class` targets.
2. For each focus node, rewrites the SPARQL `sh:select` text by
   replacing `$this` with the synthetic variable `?_pgrdf_this`
   and injecting `VALUES ?_pgrdf_this { <focus-iri> }` at the head
   of the WHERE clause. This is the SHACL Part 2 §5.2 pre-binding
   semantics, made explicit so the rewritten query is a
   self-contained SPARQL SELECT runnable through `pgrdf.sparql`.
3. Dispatches the rewritten query through `pgrdf.sparql` — the
   same dictionary-indexed hexastore path that powers
   `pgrdf.sparql` and `pgrdf.construct`. The plan cache reuses
   the prepared SQL plan across the focus-node iteration.
4. Maps each result row to a `sh:ValidationResult` JSONB with
   `sourceConstraintComponent = sh:SPARQLConstraintComponent`.

## Performance notes

- **No `InMemoryGraph` rehydrate.** `'pgrdf'` mode reads the data
  graph through SPI; it never serialises to N-Triples and reparses
  into an in-memory copy. `'sparql'` and `'native'` modes do
  rehydrate.
- **Plan cache reuse.** The rewritten SPARQL is parameterised over
  the focus-IRI VALUES binding; the same SQL plan serves every
  focus node in a target set. Big O(N) wins on `sh:targetClass`
  shapes against large data graphs.
- **No worker pool needed.** Per-shape evaluation is sequential
  inside one `pgrdf.validate` call; the per-focus SPI scans
  individually leverage PostgreSQL's existing parallel-query
  machinery where the query planner enables it.

## Limitations to know about

Today, `'pgrdf'` mode handles the most common SHACL-SPARQL
patterns. Items NOT yet handled (Track A v0.6+ work):

- `FILTER NOT EXISTS { … }` and `Not(Exists(_))` — surfaced during
  TH-7; the SPARQL executor doesn't yet translate this expression.
  Workaround: use `OPTIONAL { … } FILTER (!BOUND(?var))` (same
  semantics, supported today).
- `(expr AS ?var)` in the SELECT projection — supported in
  `pgrdf.construct` but not yet uniformly across `pgrdf.sparql`.
- The `$PATH` pre-bound variable (analogous to `$this` for
  property shapes) — TH-9 substitutes only `$this` for now.

These limitations apply to the SPARQL query body itself; SHACL's
constraint dispatch (target resolution, `$this` binding,
result-row mapping) is fully implemented.

## See also

- [`03-querying.md`](03-querying.md) — SPARQL surface that
  `'pgrdf'` mode dispatches through
- [`specs/SPEC.pgRDF.LLD.v0.5.md`](../specs/SPEC.pgRDF.LLD.v0.5.md)
  §5 — the authoritative `pgrdf.validate` contract
- [`specs/ERRATA.v0.6.md`](../specs/ERRATA.v0.6.md) §E-014 — the
  rudof `'sparql'` gap and the pgRDF-native disposition
