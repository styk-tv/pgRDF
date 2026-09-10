# Building pgRDF from source

Most people don't need to build pgRDF: prebuilt archives and a Docker
recipe are in the [install guide](guide/01-install.md). Build from
source to develop pgRDF, or to target a platform without a prebuilt
archive.

## Requirements

- PostgreSQL 18 with its development files (`pg_config`, server
  headers). On Debian / Ubuntu: `postgresql-server-dev-18`.
- Rust 1.96 or newer.
- `cargo-pgrx` **0.19.2**, which must match the `pgrx` version in
  `Cargo.toml`.

```bash
cargo install cargo-pgrx --locked --version 0.19.2
cargo pgrx init --pg18 "$(which pg_config)"
```

## Build and install

From a clone of this repository:

```bash
cargo pgrx install --release --pg-config "$(which pg_config)"
```

This builds `pgrdf.so` and copies it, together with the control and SQL
files, into that PostgreSQL installation.

### From the source archive

Each release also ships a source archive (`pgrdf-<version>.zip`), the
same one published to PGXN. Unpack it and run:

```bash
make PG_CONFIG=/path/to/pg_config
make PG_CONFIG=/path/to/pg_config install
```

or, with the PGXN client: `pgxn install pgrdf --pg_config /path/to/pg_config`.

## Configure and create the extension

1. Add pgRDF to `postgresql.conf`:

   ```ini
   shared_preload_libraries = 'pgrdf'
   ```

2. Restart PostgreSQL (a reload is not enough).

3. Create the extension and check it:

   ```sql
   CREATE EXTENSION pgrdf;
   SELECT pgrdf.version(), pgrdf.build_id();
   ```

   A local build reports a git-based `build_id()` (for example a hash
   ending in `-dirty` when the tree has uncommitted changes). Official
   releases report their tag.

## Building inside Docker

To produce a Linux `.so` without installing Rust locally, for example
on macOS, use the builder container described in
[compose/README.md](compose/README.md).

## Running the tests

```bash
cargo pgrx test pg18       # in-database tests
```

The full suites (regression, W3C SPARQL and SHACL conformance, LUBM)
run through the `Justfile`. See
[testing](https://pgrdf.styk.tv/v0.6/internals/testing) on the documentation site.

## Maintainers: source archive

```bash
make dist        # → pgrdf-<version>.zip, the PGXN-ready source archive
```
