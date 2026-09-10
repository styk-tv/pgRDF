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
  files, with parallel loaders for very large N-Triples dumps.
- **Query:** SPARQL 1.1 `SELECT` / `ASK` / `CONSTRUCT` / `DESCRIBE`,
  `OPTIONAL`, `UNION`, `MINUS`, `VALUES`, `BIND`, subqueries,
  aggregates, property paths, named graphs.
- **Update:** SPARQL 1.1 UPDATE, including graph management.
- **Reason:** OWL 2 RL or RDFS materialization, stored beside the
  asserted data.
- **Validate:** W3C SHACL Core (25/25 on the conformance suite) plus
  SHACL-SPARQL, with a JSONB report.
- **Manage graphs:** inventory, copy / move / drop, carving, write
  locks, integrity checks.
- **Identify and export:** W3C RDFC-1.0 canonical digests, canonical
  N-Triples export, portable manifests.

## Install

```bash
pgxn install pgrdf
```

This builds from source and needs Rust 1.96+ and `cargo-pgrx` 0.19.2.
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
