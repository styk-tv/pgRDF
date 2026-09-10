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

The Core engine passes the W3C SHACL Core conformance suite (25 of 25).
An unknown mode is refused with `22023`.

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
    "data_triples": 6,
    "shapes_triples": 10,
    "data_graph_id": 1,
    "shapes_graph_id": 2,
    "elapsed_ms": 2.0
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

The shape of this report is stable, so tooling can rely on it.

Because the report is JSONB, you can take it apart in SQL:

```sql
SELECT r->>'focusNode' AS node, r->>'resultPath' AS path, r->>'resultMessage' AS problem
  FROM jsonb_array_elements(
         pgrdf.validate(pgrdf.graph_id('http://example.org/people'),
                        pgrdf.graph_id('http://example.org/shapes')) -> 'results') AS r;
```

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
