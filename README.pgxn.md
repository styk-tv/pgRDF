# pgRDF

**RDF, SPARQL, SHACL and OWL reasoning inside PostgreSQL.**

pgRDF is a PostgreSQL extension written in Rust. It lets you:

- load RDF into your database;
- query and update it with SPARQL 1.1;
- reason over it with OWL 2 RL or RDFS;
- validate it with SHACL;
- fingerprint and export it.

Every operation is a SQL function call.

- **Source, releases and the full guide:** <https://github.com/styk-tv/pgRDF>
- **PostgreSQL:** 18
- **License:** MIT

## Capabilities

- **Load:** Turtle, N-Triples, TriG, N-Quads, from strings or server
  files, with parallel loaders for very large N-Triples dumps. A graph
  is a set of triples: loading a triple it already holds stores
  nothing.
- **Query:** SPARQL 1.1 `SELECT` / `ASK` / `CONSTRUCT` / `DESCRIBE`,
  `OPTIONAL`, `UNION`, `MINUS`, `VALUES`, `BIND`, subqueries,
  aggregates, property paths, named graphs.
- **Update:** SPARQL 1.1 UPDATE, including graph management.
- **Reason:** OWL 2 RL or RDFS materialization, stored beside the
  asserted data.
- **Validate:** SHACL Core plus SHACL-SPARQL, with a JSONB report. 56
  of the 98 W3C SHACL Core tests are run on every change and all pass,
  graded on the `sh:conforms` verdict (the suite's partial compliance;
  full reports are not compared).
- **Manage graphs:** inventory with stored counts (listing graphs does
  not read the triples), copy / move / drop, carving, write locks,
  integrity checks, a graph owner, and a record of every graph-level
  act (who created, loaded, cleared, locked or validated a graph) with
  a feed for clients that poll it.
- **Identify and export:** W3C RDFC-1.0 canonical digests, canonical
  N-Triples export, portable manifests.

## Install

```bash
pgxn install pgrdf
```

This builds from source and needs Rust 1.99.1+ and `cargo-pgrx` 0.20.1.
Prebuilt archives and a Docker recipe are in the
[install guide](https://github.com/styk-tv/pgRDF/blob/main/guide/01-install.md).

Then add pgRDF to `postgresql.conf` and **restart** the server:

```ini
shared_preload_libraries = 'pgrdf'
```

```sql
CREATE EXTENSION pgrdf;
```

## Quick start

```sql
SELECT pgrdf.add_graph('http://example.org/people');

SELECT pgrdf.parse_turtle($$
  @prefix ex:   <http://example.org/> .
  @prefix foaf: <http://xmlns.com/foaf/0.1/> .
  ex:alice foaf:name "Alice" ; foaf:knows ex:bob .
  ex:bob   foaf:name "Bob" .
$$, pgrdf.graph_id('http://example.org/people'));

SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?name WHERE { ?p foaf:name ?name }
$$);

SELECT pgrdf.materialize(pgrdf.graph_id('http://example.org/people'));   -- OWL 2 RL
```

## Documentation

The [user guide](https://github.com/styk-tv/pgRDF/tree/main/guide)
covers installation, a ten-minute tour, loading, querying, reasoning,
graph management, validation, identity and export, diagnostics, a
function reference, and client examples for Python, Node.js, Go and
Rust.

## License

MIT. See [LICENSE](https://github.com/styk-tv/pgRDF/blob/main/LICENSE).
