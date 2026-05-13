# pgRDF documentation index

Order is meaningful — read top-down if you're new to the project.

| # | Doc | Scope |
|---|---|---|
| 01 | [architecture.md](01-architecture.md) | What pgRDF is, the four engines (storage, query, inference, validation), how they connect. |
| 02 | [storage.md](02-storage.md) | Shmem dictionary, partitioned hexastore, hexastore indexes, COPY-BINARY ingestion. |
| 03 | [query.md](03-query.md) | SPARQL → algebra → prepared SQL; plan cache. |
| 04 | [inference.md](04-inference.md) | OWL 2 RL materialization via `reasonable`. |
| 05 | [validation.md](05-validation.md) | SHACL validation reports as JSONB. |
| 06 | [installation.md](06-installation.md) | INSTALL spec walk-through: K8s init container + entrypoint copy (PG ≤ 17) vs GUC drop-in (PG 18+). |
| 07 | [development.md](07-development.md) | Local dev with `cargo pgrx` + the compose path. |
| 08 | [testing.md](08-testing.md) | Five test layers + coverage gates by phase. |
| 09 | [release.md](09-release.md) | Tag → matrix build → release artifacts pipeline. |
| 10 | [roadmap.md](10-roadmap.md) | Phase 1–4 progression with measurable gates per phase. |

## Authoritative references

- [specs/SPEC.pgRDF.LLD.v0.2.md](../specs/SPEC.pgRDF.LLD.v0.2.md)
- [specs/SPEC.pgRDF.INSTALL.v0.2.md](../specs/SPEC.pgRDF.INSTALL.v0.2.md)
- [specs/ERRATA.v0.2.md](../specs/ERRATA.v0.2.md) — read this **first** for known v0.2 spec deltas.

When this directory disagrees with `specs/`, the spec wins. If a doc
finds the spec wrong, the correction lands in `ERRATA.v0.2.md`, not in
the doc.
