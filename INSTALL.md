# INSTALL

## PGXN / source install

pgRDF ships PGXN metadata at the repository root:

- `META.json`
- `Makefile`
- `README.pgxn.md`
- `LICENSE`

The PGXN build path is source-based. It shells out to `cargo pgrx package`,
so the build host needs the Rust and pgrx toolchain in addition to PostgreSQL.

Prebuilt binary artifacts remain on the GitHub Releases page as the existing
`pgrdf-<version>-pg<PG_MAJOR>-glibc-<arch>.tar.gz` matrix plus `SHA256SUMS`.
The PGXN archive is the source distribution, not a replacement for those
per-architecture binaries.

## Prerequisites

- PostgreSQL 14, 15, 16, or 17 development installation
- `pg_config` for the target PostgreSQL major
- Rust 1.91 or newer
- `cargo-pgrx` 0.16
- one-time `cargo pgrx init` for the target PostgreSQL installation

Example one-time pgrx setup:

```bash
cargo install cargo-pgrx --locked --version '^0.16'
cargo pgrx init --pg17 /path/to/pg_config
```

## Install via PGXN client

```bash
pgxn install pgrdf --pg_config /path/to/pg_config
psql -d yourdb -c 'CREATE EXTENSION pgrdf;'
```

If you prefer to build directly from the unpacked PGXN source archive:

```bash
make PG_CONFIG=/path/to/pg_config
make PG_CONFIG=/path/to/pg_config install
psql -d yourdb -c 'CREATE EXTENSION pgrdf;'
```

## Maintainer release artifact

Build the PGXN-ready source archive from a tagged commit:

```bash
make dist
```

This emits `pgrdf-<version>.zip`, with the standard
`pgrdf-<version>/...` directory prefix expected by PGXN Manager. The GitHub
release workflow also attaches this zip alongside the existing binary tarballs.
