# 09 — Release pipeline

Tag-based. Push a tag matching `v*` to trigger
`.github/workflows/release.yml`, which produces the release artifact
matrix specified in INSTALL spec §3.

The current cut is **`v0.6.15`** (`isPrerelease=false`,
`isLatest=true`); Cargo.toml + `pgrdf.control` read `0.6.15`, and the
tagged release carries the binary tarball matrix, the PGXN source
zip, and an SLSA-attested OCI bundle. The per-release notes below
cover the v0.5.x line and earlier; the full v0.6.0 → v0.6.14 history
(the parallel bulk loader and its levers) lives in `CHANGELOG.md`,
which is the authoritative running log — new entries land under
`[Unreleased]` and move into the next `[N.M.P]` block at tag time.

## v0.6.15 — 2026-06-26

A correctness cut: the staged loader no longer corrupts a second load into an
already-populated dictionary (#8) — the STAGE_PREP worker detects a non-empty
dict and falls back to the combined path. `.so`-only, **no schema delta**; the
`0.5.1 → 0.6.15` upgrade path is a version-string rename of
`sql/pgrdf--0.5.1--0.6.15.sql`.

### Control-version reconciliation

`Cargo.toml` `version`, `Cargo.lock` (`pgrdf` entry), `pgrdf.control`
`default_version`, `META.json` `version`, the `cargo pgrx package` SQL filename,
the `compose/compose.yml` bind-mount, and Postgres `extversion` are all
identically `0.6.15`. `00-smoke.out` literals move to `0.6.15`.

### Cut file set

`Cargo.toml` + `Cargo.lock` + `pgrdf.control` + `META.json` +
`compose/compose.yml` (the single `pgrdf--0.6.15.sql` bind-mount line) +
`tests/regression/expected/00-smoke.out` + `git mv sql/pgrdf--0.5.1--0.6.{14→15}.sql`
(no DDL) + `CHANGELOG.md` (`[Unreleased]` stays, new `[0.6.15]` block) + README /
guide / `compose/README.md` install version refreshes (the bench-table data points
stay at the v0.6.14 run) + this section. Also lands the first graph-carve
query-pattern groundwork (#6, regression `135-type-closure-lubm-patterns`). The
fix itself is `src/storage/staged/{pool,jobctl}.rs` + `src/storage/loader.rs`
(runtime / `.so`, no schema).

## v0.6.14 — 2026-06-24

The out-of-the-box at-scale staged-ingest cut. Five tuning levers
(T1–T5) make a full Wikidata-truthy load complete on stock PostgreSQL
with no custom server config. `.so` + GUC changes only, no schema
delta. Validated by a full **8.2 B-triple** Wikidata-truthy
out-of-the-box load on **E64ads_v7** (64c / 503 GiB / 3.4 T PGDATA):
quads == triples == **8,199,708,346**, dict **1.80 B** terms, **no
ENOSPC**.

- **T1 — `pgrdf.staged_temp_tablespaces`.** Routes the RESOLVE phase's
  temp spill to an operator-named tablespace (off the PGDATA volume) so
  a multi-TB hash build does not exhaust PGDATA.
- **T2 — `pgrdf.staged_resolve_strategy` (`hash` | `index` | `auto`),
  DEFAULT NOW `index`.** Forces the RESOLVE join method. The new
  `index` default is the at-scale-validated low-spill
  index-nested-loop path: at 8.2 B rows the historical all-hash join
  spills multi-TB to temp, while `index` completes with no multi-TB
  spill / no ENOSPC. The join output is identical for any method — a
  performance knob, not a correctness one.
- **T3 — parallel multi-backend STAGE COPY.** The STAGE phase fans the
  COPY across the staged-loader background-worker pool.
- **T4 — format-aware staged dispatch.** The staged loader dispatches
  on the input RDF format.
- **T5 — adaptive self-tune + self-tune log.** `work_mem` /
  `maintenance_work_mem` adapt to host RAM and the chosen strategy,
  with a self-tune log recording the resolved settings.

### Control-version reconciliation

`Cargo.toml` `version`, `pgrdf.control` `default_version`, `META.json`
`version`, the `cargo pgrx package` SQL filename, the
`compose/compose.yml` bind-mount, and Postgres `extversion` are all
identically `0.6.14`. `00-smoke.out` literals move to `0.6.14`.

### Cut file set

`Cargo.toml` (version only) + `Cargo.lock` (`pgrdf` package entry) +
`pgrdf.control` (`default_version`) + `META.json` +
`compose/compose.yml` (the single SQL bind-mount line
`pgrdf--0.6.14.sql`) + `tests/regression/expected/00-smoke.out`
(version literals) + `git mv sql/pgrdf--0.5.1--0.6.{13→14}.sql` (the
upgrade-path is a version-string bump only — no DDL) + `CHANGELOG.md`
(`[Unreleased]` stays, new `[0.6.14]` block) + `README.md` / guide /
docs install + version refreshes + `docs/09-release.md` (this section).
The T1–T5 levers themselves are `src/query/guc.rs` +
`src/storage/staged/*` (runtime / GUC / `.so`, no schema).

## v0.6.13 — 2026-06-23

A diagnostic + memory-hardening cut for the staged bulk loader, ahead
of the out-of-the-box at-scale work in 0.6.14. Two `.so`-only changes,
no schema delta.

When a staged-loader background worker died, the coordinator reported
the opaque `"staged worker: unknown panic"` — useless for diagnosing an
at-scale failure. A worker that hits a PostgreSQL `ERROR` (an
out-of-memory or query error during `RESOLVE`) has it re-raised by pgrx
as an `ErrorReportWithLevel` panic payload, which the old handler —
downcasting only to `&str` / `String` — never recognised. The worker
panic handler now downcasts in richness order (`CaughtError` /
`ErrorReportWithLevel` / `ErrorReport` / `&str` / `String`) and
surfaces the **real PostgreSQL ERROR text** in the job's `error`,
falling back to a phase+shard+pid-stamped pointer to the server log
only for a truly unrecognised payload. A staged failure is now
diagnosable.

The staged loader also set a fixed `work_mem = '2GB'` /
`maintenance_work_mem = '16GB'` regardless of host RAM. At scale on a
smaller-RAM host that is dangerous: `work_mem × hash_mem_multiplier ×
parallel workers × the 3-way RESOLVE join` implied a ~384 GB
parallel-hash budget on a 251 GiB host. `work_mem` and
`maintenance_work_mem` now **scale to host RAM** (read from
`/proc/meminfo`, with the prior fixed values as the fallback where
unreadable), bounded so the parallel-hash budget stays within ~half of
RAM — `RESOLVE` spills to temp instead of risking OOM. This is memory
**hardening**; the definitive at-scale `RESOLVE` fix and out-of-the-box
self-tuning of the full ingest are the **0.6.14** follow-up, now
diagnosable thanks to the panic-reporting above.

### Control-version reconciliation

`Cargo.toml` `version`, `pgrdf.control` `default_version`, `META.json`
`version`, the `cargo pgrx package` SQL filename, the
`compose/compose.yml` bind-mount, and Postgres `extversion` are all
identically `0.6.13`. `00-smoke.out` literals move to `0.6.13`.

### Cut file set

`Cargo.toml` (version only) + `Cargo.lock` (`pgrdf` package entry) +
`pgrdf.control` (`default_version`) + `META.json` +
`compose/compose.yml` (the single SQL bind-mount line
`pgrdf--0.6.13.sql`) + `tests/regression/expected/00-smoke.out`
(version literals) + `git mv sql/pgrdf--0.5.1--0.6.{12→13}.sql` (the
upgrade-path is a version-string bump only — no DDL) + `CHANGELOG.md`
(`[Unreleased]` stays, new `[0.6.13]` block) + `README.md` / guide /
docs install + version refreshes + `docs/09-release.md` (this section).
The worker panic-reporting + RESOLVE memory hardening themselves are
`src/storage/staged/{pool,phases}.rs` (runtime / `.so`, no schema).

## v0.6.12 — 2026-06-22

A correctness cut for the staged bulk loader. The staged loader's
`DICT` phase deduplicated literals by **lexical value alone**
(`GROUP BY o_val` with `max(datatype)` / `max(language)`), collapsing
distinct RDF literals that share a value — `"Berlin"@en`, `"Berlin"@de`,
`"1"^^xsd:integer`, and a plain `"1"` folded into a single dictionary
row stamped with an arbitrary (max) datatype **and** language (an
impossible RDF term). At Wikidata-`truthy` scale this produced
**21,666,575** dictionary rows carrying both a datatype and a language
tag and silently dropped language / datatype variants, masked by the
`quads == triples` count check that a value-only collapse still
satisfies. v0.6.12 keys the literal dictionary on the **full literal
identity** `(lexical_value, datatype, language)` and the `RESOLVE`
phase matches literal objects on that full key, so each distinct
literal resolves to its own id. A regression test locks it. The fix is
internal to the loader's set-based SQL — **runtime / `.so`, no schema
change**.

The staged loader (`load_turtle_staged_run`, the 0.6.11 R2.1
coordinator) stays **opt-in**; it is now correct on multilingual /
typed data. Promoting it to the **default** ingest path is deferred to
**0.6.13**, pending the `RESOLVE`-phase memory-adaptivity work (today
`RESOLVE` forces all-hash-joins and is not spill-tolerant: a full
8.2-billion-triple load completes on a 1.28 TiB host but exhausts a
251 GiB host during `RESOLVE`).

### Control-version reconciliation

`Cargo.toml` `version`, `pgrdf.control` `default_version`, `META.json`
`version`, the `cargo pgrx package` SQL filename, the
`compose/compose.yml` bind-mount, and Postgres `extversion` are all
identically `0.6.12`. `00-smoke.out` literals move to `0.6.12`.

### Cut file set

`Cargo.toml` (version only) + `Cargo.lock` (`pgrdf` package entry) +
`pgrdf.control` (`default_version`) + `META.json` +
`compose/compose.yml` (the single SQL bind-mount line
`pgrdf--0.6.12.sql`) + `tests/regression/expected/00-smoke.out`
(version literals) + `git mv sql/pgrdf--0.5.1--0.6.{11→12}.sql` (the
upgrade-path is a version-string bump only — no DDL) + `CHANGELOG.md`
(`[Unreleased]` stays, new `[0.6.12]` block) + `README.md` / guide /
docs install + version refreshes + `docs/09-release.md` (this section).

## v0.5.1 — 2026-05-23

Maintenance cut only. **No engine delta vs v0.5.0.** The release
packages the existing v0.5.0 RDF / SPARQL / SHACL / OWL surface as
`0.5.1`, adds PGXN source-distribution assets (`META.json`,
`README.pgxn.md`, `INSTALL.md`, `Makefile`, release `pgrdf-0.5.1.zip`),
keeps the compose artifact-parity check from the prep commit, and
aligns the legal surface to MIT (`NOTICE` removed from the tarballs and
repo root).

### Control-version reconciliation

`Cargo.toml` `version`, `pgrdf.control` `default_version`, `META.json`
`version`, the `cargo pgrx package` SQL filename, and Postgres
`extversion` are all identically `0.5.1`. `00-smoke.out` literals move
to `0.5.1`.

### Cut file set

`Cargo.toml` + `Cargo.lock` + `pgrdf.control` + `META.json` +
`README.pgxn.md` + `INSTALL.md` + `README.md` + install/doc refreshes +
`tests/regression/expected/00-smoke.out` + `CHANGELOG.md` +
`RELEASE_NOTES.md` + `.github/workflows/release.yml` /
`oci-publish.yml` (PGXN zip asset path).

### Release artifact matrix

- 8 binary tarballs: `pg14..17 × amd64/arm64`
- `pgrdf-0.5.1.zip` PGXN source archive
- aggregate `SHA256SUMS`
- OCI publish still reuses the tagged GitHub assets (no rebuild)

## v0.5.0 — 2026-05-16

The final v0.5 cut. Phase H promotes `SPEC.pgRDF.LLD.v0.5-FUTURE.md`
to the authoritative `SPEC.pgRDF.LLD.v0.5.md` (shipped contract),
opens `SPEC.pgRDF.LLD.v0.6-FUTURE.md`, adds the `oci-publish.yml`
workflow, and cuts **v0.5.0** — the complete RDF / SPARQL / SHACL /
OWL surface. **This release is NOT a prerelease**: it is marked
`isLatest=true` and supersedes `v0.4.6`.

### Engine surface delta vs v0.5.0-rc1

- **No engine change.** v0.5.0 is byte-identical to v0.5.0-rc1 at
  the `src/` level — the v0.5-gate surface (§3-§8) shipped in the
  Phase G groups G1/G2/G3 and is unchanged. v0.5.0 is the version
  bump + spec promotion + the OCI-publish workflow only.
- E-013 is **resolved** (no upstream `sh:nodeKind` bug; the W3C
  SHACL Core gate is a genuine **25/25 full-pass, no exclusion** —
  established at v0.5.0-rc1, carried into the final cut unchanged).
- E-012 is a **documented upstream-gate, final for v0.5.0** (the
  `shacl 0.3.1` SHACL-SPARQL stub; the `mode => 'sparql'` surface
  ships honest + forward-compatible, not a pgRDF defect).

### Control-version reconciliation

`Cargo.toml` `version`, `pgrdf.control` `default_version`, the
`cargo pgrx package` SQL filename, and the Postgres `extversion`
are **all identically `0.5.0`** — no reconciliation needed (a clean
semver with no pre-release tag; `cargo pgrx package` emits
`pgrdf--0.5.0.sql`; `CREATE EXTENSION pgrdf` reports `0.5.0`).
`00-smoke.out` literals updated to `0.5.0`.

### Cut file set (mirrors the v0.4.6 / v0.5.0-rc1 cut exactly)

`Cargo.toml` (version only — metadata untouched) + `Cargo.lock`
(`cargo update -p pgrdf`) + `pgrdf.control` (`default_version`) +
`compose/compose.yml` (the single SQL bind-mount line
`pgrdf--0.5.0.sql`, the v0.4.6 F4 one-line pattern) +
`tests/regression/expected/00-smoke.out` (version literals) +
`CHANGELOG.md` (`[Unreleased]` + `[0.5.0-rc1]` → `[0.5.0]`) +
`RELEASE_NOTES.md` (full rewrite for the final cut) +
`docs/09-release.md` (this section). Same 8-file shape as the
v0.4.6 / v0.5.0-rc1 cuts.

**Ritual deviations vs the v0.4.6 cut:** README.md NOT touched — a
parallel docs session owns it, same deviation as every prior cut.
The spec promotion + the new `oci-publish.yml` + the v0.5-FUTURE
cross-ref sweep + the Phase H CHANGELOG entry landed in the **prep
commit**, not here, mirroring how every prior cut keeps the
non-version-bump surface out of the release commit. No `src/` fmt
sweep needed (`cargo fmt --all -- --check` clean).

**Latest, NOT prerelease.** v0.5.0-rc1 was explicitly flagged a
prerelease (`gh release edit v0.5.0-rc1 --prerelease`); v0.5.0 is
the opposite — after `release.yml` creates the release, the cut
asserts `gh release view v0.5.0 --json isPrerelease,isLatest` and,
if needed, `gh release edit v0.5.0 --prerelease=false --latest` so
it ends `isPrerelease=false`, `isDraft=false`, `isLatest=true`,
superseding `v0.4.6`.

### OCI publish (ghcr.io/styk-tv/pgrdf-bundle)

The new `.github/workflows/oci-publish.yml` triggers on
`release: [published]` (and `workflow_dispatch` with a `tag`
input). It installs ORAS, downloads the release tarballs +
`SHA256SUMS` (no rebuild — the artifacts are exactly the
release.yml output), verifies the checksums, then pushes one OCI
artifact per PG×arch (`:0.5.0-pg17-amd64`, …) and builds the
aggregate `:0.5.0` / `:v0.5.0` index manifests.

**One-time maintainer make-public step (manual).** The Actions
`GITHUB_TOKEN` can push packages (`packages: write`) but **cannot**
change package visibility (`admin:packages` is not grantable to the
workflow token). `styk-tv` is a **user** account, so the first
publish lands `ghcr.io/styk-tv/pgrdf-bundle` **private**. To allow
anonymous `oras pull`, a maintainer flips it public once:

- UI: GitHub → your packages → `pgrdf-bundle` → Package settings →
  Danger Zone → Change visibility → Public; **or**
- API: `gh api -X PUT \
    /users/styk-tv/packages/container/pgrdf-bundle/visibility \
    -f visibility=public` (run by a maintainer with `admin:packages`).

This is a flagged maintainer action, **not** part of the automated
cut — the OCI workflow succeeding with a private package is the
expected first-publish state.

E-011 carried: `publish-crate.yml` stays disabled until upstream
[`gtfierro/reasonable#50`](https://github.com/gtfierro/reasonable/pull/50)
merges. The v0.5.0 tag fires `release.yml` (8 platform tarballs
PG14-17 × amd64/arm64 + SHA256SUMS) and `oci-publish.yml` (the OCI
bundle); **no crates.io publish this cut**.

## v0.5.0-rc1 — 2026-05-16

Phase G closes the v0.5 capability scope across three grouped
dispatches (G1 → G3, countdown 21 → 12). **All v0.5-FUTURE
v0.5-gate tracks §3-§8 are shipped** — this is the v0.5.0-rc1
headline. This is a **release candidate**: the tag is flagged a
GitHub *prerelease* so it does not become "latest" over `v0.4.6`;
the final `v0.5.0` follows after Phase H+I hygiene + the two
documented E-012 / E-013 follow-ups.

### Engine surface delta vs v0.4.6

- **Storage / inference / SPARQL UPDATE / CONSTRUCT / property
  paths / §11 backlog / TriG-N-Quads / reasoning-profile** —
  unchanged from the v0.4.x + Phase G G1/G2 surface; no breaking
  changes. Table shapes (`_pgrdf_graphs`, `_pgrdf_quads`,
  `_pgrdf_dictionary`) unchanged.
- **`pgrdf.validate` gains a third optional arg** —
  `pgrdf.validate(data, shapes, mode TEXT DEFAULT 'native')`. The
  2-arg form is byte-identical to v0.4 (defaults `'native'`). JSONB
  gains a `mode` field; unknown mode →
  `validate: unknown mode` (validated before any work).
  `'sparql'` returns a deterministic structured report — the
  upstream `shacl 0.3.1` SparqlEngine is a stub (ERRATA.v0.5
  E-012); pgRDF does not invoke it.
- **New W3C SHACL Core manifest gate** — `just test-shacl-manifest`
  (`tests/w3c-shacl/`), wired into `ci.yml` on every PG major as a
  real gate. Vendored hermetic W3C SHACL Core subset, genuine
  **25/25 full-pass** on `sh:conforms`, **no exclusion** (ERRATA.v0.5
  E-013). E-013's earlier "one W3C Core fixture documented-excluded
  for an upstream `sh:nodeKind` bug" claim was a G3 unverified
  assumption (the fixture was committed straight into
  `fixtures/excluded/` so the harness never ran it); a triple-verified
  investigation at v0.5.0-rc1 found no upstream bug — the fixture is
  restored to `fixtures/core/` and PASSes.

### Control-version reconciliation (`-rc1`)

`Cargo.toml` `version`, `pgrdf.control` `default_version`, the
`cargo pgrx package` SQL filename, and the Postgres `extversion`
are **all identically `0.5.0-rc1`** — no reconciliation needed.
Verified empirically: `cargo metadata` accepts the SemVer
pre-release; `cargo pgrx package` (pgrx 0.16.1) emits
`pgrdf--0.5.0-rc1.sql`; `CREATE EXTENSION pgrdf` on PG 17 succeeds
and `pgrdf.version()` / `pg_extension.extversion` both report
`0.5.0-rc1`. Postgres only forbids `--` and leading/trailing `-`
in an extension version; a single internal `-rc1` is valid in the
control file, the `pgrdf--<ver>.sql` filename, and Postgres's
extension-version parser. `00-smoke.out` literals updated to
`0.5.0-rc1`.

### Cut file set (mirrors the v0.4.6 cut exactly)

`Cargo.toml` (version only) + `Cargo.lock` (`cargo update -p
pgrdf`) + `pgrdf.control` (`default_version`) +
`compose/compose.yml` (the single SQL bind-mount line
`pgrdf--0.5.0-rc1.sql`, the v0.4.6 F4 one-line pattern) +
`tests/regression/expected/00-smoke.out` (version literals) +
`CHANGELOG.md` (`[Unreleased]` → `[0.5.0-rc1]`) + `RELEASE_NOTES.md`
(full rewrite) + `docs/09-release.md` (this section). Same 8-file
shape as the v0.4.6 / v0.4.5 cuts.

**Ritual deviations vs the v0.4.6 cut:** README.md NOT touched — a
parallel docs session owns it, same deviation as the
v0.4.6 / v0.4.5 / v0.4.4 cuts. The roadmap / spec / guide / docs
§5/§6 coherence edits + the new `specs/ERRATA.v0.5.md` landed in
the FEATURE commit, not here, mirroring how every prior cut kept
those out of the release commit. No `src/` fmt sweep needed
(`cargo fmt --all -- --check` clean). The compose SQL mount is a
one-line bump (`pgrdf--0.4.6.sql` → `pgrdf--0.5.0-rc1.sql`), the
same net surface as the v0.4.6 cut's F4 single-version mount —
no accumulation.

**Prerelease flag:** an rc MUST be a GitHub *prerelease* so it does
not supersede `v0.4.6` as "latest". If `release.yml` does not
auto-mark prereleases, the cut sets it explicitly
(`gh release edit v0.5.0-rc1 --prerelease`).

E-011 carried: `publish-crate.yml` stays disabled until upstream
[`gtfierro/reasonable#50`](https://github.com/gtfierro/reasonable/pull/50)
merges. The v0.5.0-rc1 tag fires `release.yml` only (8 platform
tarballs PG14-17 × amd64/arm64 + SHA256SUMS); **no crates.io
publish this cut**.

## v0.4.6 — 2026-05-16

Phase F closes with a four-group countdown (34 → 22) shipping the
full LLD v0.4 §11 SPARQL surface backlog, plus the release cut
(group F4 / slice 22). Multi-triple OPTIONAL, VALUES, downstream
BIND, aggregates over UNION, DESCRIBE, and **type-aware ORDER BY**
all execute end-to-end on the SQL engine. **§11 is complete.**

### Engine surface delta vs v0.4.5

- **Storage / OWL 2 RL inference / SHACL / SPARQL UPDATE /
  CONSTRUCT / property paths** — unchanged; no breaking changes to
  existing surfaces. No new user-facing UDF this cut — type-aware
  ORDER BY is internal to the query translator (behind the
  existing `pgrdf.sparql`); `pgrdf.describe` shipped in F3.
- **SPARQL §11 backlog (LLD v0.4 §11)** — **shipped end-to-end
  across the Phase F countdown 34 → 22**. Per group:
    - **F1 (34 → 31)** — multi-triple `OPTIONAL { BGP }` (N-triple
      right side as a LATERAL-style derived table inside the LEFT
      JOIN; atomic, W3C §6.1) + `VALUES` inline tables
      (`(VALUES …) AS vN(cols)` derived table; `UNDEF` →
      no-constraint NULL, W3C §10).
    - **F2 (30 → 27)** — downstream `BIND` (AST substitution pass:
      a BIND var rewritten into a later FILTER / BGP join key /
      chained BIND before the structural walk; unbound-var BIND →
      NULL not error, W3C §18.2.5) + aggregates over `UNION`
      (derived-table refactor; COUNT/SUM/AVG/type-aware
      MIN-MAX/GROUP_CONCAT/SAMPLE, GROUP BY, HAVING).
    - **F3 (26 → 24)** — `DESCRIBE` via the sibling UDF
      `pgrdf.describe(q TEXT) → SETOF JSONB` (byte-identical to
      `pgrdf.construct`; W3C §16.4 closure, transitive one-hop
      blank-node expansion, cycle-safe, dedup'd).
    - **F4 (23 → 22)** — **type-aware ORDER BY** (this cut's
      marquee). Every sort key expands into the SPARQL 1.1 §15.1
      value-space term list: a kind rank (numeric < dateTime <
      boolean < other) + per-kind comparator (numerics
      **numerically** so `2 < 10`, `xsd:dateTime` chronologically,
      `xsd:boolean` false<true, strings by Unicode codepoint via
      `COLLATE "C"`) + codepoint tiebreak; total/stable, never
      raises (regex-guarded casts fall through to the codepoint
      tier). `DESC()` + multi-key + expression sort keys
      (`ORDER BY (?a+?b)`, `ORDER BY STRLEN(?s)`); all four SQL
      builders + `SELECT DISTINCT` compose (an expression key on
      the aggregate/UNION shapes is a documented narrow deferral —
      stable panic, never a wrong answer). ORDER BY was already an
      unflagged SELECT modifier — no `unsupported_algebra` /
      `80-unsupported-shapes` entry to retire. Plus the Phase F
      W3C-shape consolidation (6 fixtures `42-optional-multi-triple`
      … `47-order-by-type-aware`, 41 → 47; the `46-describe`
      fixture introduced a `describe` per-fixture kind alongside
      the slice-51 `construct` kind) and the compose infra-debt
      fix (`compose/compose.yml`: the five stale per-version SQL
      bind-mount lines `pgrdf--0.4.1.sql … pgrdf--0.4.5.sql`
      collapsed to a single per-file mount of the current
      `default_version`'s `pgrdf--<ver>.sql` — a clean
      `cargo pgrx package` emits exactly that one file; the older
      lines only ever resolved from a warm BuildKit cache and were
      the source of the recurring hand-create-a-copy + cold-restart
      workaround. A directory mount was rejected: it shadows the
      stock Postgres extension dir and crash-loops `initdb` on the
      fresh-cluster path CI takes — the F4 feature commit's
      directory-mount attempt failed CI on exactly that and was
      fixed-forward to the per-file single-version mount).
- **`pgrdf.sparql_parse`** — OPTIONAL / VALUES / BIND-downstream /
  aggregate-over-UNION / DESCRIBE no longer flagged in
  `unsupported_algebra` (the LLD §11 acceptance binding).

### Test bar at the v0.4.6 cut

```
pgrx integration  250  (was 248 at v0.4.5 / Phase F3)
pg_regress         79  (was 78 — +1 100-sparql-order-by-type-aware;
                        111 expected corrected to §15.1 codepoint
                        order)
w3c-sparql         47  (was 41 — +6 Phase F fixtures 42-47)
LUBM-shape          3  (unchanged)
Total: 379 green, plus the pg_dump round-trip gate.
```

### Version touches

- `Cargo.toml` `0.4.5` → `0.4.6` (+ the `Cargo.lock` pgrdf entry).
- `pgrdf.control` `default_version = '0.4.6'`.
- `tests/regression/expected/00-smoke.out` `0.4.5` → `0.4.6`.
- `RELEASE_NOTES.md` rewritten for v0.4.6;
  `CHANGELOG.md` `[Unreleased]` → `[0.4.6] — 2026-05-16`.

**`compose/compose.yml` reconciliation with the Part-3 fix:** the
v0.4.5 cut *added* a `pgrdf--0.4.5.sql` bind-mount line (on top of
0.4.1-0.4.4). The Phase F group F4 *feature* commit then collapsed
that five-line stale block to a **single** per-file mount of the
current `default_version`'s SQL (a clean `cargo pgrx package`
emits exactly one SQL file; the older lines only resolved from a
warm BuildKit cache — the recurring infra-debt). So the v0.4.6
release cut's compose touch is a **one-line** change
(`pgrdf--0.4.5.sql` → `pgrdf--0.4.6.sql`), tracking the bumped
`default_version` — the same net surface as every prior cut's
single SQL line, just with no stale-version accumulation. The
release-cut file set is therefore the **same 8 files** as the
v0.4.5 cut (`git show 5cdebdd --stat`): Cargo.toml, Cargo.lock,
pgrdf.control, tests/regression/expected/00-smoke.out,
compose/compose.yml, CHANGELOG.md, RELEASE_NOTES.md,
docs/09-release.md.

**CI-gate fix-forward (F4-specific):** the F4 feature commit first
attempted a *directory* mount for the Part-3 fix; that shadows the
stock Postgres extension dir and crash-looped `initdb` on CI's
fresh-cluster path (`extension "plpgsql" is not available`). It
was fixed-forward in a separate commit to the per-file
single-version mount before this release cut — the CI gate is
green on that fix-forward commit.

**Ritual deviations vs the v0.4.5 cut:** README.md NOT touched — a
parallel docs session owns it, same deviation as the
v0.4.5 / v0.4.4 cuts. The roadmap / spec / guide / docs §11
coherence edits landed in the FEATURE commit, not here, mirroring
how the v0.4.5 cut kept those out of the release commit. No `src/`
fmt sweep needed (`cargo fmt --all -- --check` clean this cut).

E-011 carried: `publish-crate.yml` stays disabled until upstream
[`gtfierro/reasonable#50`](https://github.com/gtfierro/reasonable/pull/50)
merges. The v0.4.6 tag fires `release.yml` only (8 platform
tarballs PG14-17 × amd64/arm64 + SHA256SUMS).

## v0.4.5 — 2026-05-16

Phase E closes with a four-group countdown (49 → 35) shipping the
full LLD v0.4 §7 property-path surface, plus the release cut
(slice 35). `^` inverse, `+` one-or-more, `*` zero-or-more, `?`
zero-or-one, and `|` alternation all execute end-to-end on the SQL
engine and compose with named-graph scoping, BGP joins,
OPTIONAL/UNION/MINUS, and `pgrdf.construct` for free (the shared
WHERE walker recognises `GraphPattern::Path` at the single
chokepoint every query form routes through).

### Engine surface delta vs v0.4.4

- **Storage / OWL 2 RL inference / SHACL / SPARQL UPDATE /
  CONSTRUCT** — unchanged; no breaking changes to existing
  surfaces. The only new UDF is `pgrdf.sparql_sql(q) → TEXT`, a
  translator-introspection debug hook (returns the lowered SQL
  with dict ids inlined) used by the §7.3 EXPLAIN-scrape
  acceptance — not part of the user-facing query surface.
- **SPARQL property-path track (LLD v0.4 §7)** — **shipped
  end-to-end across the Phase E countdown 49 → 35**. Per group:
    - **E1 (49 → 46)** — property-path AST detection + the shared
      `query::path` dispatcher; `^` inverse (`?s ^p ?o` ≡
      `?o p ?s`; nested `^(^p)` folds by parity; bare-predicate
      degenerate `Path` lowers to a triple). New GUC
      `pgrdf.path_max_depth` (Userset, default 64, range 1–1024);
      `pgrdf.stats().path_depth_truncations` scaffold (a
      cross-backend shmem counter, 0 in E1; depth enforcement +
      the increment land with the recursive CTE in E2).
    - **E2 (45 → 42)** — `+` one-or-more: the LLD §7.2
      `WITH RECURSIVE walk(src, dst, depth)` CTE as a derived FROM
      relation, cycle-safe via Postgres's `CYCLE src, dst` clause
      (a bare `UNION` can't dedup a cycle once the tuple carries
      the depth-guard column), depth guard enforced (truncate,
      never error; a per-`+` post-execution probe accounts a
      genuine acyclic cap-hit). All property-path SQL generation
      carved into `src/query/path.rs`.
    - **E3 (41 → 38)** — `*` zero-or-more (the cycle-safe `+` walk
      `UNION` the W3C §9.3 zero-length node-set) and `?`
      zero-or-one (non-recursive: the direct edge `UNION` the same
      node-set). Full W3C SPARQL 1.1 §9.3 `ZeroLengthPath` rules
      (bound endpoint's self-pair unconditional; unbound
      endpoint's node-set = active scope's subject∪object);
      inverse composition `^(p*)` / `(^p)*` / `^(p?)` / `(^p)?`.
    - **E4 (37 → 35)** — `|` alternation: the §7.1 gated stretch
      shipped in full via the predicate-set generalisation
      (`predicate_id = $P` → `predicate_id IN (…)`, a uniform
      one-line change at each builder; a 1-element set is
      byte-identical, so `+`/`*`/`?` are unchanged). Top-level
      `a|b`, n-ary `a|b|c`, the recursion compositions
      `(a|b)+`/`(a|b)*`/`(a|b)?`, and the inverse
      `^(a|b)`/`(^a|^b)` all execute. The materialised-closure
      no-CTE fallback (§7.2 v0.4 heuristic / §7.3 acceptance)
      elides the recursive CTE for a `+`/`*` over a single
      well-known transitive predicate (`rdfs:subClassOf` /
      `rdfs:subPropertyOf` / `owl:sameAs`) once
      `pgrdf.materialize` has entailed the closure — no `CTE
      Scan` in the executed plan, result byte-identical. The
      §7.1-permitted gated remainder (an alternation arm that is
      itself a sequence/recursive path; a recursive op whose
      inner box is a sequence) stays preview-panicking by spec
      allowance; negated property sets remain out of v0.4 scope.
      Phase E W3C-shape consolidation: 6 new fixtures
      `36-path-inverse` … `41-path-materialised` (35 → 41).

### Version-bearing files touched (mirrors the v0.4.4 cut)

- `Cargo.toml` `version` `0.4.4` → `0.4.5` (and the `Cargo.lock`
  `pgrdf` entry via `cargo update -p pgrdf`).
- `pgrdf.control` `default_version = '0.4.5'`.
- `compose/compose.yml` — adds the `pgrdf--0.4.5.sql` bind mount
  (mirrors how the v0.4.4 cut added `pgrdf--0.4.4.sql`).
- `tests/regression/expected/00-smoke.out` — version literals
  `0.4.4` → `0.4.5`.
- `CHANGELOG.md` — the accreted `[Unreleased]` Phase E bullets
  move into `## [0.4.5] — 2026-05-16` with a marquee; the empty
  `[Unreleased]` header stays (mirrors the v0.4.4 cut shape).
- `RELEASE_NOTES.md` — full rewrite for v0.4.5.

### Test bar

```
pgrx integration  230  (was 222 at v0.4.4 / Phase E3)
pg_regress         73  (property-path coverage 108–111)
w3c-sparql         41  (was 35 — +6 property-path fixtures)
LUBM-shape          3  (unchanged)
Total: 347 green, plus the pg_dump round-trip gate.
```

### Ritual deviations vs v0.4.4 cut

- README.md NOT touched (a parallel docs session owns an
  uncommitted logo-header edit; out of release-cut scope —
  mirrors the v0.4.4 cut's same deviation).
- No `src/` fmt sweep needed (`cargo fmt --all -- --check` clean
  this cut).
- The roadmap / spec / guide / docs §7 coherence edits land in
  the **feature** commit (Phase E close-out), not the release
  commit — the v0.4.4 cut did not touch those files, so neither
  does this release commit (the `git show df1e2f6 --stat` file
  set is authoritative).

### E-011 carried

`publish-crate.yml` stays disabled until upstream
`gtfierro/reasonable#50` merges. The v0.4.5 tag fires
`release.yml` only (8 platform tarballs PG14-17 × amd64/arm64 +
SHA256SUMS); no crates.io publish this cut.

## v0.4.4 — 2026-05-15

Phase D closes with nine countdown slices (59 → 51) shipping LLD
v0.4 §6 (SPARQL 1.1 CONSTRUCT) end-to-end, plus the release cut
(slice 50). The marquee surface lands the full CONSTRUCT query
form on the SQL engine via the sibling UDF `pgrdf.construct(q
TEXT) → SETOF JSONB`: constant / variable / blank-node /
multi-triple templates, the `CONSTRUCT WHERE { pattern }`
shorthand, GRAPH-scoped WHERE (`GRAPH <iri>` literal + `GRAPH ?g`
variable), round-trip ingest (`pgrdf.put_construct_row` /
`put_construct_rows`), and `pgrdf.sparql_parse` CONSTRUCT
classification.

### Engine surface delta vs v0.4.3

- **Storage / OWL 2 RL inference / SHACL / SPARQL UPDATE** —
  incrementally extended; no breaking changes to existing
  surfaces.
- **SPARQL CONSTRUCT track (LLD v0.4 §6)** — **shipped end-to-end
  via nine countdown slices (59 → 51)**. `pgrdf.construct(q)` is
  a sibling UDF to `pgrdf.sparql` (intent signalled at the SQL
  boundary). It evaluates the WHERE pattern through the existing
  SELECT-side translator, instantiates the template once per
  solution, and emits one structured-term JSONB row per template
  triple. Per-slice:
    - slice 59 — foundation, constant-only templates (W3C §16.2;
      DISTINCT / ORDER BY / GROUP BY / aggregate rejected at
      execute time per LLD §6.2).
    - slice 58 — template variable substitution (subject /
      predicate / object; full structured-term shape for typed +
      language-tagged literals; unbound vars panic).
    - slice 57 — blank-node templates (fresh-per-solution labels;
      within-solution label sameness; predicate-position bnodes
      reject at parse).
    - slice 56 — multi-triple templates (N triples → N rows per
      solution; blank-node labels shared across the N triples
      within one solution; empty template rejects).
    - slice 55 — GRAPH-scoped WHERE (`GRAPH <iri>` + `GRAPH ?g`;
      default-graph quads excluded per §13.3 — also corrected a
      latent slice-79 / slice-87 SELECT-side bleed).
    - slice 54 — `CONSTRUCT WHERE { pattern }` shorthand (§16.2.4;
      pure-BGP, blank-node-free).
    - slice 53 — round-trip ingest (`pgrdf.put_construct_row` /
      `put_construct_rows`), closing §6.3 (typed literals, lang
      tags, within-batch bnode joining preserved; idempotent;
      NULL-array no-op).
    - slice 52 — `pgrdf.sparql_parse` CONSTRUCT enrichment
      (`form: "CONSTRUCT"`, `template` + `where_shape` blocks,
      `shorthand` flag, `unsupported_algebra`).
    - slice 51 — six W3C-shape CONSTRUCT conformance fixtures
      (`tests/w3c-sparql/30-35`) + per-fixture `kind: construct`
      harness selector + docs / spec / guide coherence sweep.
- **CI-perf hardening** — the partition-DDL window in the SPARQL
  UPDATE / lifecycle paths takes a statement-outermost
  transaction advisory lock, so the default parallel pgrx-test
  scheduler no longer flakes on concurrent partition DDL. Parallel
  test threads restored (test bar verified without
  `--test-threads=1`).

### crates.io — not published

v0.4.4 is **not** published to crates.io. The `[patch.crates-io]`
block for `reasonable` (E-011) continues to block `cargo publish`.
The `publish-crate.yml` workflow remains disabled per the v0.4.1
post-release ops note; tag push fires `release.yml` only (8
prebuilt tarballs + GH Release). Re-enables once upstream
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
merges and the patch retires.

### Test bar

- 194 pgrx integration tests (`cargo pgrx test`, +28 vs v0.4.3)
- 69 pg_regress golden tests (+8 vs v0.4.3 — CONSTRUCT per-form
  regressions plus the round-trip / sparql_parse files)
- 35 W3C-shape SPARQL conformance tests (+6 vs v0.4.3 — fixtures
  30-35)
- 3 LUBM-shape correctness tests (unchanged from v0.4.3)
- Plus `tests/regression/scripts/pg-dump-roundtrip.sh` driving
  `_pgrdf_graphs` pg_dump round-trip (binary mode, unchanged from
  v0.4.3)

Total: 301 automated tests + 1 round-trip gate.

### Supported Postgres

PG 14, 15, 16, 17 across {amd64, arm64} = 8 prebuilt tarballs.
PG 18 deferred per [ERRATA E-006](../specs/ERRATA.v0.2.md).

### Tarball layout

Same as v0.4.3 — `lib/pgrdf.so`, `share/extension/{pgrdf.control,
pgrdf--0.4.4.sql, pgrdf--0.4.3.sql, pgrdf--0.4.2.sql,
pgrdf--0.4.1.sql, pgrdf--0.4.0.sql}`, `LICENSE`, `NOTICE`. The
`pgrdf--N.M.P.sql` files accumulate so a `CREATE EXTENSION pgrdf
VERSION '0.4.3'` against a v0.4.4 install still resolves; only the
version literal changes.

### Known issues — carried from v0.4.3

- **E-011** — `[patch.crates-io]` fork-dep for `reasonable` still
  in place (carried). Drops once upstream PR
  [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
  merges.
- **E-006** — pgrx 0.18 / Postgres 18 deferred (carried).
- **E-007** — `extension_control_path` GUC blocked by E-006
  (carried).
- **E-009** — original SHACL upstream-block resolved at the
  validation-engine half (carried).
- **E-010** — cargo audit informational advisories (carried).

### v0.4.2-introduced — resolved in v0.4.4

- **pgrx-tests parallelism flake on partition DDL.** The two
  Phase A tests (`pg_add_graph_iri_idempotent`,
  `pg_add_graph_id_iri_synthetic_upgrade`) that occasionally raced
  under pgrx-tests 0.16's parallel scheduler are now stable — the
  partition-DDL window takes a statement-outermost transaction
  advisory lock (CI-perf hardening, this release); parallel test
  threads restored.

### What's deferred from the v0.4 LLD

Still 🚧 in [`SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md):

- Property paths (§7) — v0.4.5
- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11) — v0.4.6
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

## v0.4.3 — 2026-05-15

Phase C closes with seven countdown slices (84 → 78) shipping LLD
v0.4 §4 (SPARQL UPDATE surface) end-to-end, plus a release
preflight countdown (77 → 60). The marquee surface lands every
documented UPDATE form on the SQL engine: `INSERT DATA`,
`DELETE DATA`, `INSERT WHERE`, `DELETE WHERE` (and its shorthand),
`DELETE+INSERT WHERE`, the `WITH <iri>` / `GRAPH <iri>` graph-
scoped variants, and the `DROP / CLEAR / CREATE GRAPH` lifecycle
algebra (with `DEFAULT / NAMED / ALL` targets and `SILENT`).

### Engine surface delta vs v0.4.2

- **Storage / OWL 2 RL inference / SHACL** — incrementally
  extended; no breaking changes to existing surfaces.
- **SPARQL UPDATE track (LLD v0.4 §4)** — **shipped end-to-end via
  seven countdown slices (84 → 78)**. `pgrdf.sparql(q)` now
  detects UPDATE queries via a try-parse-then-fallback strategy
  (`parse_query` first, `parse_update` if that fails). UPDATE
  forms return a single summary row of shape `{"_update": …}`
  carrying `form`, `triples_inserted`, `triples_deleted`,
  `graphs_touched`, and `elapsed_ms`. Per-form slices:
    - slice 84 — `INSERT DATA` (default + named graph; auto-
      allocates unknown IRIs via `pgrdf.add_graph`; idempotent
      via `ON CONFLICT DO NOTHING`).
    - slice 83 — `DELETE DATA` (ground quads only; lookup-only
      dict path; spec-correct no-op on absent terms).
    - slice 82 — `INSERT { template } WHERE { pattern }`.
    - slice 81 — `DELETE { template } WHERE { pattern }` +
      shorthand `DELETE WHERE`.
    - slice 80 — `DELETE { … } INSERT { … } WHERE { … }` (atomic
      modify; one WHERE-pattern evaluation feeds both halves).
    - slice 79 — graph-scoped variants (`WITH <iri>`,
      `GRAPH <iri>` in template / WHERE; cross-graph copy).
    - slice 78 — lifecycle algebra (`DROP / CLEAR / CREATE GRAPH`
      + `DEFAULT / NAMED / ALL` + `SILENT`); routes through the
      §5 lifecycle UDFs via SPI, not direct Rust calls.
- **`pgrdf.sparql_parse` UPDATE detail** (slice 74) — per-op
  enrichment surfaces `kind` (mirrors executor `_update.form`),
  `template_graphs`, `with_graph`, and lifecycle `target`
  labels.
- **W3C-shape conformance harness** — three new UPDATE-form
  fixtures (`tests/w3c-sparql/27-29`) plus `elapsed_ms`
  normalisation in `run.sh`.

### crates.io — not published

v0.4.3 is **not** published to crates.io. The `[patch.crates-io]`
block for `reasonable` (E-011) continues to block `cargo publish`.
The `publish-crate.yml` workflow remains disabled per the v0.4.1
post-release ops note; tag push fires `release.yml` only (8
prebuilt tarballs + GH Release). Re-enables once upstream
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
merges and the patch retires.

### Test bar

- 166 pgrx integration tests (`cargo pgrx test`, +33 vs v0.4.2)
- 61 pg_regress golden tests (+7 vs v0.4.2 — files `93-99` per
  UPDATE form plus the lifecycle-algebra regression)
- 29 W3C-shape SPARQL conformance tests (+3 vs v0.4.2)
- 3 LUBM-shape correctness tests (unchanged from v0.4.2)
- Plus `tests/regression/scripts/pg-dump-roundtrip.sh` driving
  `_pgrdf_graphs` pg_dump round-trip (binary mode, unchanged from
  v0.4.2)

Total: 259 automated tests + 1 round-trip gate.

### Supported Postgres

PG 14, 15, 16, 17 across {amd64, arm64} = 8 prebuilt tarballs.
PG 18 deferred per [ERRATA E-006](../specs/ERRATA.v0.2.md).

### Tarball layout

Same as v0.4.2 — `lib/pgrdf.so`, `share/extension/{pgrdf.control,
pgrdf--0.4.3.sql, pgrdf--0.4.2.sql, pgrdf--0.4.1.sql,
pgrdf--0.4.0.sql}`, `LICENSE`, `NOTICE`. The `pgrdf--N.M.P.sql`
files accumulate so a `CREATE EXTENSION pgrdf VERSION '0.4.2'`
against a v0.4.3 install still resolves; only the version literal
changes.

### Known issues — carried from v0.4.2

- **E-011** — `[patch.crates-io]` fork-dep for `reasonable` still
  in place (carried). Drops once upstream PR
  [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
  merges.
- **E-006** — pgrx 0.18 / Postgres 18 deferred (carried).
- **E-007** — `extension_control_path` GUC blocked by E-006
  (carried).
- **E-009** — original SHACL upstream-block resolved at the
  validation-engine half (carried).
- **E-010** — cargo audit informational advisories (carried).

### v0.4.2-introduced — carried

- **pgrx-tests parallelism flake on partition DDL.** Two Phase A
  tests (`pg_add_graph_iri_idempotent`,
  `pg_add_graph_id_iri_synthetic_upgrade`) occasionally race under
  pgrx-tests 0.16's parallel scheduler. Pre-existing on v0.4.1
  (verified empirically). CI re-runs absorb the noise.

### What's deferred from the v0.4 LLD

Still 🚧 in [`SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md):

- CONSTRUCT (§6) — v0.4.4
- Property paths (§7) — v0.4.5
- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11) — v0.4.6
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

## v0.4.2 — 2026-05-15

Phase B closes with five countdown slices (99 → 95) shipping LLD v0.4
§5 (graph-level lifecycle UDFs) end-to-end, plus a release preflight
countdown (95 → 85). The marquee surface lands four partition-level
primitives: `pgrdf.drop_graph` (slice 99), `pgrdf.clear_graph` (slice
98), `pgrdf.copy_graph` (slice 97), `pgrdf.move_graph` (slice 96), and
an end-to-end integration regression (slice 95) wiring the four UDFs
together against a load → mutate → verify flow.

### Engine surface delta vs v0.4.1

- **Storage / SPARQL / OWL 2 RL inference / SHACL** — incrementally
  extended; no breaking changes to existing surfaces.
- **Lifecycle UDF track (LLD v0.4 §5)** — **shipped end-to-end via
  five countdown slices (99 → 95)**. Four new partition-level UDFs
  on `_pgrdf_quads`:
    - `pgrdf.drop_graph(id, cascade => TRUE) → BIGINT` — DETACH +
      DROP partition; deletes `_pgrdf_graphs` row; returns the
      pre-drop row count. `cascade => FALSE` errors with the stable
      `drop_graph: inferred rows present` prefix if any `is_inferred
      = TRUE` row is present. Idempotent on absent graphs.
    - `pgrdf.clear_graph(id) → BIGINT` — TRUNCATE ONLY the per-graph
      partition. Partition shell + IRI binding survive. `clear_graph(0)`
      permitted (operates on explicit `_pgrdf_quads_g0`); negative ids
      rejected with the stable prefix.
    - `pgrdf.copy_graph(src, dst) → BIGINT` — INSERT INTO … SELECT
      between per-graph partitions; carries forward both base and
      `is_inferred = TRUE` rows. Auto-creates dst partition + IRI.
      The only lifecycle UDF that touches every row.
    - `pgrdf.move_graph(src, dst) → BIGINT` — `copy + drop` compose.
      The LLD §5.2 metadata-only partition rebind is aspirational
      for v0.4.2; flagged as a v0.5 perf optimisation.

### crates.io — not published

v0.4.2 is **not** published to crates.io. The `[patch.crates-io]`
block for `reasonable` (E-011) continues to block `cargo publish`.
The `publish-crate.yml` workflow remains disabled per the v0.4.1
post-release ops note; tag push fires `release.yml` only (8
prebuilt tarballs + GH Release). Re-enables once upstream
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
merges and the patch retires.

### Test bar

- 133 pgrx integration tests (`cargo pgrx test`, +15 vs v0.4.1 —
  runtime count; static `#[pg_test]` attribute count is 127)
- 54 pg_regress golden tests (+5 vs v0.4.1 — files `88-91` per UDF
  plus `92` end-to-end integration)
- 26 W3C-shape SPARQL conformance tests (unchanged from v0.4.1)
- 3 LUBM-shape correctness tests (unchanged from v0.4.1)
- Plus `tests/regression/scripts/pg-dump-roundtrip.sh` driving
  `_pgrdf_graphs` pg_dump round-trip (binary mode, unchanged from
  v0.4.1)

Total: 216 automated tests + 1 round-trip gate.

### Supported Postgres

PG 14, 15, 16, 17 across {amd64, arm64} = 8 prebuilt tarballs.
PG 18 deferred per [ERRATA E-006](../specs/ERRATA.v0.2.md).

### Tarball layout

Same as v0.4.1 — `lib/pgrdf.so`, `share/extension/{pgrdf.control,
pgrdf--0.4.2.sql, pgrdf--0.4.1.sql, pgrdf--0.4.0.sql}`, `LICENSE`,
`NOTICE`. The `pgrdf--N.M.P.sql` files accumulate so a
`CREATE EXTENSION pgrdf VERSION '0.4.1'` against a v0.4.2 install
still resolves; only the version literal changes.

### Known issues — carried from v0.4.1

- **E-011** — `[patch.crates-io]` fork-dep for `reasonable` still
  in place (carried). Drops once upstream PR
  [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
  merges.
- **E-006** — pgrx 0.18 / Postgres 18 deferred (carried).
- **E-007** — `extension_control_path` GUC blocked by E-006
  (carried).
- **E-009** — original SHACL upstream-block resolved at the
  validation-engine half (carried).
- **E-010** — cargo audit informational advisories (carried).

### v0.4.2-introduced

- **pgrx-tests parallelism flake on partition DDL.** Two Phase A
  tests (`pg_add_graph_iri_idempotent`,
  `pg_add_graph_id_iri_synthetic_upgrade`) occasionally race under
  pgrx-tests 0.16's parallel scheduler because both exercise
  partition DDL inside `add_graph(iri)` / `add_graph(id BIGINT)`
  through SPI. Pre-existing on v0.4.1 (verified empirically); the
  v0.4.2 Phase B test annotations were tightened to exact-match the
  panic strings so the four lifecycle-UDF rejection-path tests are
  deterministic.

### What's deferred from the v0.4 LLD

Still 🚧 in [`SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md):

- SPARQL UPDATE (§4) — Phase C opens at v0.4.3
- CONSTRUCT (§6) — v0.4.4
- Property paths (§7) — v0.4.5
- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11) — v0.4.6
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

## v0.4.1 — 2026-05-15

Phase A closes with thirteen countdown slices (120 → 108) shipping
LLD v0.4 §3 (named-graph SPARQL scoping) end-to-end, plus a release
preflight countdown (107 → 100). The marquee surface lands the
`_pgrdf_graphs(graph_id, iri)` mapping table, three `add_graph`
overloads, two symmetric lookup UDFs (`graph_id` / `graph_iri`),
SPARQL `GRAPH <iri> { … }` literal-form and `GRAPH ?g { … }`
variable-form translation, GRAPH composition with OPTIONAL / UNION /
MINUS, and `pg_dump` round-trip discipline (LLD v0.4 §3.1 acceptance
criterion).

### Engine surface delta vs v0.4.0

- **Storage / SPARQL / OWL 2 RL inference / SHACL** — incrementally
  extended; no breaking changes to existing surfaces.
- **Named-graph track (LLD v0.4 §3)** — **shipped end-to-end via
  thirteen countdown slices (120 → 108)**. New table
  `pgrdf._pgrdf_graphs(graph_id BIGINT PRIMARY KEY, iri TEXT NOT
  NULL UNIQUE)` with seed row `(0, 'urn:pgrdf:graph:0')`. Three
  `pgrdf.add_graph` overloads (integer-keyed, IRI-keyed,
  explicit-pair-binding) plus `pgrdf.graph_id(iri)` and
  `pgrdf.graph_iri(id)` symmetric lookups. SPARQL executor learns
  per-triple `GraphScope` planning with `Literal(i64)` and
  `Variable { name, scope_id }` arms; SQL builder grows a
  `ScopePlan` driving INNER vs LEFT JOIN to `_pgrdf_graphs`.
  GRAPH composes into OPTIONAL (LEFT JOIN, W3C §13.3 semantics),
  UNION (per-branch scope), and MINUS (scope local to the
  `NOT EXISTS` subquery).
- **pg_dump round-trip** — LLD v0.4 §3.1 acceptance criterion
  locked via `tests/regression/scripts/pg-dump-roundtrip.sh` and
  `pg_extension_config_dump('_pgrdf_graphs', '')` registration on
  the schema migration.

### crates.io — first publish

v0.4.1 is the first pgRDF release on crates.io. The
`.github/workflows/publish-crate.yml` workflow fires on
`release: published` and runs `cargo publish` against
`CARGO_REGISTRY_TOKEN`. From-source consumers can now
`cargo add pgrdf` or write `pgrdf = "0.4.1"` in their Cargo.toml.

Cargo.toml metadata polished at slice 107 prep
(`documentation`, `keywords`, `categories`, `readme`,
`description`, `license`, `repository`, `homepage`, `authors`).

### The fork-patch caveat — carried

`Cargo.toml`'s `[patch.crates-io]` block overriding `reasonable` to
[`styk-tv/reasonable@rdf12-passthrough`](https://github.com/styk-tv/reasonable/tree/rdf12-passthrough)
stays in place through v0.4.x while
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
is in review. v0.4.2 (or whichever release lands after the upstream
merge) drops the patch and pins the released `reasonable` version.
Tracked at [`specs/ERRATA.v0.4.md`](../specs/ERRATA.v0.4.md) E-011.

### Test bar

- 117 pgrx integration tests (`cargo pgrx test`, +23 vs v0.4.0)
- 49 pg_regress golden tests (+9 vs v0.4.0 — files `72-79` for the
  named-graph surface + `87-sparql-graph-composition.sql`)
- 26 W3C-shape SPARQL conformance tests (+3 vs v0.4.0 — fixtures
  `24-graph-named-iri`, `25-graph-var-projection`,
  `26-graph-var-groupby`)
- 3 LUBM-shape correctness gates (unchanged)
- Plus `tests/regression/scripts/pg-dump-roundtrip.sh` end-to-end
  round-trip gate on `_pgrdf_graphs`
- Plus manual smoke: 24 ontologies, 17,134 triples (totals locked
  in `tests/perf/smoke-ontologies.expected.tsv` with `--check`
  mode, unchanged from v0.4.0)

**Total: 195 automated + pg_dump round-trip gate + 24-ontology
manual smoke.** All green at cut time.

### Supported Postgres

PG 14, 15, 16, 17 × {amd64, arm64} = 8 prebuilt tarballs.
PG 18 still deferred per ERRATA E-006 (carried).

### Tarball layout

`pgrdf-0.4.1-pg<N>-glibc-<arch>.tar.gz`:

```
pgrdf-0.4.1-pg<N>-glibc-<arch>/
├── lib/pgrdf.so
├── share/extension/pgrdf.control
├── share/extension/pgrdf--0.4.1.sql
├── LICENSE
├── NOTICE
└── SHA256SUMS   (per-tarball, covers every file above)
```

Plus an aggregate `SHA256SUMS` attached to the GitHub Release
covering every `pgrdf-*.tar.gz` asset. Same INSTALL §3 layout as
v0.4.0; only the version literal changes.

### Known issues — carried from v0.4.0

- **E-011** — `[patch.crates-io]` to the `styk-tv/reasonable`
  fork still in place. Drops once
  [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
  merges.
- **E-006** / **E-007** / **E-009** / **E-010** — carried, see
  v0.4.0 entry below.

### What's deferred from the v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md): SPARQL
UPDATE (§4), graph-level lifecycle UDFs (§5 — Phase B opens at
slice 99), CONSTRUCT (§6), property paths (§7), the SPARQL backlog
from v0.3 (§11), heap_multi_insert phase B (§12), and the W3C
SPARQL 1.1 manifest runner (§13). These land in subsequent v0.4.x
point releases or in a refreshed v0.5.0 cut.

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

MIT (re-licensed from Apache 2.0 in v0.5.1). Copyright 2026
Peter Styk. The `LICENSE` file is the canonical attribution
source — see it for the maintainer email. v0.5.1+ ships only
`LICENSE` (no `NOTICE`); the file is distributed inside every
per-arch tarball. `Cargo.toml` declares `authors = ["Peter
Styk"]` (no email — per the
[email-in-license-and-commits-only](../PROVENANCE.md) discipline,
the address is canonical in LICENSE and is not duplicated across
release artifacts).

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
