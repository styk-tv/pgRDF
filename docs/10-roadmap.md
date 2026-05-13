# 10 — Roadmap

Four phases, each ending with a measurable gate. The gate is the
**floor** for all subsequent phases — once a coverage layer is green
it stays green.

The progression mirrors LLD §7 ("Development Checklist & Progression")
with the testing posture from [08-testing.md](08-testing.md) layered on.

---

## Phase 1 — Core Storage & Build Automation (current)

**Outcome:** the extension registers cleanly in stock postgres:18-bookworm
and the build matrix produces release artifacts. SPARQL/SHACL/inference
are present as stubs only.

Gates:
- [x] `cargo pgrx new`-style scaffold compiles on PG 14–17 (PG 18 deferred
      pending upstream pgrx fix — `specs/ERRATA.v0.2.md` E-006).
- [ ] `cargo pgrx test pg17` is green for `pgrdf.version()`.
- [ ] `just compose-up` boots the stack and `CREATE EXTENSION pgrdf` succeeds.
- [ ] `_pgrdf_dictionary` and `_pgrdf_quads` (partitioned) are created
      by the install SQL and visible after `CREATE EXTENSION`.
- [ ] Hexastore SPO/POS/OSP indexes present.
- [ ] CI matrix (pg14..pg17 × {amd64, arm64}) is green on a tag push to a
      `v0.2.0-alpha.*` pre-release.
- [ ] Pre-built release tarballs land on a GitHub release matching the
      INSTALL spec §3 layout.

Exit criterion: every box above checked. The repo at this point is
useful as **infrastructure** even if the engines are not yet real.

---

## Phase 2 — Query Engine & Shared Memory

**Outcome:** SPARQL SELECT queries with BGPs work end-to-end. Cross-
backend dictionary cache is real.

Gates (in addition to Phase 1):
- [ ] `pgrx::shmem` dictionary cache populated lock-free or under
      RwLock; lookup latency on cache hit < 1 µs.
- [ ] `spargebra::Query::parse` integrated; AST traversal covered by
      unit tests for `SELECT … WHERE { BGP }` forms.
- [ ] `pgrdf.sparql(q TEXT) RETURNS SETOF RECORD` UDF surface.
- [ ] Plan cache hit ratio reported via `pgrdf.stats()`.
- [ ] W3C SPARQL 1.1: ≥ 30% of `manifest.ttl` test cases passing.
- [ ] pg_regress: covers `SELECT`, `WHERE`, BGP, simple `FILTER` constants.

---

## Phase 3 — Semantic Engine

**Outcome:** materialized OWL 2 RL inference and SHACL validation work
against real ontologies.

Gates:
- [ ] `pgrdf.materialize(graph_id BIGINT)` materializes inferred quads
      with `is_inferred = TRUE` via streaming COPY back to hexastore.
- [ ] `pgrdf.validate(data BIGINT, shapes BIGINT) RETURNS JSONB`
      returns W3C-conformant ValidationReport JSONB.
- [ ] W3C SPARQL 1.1: ≥ 70% pass. SHACL: ≥ 50% pass.
- [ ] Reasoner correctness gated by a small fixed OWL 2 RL fixture
      set (pizza ontology subset) + diff against expected closure.

---

## Phase 4 — Release & Containerization

**Outcome:** pgRDF is consumable by external operators (CloudNativePG,
StackGres) following INSTALL spec methodology. Benchmarked.

Gates:
- [ ] LUBM-100 results in `target/perf-report.json` compared against
      Apache Jena TDB and Apache AGE.
- [ ] OCI artifact published at `ghcr.io/styk-tv/pgrdf-bundle:<ver>`
      (INSTALL §11 OQ1 satisfied).
- [ ] Conformance test from INSTALL §12 runs in CI against a fresh K8s
      cluster (kind or k3s).
- [ ] SHA256SUMS.asc detached GPG signature attached to every release.
- [ ] W3C SPARQL 1.1: ≥ 95% pass. SHACL: ≥ 90% pass.

---

## Out of scope (v0.x)

- Streaming replication / logical decoding of RDF state.
- Federated SPARQL `SERVICE`.
- Full OWL 2 (EL / QL) reasoning. ERRATA E-002.
- Backup/restore for opaque binary state (tracked by future
  `SPEC.pgRDF.BACKUP.v0.x`, INSTALL §11 OQ5).
