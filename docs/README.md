# pgRDF documentation index

Order is meaningful — read top-down if you're new to the project.

| # | Doc | Scope |
|---|---|---|
| 01 | [architecture.md](01-architecture.md) | What pgRDF is, the four engines (storage, query, inference, validation), how they connect. |
| 02 | [storage.md](02-storage.md) | Dictionary, partitioned hexastore, hexastore indexes, ingest path (shmem dict cache + prepared bulk-INSERT + the parallel bulk loader shipped; deeper heap_multi_insert / COPY BINARY a follow-up). |
| 03 | [query.md](03-query.md) | SPARQL → algebra → dynamic SQL; full SPARQL 1.1 SELECT/ASK/CONSTRUCT/DESCRIBE/UPDATE surface; prepared-plan cache shipped. |
| 04 | [inference.md](04-inference.md) | OWL 2 RL materialization via `reasonable` (shipped, Phase 4). |
| 05 | [validation.md](05-validation.md) | SHACL validation reports as JSONB — real SHACL Core (W3C full-pass 25/25) + native SHACL-SPARQL via `mode => 'pgrdf'`. |
| 06 | [installation.md](06-installation.md) | INSTALL spec walk-through: K8s init container + entrypoint copy (PG ≤ 17) vs GUC drop-in (PG 18+). |
| 07 | [development.md](07-development.md) | Local dev with `cargo pgrx` + the compose path. |
| 08 | [testing.md](08-testing.md) | Five test layers + coverage gates by phase. |
| 09 | [release.md](09-release.md) | Tag → matrix build → release artifacts pipeline. |
| 10 | [roadmap.md](10-roadmap.md) | Phase 1–6 progression with measurable gates per phase. |

## Authoritative references

- [specs/SPEC.pgRDF.LLD.v0.5.md](../specs/SPEC.pgRDF.LLD.v0.5.md) — **current** shipped LLD contract.
- [specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md](../specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md) — forward-looking design (the v0.6.x bulk-ingest line + beyond).
- [specs/SPEC.pgRDF.BENCH.v0.6.0.md](../specs/SPEC.pgRDF.BENCH.v0.6.0.md) — benchmark methodology (LUBM-10→500).
- [specs/SPEC.pgRDF.INSTALL.v0.2.md](../specs/SPEC.pgRDF.INSTALL.v0.2.md) — install spec.
- Earlier LLDs (v0.2–v0.4) and [ERRATA.v0.2 / .v0.4 / .v0.5 / .v0.6](../specs/) remain as the historical contract + errata records.

When this directory disagrees with `specs/`, the spec wins. If a doc
finds the spec wrong, the correction lands in `ERRATA.v0.2.md`, not in
the doc.
