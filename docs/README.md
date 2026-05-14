# pgRDF documentation index

Order is meaningful — read top-down if you're new to the project.

| # | Doc | Scope |
|---|---|---|
| 01 | [architecture.md](01-architecture.md) | What pgRDF is, the four engines (storage, query, inference, validation), how they connect. |
| 02 | [storage.md](02-storage.md) | Dictionary, partitioned hexastore, hexastore indexes, ingest path (shmem dict cache + prepared bulk-INSERT shipped; heap_multi_insert / true COPY BINARY queued for v0.4). |
| 03 | [query.md](03-query.md) | SPARQL → algebra → dynamic SQL; current SELECT + ASK surface; prepared-plan cache shipped. |
| 04 | [inference.md](04-inference.md) | OWL 2 RL materialization via `reasonable` (shipped, Phase 4). |
| 05 | [validation.md](05-validation.md) | SHACL validation reports as JSONB (stub shipped — Phase 5; real `shacl_validation` integration blocked by ERRATA E-009). |
| 06 | [installation.md](06-installation.md) | INSTALL spec walk-through: K8s init container + entrypoint copy (PG ≤ 17) vs GUC drop-in (PG 18+). |
| 07 | [development.md](07-development.md) | Local dev with `cargo pgrx` + the compose path. |
| 08 | [testing.md](08-testing.md) | Five test layers + coverage gates by phase. |
| 09 | [release.md](09-release.md) | Tag → matrix build → release artifacts pipeline. |
| 10 | [roadmap.md](10-roadmap.md) | Phase 1–6 progression with measurable gates per phase. |

## Authoritative references

- [specs/SPEC.pgRDF.LLD.v0.3.md](../specs/SPEC.pgRDF.LLD.v0.3.md) — **current** LLD; supersedes v0.2 at the contract level.
- [specs/SPEC.pgRDF.LLD.v0.2.md](../specs/SPEC.pgRDF.LLD.v0.2.md) — historical, still referenced for §4.1–4.3 internals.
- [specs/SPEC.pgRDF.INSTALL.v0.2.md](../specs/SPEC.pgRDF.INSTALL.v0.2.md) — install spec; unchanged in v0.3.
- [specs/ERRATA.v0.2.md](../specs/ERRATA.v0.2.md) — read alongside v0.2 LLD; still authoritative for the deltas it lists.

When this directory disagrees with `specs/`, the spec wins. If a doc
finds the spec wrong, the correction lands in `ERRATA.v0.2.md`, not in
the doc.
