# pgRDF

**Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning.**

Status: **Alpha — Phase 1 (Core Storage & Build Automation)** per
[docs/10-roadmap.md](docs/10-roadmap.md). The extension registers and
exposes a smoke surface. SPARQL/SHACL/inference modules are skeletons.

Specs:
- [specs/SPEC.pgRDF.LLD.v0.2.md](specs/SPEC.pgRDF.LLD.v0.2.md) — low-level design
- [specs/SPEC.pgRDF.INSTALL.v0.2.md](specs/SPEC.pgRDF.INSTALL.v0.2.md) — runtime install on stock PG containers
- [specs/ERRATA.v0.2.md](specs/ERRATA.v0.2.md) — corrections discovered during implementation

## Quick start (local dev)

Prereqs: Rust 1.88+, `cargo-pgrx`, `podman`, `just`.

```bash
just pgrx-init        # one-time: download + compile PG sources for pgrx
just dev              # boots pgrx's managed Postgres with the extension loaded
# in another shell:
psql -h localhost -p 28818 -d pgrdf -c 'CREATE EXTENSION pgrdf; SELECT pgrdf.version();'
```

## Deployment-style verification

Boots stock `postgres:17.4-bookworm` and loads the locally-built
extension via per-file bind mounts at canonical Postgres paths.
No image rebuild, no entrypoint wrappers, no init scripts.

```bash
just build-ext        # builds linux/glibc artifacts into compose/extensions/
just compose-up
just psql
pgrdf=# CREATE EXTENSION pgrdf;
pgrdf=# SELECT pgrdf.version();
```

See [compose/README.md](compose/README.md). The PG 18 GUC drop-in
path from INSTALL spec §7 is the eventual target — blocked today by
upstream pgrx 0.17/0.18 not building on current Rust ([`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) E-006).

## Layout

```
.                          # this README + pgrx-canonical layout
├── Cargo.toml             # extension crate
├── pgrdf.control          # extension metadata
├── src/                   # Rust source (storage / query / inference / validation)
├── sql/                   # extension SQL (loaded via extension_sql_file!)
├── compose/               # local-dev runtime (INSTALL spec §7 incarnation)
├── tests/                 # pgrx integration, W3C SPARQL/SHACL, pg_regress, perf
├── docs/                  # 01-architecture … 10-roadmap
├── specs/                 # SPEC.* + ERRATA
├── examples/              # sample TTL / SPARQL / SHACL
└── .github/workflows/     # ci + release matrix + nightly regression
```

## Documentation index

- [docs/01-architecture.md](docs/01-architecture.md)
- [docs/02-storage.md](docs/02-storage.md)
- [docs/03-query.md](docs/03-query.md)
- [docs/04-inference.md](docs/04-inference.md)
- [docs/05-validation.md](docs/05-validation.md)
- [docs/06-installation.md](docs/06-installation.md)
- [docs/07-development.md](docs/07-development.md)
- [docs/08-testing.md](docs/08-testing.md)
- [docs/09-release.md](docs/09-release.md)
- [docs/10-roadmap.md](docs/10-roadmap.md)

## License

Apache-2.0. See [LICENSE](LICENSE).
