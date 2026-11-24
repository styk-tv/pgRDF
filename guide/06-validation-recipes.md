# 06 — Validation (SHACL)

`pgrdf.validate()` checks a data graph against a graph of
[SHACL](https://www.w3.org/TR/shacl/) shapes and returns a W3C-style
validation report as JSONB.

```text
pgrdf.validate(data_graph_id  bigint,
               shapes_graph_id bigint,
               mode   text    DEFAULT 'native',
               strict boolean DEFAULT true) → jsonb
```

Shapes are ordinary RDF, so you load them into a graph of their own
like any other data.

## Pick a mode

| Mode | Evaluates | Use when |
|---|---|---|
| `'native'` (default) | SHACL Core: `sh:minCount`, `sh:maxCount`, `sh:datatype`, `sh:class`, `sh:nodeKind`, `sh:pattern`, `sh:in`, `sh:hasValue`, string, range, logical and property-pair constraints | Your shapes use only Core constraints. |
| `'pgrdf'` | SHACL Core **and** SHACL-SPARQL: `sh:sparql` with `sh:select` on node shapes | Your shapes contain a SPARQL-based constraint. |
| `'sparql'` | An alternative engine that evaluates few constraints; under strict mode it refuses most shapes | Not recommended. |

An unknown mode is refused with `22023`.

**Conformance, stated exactly.** The W3C SHACL test suite has 98 Core
tests. 56 of them are vendored into pgRDF's repository and run on every
change, and all 56 pass in `'native'` mode. A pass means `validate`
returns the same `sh:conforms` verdict as the test expects, which the
W3C suite calls *partial compliance*. Full compliance, a validation
report that matches the expected one result for result, is not graded.
55 of the 56 tests expect `conforms: false`, so a pass shows that the
verdict agrees, not that every expected result was reported. The other
42 Core tests are not part of that gate.
`pgrdf.shacl_capability()` lists, feature by feature, what `validate`
enforces.

## A Core example

```sql
SELECT pgrdf.add_graph('http://example.org/people');
SELECT pgrdf.parse_turtle($$
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
ex:alice a foaf:Person ; foaf:name "Alice" ; foaf:age 34 .
ex:carol a foaf:Person ; foaf:name "Carol" .
$$, pgrdf.graph_id('http://example.org/people'));

SELECT pgrdf.add_graph('http://example.org/shapes');
SELECT pgrdf.parse_turtle($$
@prefix sh:   <http://www.w3.org/ns/shacl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .
@prefix ex:   <http://example.org/> .

ex:PersonShape a sh:NodeShape ;
  sh:targetClass foaf:Person ;
  sh:property [ sh:path foaf:name ; sh:minCount 1 ; sh:datatype xsd:string ] ;
  sh:property [ sh:path foaf:age  ; sh:minCount 1 ; sh:datatype xsd:integer ] .
$$, pgrdf.graph_id('http://example.org/shapes'));

SELECT jsonb_pretty(pgrdf.validate(
  pgrdf.graph_id('http://example.org/people'),
  pgrdf.graph_id('http://example.org/shapes')));
```

```json
{
    "mode": "native",
    "conforms": false,
    "results": [
        {
            "focusNode": "http://example.org/carol",
            "resultPath": "http://xmlns.com/foaf/0.1/age",
            "resultMessage": "MinCount(1) not satisfied",
            "resultSeverity": "sh:Violation",
            "sourceConstraintComponent": "http://www.w3.org/ns/shacl#MinCountConstraintComponent",
            "sourceShape": "_:f56554c1b96c6184f1d9df50c5c2d487",
            "value": null
        }
    ],
    "data_triples": 5,
    "shapes_triples": 10,
    "data_graph_id": 1,
    "shapes_graph_id": 2,
    "elapsed_ms": 2.0,
    "act_recorded": true,
    "act_id": 5
}
```

## The report

| Field | Meaning |
|---|---|
| `conforms` | `true` when there are no results. |
| `results[]` | One entry per violation: `focusNode`, `resultPath`, `value`, `sourceShape`, `sourceConstraintComponent`, `resultSeverity` (`sh:Violation` / `sh:Warning` / `sh:Info`), `resultMessage`. |
| `mode` | The mode that ran. |
| `data_triples`, `shapes_triples` | How many triples each graph held. |
| `elapsed_ms` | Time spent. |
| `act_recorded`, `act_id` | Whether this verdict was written to the act record, and the id of its row (`null` when it was not). Since 0.6.41. |
| `act_reason` | Present only when `act_recorded` is `false`: why. |

The shape of this report is stable, so tooling can rely on it. The
`act_*` keys were added in 0.6.41; nothing was removed or renamed.

Because the report is JSONB, you can take it apart in SQL:

```sql
SELECT r->>'focusNode' AS node, r->>'resultPath' AS path, r->>'resultMessage' AS problem
  FROM jsonb_array_elements(
         pgrdf.validate(pgrdf.graph_id('http://example.org/people'),
                        pgrdf.graph_id('http://example.org/shapes')) -> 'results') AS r;
```

## The verdict is recorded

Every call that answers is written to the
[act record](05-graphs.md#the-act-record) as a `validate` act on the
**data** graph: who validated, against which shapes, and what came
out. Every further call adds a row. The first call on this page left
this one:

```sql
SELECT act_id, acting_role, jsonb_pretty(detail)
  FROM pgrdf.graph_acts(pgrdf.graph_id('http://example.org/people'))
 WHERE act = 'validate';
--  act_id | acting_role |                                       jsonb_pretty
-- --------+-------------+------------------------------------------------------------------------------------------
--       5 | postgres    | {                                                                                       +
--         |             |     "mode": "native",                                                                   +
--         |             |     "results": 1,                                                                       +
--         |             |     "conforms": false,                                                                  +
--         |             |     "shapes_digest": "320dc8d6e812653f1644d2f8d449199794096fc417483f221160e4e66c63caee",+
--         |             |     "materialization": "never",                                                         +
--         |             |     "shapes_graph_id": 2,                                                               +
--         |             |     "shapes_graph_iri": "http://example.org/shapes"                                     +
--         |             | }
```

| Key of `detail` | Meaning |
|---|---|
| `conforms`, `results` | The verdict, and how many results the report held. The results themselves are not stored. |
| `mode` | The mode that ran. |
| `shapes_graph_id`, `shapes_graph_iri` | The shapes graph used. |
| `shapes_digest` | The shapes graph's `graph_digest` at that moment, so a later reader can tell whether the shapes have changed since. `null`, with a `shapes_digest_reason`, when it could not be computed. |
| `materialization` | The data graph's materialization state when it was read (`never`, `current`, `stale`, `unknown`). Validation sees inferred triples, so this is part of what the verdict means. |

Two cases answer without recording, and say so in the report:

```sql
BEGIN READ ONLY;
SELECT v -> 'conforms' AS conforms, v -> 'act_recorded' AS act_recorded, v -> 'act_reason' AS act_reason
  FROM pgrdf.validate(pgrdf.graph_id('http://example.org/people'),
                      pgrdf.graph_id('http://example.org/shapes')) AS v;
--  conforms | act_recorded |           act_reason
-- ----------+--------------+--------------------------------
--  false    | false        | "the transaction is read-only"
COMMIT;
```

The other is a data graph that was never created with `add_graph`
(triples loaded into a bare numeric id): it has no act record to write
to. A call that raises an error records nothing either.

## A SHACL-SPARQL example: unique values

SHACL Core has no uniqueness constraint; SHACL-SPARQL can express one.
Use `mode => 'pgrdf'`.

```sql
SELECT pgrdf.add_graph('http://example.org/staff');
SELECT pgrdf.parse_turtle($$
@prefix ex: <http://example.org/> .
ex:alice a ex:Employee ; ex:ssn "123-45-6789" .
ex:bob   a ex:Employee ; ex:ssn "555-12-3456" .
ex:carol a ex:Employee ; ex:ssn "123-45-6789" .
$$, pgrdf.graph_id('http://example.org/staff'));

SELECT pgrdf.add_graph('http://example.org/staff-shapes');
SELECT pgrdf.parse_turtle($$
@prefix ex: <http://example.org/> .
@prefix sh: <http://www.w3.org/ns/shacl#> .
ex:EmployeeShape a sh:NodeShape ;
  sh:targetClass ex:Employee ;
  sh:sparql [ a sh:SPARQLConstraint ;
    sh:message "SSN must be unique" ;
    sh:select """
      PREFIX ex: <http://example.org/>
      SELECT $this ?value WHERE {
        GRAPH <http://example.org/staff> {
          $this ex:ssn ?value .
          ?other ex:ssn ?value .
        }
        FILTER ($this != ?other)
      }""" ] .
$$, pgrdf.graph_id('http://example.org/staff-shapes'));

SELECT pgrdf.validate(pgrdf.graph_id('http://example.org/staff'),
                      pgrdf.graph_id('http://example.org/staff-shapes'),
                      'pgrdf');
```

Alice and Carol are both reported, each with `value` `"123-45-6789"`
and `sourceConstraintComponent` `sh:SPARQLConstraintComponent`.

> **Scope your `sh:select` with `GRAPH`.** Queries inside SPARQL-based
> constraints currently see triples from *all* graphs in the database,
> not only the data graph. That is why the example wraps its pattern in
> `GRAPH <http://example.org/staff> { … }`. Without it, an `ex:ssn` in
> any other graph would count as a duplicate. Core constraints are not
> affected.

> **Put SPARQL constraints on node shapes.** `sh:sparql` placed
> inside a property shape (`sh:property [ … sh:sparql … ]`), and
> custom constraint components built on SPARQL validators, are
> currently not evaluated. Validation reports `conforms: true` without
> checking them. Attach SPARQL constraints directly to a
> `sh:NodeShape`, as above.

The query inside `sh:select` runs on the same SPARQL engine as
`pgrdf.sparql()`, so the same [supported features](03-querying.md#not-supported)
apply. For example, write `FILTER NOT EXISTS { … }` as a `MINUS` or as
`OPTIONAL { … } FILTER(!BOUND(?x))`.

## Strict mode

By default (`strict => true`) validation refuses to hand you a verdict
that means nothing:

- **A shapes graph with no targets** (a wrong or empty graph id, for
  instance) would select nothing and report `conforms: true`. It is
  refused instead:

  ```
  validate: shapes graph 12346 declares no SHACL target (0 triples). Nothing would be selected,
  so a verdict would be vacuous — a missing or wrong graph id reports the same `conforms:true`
  as a clean validation. Re-run with strict => false to accept a vacuous pass.
  ```

- **Shapes that use a constraint the chosen mode can't evaluate** (for
  example `sh:sparql` under `'native'`) are refused, and the message
  names the mode that would work.

Pass `strict => false` to validate anyway. Constraints the mode can't
evaluate are then skipped.

A graph id of `NULL` doesn't reach these checks. If
`pgrdf.graph_id('…')` is given an IRI that doesn't exist, it returns
`NULL`, and `validate` then returns `NULL` instead of a report. Treat
a `NULL` result as a failure, or check the ids first.

## Large graphs

`validate` reads the data graph and the shapes graph into the memory
of the database process, and `materialize` does the same with its
graph. That is by design, and it means memory use grows with the
graph. `pgrdf.max_inmemory_triples` puts a ceiling on it: when it is
above `0`, a call that would read more triples than that (for
`validate`, data plus shapes) is refused with `54000` before it reads
anything.

```sql
SET pgrdf.max_inmemory_triples = 10;
SELECT pgrdf.validate(pgrdf.graph_id('http://example.org/people'),
                      pgrdf.graph_id('http://example.org/shapes'));
-- ERROR:  54000: validate: data graph 1 plus shapes graph 2 holds 15 triples, more than
--         pgrdf.max_inmemory_triples=10; validate reads them all into backend memory
-- HINT:  Nothing was read or changed. Raise pgrdf.max_inmemory_triples (0 is no limit) when the
--        backend has the memory for it, or run on a smaller graph.
```

The default is `0`, no limit. Any role can set it for its session; an
operator usually sets it per database with `ALTER DATABASE … SET`.

## Validating reasoned data

Validation sees inferred triples as well as asserted ones. To validate
the consequences of your ontology, run `materialize` first.

This matters for targets too. `sh:targetClass foaf:Person` currently
selects only resources typed `foaf:Person` directly. An `ex:Engineer`
whose class is a subclass of `foaf:Person` is selected only after
`materialize` has added its `foaf:Person` type. A shape
that targets `foaf:Person` will then also check resources that are
Persons only by inference. The [tour](tour.md#4-validate-it-with-shacl)
shows this end to end.

## Next

[07 — Identity and export](07-identity-and-export.md)
