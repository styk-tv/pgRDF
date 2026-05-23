# pgRDF

`pgRDF 0.5.1` is a Rust-native PostgreSQL extension for RDF storage,
SPARQL 1.1 query and update, SHACL Core validation, and OWL 2 RL or
RDFS materialization inside PostgreSQL.

## Current package

- PGXN package version: `0.5.1`
- PostgreSQL support: `14`, `15`, `16`, `17`
- License: `MIT`
- Full documentation: <https://pgrdf.styk.tv/>

## Feature summary

- RDF storage inside PostgreSQL, including named-graph support
- Turtle, TriG, and N-Quads ingest
- SPARQL 1.1 query and update
- `SELECT`, `ASK`, `CONSTRUCT`, `DESCRIBE`
- FILTER, OPTIONAL, UNION, MINUS, VALUES, BIND, aggregates, named
  graphs, property paths
- SHACL Core validation via `pgrdf.validate(data, shapes [, mode])`
- RDFS and OWL 2 RL materialization via
  `pgrdf.materialize(graph_id, profile)`

## Install model

This PGXN package is the source distribution for `pgrdf`. `pgxn install`
builds the extension locally via `cargo pgrx package`, so the target
machine needs:

- PostgreSQL development files for the target major
- `pg_config` for the target PostgreSQL installation
- Rust `1.91` or newer
- `cargo-pgrx` `0.16`
- a one-time `cargo pgrx init`

See `INSTALL.md` for the exact setup and build commands.

## Install

Typical install flow:

```bash
pgxn install pgrdf --pg_config /path/to/pg_config
psql -d yourdb -c 'CREATE EXTENSION pgrdf;'
```

For best performance, add `pgrdf` to `shared_preload_libraries` and
restart PostgreSQL before creating the extension. Without preload, the
extension still works, but the shared-memory dictionary cache stays off.

## Prebuilt binaries

PGXN is the source-install path. Prebuilt binaries remain on GitHub
Releases.

The current release asset set is:

- `pgrdf-0.5.1.zip` (PGXN source archive)
- `pgrdf-0.5.1-pg14-glibc-amd64.tar.gz`
- `pgrdf-0.5.1-pg14-glibc-arm64.tar.gz`
- `pgrdf-0.5.1-pg15-glibc-amd64.tar.gz`
- `pgrdf-0.5.1-pg15-glibc-arm64.tar.gz`
- `pgrdf-0.5.1-pg16-glibc-amd64.tar.gz`
- `pgrdf-0.5.1-pg16-glibc-arm64.tar.gz`
- `pgrdf-0.5.1-pg17-glibc-amd64.tar.gz`
- `pgrdf-0.5.1-pg17-glibc-arm64.tar.gz`
- `SHA256SUMS`

Project repository, issues, release assets, and OCI artifacts:

- <https://github.com/styk-tv/pgRDF>
