# A ten-minute tour

This walk-through touches every major part of pgRDF in one `psql`
session: load a small graph, query it, reason over it, validate it,
change it, lock it, and fingerprint it. Every output below is real,
copied from a fresh `postgres:18` server.

You need a PostgreSQL 18 server with pgRDF installed. The fastest way
to get one is the Docker recipe in the [README](../README.md#try-it-in-two-minutes)
(four shell commands). Then open a shell:

```sh
docker exec -it pgrdf psql -U postgres
```

```sql
CREATE EXTENSION pgrdf;
```

## 1. Create a graph and load some data

Every triple lives in a named graph. Create one by IRI; you get back
its numeric id.

```sql
SELECT pgrdf.add_graph('http://example.org/people');
--  add_graph
-- -----------
--          1
```

Load Turtle straight from SQL. `pgrdf.graph_id(iri)` turns the IRI
back into the id the loaders take.

```sql
SELECT pgrdf.parse_turtle($$
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .

ex:alice a ex:Engineer ; foaf:name "Alice" ; foaf:age 34 ; foaf:knows ex:bob .
ex:bob   a ex:Manager  ; foaf:name "Bob"   ; foaf:age 41 ; foaf:knows ex:carol .
ex:carol a ex:Engineer ; foaf:name "Carol" .

ex:Engineer rdfs:subClassOf ex:Employee .
ex:Manager  rdfs:subClassOf ex:Employee .
ex:Employee rdfs:subClassOf foaf:Person .
$$, pgrdf.graph_id('http://example.org/people'));
--  parse_turtle
-- --------------
--            13
```

## 2. Ask a question in SPARQL

`pgrdf.sparql()` returns one JSONB row per solution.

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?name ?age
  WHERE { ?p foaf:name ?name OPTIONAL { ?p foaf:age ?age } }
  ORDER BY ?name
$$);
--              sparql
-- --------------------------------
--  {"age": "34", "name": "Alice"}
--  {"age": "41", "name": "Bob"}
--  {"age": null, "name": "Carol"}
```

Follow a chain of relationships with a property path:

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  PREFIX ex:   <http://example.org/>
  SELECT ?who WHERE { ex:alice foaf:knows+ ?who }
$$);
--  {"who": "http://example.org/carol"}
--  {"who": "http://example.org/bob"}
```

## 3. Reason over it

Nobody in the data is declared a `foaf:Person`, so this finds nothing
yet:

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?p WHERE { ?p a foaf:Person }
$$);
-- (0 rows)
```

The class hierarchy says every Engineer and Manager is an Employee, and
every Employee is a Person. Materialize the RDFS entailments:

```sql
SELECT pgrdf.materialize(pgrdf.graph_id('http://example.org/people'), 'rdfs');
-- {"profile": "rdfs", "base_triples": 13, "inferred_triples_written": 8, ...}
```

Run the same query again:

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?p WHERE { ?p a foaf:Person } ORDER BY ?p
$$);
--  {"p": "http://example.org/alice"}
--  {"p": "http://example.org/bob"}
--  {"p": "http://example.org/carol"}
```

Inferred triples are stored beside your data, never mixed into it. The
inventory keeps them apart:

```sql
SELECT * FROM pgrdf.graph_inventory();
--  graph_id |            iri            | asserted | inferred | locked | lock_reason | materialization
-- ----------+---------------------------+----------+----------+--------+-------------+-----------------
--         0 | urn:pgrdf:graph:0         |        0 |        0 | f      |             | never
--         1 | http://example.org/people |       13 |        8 | f      |             | current
```

(Graph `0` is the built-in default graph.)

## 4. Validate it with SHACL

Shapes are just another graph. This one says every person needs a
name and an integer age:

```sql
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
```

```sql
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
    "data_triples": 21,
    "shapes_triples": 10,
    "data_graph_id": 1,
    "shapes_graph_id": 2,
    "elapsed_ms": 2.007986
}
```

The shape targets `foaf:Person`, and the people only became Persons
through reasoning. Validation sees the inferred triples too.

## 5. Fix the data with SPARQL UPDATE

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  PREFIX ex:   <http://example.org/>
  INSERT DATA { GRAPH <http://example.org/people> { ex:carol foaf:age 29 } }
$$);
-- {"_update": {"form": "INSERT_DATA", "graphs_touched": ["http://example.org/people"],
--              "triples_inserted": 1, "triples_deleted": 0, ...}}

SELECT pgrdf.validate(
  pgrdf.graph_id('http://example.org/people'),
  pgrdf.graph_id('http://example.org/shapes')) -> 'conforms' AS conforms;
--  conforms
-- ----------
--  true
```

The data changed after it was materialized, and the inventory notices:

```sql
SELECT iri, asserted, inferred, materialization FROM pgrdf.graph_inventory();
--             iri            | asserted | inferred | materialization
-- ---------------------------+----------+----------+-----------------
--  urn:pgrdf:graph:0         |        0 |        0 | never
--  http://example.org/people |       14 |        8 | stale
--  http://example.org/shapes |       10 |        0 | never
```

Run `materialize` again whenever you want the inferred triples to catch
up. It replaces the previous inferred set.

## 6. Lock the graph

A lock makes a graph read-only for every write path until someone
unlocks it. Locks always carry a reason.

```sql
SELECT pgrdf.lock_graph(pgrdf.graph_id('http://example.org/people'), 'release review');

SELECT * FROM pgrdf.sparql($$
  INSERT DATA { GRAPH <http://example.org/people> {
    <http://example.org/dave> <http://xmlns.com/foaf/0.1/name> "Dave" } }
$$);
-- ERROR:  55P03: pgrdf: graph 1 is locked (release review): SPARQL UPDATE refused.
--         Unlock with pgrdf.unlock_graph(1, '<reason>').

SELECT pgrdf.unlock_graph(pgrdf.graph_id('http://example.org/people'), 'review done');
```

Every refusal carries a standard SQLSTATE (`55P03` here) and a message
that says what to do next. See
[errors and diagnostics](08-errors-and-diagnostics.md).

## 7. Fingerprint and export

`graph_digest` is the graph's canonical identity (W3C RDFC-1.0). Two
graphs with the same triples have the same digest, however they were
loaded.

```sql
SELECT pgrdf.graph_digest(pgrdf.graph_id('http://example.org/people'));
-- c8500c4878f33cd5ce7939fd8f0da39edc34ba8500485108624122a05c841412
```

`export_graph` emits the asserted triples as sorted N-Triples, and
`graph_manifest` describes that export: digests, counts and engine
version.

```sql
SELECT * FROM pgrdf.export_graph(pgrdf.graph_id('http://example.org/people')) LIMIT 3;
-- <http://example.org/Employee> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://xmlns.com/foaf/0.1/Person> .
-- <http://example.org/Engineer> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://example.org/Employee> .
-- <http://example.org/Manager> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://example.org/Employee> .

SELECT jsonb_pretty(pgrdf.graph_manifest(pgrdf.graph_id('http://example.org/people')));
```

Write both to files and anyone can check the copy with nothing but
`sha256sum`. See [identity and export](07-identity-and-export.md).

## 8. Ask the engine what it supports

```sql
SELECT name, identity_args, note FROM pgrdf.surface() WHERE class = 'stable' ORDER BY 1;
```

`surface()` lists every function with a stability class and a short
usage note. The `stable` ones are the supported API.

## Clean up

```sql
SELECT pgrdf.drop_graph('http://example.org/people');
SELECT pgrdf.drop_graph('http://example.org/shapes');
```

or throw the whole container away with `docker rm -f pgrdf`.

## Where next

- [Loading RDF](02-loading-rdf.md): files, TriG / N-Quads, bulk loads.
- [Querying](03-querying.md): the full SPARQL surface.
- [Reasoning](04-reasoning.md) · [Managing graphs](05-graphs.md) ·
  [Validation](06-validation-recipes.md)
- [Function reference](09-function-reference.md)
