# 09 — Release pipeline

Tag-based. Push a tag matching `v*` to trigger
`.github/workflows/release.yml`, which produces the release artifact
matrix specified in INSTALL spec §3.

The current cut is `v0.4.0`. Cargo.toml reads `version = "0.4.0"`
(bumped from `0.3.0` during the v0.4 release pre-flight). See
`CHANGELOG.md` for the running set of `[Unreleased]` entries that
move into the next `[N.M.P]` block at tag time.

## v0.4.0 — 2026-05-15

The first pgRDF release with the full four-engine mission shipping
in earnest: storage, SPARQL, OWL 2 RL inference, and now W3C SHACL
Core validation. The validation engine stops being a stub — the
v0.3.0 `pgrdf.validate(data, shapes)` returned
`{"status": "stub", …}`; v0.4.0 returns a real W3C
`sh:ValidationReport`-shape JSONB via `shacl 0.3.1`.

### Engine surface delta vs v0.3.0

- **Storage / SPARQL / OWL 2 RL inference** — unchanged from v0.3.0.
- **SHACL Validation (Phase 5)** — **real impl shipped** (commit
  `ac40bc2`). `pgrdf.validate(data_graph_id, shapes_graph_id)`
  rehydrates shapes from the dictionary-encoded graph, builds the
  `shacl 0.3.1` validator, runs validation, and serialises the
  W3C `sh:ValidationReport` to JSONB. Covers `sh:NodeShape` +
  `sh:property` + `sh:class` / `sh:datatype` + cardinality,
  value-type, value-range, node-kind, pattern, and `sh:in`
  constraints — whatever `shacl 0.3.1`'s SHACL Core
  implementation covers. New regression `71-shacl-real.sql`
  exercises `sh:datatype` violations; the existing
  `70-validate-stub.sql` was repurposed to lock the real-impl
  basic shape (vacuously-conforming + unknown-graph degenerate
  cases). Three new `#[pg_test]` integration tests
  (`validation::shacl::tests`).

### The fork-patch caveat

`Cargo.toml` carries a `[patch.crates-io]` block overriding
`reasonable` to the
[`styk-tv/reasonable@rdf12-passthrough`](https://github.com/styk-tv/reasonable/tree/rdf12-passthrough)
fork. The patch adds a `TermRef::Triple(_)` arm needed for
coexistence with `shacl 0.3.x` under `oxrdf`'s `rdf-12` feature
(workspace-wide enablement via `rudof_rdf`). Strictly additive
when the feature is off; panics with a clear message when on.

Upstream PR: [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50).
Once that merges, **v0.4.1** drops the `[patch.crates-io]` block
and pins the released `reasonable` version. Tracked at
[`specs/ERRATA.v0.4.md`](../specs/ERRATA.v0.4.md) E-011.

Users `cargo build`ing from source pull the fork transparently
via Cargo's `[patch.crates-io]` resolution — no manual git pull
required.

### Test bar

- 94 pgrx integration tests (`cargo pgrx test`, +1 vs v0.3.0)
- 40 pg_regress golden tests (+1 vs v0.3.0 — `71-shacl-real.sql`)
- 23 W3C-shape SPARQL conformance tests (unchanged)
- 3 LUBM-shape correctness gates (unchanged)
- Plus manual smoke: 24 ontologies, 17,134 triples
  (totals locked in `tests/perf/smoke-ontologies.expected.tsv`
  with `--check` mode)

**Total: 160 automated + 24-ontology manual smoke.** All green
at cut time.

### Supported Postgres

PG 14, 15, 16, 17 × {amd64, arm64} = 8 prebuilt tarballs.
PG 18 still deferred per ERRATA E-006 (carried).

### Tarball layout

`pgrdf-0.4.0-pg<N>-glibc-<arch>.tar.gz`:

```
pgrdf-0.4.0-pg<N>-glibc-<arch>/
├── lib/pgrdf.so
├── share/extension/pgrdf.control
├── share/extension/pgrdf--0.4.0.sql
├── LICENSE
├── NOTICE
└── SHA256SUMS   (per-tarball, covers every file above)
```

Plus an aggregate `SHA256SUMS` attached to the GitHub Release
covering every `pgrdf-*.tar.gz` asset.

### Known issues

Carried from v0.3.0 plus the new E-011 entry. See
[`specs/ERRATA.v0.4.md`](../specs/ERRATA.v0.4.md):

- **E-011** — `[patch.crates-io]` to the `styk-tv/reasonable`
  fork in place. Drops in v0.4.1 once
  [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
  merges.
- **E-006** / **E-007** / **E-009** / **E-010** — carried, see
  v0.3.0 entry below.

### What's deferred from the v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md):
named-graph + SPARQL UPDATE + lifecycle UDFs + CONSTRUCT +
property paths (§3-§7), the SPARQL backlog from v0.3 (§11),
heap_multi_insert phase B (§12), and the W3C SPARQL 1.1
manifest runner (§13). These land in subsequent v0.4.x point
releases or in a refreshed v0.5.0 cut.

## v0.3.0 — 2026-05-14 (planned)

The first official pgRDF release. Ships the v0.3 engine surface
feature-complete state: dictionary-encoded quad storage, the SELECT /
ASK SPARQL surface, OWL 2 RL inference, a SHACL validation stub, the
regression + W3C-shape + LUBM-shape harnesses in CI, and the
{pg14..pg17}×{amd64, arm64} release tarball pipeline. The actual tag
date stamps when the release commit lands and the matrix turns green.

### Engine surface

- **Storage (Phase 1, Phase 2.0, Phase 2.1, Phase 2.2)** —
  dictionary-encoded terms (`_pgrdf_dictionary`, HASH index on
  `lexical_value`), LIST-partitioned quads keyed by `graph_id`, the
  SPO / POS / OSP hexastore covering indexes, and Turtle ingest.
  Surface UDFs: `pgrdf.parse_turtle`, `pgrdf.load_turtle`,
  `pgrdf.load_turtle_verbose`, `pgrdf.put_term`, `pgrdf.get_term`,
  `pgrdf.put_quad`, `pgrdf.count_quads`, `pgrdf.add_graph`,
  `pgrdf.version`, `pgrdf.stats`, `pgrdf.shmem_reset`. See LLD §2.
- **SPARQL (Phase 2.2 + Phase 3 SPARQL steps 1–12)** — `pgrdf.sparql`
  answers `SELECT` and `ASK` with N-pattern BGP shared-variable inner
  joins; `FILTER` over identity / boolean / term-type / numeric
  ordering / arithmetic / `REGEX` / `CONTAINS` / `STR*` / `IN`;
  `DISTINCT` / `REDUCED` / `LIMIT` / `OFFSET` / `ORDER BY` with
  `ASC` / `DESC`; single-triple `OPTIONAL` with chained blocks; n-way
  `UNION`; multi-triple `MINUS`; aggregates `COUNT` / `SUM` / `AVG` /
  type-aware `MIN` / `MAX` / `GROUP_CONCAT` (with `SEPARATOR`) /
  `SAMPLE` over `GROUP BY` with `HAVING` (alias form and the inline
  `HAVING(SUM(?v) > c)` shape); `BIND(expr AS ?v)` in projection
  position. `pgrdf.sparql_parse` surfaces the spargebra AST and an
  `unsupported_algebra` array so callers can preview translatability
  without execution. See LLD §3 for the full capability matrix.
- **Storage performance (Phase 3 steps 1–3 phase A)** — shmem
  dictionary cache (LLD §4.1), prepared-plan cache (LLD §4.2), and
  the prepared bulk-INSERT path (LLD §4.3 phase A). The §4.3
  acceptance criterion of a 2× ingest wall-clock improvement is NOT
  met by phase A alone; phase B (`heap_multi_insert` / `COPY BINARY`)
  is deferred to v0.4 per LLD §4.3.
- **Inference (Phase 4)** — `pgrdf.materialize` runs forward-chaining
  OWL 2 RL via `reasonable 0.4`. See LLD §5.2.
- **Validation (Phase 5)** — `pgrdf.validate` ships as a stub
  (`{"status": "stub", …}`) with the stable SQL surface in place.
  Real SHACL execution is blocked upstream per ERRATA E-009; clients
  and tooling can wire against the surface now and pick up real
  validation when the upstream crate set re-aligns. See LLD §5.3.
- **CI + release (Phase 6 steps 1–3, partial)** — the regression
  suite runs in CI (`.github/workflows/ci.yml::regression`), the
  W3C-shape and LUBM-shape harnesses run alongside, and the release
  workflow (`.github/workflows/release.yml`) builds the
  {pg14..pg17}×{amd64, arm64} matrix on tag push. The
  `SHA256SUMS.asc` GPG-signing follow-up is deferred to v0.4
  (no signing key provisioned yet — see §Aggregate checksums above).

### Test bar

- 93 pgrx integration tests (`cargo pgrx test`)
- 39 pg_regress golden tests
- 23 W3C-shape SPARQL conformance tests (hand-authored harness)
- 3 LUBM-shape correctness gates
- Plus manual smoke: 24 ontologies, 17 134 triples (W3C / Apache
  Jena / ValueFlows / ConceptKernel), totals locked in
  `tests/perf/smoke-ontologies.expected.tsv` with `--check` mode

**Total: 158 automated + 24 manual smoke.** All green at cut time.
Expected outputs are hand-computed — no autobaselining of new query
coverage (LLD §6.2).

### Performance characteristics

- **Shmem dict cache** — lookup latency on cache hit < 1 µs (LWLock
  share + ≤ 8 slot probes, ~120 ns on commodity hardware).
  Cross-backend hit rate verified empirically by
  `tests/regression/sql/50-shmem-dict-cache.sql`.
- **Prepared-plan cache** — identical algebra reuses the cached plan
  on its second and subsequent executions; per-backend, keyed by the
  canonical algebra SQL string. Bypasses Postgres parse + plan.
- **Bulk ingest (phase A)** — `flush_batch` routes through the same
  prepared-plan path as SPARQL. The 2× wall-clock target is NOT met
  by phase A alone — observed `synth-10k.ttl` load time is ~85 ms
  steady-state both before and after, dominated by the per-batch
  executor walk. Hitting the bar requires phase B's
  `heap_multi_insert` / `COPY BINARY` work, deferred to v0.4.

### Supported Postgres

PG 14, 15, 16, 17 across `{amd64, arm64}` — **8 prebuilt tarballs
per release.** PG 18 is held out of the matrix pending ERRATA E-006
(pgrx upstream now supports PG 18 at 0.18.0, but local-compile
blockers and a breaking migration — `pgrx_embed` removal,
`crate-type` change — keep us on 0.16.1 for v0.3; the pgrx-0.18
bump is a planned v0.4 work item).

### License + attribution

Apache License 2.0. Copyright 2026 Peter Styk
&lt;peter@styk.tv&gt;. The `LICENSE` file carries the resolved
copyright notice (project URL in place of the upstream `[yyyy]
[name of copyright owner]` placeholders) and a `NOTICE` file at
the repo root carries the Apache convention header. Both files
are distributed inside every per-arch tarball per Apache 2.0
§4(d). `Cargo.toml` declares `authors = ["Peter Styk
<peter@styk.tv>"]` and a `homepage` URL alongside `repository`.

### MSRV

`rust-version = "1.91"` (Cargo.toml). The Linux builder pins
`rust:1.91-bookworm`.

### Tarball layout (INSTALL §3)

`pgrdf-0.3.0-pg<N>-glibc-<arch>.tar.gz`:

```
pgrdf-0.3.0-pg<N>-glibc-<arch>/
├── lib/pgrdf.so
├── share/extension/pgrdf.control
├── share/extension/pgrdf--0.3.0.sql
├── LICENSE
├── NOTICE
└── SHA256SUMS   (per-tarball, covers every file above)
```

Plus an aggregate `SHA256SUMS` attached to the GitHub Release that
covers every `pgrdf-*.tar.gz` asset. Internal layout verified
end-to-end by slice #25 (manual repack) and slice #24 (clean-container
smoke-install round-trip).

### Upgrade policy

pgRDF v0.x reserves the right to break schema and UDF signatures
between minor releases. There is no in-place upgrade path:
`ALTER EXTENSION pgrdf UPDATE` is not supported and is deferred
until v1.0. The supported v0.x → v0.x procedure is dump-via-SQL
(decode `_pgrdf_quads` against `_pgrdf_dictionary` per graph,
serialise to Turtle externally), `DROP EXTENSION pgrdf CASCADE`,
install the new version, then `CREATE EXTENSION` + re-load. See
[`docs/06-installation.md` § Upgrade between v0.x versions](06-installation.md#upgrade-between-v0x-versions)
for the full procedure, the rationale, and the cluster-managed
guidance. v1.0 will introduce proper `ALTER EXTENSION pgrdf UPDATE`
migrations alongside a frozen on-disk schema.

### Known issues

See [`specs/ERRATA.v0.2.md`](../specs/ERRATA.v0.2.md):

- **E-006** — pgrx held at 0.16.1; PG 18 deferred to v0.4.
- **E-007** — INSTALL §7's `extension_control_path` forward path
  blocked by E-006; per-file bind mounts retain the same observable
  end-state.
- **E-009** — `pgrdf.validate` ships as a stub; real SHACL execution
  blocked by upstream `shacl_validation` / `reasonable` feature
  unification.
- **E-010** — 4 informational `cargo audit` advisories accepted for
  v0.3 (all in subtrees of pgrx 0.16.1 / `reasonable 0.4.1` and clear
  automatically when E-006 / E-009 resolve).

### Deferred to v0.4

See
[`specs/SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md)
§2 for the canonical scope (now the authoritative-in-progress v0.4
contract, promoted from `-FUTURE` once SHACL real-impl landed on
`main`). Highlights:

- Named-graph scoping (`GRAPH { … }`) with an IRI ↔ `graph_id`
  mapping table (LLD §3).
- SPARQL UPDATE including graph-scoped variants (LLD §4).
- Graph-level lifecycle UDFs over the LIST-partitioned quads table
  (LLD §5).
- `CONSTRUCT` returning triple-shaped JSONB rows (LLD §6).
- Property paths (`*` / `+` / `?` / `^`) beyond simple sequences
  (LLD §7).
- SPARQL surface backlog carried from v0.3: multi-triple `OPTIONAL`,
  `VALUES` inline tables, `BIND`-output usable downstream of FILTER,
  aggregates over `UNION`, and `DESCRIBE` (LLD §11).
- `heap_multi_insert` / `COPY BINARY` ingestion (LLD §4.3 phase B,
  to meet the 2× wall-clock bar; LLD §12).
- `SHA256SUMS.asc` GPG signature for release artifacts (release
  engineering — not in LLD §2, tracked under INSTALL OQ4 / roadmap
  Phase 6 step 3).
- pgrx 0.18 migration + PG 18 in the build matrix (ERRATA E-006 —
  not in LLD §2, tracked via the v0.3 errata block above).

v0.5 / v1.0 forward look (reasoning-profile selector, real SHACL
output once E-009 clears, TriG / N-Quads ingest, incremental
materialisation, RDF 1.2 triple terms) lives in the LLD's §8-§10
and §15 and is not duplicated here.

## The matrix

```
{14, 15, 16, 17}   ×   {amd64, arm64}   =   8 tarballs per release
```

ARM64 builds run on `ubuntu-24.04-arm` (native, no QEMU). AMD64 on
`ubuntu-22.04`. PG 18 is held out of the matrix pending ERRATA E-006
clear (pgrx 0.16.1 supports PG 14-17); PG 18 lands with v0.4.

## Per-job output

`cargo pgrx package --pg-config /usr/lib/postgresql/${PG}/bin/pg_config`
produces:

```
target/release/pgrdf-pg${PG}/
└── usr/
    ├── lib/postgresql/${PG}/lib/pgrdf.so
    └── share/postgresql/${PG}/extension/
        ├── pgrdf.control
        └── pgrdf--<version>.sql
```

We repack to the INSTALL §3 layout:

```
pgrdf-<ver>-pg${PG}-glibc-<arch>.tar.gz
├── lib/pgrdf.so
├── share/extension/pgrdf.control
├── share/extension/pgrdf--<ver>.sql
├── LICENSE
└── SHA256SUMS
```

## Aggregate checksums

SHA256SUMS coverage is wired in `release.yml` at **both** levels (per
slice #28 audit; supersedes the older slice #36 note that flagged this
as outstanding):

- **Per-tarball.** Each `pgrdf-<ver>-pg<PG>-glibc-<arch>.tar.gz` carries
  its own internal `SHA256SUMS` covering every file inside the tarball
  (`lib/pgrdf.so`, `share/extension/*`, `LICENSE`, `NOTICE`). Generated
  by the `Repack to INSTALL-spec layout` step in the `build` job.
- **Aggregate.** The `release` job downloads all per-arch tarballs and
  emits a top-level `SHA256SUMS` covering every `pgrdf-*.tar.gz` in the
  release, attached as a separate asset alongside the tarballs.

INSTALL spec §3 also calls for a detached GPG signature
(`SHA256SUMS.asc`). **This is deferred to v0.4** — no `GPG_PRIVATE_KEY`
secret or signing key is yet provisioned for the workflow; v0.3 ships
with SHA256SUMS-only integrity. The `.asc` follow-up requires (a)
sourcing a release-signing key, (b) publishing the public half on a
keyserver or release page, (c) wiring `GPG_PRIVATE_KEY` into the
workflow's release job. Tracked under INSTALL OQ4 and roadmap Phase 6
step 3 (`docs/10-roadmap.md`).

### Verification (consumer side)

To verify a downloaded release tarball matches the published checksum:

```bash
# Download the tarball + the aggregate SHA256SUMS from the GitHub release.
curl -LO https://github.com/styk-tv/pgRDF/releases/download/v0.3.0/pgrdf-0.3.0-pg17-glibc-amd64.tar.gz
curl -LO https://github.com/styk-tv/pgRDF/releases/download/v0.3.0/SHA256SUMS

# Verify (filters to the line matching the downloaded file).
sha256sum -c SHA256SUMS --ignore-missing
# expected: pgrdf-0.3.0-pg17-glibc-amd64.tar.gz: OK
```

The internal per-tarball `SHA256SUMS` can be verified after extraction
via `cd pgrdf-<ver>-pg<PG>-glibc-<arch> && sha256sum -c SHA256SUMS` —
this catches in-flight corruption of individual files within the
tarball, orthogonal to the aggregate-tarball check above. Once
`SHA256SUMS.asc` lands in v0.4, the additional step is
`gpg --verify SHA256SUMS.asc SHA256SUMS` against the published signing
key.

## Trigger

```bash
git tag v0.3.0
git push origin v0.3.0
```

## Manual re-runs

The workflow today only fires on `push: tags: ["v*"]` — there is no
`workflow_dispatch` entry. If a single matrix cell fails, use the
GitHub Actions UI to "Re-run failed jobs" on the existing tag run;
do not delete and re-push the tag (that produces a new run and a
duplicate release draft). Adding `workflow_dispatch` is a small
follow-up if manual triggering becomes useful.

## Pre-release vs release

Tags matching `v*-alpha.*`, `v*-beta.*`, `v*-rc.*` are treated as
pre-releases by `softprops/action-gh-release@v2` (default behaviour
of the action's tag-name heuristic). GitHub's own `releases/latest`
endpoint then points at the most recent non-prerelease tag only, so
consumers tracking `latest` see stable tags only. INSTALL spec §5
itself pins to a specific `RELEASE_URL` (no `latest` template); the
prerelease distinction is GitHub-side, not INSTALL-side.

## Verification after release

Run the conformance check from a clean K8s namespace using INSTALL
spec §5 manifest with the newly-tagged version. CI doesn't do this
yet — Phase 6 deliverable per `docs/10-roadmap.md` (the INSTALL §12
fresh-cluster check listed under "Phase 6 — CI + Conformance +
Release").

## Release notes

GitHub auto-generates from PR titles by virtue of `generate_release_notes: true`.
Update [`CHANGELOG.md`](../CHANGELOG.md) before tagging so the
human-readable summary (moved out of `[Unreleased]` into the new
`[N.M.P] — YYYY-MM-DD` block) exists alongside the auto-generated
PR-title list. Per-release LUBM benchmark numbers and noteworthy
deltas land here too once Phase 6 step 2 wires the LUBM-10 /
LUBM-100 harness (see `docs/08-testing.md` Layer 8).
