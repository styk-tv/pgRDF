# Changelog

All notable changes to pgRDF are tracked here. Format follows
[Keep a Changelog](https://keepachangelog.com/). Versioning is SemVer
once we cut v1.0; pre-1.0 minor bumps may include breaking changes.

## [Unreleased]

### Phase A slice 120 — `_pgrdf_graphs` schema lands (LLD v0.4 §3.1)

New `pgrdf._pgrdf_graphs(graph_id BIGINT PRIMARY KEY, iri TEXT NOT
NULL UNIQUE)` table establishes the IRI ↔ graph_id mapping that
SPARQL `GRAPH { … }` (slices 111-110), the IRI-keyed UDF overloads
(slices 118-115), and §4/§5/§6/§7 graph-scoped surfaces all depend
on. The seed row `(0, 'urn:pgrdf:graph:0')` covers the existing
default-partition catch-all bucket.

Schema-only this slice — no UDF surface change, no behaviour change
to existing `pgrdf.add_graph(id BIGINT)`. Regression coverage:
`tests/regression/sql/72-graphs-table-shape.sql` + one `#[pg_test]`
in `src/storage/graphs.rs`. Test bar: 95 pgrx + 41 pg_regress + 23
W3C + 3 LUBM = 162 green.

### Release — v0.4.0 shipped

v0.4.0 tagged and released 2026-05-15
([release page](https://github.com/styk-tv/pgRDF/releases/tag/v0.4.0)).
8 prebuilt tarballs (PG 14-17 × {amd64, arm64}) + aggregate
`SHA256SUMS` attached. release.yml run
[25902864745](https://github.com/styk-tv/pgRDF/actions/runs/25902864745)
green end-to-end in ~8 min. Tarball smoke verified: aggregate
checksum OK, internal `SHA256SUMS` OK, layout includes
`lib/pgrdf.so` + `share/extension/{pgrdf.control, pgrdf--0.4.0.sql}`
+ LICENSE + NOTICE.

The `[patch.crates-io]` block in `Cargo.toml` stays in place
through v0.4.x — see ERRATA.v0.4 E-011. v0.4.1 drops the patch
once gtfierro/reasonable#50 merges.

## [0.4.0] — 2026-05-15

The first pgRDF release with the full four-engine mission shipping
in earnest. **SHACL Core validation is real** —
`pgrdf.validate(data, shapes)` returns a W3C
`sh:ValidationReport`-shape JSONB via `shacl 0.3.1`, replacing the
v0.3.0 stub. Unblocked via a `[patch.crates-io]` override pointing
at the `styk-tv/reasonable@rdf12-passthrough` fork (upstream
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
awaiting maintainer review/merge; v0.4.1 will drop the patch once
the upstream merges). All four engines (storage, SPARQL, OWL 2 RL
inference, SHACL Core validation) are now real implementations.
Test bar: 94 pgrx + 40 pg_regress + 23 W3C-shape + 3 LUBM-shape =
160 automated tests green, plus the 24-ontology / 17,134-triple
manual smoke. PG 14-17 × {amd64, arm64} = 8 prebuilt tarballs.
Apache 2.0.

The v0.4 LLD's named-graph + SPARQL UPDATE + lifecycle UDFs +
CONSTRUCT + property paths + heap_multi_insert phase B + W3C
manifest runner tracks stay 🚧 — they land in subsequent v0.4.x
points or in a refreshed v0.5.0 cut.

### Upstream — `reasonable` PR filed (E-011 step 4)

Filed the upstream patch as
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
on 2026-05-15. The PR description carries the downstream verification
data (pgRDF 160 tests green, real SHACL `pgrdf.validate` running
against `shacl 0.3.1` + the patched `reasonable` via
`[patch.crates-io]`). `specs/ERRATA.v0.4.md` E-011 status flipped
from "verified locally" to "verified locally + upstream PR open
(awaiting maintainer review/merge)". Once the upstream merges, the
`[patch.crates-io]` block in `Cargo.toml` drops and the dep pins
to whatever release ships the patch.

### Spec — v0.4 LLD promoted from FUTURE, v0.5-FUTURE opened

SHACL real impl shipped on `main` in commit `ac40bc2`; v0.4 LLD is
no longer forward-looking. `specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`
renamed to `specs/SPEC.pgRDF.LLD.v0.4.md` via `git mv` (history
preserved). The renamed file's §0 status flips from "draft /
forward-looking / target: pgRDF v0.4 cut" to
"in-progress authoritative contract for the v0.4 cycle". §9 SHACL
restructured from "v0.5 — gated on E-009" to "✅ shipped in v0.4
cycle" — cites commit `ac40bc2`, ERRATA.v0.4 E-011, and regression
`71-shacl-real.sql`. Capability matrix (§2) marks SHACL ✅; all
other v0.4 tracks (named-graph, UPDATE, lifecycle, CONSTRUCT,
paths, SPARQL backlog) stay 🚧.

New `specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md` opened as the next
forward-look sibling. Carries the v0.5-targeted content split out
of the prior v0.4-FUTURE: §3 reasoning profile selector (was v0.4
§8), §4 TriG/N-Quads ingest (was v0.4 §10), §5 SHACL-SPARQL
constraint mode + materialised-graph coverage (was v0.4 §9.5), §6
W3C SHACL manifest runner (was v0.4 §9.5 / §13), §7 IRI overloads
for lifecycle UDFs (was v0.4 §5.1 forward note), §8
aggregates-over-UNION refinements (was v0.4 §11 forward note), §9
v1.0 forward look (was v0.4 §15).

Cross-link updates: v0.3 LLD §0 supersession block now points at
`v0.4.md` (not `-FUTURE`); ERRATA.v0.4 E-011 next-steps row repoints
to LLD.v0.4 §9; `docs/04-inference.md` reasoning-selector pointer
moves to v0.5-FUTURE §3; `docs/05-validation.md` SHACL spec pointer
moves to v0.4 §9; `docs/06-installation.md` two pointers move to
v0.4; `docs/09-release.md` "Deferred to v0.4" pointer moves to v0.4
§2 with promotion note; `docs/10-roadmap.md` ~25 pointers rewritten
(most to v0.4, the v0.5-targeted ones to v0.5-FUTURE);
`RELEASE_NOTES.md` deferral pointer moves to v0.4 §2;
`src/query/{parser,executor}.rs` doc-comment pointers move to v0.4.

No code changes; no test count changes (still 160 = 94 pgrx + 40
pg_regress + 23 W3C + 3 LUBM).

### Phase 5 — Real SHACL validation lands (E-009 / E-011 resolved upstream-pending)

`pgrdf.validate(data_graph, shapes_graph) → JSONB` now executes
real SHACL Core validation via `shacl 0.3.x`. The stub body is
replaced with rehydrate-shapes / build-validator / run-validation
/ shape-W3C-report-as-JSONB. Coverage: new
`71-shacl-real.sql` exercising sh:NodeShape + sh:property +
sh:datatype violations, plus three `#[pg_test]` integration tests
(conforming, violations, unknown graphs). Existing
`70-validate-stub.sql` repurposed to lock the real-impl basic
shape (vacuously-conforming + unknown-graph degenerate cases);
filename retained for diff-friendly history.

Unblocked via `[patch.crates-io]` to the styk-tv/reasonable fork
branch `rdf12-passthrough`, which adds the `TermRef::Triple(_)`
arm needed for coexistence with shacl 0.3.x. Drop the patch once
gtfierro/reasonable merges the upstream PR (held in fork
PR-DRAFT.md).

Surface: `Cargo.toml` gains `shacl = "0.3"`, `rudof_rdf = "0.3"`,
flips `reasonable` to `{ version = "0.4", features = ["rdf-12"] }`,
and adds a `[patch.crates-io]` block pointing `reasonable` at the
fork. `src/validation/shacl.rs` rewritten from stub to real
impl. Specs: LLD v0.4-FUTURE §9 moves SHACL from v0.5 → v0.4 (real
impl section); §2 capability matrix flips Real SHACL output to ✅;
scope list expanded from five tracks to six; ERRATA.v0.4 E-011
gains a "Verified locally in pgRDF" section with the post-slice
test bar.

Test bar after: **94 pgrx + 40 pg_regress + 23 W3C + 3 LUBM = 160
tests** green (was 158).

### Spec — ERRATA.v0.4 file created (v0.4 cycle tracking)

New [`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) carries v0.4-era
spec deltas. E-011 first entry tracks the upstream `reasonable` patch
for RDF 1.2 coexistence (unblocks the remaining
`rdf-12 / TermRef::Triple` half of E-009). Branch
[`styk-tv/reasonable@rdf12-passthrough`](https://github.com/styk-tv/reasonable/tree/rdf12-passthrough)
is pushed; PR draft is held in the fork for review before filing
upstream. v0.2-era entries (E-006, E-007, E-008, E-009, E-010) are
carried forward by cross-link rather than duplicated.

## [0.3.0] — 2026-05-14

The first official pgRDF release. Ships the v0.3 engine surface
feature-complete state: storage (dictionary-encoded terms,
LIST-partitioned quads, hexastore indexes), SPARQL SELECT/ASK
surface, OWL 2 RL inference, SHACL validation stub, storage
performance (shmem dict cache + prepared-plan cache + prepared
bulk-insert), 158 automated tests + 24-ontology smoke, License
attribution + MSRV declared, and the full release pipeline
exercised end-to-end. PG 14-17 across {amd64, arm64} = 8
prebuilt tarballs.

### Release pre-flight — final CI sweep (slices #10-#5)

Last verification gate before tagging v0.3.0. Ran every test/lint
layer locally against the post-version-bump tree (HEAD was `ac514fe`
at start of sweep) to confirm the codebase is release-ready.

Per-layer results:

- **Slice #10 — `cargo fmt --check`**: drift in
  `src/inference/reasonable.rs` (40 lines reflowed; long `.expect()`
  chains broken across multiple lines by rustfmt). Applied
  `cargo fmt --all` and committed as a style-only commit.
- **Slice #9 — `cargo clippy -D warnings`** (in builder container,
  rustc 1.91): one error surfaced — unused import
  `use pgrx::prelude::*;` at `src/storage/shmem_cache.rs:349`. The
  `use super::*;` on the line above already brings `pgrx::prelude::*`
  into scope through the parent module, so the line was genuinely
  redundant. Removed it. Clippy now exits 0.
- **Slice #8 — `just test`** (pgrx integration in Linux builder):
  `test result: ok. 93 passed; 0 failed; 0 ignored`. Matches the
  93-count in README and `docs/08-testing.md`; no doc drift.
- **Slice #7 — `just test-regression`** (pg_regress harness against
  compose Postgres): `39 pass, 0 fail, 0 new baselines`.
- **Slice #6 — `just test-w3c`** (W3C-shape SPARQL harness):
  `23 pass, 0 fail, 0 new baselines`.
- **Slice #5 — `just test-lubm`** (LUBM-shape harness):
  `3 pass, 0 fail, 0 new baselines`.

Final aggregate: **93 (pgrx) + 39 (pg_regress) + 23 (W3C) + 3 (LUBM)
= 158 tests, all green**. Matches the 158 figure cited across
README.md, `docs/08-testing.md`, `docs/10-roadmap.md`,
`docs/09-release.md`, and `RELEASE_NOTES.md` — no test-count doc
updates needed.

Code changes from this sweep:

- `src/inference/reasonable.rs` — rustfmt-only reflow (no semantic
  change), landed in its own commit.
- `src/storage/shmem_cache.rs` — removed redundant
  `use pgrx::prelude::*;` from `mod tests`.

### Release pre-flight — version bump to 0.3.0 (slices #18-#11)

Mechanical version bump landing slices #18 through #11 of the 66→1
release countdown. Touches every surface that pins the package
version. The `pgrdf.version()` UDF (which returns
`env!("CARGO_PKG_VERSION")`) and the extension's `extversion` now
both report `0.3.0`.

Files touched:

- `Cargo.toml` — `version = "0.2.0"` → `version = "0.3.0"` (slice #18).
- `pgrdf.control` — `default_version = '0.2.0'` → `'0.3.0'` (slice #17).
- `compose/compose.yml` — bind-mount path
  `pgrdf--0.2.0.sql` → `pgrdf--0.3.0.sql` (slice #16).
- `compose/README.md` — bind-mount table + `pgrdf.version()` worked
  example output → `"0.3.0"` (slice #16).
- `README.md` — `pgrdf.version()` example output → `0.3.0`
  (slices #15/#14; status pill at `v0.3` was already correct).
- `guide/01-install.md` — `pgrdf.version()` example outputs (Path A
  compose flow, Verify section) → `0.3.0`; Path C manual-install
  worked-example tarball URL → `v0.3.0/pgrdf-0.3.0-...tar.gz`
  (slice #13).
- `docs/02-storage.md` — "No PostgreSQL custom scan hooks at v0.3.0"
  current-version reference (slice #13).
- `docs/06-installation.md` — `pgrdf.version()` example output →
  `'0.3.0'` (slice #13).
- `docs/09-release.md` — preamble reframed to reflect that Cargo.toml
  now reads `version = "0.3.0"` (bump landed), instead of "still
  reads `version = "0.2.0"`; bump-to-0.3.0 happens as part of the
  cut" (slice #13).
- `tests/regression/expected/00-smoke.out` — pgrdf.version() and
  extversion lines `0.2.0` → `0.3.0` (slice #13).
- `Cargo.lock` — `pgrdf 0.2.0 → 0.3.0` via `cargo update -p pgrdf`
  (slice #11).

Build artifact verification (slice #12):

- `just build-ext` produces
  `compose/extensions/share/extension/pgrdf--0.3.0.sql` and
  `pgrdf.control` reads `default_version = '0.3.0'`. The cached
  `pgrdf--0.2.0.sql` left over in the build output was removed (v0.x
  doesn't support `ALTER EXTENSION pgrdf UPDATE` per the slice #21
  upgrade policy, so the legacy migration file isn't needed).
- `just test-regression` reports `39 pass, 0 fail, 0 new baselines`.

Historical references to `0.2.0` are intentionally preserved:

- All `CHANGELOG.md` `[Unreleased]` entries from slices #24-#27 that
  document past pre-flight verifications (manual repack test,
  cargo pgrx package dry-run, etc.) — they accurately describe the
  state at the time those slices ran.
- `sql/schema_v0_2_0.sql` — historical bootstrap-schema filename
  (still referenced by `extension_sql_file!` in `src/lib.rs`); v0.3
  doesn't change the schema layout.
- `specs/SPEC.pgRDF.LLD.v0.2.md` — the v0.2 LLD contract.

CHANGELOG `[Unreleased]` → `[0.3.0]` block conversion stays deferred
to slices #4-#1 (the cut itself).

### Release notes — v0.4 deferral list audit (slice #19)

Bi-directional audit of the "Deferred to v0.4" lists in
`RELEASE_NOTES.md` and `docs/09-release.md` against
`specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md` §2 (canonical v0.4 scope). The
spec lists five major tracks (§3 named-graph + IRI mapping; §4 SPARQL
UPDATE; §5 graph-level lifecycle UDFs; §6 CONSTRUCT; §7 property
paths) plus the carried SPARQL backlog (§11: multi-triple OPTIONAL,
VALUES, BIND-downstream, aggregates over UNION, DESCRIBE) and the
ingest-performance carry (§12: `heap_multi_insert` 2× target).

Drift findings, both directions:

- **`docs/09-release.md` was missing the entire §11 SPARQL backlog** —
  multi-triple OPTIONAL, VALUES, BIND-downstream, aggregates over
  UNION, and DESCRIBE were absent from its "Deferred to v0.4" list
  even though they are listed in `RELEASE_NOTES.md` and named
  explicitly in the spec as in-scope for v0.4. Fixed by adding a
  dedicated bullet covering all five, with the LLD §11 cross-link.
- **`RELEASE_NOTES.md` mentioned `GRAPH { … }` without the IRI
  mapping** — the IRI ↔ `graph_id` mapping table is the hard
  prerequisite for the SPARQL surface (LLD §3.1), and `docs/09-release.md`
  already names it. Fixed by adding "with IRI ↔ `graph_id` mapping"
  to the named-graph entry plus the property-path operator set, and a
  `§2` anchor on the LLD cross-link for parity with `docs/09-release.md`.
- **No v0.5/v1.0 items mislabeled as v0.4** — the LLD's §8
  (reasoning profile selector), §9 (real SHACL output, gated on
  E-009), §10 (TriG / N-Quads ingest), and §15 (incremental
  materialisation, RDF 1.2 triple terms) are correctly absent from
  both consumer-facing files. A short pointer paragraph added to
  `docs/09-release.md` so readers know where the v0.5/v1.0 forward
  look lives (LLD §8-§10 and §15) rather than guessing those items
  were forgotten.
- **Items in consumer-facing lists not in LLD §2**: `SHA256SUMS.asc`
  GPG signature and pgrx 0.18 / PG 18 migration are both legitimate
  v0.4 work items but are release-engineering and toolchain concerns
  outside the LLD scope (tracked under INSTALL OQ4 / roadmap Phase 6
  step 3, and ERRATA E-006 respectively). Annotated as such in
  `docs/09-release.md`; left in place in both files because they're
  user-visible v0.4 deliverables consumers should see in release
  notes.
- **Cross-link anchors**: both files link to the LLD doc top (no
  in-document anchor); the file resolves on disk. Added an explicit
  `§2` reference in both prose pointers so a future reader can find
  the canonical scope section without scrolling.

Outcome: drift closed in both directions. `RELEASE_NOTES.md`'s
"Deferred to v0.4" line now reads parallel to the LLD §2 capability
matrix; `docs/09-release.md` no longer drops the SPARQL backlog. No
spec edits; the LLD remains source of truth. This audit slice is
documentation-only — no code, no tests, no other docs touched.

### Release notes — known issues block consolidated (slice #20)

Cross-file audit of the Known Issues surface across `RELEASE_NOTES.md`,
`docs/09-release.md`, and `specs/ERRATA.v0.2.md` to make sure
consumer-facing release docs cite the v0.3.0-era errata consistently
with the authoritative ERRATA table. Triggered by the E-007
"workflow.ttl" mis-cite caught in slice #22 (corrected in `52c13bf`);
this pass verifies nothing else drifted across the remaining v0.3.0
errata entries (E-006, E-007, E-008, E-009, E-010) and that the
pre-v0.3 entries (E-001, E-002, E-003, E-004, E-005) are correctly
omitted from the v0.3.0 release surface as resolved-by-design or
out-of-scope-for-consumers.

Findings, per E-NNN:

- **E-001** (`shacl-rust` → `shacl_validation` supersession), **E-002**
  (OWL 2 RL only, EL/QL out-of-scope), **E-003** (PG 18 GUC path,
  effectively rolled into E-006), **E-004** (init-script-on-PG18+ no
  longer needed; compose has no init script), **E-005** (repo URL
  `styk-tv/pgRDF` placeholder fix) — **all pre-v0.3 spec corrections
  folded into the v0.3 LLD body or otherwise resolved-by-design.**
  Correctly omitted from v0.3.0 release notes in both files; no
  consumer-facing impact on the v0.3.0 tarball.
- **E-006** (pgrx 0.18 / PG 18 deferred) — cited in both files,
  consistent: `RELEASE_NOTES.md` "pgrx 0.18 / Postgres 18 deferred to
  v0.4."; `docs/09-release.md` "pgrx held at 0.16.1; PG 18 deferred
  to v0.4." Both match ERRATA's "Hold pgrx 0.16.1 for v0.3. Support
  matrix: PG 14–17."
- **E-007** (`extension_control_path` GUC forward path blocked by
  E-006) — cited in both files, consistent: both call out INSTALL §7,
  the E-006 blocker, and the per-file bind-mount workaround. The
  earlier "workflow.ttl" mis-cite from slice #22 is gone (fixed in
  `52c13bf`).
- **E-008** (Linux builder container instead of native macOS) —
  **correctly omitted from both consumer-facing files.** This is a
  contributor / build-environment fact, not a tarball-consumer fact;
  end-users of the release artifacts never encounter the dev-only
  macOS → Linux builder routing. Listed in ERRATA for the dev path.
- **E-009** (SHACL real integration blocked upstream) — cited in
  both files, consistent: `RELEASE_NOTES.md` "SHACL real integration
  blocked by upstream dep conflict."; `docs/09-release.md`
  "`pgrdf.validate` ships as a stub; real SHACL execution blocked by
  upstream `shacl_validation` / `reasonable` feature unification."
  Both match ERRATA's `iri_s` migration + `rdf-12` feature-unification
  story.
- **E-010** (cargo audit informational advisories) — cited in both
  files, consistent: `RELEASE_NOTES.md` "cargo audit advisories — all
  informational, no security impact."; `docs/09-release.md` "4
  informational `cargo audit` advisories accepted for v0.3 (all in
  subtrees of pgrx 0.16.1 / `reasonable 0.4.1` and clear automatically
  when E-006 / E-009 resolve)." Both match ERRATA's "Accept the 4
  informational warnings for v0.3."

Cross-link verification: `specs/ERRATA.v0.2.md` resolves on disk from
both consumer-facing files (`RELEASE_NOTES.md` root-relative;
`docs/09-release.md` via `../specs/ERRATA.v0.2.md`).

**Outcome: zero drift. No edits to `RELEASE_NOTES.md` or
`docs/09-release.md` required.** The two files cite exactly the same
set (E-006, E-007, E-009, E-010), describe each at appropriate
granularity for their audience (marketing-style summary vs engineering
release note), and classify all four as known-issues with no
security-blocking impact. ERRATA remains the source of truth; both
consumer-facing docs are aligned to it. This audit slice is
documentation-only — no code, no spec, no other doc touched.

### Release notes — upgrade policy documented (slice #21)

The v0.x upgrade discipline written down as consumer-facing contract.
Headline: pgRDF v0.x reserves the right to break schema and UDF
signatures between minor releases, `ALTER EXTENSION pgrdf UPDATE` is
not supported until v1.0, and the supported upgrade path is dump-via-SQL
(decode `pgrdf._pgrdf_quads` against `pgrdf._pgrdf_dictionary` per
graph, serialise to Turtle externally), `DROP EXTENSION pgrdf CASCADE`,
install the new version, then `CREATE EXTENSION` + re-load. v1.0 is
flagged as the boundary where proper `ALTER EXTENSION pgrdf UPDATE`
migrations land alongside a frozen on-disk schema; no date committed.

The detailed policy lives in `docs/06-installation.md` as a new
`## Upgrade between v0.x versions` section (procedure with SQL dump
template, "why no in-place upgrade?" rationale calling out fluid
pre-1.0 schema / non-stable dict id space / `is_inferred` flux, and
cluster-managed-installation guidance pointing CloudNativePG /
StackGres / Apache AGE operators at planned maintenance windows +
volume snapshots + staging verification). Two short cross-link
summaries land alongside: `docs/09-release.md` v0.3.0 section gets a
new `### Upgrade policy` subsection above `### Known issues`, and
`RELEASE_NOTES.md` gets a new `## Upgrading` section above
`## License`. Both summaries point back at the canonical
`docs/06-installation.md` anchor. SQL examples schema-qualify the
internal tables (`pgrdf._pgrdf_quads` / `pgrdf._pgrdf_dictionary`)
matching the actual extension schema. No fabricated dates for v0.4 /
v1.0.

### Release notes — RELEASE_NOTES.md drafted + release.yml body_path wired (slice #22)

The GitHub Release body for v0.3.0 — the marketing-style summary that
consumers see in the GH UI when they land on the release page. Lives at
the repo root as `RELEASE_NOTES.md` (Option A per the slice brief: simple,
conventional, rewritten each release). Wired into the workflow via
`body_path: RELEASE_NOTES.md` on the `softprops/action-gh-release@v2`
step alongside the existing `generate_release_notes: true` (GH appends
the auto-generated PR-title commit list under the curated body).

Content is consumer-facing, ~370 words: a one-line elevator pitch,
the feature surface (storage / Turtle / SPARQL SELECT-ASK / OWL 2 RL /
SHACL stub / performance), the consolidated test bar (158 automated +
24 manual smoke matching `docs/09-release.md`), a drop-in install
recipe with the exact `curl` / `sha256sum -c` / `cp` flow, the docker
compose pointer, the {pg14..pg17}×{amd64, arm64} support matrix,
known issues (E-006 / E-007 / E-009 / E-010), the v0.4 deferral list,
and Apache 2.0 attribution. Every relative path (`specs/SPEC.pgRDF.INSTALL.v0.2.md`,
`guide/01-install.md`, `specs/ERRATA.v0.2.md`,
`specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`, `CHANGELOG.md`) resolves on disk.
Numbers cross-checked against the engineering release note in
`docs/09-release.md` (slice #23); no new claims.

### Release notes — docs/09-release v0.3.0 section drafted (slice #23)

Replaces the "No release cut yet" preamble in `docs/09-release.md`
with a `## v0.3.0 — 2026-05-14 (planned)` section: engine surface
recap (storage / SPARQL / Phase 3 perf / Phase 4 inference / Phase 5
stub / Phase 6 CI), the consolidated test bar (93 pgrx + 39 pg_regress
+ 23 W3C-shape + 3 LUBM-shape + 24 ontology smoke / 17 134 triples),
performance characteristics (sub-µs dict cache hit, prepared-plan
reuse, phase A bulk ingest with the 2× wall-clock target carried to
v0.4 phase B), the {pg14..pg17}×{amd64, arm64} matrix with PG 18
deferred per E-006, license + attribution (Apache 2.0, Copyright 2026
Peter Styk, LICENSE + NOTICE inside every tarball per §4(d)), MSRV
1.91, tarball layout per INSTALL §3, known issues (E-006 / E-007 /
E-009 / E-010), and the v0.4 deferral list. Content sourced from
CHANGELOG `[Unreleased]` and LLD v0.3 §2–§7; no new claims, no
fabricated numbers.

The CHANGELOG `[Unreleased]` block is **not yet cut** to a
`[0.3.0] — YYYY-MM-DD` block — that move lands in a later slice
(group 4-1, the actual tag commit). This slice only drafts the
engineering-side narrative inside `docs/09-release.md`; the
GitHub Release body itself is a separate slice (#22).

### Release pre-flight — smoke-install verification (slice #24)

End-to-end install rehearsal of the slice #25 tarball
(`pgrdf-0.2.0-pg17-glibc-arm64.tar.gz`, 872,067 B) against a clean
`postgres:17.4-bookworm` container. This exercises the real consumer
install path the GH Release will ask users to perform: extract tarball,
drop artifacts into PG library paths, `CREATE EXTENSION pgrdf`, parse
Turtle, run SPARQL. Goal: confirm the v0.3.0 release artifact is
ship-ready.

**Procedure (Option A from the slice brief — fresh container, no
compose, bind-mount FROM staged tarball contents):**

```bash
STAGING=$PWD/.smoke-install-test                    # podman-visible
rm -rf "${STAGING}" && mkdir -p "${STAGING}"
tar -xzf /tmp/pgrdf-repack-test/pgrdf-0.2.0-pg17-glibc-arm64.tar.gz \
        -C "${STAGING}"
podman run -d --name pgrdf-smoke \
  -e POSTGRES_USER=pgrdf -e POSTGRES_PASSWORD=pgrdf -e POSTGRES_DB=pgrdf \
  -v "${STAGING}/pgrdf-0.2.0-pg17-glibc-arm64/lib/pgrdf.so:/usr/lib/postgresql/17/lib/pgrdf.so:ro" \
  -v "${STAGING}/pgrdf-0.2.0-pg17-glibc-arm64/share/extension/pgrdf.control:/usr/share/postgresql/17/extension/pgrdf.control:ro" \
  -v "${STAGING}/pgrdf-0.2.0-pg17-glibc-arm64/share/extension/pgrdf--0.2.0.sql:/usr/share/postgresql/17/extension/pgrdf--0.2.0.sql:ro" \
  -p 5433:5432 docker.io/library/postgres:17.4-bookworm \
  -c shared_preload_libraries=pgrdf
```

Container `pgrdf-smoke` runs on port 5433 to avoid colliding with the
regular compose container on 5432. Postgres `pg_isready` returned ok
after 2s. Bind-mounts are read-only — exercises the real distribution
shape (immutable artifact, mutated only at boot via Postgres config
args).

**Initial environment note (caught by the smoke):** the slice brief's
`podman run` snippet omitted `-c shared_preload_libraries=pgrdf`. On
first run, `CREATE EXTENSION` + `pgrdf.version()` succeeded but the
first stateful call (`parse_turtle`) returned `ERROR: PgAtomic was
not initialized` — the canonical signature of pgRDF not being loaded
via `shared_preload_libraries`. This is documented in
[`SPEC.pgRDF.INSTALL.v0.2 §6 + §7`](specs/SPEC.pgRDF.INSTALL.v0.2.md),
[`guide/01-install.md §3`](guide/01-install.md), and
[`docs/06-installation.md §1.2`](docs/06-installation.md), so the
diagnostic chain held: error → check `SHOW shared_preload_libraries`
(empty) → re-launch with `-c shared_preload_libraries=pgrdf` → SHOW
returns `pgrdf` → everything works. **The tarball + install docs are
both correct; only the smoke brief's `podman run` command was
incomplete.**

**Smoke test results (after relaunching with the preload arg):**

| Step | Command | Output | Verdict |
|---|---|---|---|
| 1 | `CREATE EXTENSION pgrdf;` | `CREATE EXTENSION` | OK |
| 2 | `SELECT pgrdf.version();` | `0.2.0` (1 row) | OK |
| 3 | `SELECT pgrdf.add_graph(1);` | `t` (1 row) | OK |
| 4 | `SELECT pgrdf.parse_turtle('@prefix ex: <http://example.org/> . ex:a ex:b ex:c .', 1);` | `1` (1 triple inserted) | OK |
| 5 | `SELECT * FROM pgrdf.sparql('SELECT ?o WHERE { ?s ?p ?o }');` | `{"o": "http://example.org/c"}` (1 row) | OK |

End-to-end round-trip: tarball → bind-mount → `CREATE EXTENSION` →
parse Turtle → SPARQL SELECT, all on a stock upstream Postgres image
with zero source build. The 872 KiB artifact is sufficient.

**Total elapsed (stage → CREATE EXTENSION → final SPARQL → teardown):
~50s** on M-series darwin, podman 4-ARM hypervisor.

**Bind-mount caveats discovered:**

- Podman on darwin runs in a VM (applehv) that does NOT auto-mount
  `/tmp`. First attempt staged the tarball under `/tmp/pgrdf-install-test`
  per the slice brief and `podman run` returned
  `statfs /tmp/.../pgrdf.so: no such file or directory`. Restaging
  under `$PWD/.smoke-install-test` (under `/Users`, auto-mounted by
  podman's machine config) resolved it. **Implication for the public
  install guide:** the existing guide already tells users to write
  artifacts to PG's actual `pkglibdir` + `sharedir` (via
  `pg_config --pkglibdir`), not to bind-mount from `/tmp`, so this
  is a smoke-test infrastructure quirk only — not a documentation gap.

- glibc version mismatch was a worry going in (tarball built on glibc
  2.36 from the slice #26 manylinux container, smoke runs against
  bookworm's glibc 2.36) — and it was a non-event, since the build +
  smoke happen to use the same glibc minor. Cross-glibc verification
  is still owed once aarch64 + amd64 release builds land on the real
  GH runner.

**Teardown:**

```bash
podman rm -f pgrdf-smoke
rm -rf "${STAGING}"           # $PWD/.smoke-install-test
```

`pgrdf-smoke` is a one-shot container, never persisted; the regular
`pgrdf-postgres` (from `just compose-up`) is unaffected — it was
stopped at start of the smoke and can be restarted by the user with
`just compose-up` at any time.

**Slice outcome: PASS.** The v0.3.0 tarball-shaped release artifact
installs cleanly into a stock Postgres image. The pre-flight group
(slices #66 → #24, 43 entries) closes here. Remaining slices #23 → #1
shift focus to feature work in the v0.3.0 / v0.4.0 scope (per the
roadmap in slice #29).

### Release pre-flight — manual tarball repack verification (slice #25)

Manually executed the `release.yml` repack step (lines 38-53) on the
slice #26 build artifacts to confirm the staging → tarball pipeline
produces the exact INSTALL §3 layout the GH Release would publish.
Goal: catch any tar/find/sha256sum corner case *before* the tagged
release runs across 4 PG majors x 2 arches.

**Procedure (aarch64 darwin, slice #26 artifacts re-used — no rebuild):**

```bash
VER=0.2.0; PG=17; ARCH=arm64
STAGING=/tmp/pgrdf-repack-test
OUT="${STAGING}/pgrdf-${VER}-pg${PG}-glibc-${ARCH}"
rm -rf "${STAGING}"
mkdir -p "${OUT}/lib" "${OUT}/share/extension"
cp compose/extensions/lib/pgrdf.so                       "${OUT}/lib/"
cp compose/extensions/share/extension/pgrdf.control       "${OUT}/share/extension/"
cp compose/extensions/share/extension/pgrdf--${VER}.sql   "${OUT}/share/extension/"
cp LICENSE NOTICE                                          "${OUT}/"
( cd "${OUT}" && find . -type f ! -name SHA256SUMS -print0 \
    | xargs -0 sha256sum > SHA256SUMS )
tar -czf "${STAGING}/$(basename ${OUT}).tar.gz" -C "${STAGING}" "$(basename ${OUT})"
```

Mirrors `release.yml` L41-52 byte-for-byte (substituting
`compose/extensions/{lib,share}` for `target/release/pgrdf-pg17/usr/{lib,share}/postgresql/17/`,
since slice #26 already copied those out — same files, different path
prefix). GNU `sha256sum` from coreutils 9.7 is installed on darwin via
brew so the `sha256sum > SHA256SUMS && sha256sum -c SHA256SUMS` flow
matches Ubuntu runner behaviour exactly.

**Tarball produced:**

| Field | Value |
|---|---|
| Name | `pgrdf-0.2.0-pg17-glibc-arm64.tar.gz` |
| Size | 872,067 B (852 KiB) |
| Contents | 6 files + 4 dirs |
| Compression ratio | ~2.6:1 vs `pgrdf.so` (2.2 MB → 870 KB) |

**`tar -tzf | sort` (every entry, in lexicographic order):**

```
pgrdf-0.2.0-pg17-glibc-arm64/
pgrdf-0.2.0-pg17-glibc-arm64/LICENSE
pgrdf-0.2.0-pg17-glibc-arm64/NOTICE
pgrdf-0.2.0-pg17-glibc-arm64/SHA256SUMS
pgrdf-0.2.0-pg17-glibc-arm64/lib/
pgrdf-0.2.0-pg17-glibc-arm64/lib/pgrdf.so
pgrdf-0.2.0-pg17-glibc-arm64/share/
pgrdf-0.2.0-pg17-glibc-arm64/share/extension/
pgrdf-0.2.0-pg17-glibc-arm64/share/extension/pgrdf--0.2.0.sql
pgrdf-0.2.0-pg17-glibc-arm64/share/extension/pgrdf.control
```

**SHA256SUMS contents (5 lines, SHA256SUMS itself absent — self-exclude OK):**

```
a6dc47dea368e1cb479f456538144939060fa72bb2a96c4eabf23477d1a5ece8  ./LICENSE
7ee0daa51a51f29729f80e96192b6df4874b02a39f131c34f486c5365b3726c8  ./NOTICE
c8c661eada2255fa85e441a50240c3eaad4e1c12197a2102a1554bf5574ab90c  ./lib/pgrdf.so
7584c499464333b53dc7bd106aafd37ffa5071cb33980bd5214e6da8c72284b4  ./share/extension/pgrdf.control
3a785b2b483bd510ecf810af029bb47cd8dab032071c548f3735e759564e7f69  ./share/extension/pgrdf--0.2.0.sql
```

`pgrdf.so` SHA matches slice #26's recorded `c8c661ea…ab90c` — same
binary, no rebuild, by construction.

**Round-trip verify (extract fresh, `sha256sum -c`):**

```
./LICENSE: OK
./NOTICE: OK
./lib/pgrdf.so: OK
./share/extension/pgrdf.control: OK
./share/extension/pgrdf--0.2.0.sql: OK
```

5 of 5 OK. SHA256SUMS does not appear in its own manifest — the
`find -type f ! -name SHA256SUMS` predicate works as intended.

**INSTALL §3 layout conformance:**

| INSTALL §3 entry | tarball entry | match |
|---|---|---|
| `lib/pgrdf.so` | `lib/pgrdf.so` | exact |
| `share/extension/pgrdf.control` | `share/extension/pgrdf.control` | exact |
| `share/extension/pgrdf--<version>.sql` | `share/extension/pgrdf--0.2.0.sql` | exact |
| `share/extension/pgrdf--<prev>--<version>.sql` (zero or more) | (none — 0.2.0 is the first cut) | n/a |
| `LICENSE` | `LICENSE` | exact |
| `SHA256SUMS` | `SHA256SUMS` | exact |
| (none) | `NOTICE` | **spec gap — see below** |

Byte-for-byte conformant with INSTALL §3 except for `NOTICE`, which
landed in the tarball via slice #28 (Apache 2.0 §4(d) compliance) but
the corresponding INSTALL §3 file list was not updated then. Adding
`NOTICE` to the spec's enumerated layout is a one-line surface edit
deferred to a separate spec-grooming slice — the tarball mechanics
themselves are correct.

**Aggregate SHA256SUMS (release.yml L67-72) — not verified this slice:**

The aggregate step runs over `pgrdf-*.tar.gz` produced by all
`(pg, arch)` matrix legs and lives in the `release` job. With only
one local tarball it'd be a single-line file; the multi-leg
aggregation is fundamentally a GH Actions concern (artifact upload +
`download-artifact merge-multiple: true`). Single-tarball spot check
mirrors the same `sha256sum pgrdf-*.tar.gz > SHA256SUMS` invocation —
no behavioural surprise expected.

**Why this matters:**

`release.yml` is a single-shot, tag-triggered workflow. A failure
inside the `Repack to INSTALL-spec layout` step would leave a
half-published release on GitHub with no artifacts attached. Dry-running
the repack locally on the same artifact tree the workflow consumes
catches `cp` glob mismatches, `find -print0` portability surprises,
and tar layout regressions before they cost a re-tag + force-push.
Combined with slice #26 (path mapping verified) and slice #27 (verify
docs + GPG defer): the v0.2.0 cut is end-to-end traced.

Status: repack mechanics verified end-to-end on aarch64 darwin. The
GH Actions runner uses identical GNU coreutils + GNU tar, so the
behaviour transfers. The lone surface gap is `NOTICE` missing from
INSTALL §3's file list — flagged for a follow-up spec edit, no
runtime impact.

### Release pre-flight — cargo pgrx package dry-run (slice #26)

Verified `cargo pgrx package` produces the artifact tree the
`release.yml` repack step (lines 38-53) consumes. Goal: catch any
path mismatch *before* `git push --tags` triggers the real workflow
across 4 PG majors x 2 arches.

**Procedure (Colima docker builder, aarch64):**

1. `rm -rf compose/extensions/{lib,share}` to clean state.
2. `DOCKER_BUILDKIT=1 docker build --target builder --no-cache
   -t pgrdf-builder-rust:pg17 -f compose/builder.Containerfile .`
   — forces a fresh `cargo pgrx package` run (busts cargo cache).
3. `just build-ext` — completes export-stage and copies artifacts
   to `compose/extensions/`.

**Live `cargo pgrx package` output (from step 2, builder stage 7/7):**

```
Installing extension
Copying control file to target/release/pgrdf-pg17/usr/share/postgresql/17/extension/pgrdf.control
Copying shared library to target/release/pgrdf-pg17/usr/lib/postgresql/17/lib/pgrdf.so
Writing SQL entities to /work/target/release/pgrdf-pg17/usr/share/postgresql/17/extension/pgrdf--0.2.0.sql
Finished installing pgrdf
```

**Path mapping (cargo pgrx package output → release.yml expectation):**

| cargo pgrx package emits | release.yml repack reads | match |
|---|---|---|
| `target/release/pgrdf-pg17/usr/lib/postgresql/17/lib/pgrdf.so` | `${PKG}/usr/lib/postgresql/${pg}/lib/*.so` (L46) | exact |
| `target/release/pgrdf-pg17/usr/share/postgresql/17/extension/pgrdf.control` | `${PKG}/usr/share/postgresql/${pg}/extension/*.control` (L47) | exact |
| `target/release/pgrdf-pg17/usr/share/postgresql/17/extension/pgrdf--0.2.0.sql` | `${PKG}/usr/share/postgresql/${pg}/extension/*.sql` (L48) | exact |

`PKG=target/release/pgrdf-pg${pg}` at release.yml L43; the inner
`usr/{lib,share}` prefix is what cargo pgrx package writes by
default and matches what release.yml then `cp`s out of.

**Artifact set produced (aarch64, glibc-bookworm, pg17):**

| File | Size | Notes |
|---|---|---|
| `lib/pgrdf.so` | 2,214,040 B | ELF 64 LSB aarch64, BuildID `80021f2c…`, not stripped |
| `share/extension/pgrdf.control` | 216 B | `default_version = '0.2.0'`, `module_pathname = '$libdir/pgrdf'` |
| `share/extension/pgrdf--0.2.0.sql` | 7,220 B | 234 lines, auto-generated by pgrx |

`pgrdf.so` SHA-256 `c8c661ea…ab90c`. Sizes will differ slightly on
amd64 vs aarch64 and across pg14/15/16/17 (different pgrx-generated
FFI shim sets), but the **path layout is invariant** — that's all
the release workflow's repack step needs.

**Build time (fresh `cargo pgrx package`, single PG major, single
arch, all cargo deps cached in BuildKit mount):**

- `--no-cache` builder rebuild: 175.30s real (~3 min)
- of which `cargo pgrx package` proper: ~80s (deps mostly
  warm-cached in `/usr/local/cargo/registry` mount, fresh
  `pgrdf` compile + SQL extraction)
- subsequent `just build-ext` (all cached, just re-runs export
  container): 1.46s

The release workflow runs each `(pg, arch)` cell on a clean GitHub
runner with no cargo cache, so 8 cells x ~10-15min cold-cache build
= ~80-120 min total wall time. Within tolerance.

**Warnings worth flagging:** none. `cargo pgrx package` exits 0,
both `pgrdf-builder-rust:pg17` (~3.35 GB) and `pgrdf-builder:pg17`
(~99 MB) images build clean, export container copies the three
artifacts into `compose/extensions/` with no errors.

**Verdict:** layout matches. Release workflow's repack step
(release.yml lines 46-48) will find every path it expects. No
follow-up needed for slice #26; downstream slice #25 (manual
tarball repack dry-run) will exercise the LICENSE + NOTICE +
SHA256SUMS aggregation end-to-end.

### Release pre-flight — SHA256SUMS verify + GPG signing defer (slice #27)

Follow-up to slice #28's `release.yml` audit. Slice #28 confirmed
SHA256SUMS coverage is already wired at **both** levels (per-tarball
internal manifest + aggregate top-level over all 8 tarballs). This
slice surfaces the orthogonal piece — the detached GPG signature
`SHA256SUMS.asc` mentioned in INSTALL §3 / LLD §5.4 step 3 — and
decides scope for it.

**SHA256SUMS state (confirmed):** the `Repack to INSTALL-spec layout`
step in the `build` job emits per-tarball internal `SHA256SUMS`
(line 51 of `release.yml`) covering `lib/pgrdf.so`,
`share/extension/*`, `LICENSE`, `NOTICE`. The downstream `release`
job's `Generate aggregate SHA256SUMS` step emits a top-level
`SHA256SUMS` covering every `pgrdf-*.tar.gz` and attaches it as a
release asset (lines 67-77). No release.yml change needed.

**GPG signing decision: defer to v0.4.** Rationale:

- No `GPG_PRIVATE_KEY` secret or release-signing key is provisioned
  for the workflow today — `grep -rn "GPG_PRIVATE_KEY\|secrets\."
  .github/` returns zero matches.
- No public-half signing key is published anywhere visible
  (keyserver, release page, repo).
- SHA256SUMS itself is the primary integrity check most extension
  consumers verify (`sha256sum -c SHA256SUMS`); the `.asc` signature
  layer is a downstream supply-chain hardening, not a v0.3-cut
  blocker.
- Wiring `.asc` properly requires (a) sourcing a real signing key
  (Peter Styk maintainer key — not in repo), (b) publishing the
  public half on a keyserver or release page, (c) adding the GitHub
  secret + a `gpg --detach-sign` step to the workflow's `release`
  job. All out of scope for a verification-and-defer slice.

**Docs edits applied:**

- `docs/09-release.md` "Aggregate checksums" section: rewrote to
  confirm SHA256SUMS is wired at both levels (per-tarball +
  aggregate) and to flag `.asc` GPG signing as v0.4 follow-up
  (previously said "not yet wired in `release.yml`" which conflated
  SHA256SUMS itself with the `.asc` signing). Added a new
  "Verification (consumer side)" subsection showing the `curl` →
  `sha256sum -c SHA256SUMS --ignore-missing` recipe plus the
  in-tarball verification path; closes with a one-liner pointing
  at what changes when `.asc` lands in v0.4
  (`gpg --verify SHA256SUMS.asc SHA256SUMS`).
- `docs/10-roadmap.md` Phase 6 step 3 bullet: split the single
  conflated bullet into a positive confirmation (SHA256SUMS wired
  per slice #28) plus an explicit v0.4 defer for `.asc` listing
  the three prerequisites (signing key, public-half publication,
  secret wiring).

**No `.github/workflows/release.yml` change.** This is a
verify-and-document slice; the workflow already does what slice
#27's original plan would have done. The actual `.asc` wiring lands
in a v0.4 slice once a signing key is sourced.

Test bar unchanged: still 93 pgrx + 39 pg_regress + 23 W3C-shape +
3 LUBM-shape = 158 across all five layers.

### Release pre-flight — release.yml audit + NOTICE inclusion fix (slice #28)

End-to-end audit of `.github/workflows/release.yml` ahead of v0.3 cut.
Verified workflow shape:

- Trigger `on: push: tags: ["v*"]`, matrix `pg14/15/16/17 × {amd64, arm64}`
  (8 tarballs), GH Release job gated on `needs: build`.
- Action pins: `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`,
  `actions/upload-artifact@v4`, `actions/download-artifact@v4`,
  `softprops/action-gh-release@v2` (major-pin policy preserved).
- Auth: relies on default `GITHUB_TOKEN` with top-level
  `permissions: contents: write`. No third-party secrets referenced.
- Pre-release detection: implicit via `softprops/action-gh-release@v2`'s
  SemVer pre-release tag heuristic (e.g. `v1.0.0-rc1`); no explicit
  `prerelease:` flag — relying on action default.
- SHA256SUMS: already wired in **both** per-tarball form (inside each
  `pgrdf-<ver>-pg<PG>-glibc-<arch>.tar.gz`) and aggregate form (top-level
  `SHA256SUMS` over all 8 tarballs, attached to the GH Release).
  Supersedes the slice #36 audit note that flagged this as "not yet
  wired"; no TODO needed.

**Bug fixed (Apache 2.0 §4(d) compliance):** the `Repack to INSTALL-spec
layout` step previously copied only `LICENSE` into the staging directory.
Apache 2.0 §4(d) requires that where a `NOTICE` file exists, its
attribution notices MUST be included in distributed derivative works.
Added `cp NOTICE "${OUT}/"` directly after the existing `cp LICENSE`
line, mirroring the LICENSE pattern exactly. Also updated the layout
comment block to list `NOTICE` between `LICENSE` and `SHA256SUMS`.

Net effect: each of the 8 published tarballs now ships `LICENSE` +
`NOTICE` + `SHA256SUMS` alongside the extension binaries, satisfying
the upstream license terms inherited from `oxigraph`, `spargebra`,
`sophia`, and other Apache-2.0 dependencies whose attribution flows
through pgRDF's own `NOTICE`.

### Roadmap — v0.4 scope cohesion check (slice #29)

Bi-directional cohesion audit between `specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`
(the source of truth for v0.4 scope) and the
`## v0.4 — next milestone (forward-looking)` section in
`docs/10-roadmap.md` (added by slice #31). The LLD wins on disagreement;
this slice fixes drift in the roadmap.

Coverage table (LLD §2 + ancillary v0.4 items → roadmap section):

| LLD item                                    | Roadmap before                | Roadmap after                              | Match? |
|---|---|---|---|
| §3 named-graph + IRI mapping                | Track 1                       | Track 1                                    | ✅ already |
| §4 SPARQL UPDATE                            | Track 2                       | Track 2                                    | ✅ already |
| §5 graph-level lifecycle UDFs               | Track 3                       | Track 3                                    | ✅ already |
| §6 CONSTRUCT                                | Track 4                       | Track 4                                    | ✅ already |
| §7 property paths                           | Track 5                       | Track 5                                    | ✅ already |
| §11 SPARQL surface backlog                  | "Carried backlog"             | "Carried backlog"                          | ✅ already |
| §12 perf work (heap_multi_insert + scans)   | absent                        | NEW: "Performance work carried forward"    | ✅ added |
| §13 W3C SPARQL 1.1 manifest runner wired v0.4 | absent                      | NEW: "Conformance runner wiring (v0.4)"    | ✅ added |
| §8 reasoning profile selector → v0.5        | "Excluded from v0.4"          | "Excluded from v0.4"                       | ✅ already |
| §9 real SHACL → v0.5 (E-009)                | "Excluded from v0.4"          | "Excluded from v0.4"                       | ✅ already |
| §10 TriG / N-Quads → v0.5                   | "Excluded from v0.4"          | "Excluded from v0.4"                       | ✅ already |

Reverse direction (every roadmap v0.4 subsection → LLD anchor) — every
existing subsection (Track 1-5, Carried backlog, Excluded) maps cleanly
to a numbered LLD section. No orphans in the roadmap.

Drift entries fixed:

- **Missing-in-roadmap: LLD §12 (Performance work carried forward)** —
  the LLD explicitly says "v0.4 targets shipping this" for Phase 3 step
  3 phase B (`heap_multi_insert` / `COPY BINARY`) and "v0.4 is the
  earliest target" for Postgres custom-scan hooks. Both were absent from
  the roadmap's v0.4 milestone section even though the roadmap's
  pre-existing Phase 3 narrative already refers to phase B as v0.4 work.
  Added a "Performance work carried forward from v0.3" subsection
  pointing at v0.4-FUTURE §12.
- **Missing-in-roadmap: LLD §13 (W3C SPARQL 1.1 manifest runner wired
  in v0.4)** — the LLD §13 test-policy paragraph says the manifest
  runner "is wired in v0.4 — it gates §11's SPARQL backlog automatically
  as the deferred forms come online". The roadmap's v0.4 milestone
  section had no entry for this; the Phase 6 narrative covers the v0.3
  state but the v0.4 wiring was unsurfaced in the forward-look. Added
  a "Conformance runner wiring (v0.4)" subsection.

Framing checks (LLD wording → roadmap wording):

- LLD §2: "v0.4 ships five major tracks" → roadmap: "five major tracks
  — the full contract lives in the spec". ✅ consistent.
- LLD §11: "ship together for economy" (translator machinery shared
  with §4 + §6) → roadmap: "Shipped in the same cut because they share
  the translator machinery §4 + §6 already require". ✅ consistent.
- LLD §8/§9/§10: framed as v0.5 work, "v0.4 keeps the v0.3 surface
  unchanged" / "v0.4 does not attempt" / "v0.4 does not ship this; v0.5
  does" → roadmap: "Excluded from v0.4 (planned v0.5)". ✅ consistent.

Total fixes applied: 2 new subsections added to
`docs/10-roadmap.md` v0.4 milestone section.

Test bar unchanged: docs-only slice. The LLD was not edited — only the
navigation aid.

### Roadmap — coverage ratchet table (slice #30)

Added a `## Coverage ratchet — release-by-release targets` section to
`docs/10-roadmap.md`, placed between the new
`## v0.4 — next milestone (forward-looking)` H2 (slice #31) and the
pre-existing `## Out of scope (v0.x)` H2 so the reader's eye flows
shipped-phases → next-milestone → ratchet-trajectory →
out-of-scope.

The new section consolidates targets already declared in scattered
prose across `specs/SPEC.pgRDF.LLD.v0.3.md` §5.4 + §6.1,
`specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md` §13, and `docs/08-testing.md`
("What we don't test (yet)") into a single 7-row × 5-column table:

- Rows: pgrx integration, pg_regress golden, W3C-shape SPARQL
  harness, LUBM-shape correctness harness, W3C SPARQL 1.1
  conformance manifest, W3C SHACL conformance manifest, LUBM
  cross-engine benchmark.
- Columns: v0.3 (current) shipped baselines, v0.4 target, v0.5
  target, v1.0 target.

Every cell anchors to a documented source — none are fabricated.
Cells without a published target carry `TBD` rather than a guess
(the pgrx and pg_regress columns for v0.5 / v1.0 are TBD because
the v0.5 / v1.0 LLDs aren't drafted yet; `v0.4-FUTURE` §13 only
gives counts for v0.4).

A one-paragraph explainer below the table pins the ratchet
enforcement rule: each release's CI must hit at least that
release's column, once a target is met it becomes a floor and can
never regress, citing `docs/08-testing.md`'s "Coverage gates
ratchet but never lower" line.

Sub-edits:

- `docs/10-roadmap.md` — new H2 + table + enforcement paragraph,
  inserted between `## v0.4 — next milestone` and
  `## Out of scope (v0.x)`. Caption cross-links
  `specs/SPEC.pgRDF.LLD.v0.3.md` §6.1, `v0.4-FUTURE` §13, and
  `docs/08-testing.md`.

Test bar unchanged: no new pg_regress or pgrx fixtures, this is a
docs-only slice.

Source-citation per cell (so future contributors can verify before
ratchet'ing a column):

- pgrx v0.3 = `93` from `docs/08-testing.md` line 25 +
  `docs/10-roadmap.md` v0.3 cut row.
- pgrx v0.4 = `+ heap_multi_insert tests` from
  `docs/08-testing.md` line 25.
- pg_regress v0.3 = `39` from `docs/08-testing.md` line 26 +
  v0.3 cut row.
- pg_regress v0.4 = `~60` from `v0.4-FUTURE` §13 breakdown
  (§3 6-8 + §4 8-10 + §5 4 + §6 3-4 + §7 5-6 + §11 5-6).
- W3C-shape harness v0.3 = `23` from `docs/08-testing.md` line 27.
- W3C-shape harness v0.4+ = "superseded by TTL-manifest runner"
  from `docs/08-testing.md` line 27.
- LUBM-shape v0.3 = `3` from `docs/08-testing.md` line 28.
- LUBM-shape v0.4+ = "superseded by LUBM-1/10/100 real benchmarks"
  from `docs/08-testing.md` line 28.
- SPARQL conformance v0.3 = `not wired` from
  `docs/08-testing.md` line 30 + `LLD v0.3` §5.4.
- SPARQL conformance v0.4 = `runner wired + ≥ 30 %` from
  `LLD v0.3` §5.4 line 389 + `docs/08-testing.md` line 30, 182 +
  `v0.4-FUTURE` §13 (runner wired in v0.4).
- SPARQL conformance v0.5 = `≥ 70 %` from `LLD v0.3` §6.1
  Phase 4 column line 415.
- SPARQL conformance v1.0 = `≥ 95 %` from `LLD v0.3` §5.4
  line 389 + §6.1 Phase 6 column line 415.
- SHACL conformance v0.3 = `not wired (E-009)` from
  `LLD v0.3` §5.4 line 392-394 + `docs/08-testing.md` line 31.
- SHACL conformance v0.4 = `not wired (still E-009)` from
  `v0.4-FUTURE` §9 (E-009 still gates SHACL real integration to
  v0.5).
- SHACL conformance v0.5 = `≥ 50 %` from `LLD v0.3` §6.1
  Phase 4 column line 415 + `docs/08-testing.md` line 184.
- SHACL conformance v1.0 = `≥ 90 %` from `LLD v0.3` §5.4
  line 392 + §6.1 Phase 6 column line 415.
- LUBM benchmark v0.3 = `scaffold only` from
  `docs/08-testing.md` line 32.
- LUBM benchmark v0.4 = `LUBM-1 smoke` from `LLD v0.3` §6.1
  Phase 3 column line 416.
- LUBM benchmark v0.5 = `LUBM-10 baseline vs Apache Jena TDB /
  Apache AGE` from `LLD v0.3` §6.1 Phase 4 column line 416 +
  §5.4 line 395 + `docs/08-testing.md` line 32.
- LUBM benchmark v1.0 = `LUBM-100 vs Apache Jena TDB /
  Apache AGE` from `LLD v0.3` §6.1 Phase 6 column line 416 +
  `docs/10-roadmap.md` Phase 6 step 3 line 301.

### Roadmap — v0.4 milestone section (slice #31)

Added an explicit `## v0.4 — next milestone (forward-looking)` section
to `docs/10-roadmap.md`, placed between Phase 6 and "Out of scope" so
the reader's eye flows shipped-phases → next-milestone →
out-of-scope. The new section surfaces the five v0.4 tracks at H3
heading granularity (named-graph + IRI mapping, SPARQL UPDATE,
graph-level lifecycle UDFs, CONSTRUCT, property paths) plus the
carried SPARQL-surface backlog from v0.3 and the explicit "excluded
from v0.4 (planned v0.5)" list, with each H3 cross-linking the
specific anchor in `specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`.

The intent is navigation, not new contract material: each H3 is 2–4
lines pointing at v0.4-FUTURE for detail — the v0.4-FUTURE spec
remains the single source of truth, this section is a section-TOC
entry so readers can land on "what comes next" without spelunking
the Phase 1–6 bullets.

Sub-edits:

- `docs/10-roadmap.md` — new H2 + 7 H3s (5 tracks + carried backlog +
  excluded-from-v0.4), inserted at line ~321 just above the existing
  `## Out of scope (v0.x)` H2.
- `docs/10-roadmap.md` — "Test bar over time" preamble gains a
  one-paragraph forward note that future v0.4 rows land under
  `v0.4 cut` labels per the new section's track grouping; existing
  v0.3 rows remain frozen as the shipped baseline.

Test bar unchanged: no new pg_regress or pgrx fixtures, this is a
docs-only slice.

Anchor verification: each cross-link uses the GitHub heading-slug
rules (lowercase, spaces → `-`, drop non-alphanumeric-or-hyphen,
em-dash → empty leaving a double-hyphen at its position). Targets
are §3 (`#3-named-graph-scoping-and-iri-mapping-new`), §4
(`#4-sparql-update-new`), §5 (`#5-graph-level-lifecycle-udfs-new`),
§6 (`#6-construct-deferred-from-v03-now-in-scope`), §7
(`#7-property-paths-deferred-from-v03-now-in-scope`), §11
(`#11-sparql-surface-backlog-deferred-from-v03-now-in-scope`), §8
(`#8-reasoning-profile-selector-v05--flagged-here-for-planning`),
§9 (`#9-shacl-real-integration-v05--gated-on-errata-e-009`), §10
(`#10-trig--n-quads-ingest-v05`).

### Docs — markdown link verification (slice #32)

Final docs-group lockdown pass before release-pre-flight: a full,
mechanical sweep of every internal markdown link across the repo to
confirm zero broken targets going into v0.3 release prep.

Scope and method:

| Surface | Count | Result |
| --- | --- | --- |
| Markdown files scanned (excl. `target/`, `node_modules/`, `fixtures/ontologies/`) | 61 | inventory complete |
| Total markdown links extracted (incl. external) | 153 | parsed via `\[…\]\(…\)` with code-fence stripping |
| External links (`http`/`https`/`mailto`/etc.) | 18 | not verified (out of scope) |
| Internal relative links (the audit surface) | 135 | every target resolved on disk |
| Same-file `#anchor` links | 1 | resolves to a real H2 in `docs/10-roadmap.md` |
| Cross-file `path.md#anchor` links | 0 | none in the repo |
| Directory-style links (e.g. `guide/`, `docs/`) | 4 | every target is a real directory |
| Non-markdown internal targets (`LICENSE`, `NOTICE`, `.sql`, `.rs`, `.tsv`) | 14 distinct | every file exists on disk |

Audit table — broken links found:

| File:line | Bad target | Type of break | Fix applied |
| --- | --- | --- | --- |
| _(none)_ | _(none)_ | _(none)_ | _(none)_ |

Counts: **broken 0 / fixed 0 / left-as-flagged 0**.

Verification approach (recorded for future slices):
- Resolver walks every `[text](path)` link, splits off `#anchor` and
  `?query`, resolves `path` relative to the source file, then
  `path.exists()` against the filesystem.
- For `#anchor` suffixes on `.md` targets, the anchor is matched
  against GitHub's heading-slug rules: strip inline markdown
  (`**`, `*`, `\``), lowercase, spaces → hyphens, drop any char that
  isn't `[a-z0-9-_]`. Em-dashes and emoji collapse to nothing
  (yielding double-hyphen runs) which is exactly how GitHub renders
  them.
- Code fences (`\`\`\``) and inline-code spans (`\`…\``) are stripped
  before link extraction to avoid false-positives on documented
  example links.
- Excluded paths: `target/`, `node_modules/`, `.git/`,
  `fixtures/ontologies/` (per task spec — the W3C ontology fixtures
  carry their own internal links unrelated to repo docs).

Surprising findings: none. The repo is already link-clean. This
reflects the cumulative discipline of slices #33–#66 (each prior
slice has fixed its own anchor / path drift in-flight rather than
deferring), so the final lockdown pass finds nothing to fix. If a
permanent CI gate is wanted for v0.3 GA, the audit logic (61 files →
153 links → 0 broken in under a second) is small enough to land as
a `just check-links` recipe in a follow-up slice.

### Docs — guide intro + install audit (slice #44)

Companion pass to slice #45: walked the three user-guide entry-point files
(`guide/README.md`, `guide/00-intro.md`, `guide/01-install.md`) end-to-end
against current shipped reality — same discipline as the README audit, no
restructuring, only drift correction.

Audit scope and results:

| File | Surface | Result |
| --- | --- | --- |
| `guide/README.md` | All internal + external links, page-row blurbs, client-page targets, GitHub-issues URL | clean — every link resolves; the four pages and four client guides exist on disk |
| `guide/00-intro.md` | Status block ("Alpha, v0.3 engine feature-complete"), SPARQL feature row vs LLD §3 capability matrix, deferred-to-v0.4 list vs LLD §3 ⏳ entries, ERRATA E-009 link, naming + conventions claims | one fix — RDF-star out-of-scope citation pointed at "SPEC.pgRDF.LLD §2" but neither LLD v0.2 §2 ("High-Level Architecture") nor LLD v0.3 §2 ("What's shipped") addresses RDF-star / quoted triples. The citation is unfounded; re-pointed to ERRATA E-009 (the actual upstream feature-unification block on RDF 1.2 triple-term support, same root cause that gates the real SHACL impl) |
| `guide/00-intro.md` | SPARQL feature line: SELECT/ASK with BGP + FILTER + DISTINCT/LIMIT/OFFSET/ORDER BY + OPTIONAL + UNION + MINUS + aggregates (COUNT, SUM, AVG, type-aware MIN/MAX, GROUP_CONCAT, SAMPLE) + HAVING (alias **and** inline aggregate) + BIND | clean — matches LLD §3 verbatim and the README status pill |
| `guide/00-intro.md` | Code-block UDF signatures (`load_turtle`, `count_quads`, `sparql`, `materialize`, `add_graph`) | clean — every example matches `src/storage/`, `src/inference/`, `src/query/` current arity |
| `guide/00-intro.md` | "What's NOT" list (federated SPARQL, full OWL 2 reasoner, RDF-star, replacement for graph DB) | clean |
| `guide/01-install.md` | Path A compose flow (`build-ext`, `compose-up`, `psql`, `CREATE EXTENSION`, `pgrdf.version() → 0.2.0`) | clean — every recipe is in the `Justfile`; `compose/.env.example` exists; `pgrdf.version()` returns `env!("CARGO_PKG_VERSION") = "0.2.0"` |
| `guide/01-install.md` | PG-version range claim (`postgres:14..postgres:17`) | clean — matches ERRATA E-006 hold (pgrx 0.16.1, PG 14-17) for v0.3; PG 18 deferred to v0.4 |
| `guide/01-install.md` | Path B Kubernetes ref to `specs/SPEC.pgRDF.INSTALL.v0.2.md` | clean — spec is unchanged in v0.3 per LLD §0 |
| `guide/01-install.md` | Path C manual-install URL (`releases/download/v0.2.0/pgrdf-0.2.0-pg17-glibc-amd64.tar.gz`) | acknowledged as illustrative — no `v0.2.0` GitHub release exists yet (no git tag in the repo), but the section header is "If you have a Postgres server you control" with a "Download the matching tarball" comment that reads as a worked example for the post-release case. INSTALL spec §3 uses the same placeholder pattern with `0.4.1`. Left as-is per the conservative rule. |
| `guide/01-install.md` | Verify-install snippet (`SHOW shared_preload_libraries`, `pgrdf.stats() -> 'shmem_ready'`) | clean — `shmem_ready` is the documented field in `src/storage/stats.rs:45` |
| All three files | "Phase 3 — Extended SPARQL surface" stale-label check (slice #45 adjacent finding) | not present — the guide files don't carry the conflicting roadmap label |
| All three files | `just test-all` / new recipe coverage | not referenced — the guide intentionally points users at `compose-up` + `psql`, not the test harness |

**Result: one citation correction.** The §2 reference for the RDF-star
out-of-scope policy was unfounded — the policy stands as a project
decision rooted in ERRATA E-009 (`oxrdf` `rdf-12` feature surface
conflicting with `reasonable 0.4.1`), not in any LLD section. Re-pointed
the citation; no other text moved.

Drift sources surveyed for completeness:
- v0.3 vs v0.4 target labels — every deferred bullet in the guide files
  already says `⏳ v0.4` correctly.
- "Alpha"/"unstable"/"experimental" framing — the guide says
  "Alpha, v0.3 engine feature-complete" which is the same status the
  README pill carries. No "experimental" / "unstable" wording leaked in.
- Test counts (93/39/23/3 = 158) — guide files don't cite specific
  counts (those live in the README + `docs/08-testing.md`); no drift
  surface here.

This completes the three-file user-guide entry-point sweep.

### Docs — README audit (slice #45)

Final pre-release pass over `README.md` — every badge, every link, every
code block, every test-count claim, every status pill walked against
current shipped reality.

Audit scope and results:

| Surface | Check | Result |
| --- | --- | --- |
| Top-of-file badges (12) | URL resolves, value matches reality | clean |
| Status row pill | "v0.3 engine surface feature-complete", SPARQL feature list, Phase 3/4/5/6 labels, PG version list, deferred-to-v0.4 list | clean — Phase numbering matches `specs/SPEC.pgRDF.LLD.v0.3.md` §5; SPARQL feature list matches `docs/10-roadmap.md` §Phase 3 steps 1–12 plus the brought-forward HAVING-inline-aggregate and type-aware MIN/MAX |
| Local-file link targets (~25) | Each path exists on disk | clean — every `LICENSE`, `NOTICE`, `docs/*`, `guide/*`, `specs/*`, `tests/perf/*`, `tests/w3c-sparql/`, `TEST.ONTOLOGY-SET.md` link resolves |
| Code-block UDF signatures (`load_turtle`, `load_turtle_verbose`, `parse_turtle`, `add_graph`, `count_quads`, `materialize`, `sparql`, `sparql_parse`, `version`) | Signature in README matches `src/` | clean — every example matches current arity and return type |
| `just` recipes referenced (`build-ext`, `compose-up`, `psql`, `test`, `test-regression`, `test-w3c`, `test-lubm`, `test-all`, `test-conformance`, `test-everything`, `smoke-cold`) | Recipe exists in `Justfile` | clean |
| Test counts (93 pgrx + 39 pg_regress + 23 W3C-shape + 3 LUBM-shape = 158) | Match disk reality | clean — pgrx `#[pg_test]` grep returns 93; `tests/regression/sql/*.sql` count is 39; `tests/w3c-sparql/` has 23 test dirs; `tests/perf/lubm-shape/` has 3 query dirs |
| Smoke claim (24 ontologies, 17,134 triples) | Matches `tests/perf/smoke-ontologies.expected.tsv` | clean — 24 rows, sum of triple-count column is 17,134 |
| License section | Matches `LICENSE` + `NOTICE` (Copyright 2026 Peter Styk, Apache-2.0) | clean |
| ERRATA E-006 re-check date (2026-05-14) | Matches `specs/ERRATA.v0.2.md` | clean |
| `pgrdf.version()` return ("0.2.0") | Matches `Cargo.toml` `version` field | clean |
| CI W3C-shape + LUBM-shape wiring | Workflow runs `tests/w3c-sparql/run.sh` and `tests/perf/lubm-shape/run.sh` | clean — `.github/workflows/ci.yml` lines 119 + 128 |

**Result: zero drift.** No facts required correction; no links required
re-targeting; no signatures required updating. README is consistent with
the v0.3 LLD, the current Justfile, the current test fixtures, the
current Cargo.toml, and the current ERRATA. The audit re-establishes the
baseline before the v0.3 tag.

One adjacent finding — out of scope for this audit but noted for the
follow-on: `docs/10-roadmap.md` carries **two overlapping Phase 3
labels** (a `### Phase 3 — Extended SPARQL surface` heading at line 130
inside the `## Phase 2` section, AND a "Phase 3 storage performance" use
at the intro line 5 and the §270 test-bar-over-time table). Both are
internally consistent with the LLD §5 phase numbering (`Phase 3 =
Storage Performance`), but the in-line heading creates a local
ambiguity. Not corrected here — roadmap surgery is its own slice — and
the README correctly tracks the LLD scheme. Filed mentally for the
roadmap maintenance group.

### Hygiene — Cargo.lock freshness audit (slice #46)

Final entry in the hygiene group (54 → 46, sixty-six → forty-six). Verified
`Cargo.lock` is committed (`git ls-files Cargo.lock` returns the path) and
matches `Cargo.toml`: `cargo metadata --format-version 1` resolves clean on
the online index. **Reproducibility check** — captured the lock's MD5
(`1627cb986cfb73ca300550854b9564d5`), ran `cargo build --no-default-features
--features pg17` (via the rustup-managed `stable-aarch64-apple-darwin`
toolchain at rustc 1.95, since Homebrew's PATH-first rustc 1.88 is below
the declared MSRV); link step fails on the workstation as expected (pgrx
final link wants `pg_config` on PATH, not in scope here), but resolution
completes and the post-build MD5 is byte-for-byte identical. The lock is
stable — Cargo did not touch it during a fresh resolution pass.

**`cargo update --dry-run --verbose`** (online):

```
Updating crates.io index
 Locking 1 package to latest compatible version
Unchanged pgrx v0.16.1 (available: v0.18.0)
Unchanged pgrx-tests v0.16.1 (available: v0.18.0)
Updating winnow v1.0.2 -> v1.0.3
warning: not updating lockfile due to dry run
```

| Crate | Bump | Classification | Root |
| --- | --- | --- | --- |
| `winnow` | 1.0.2 → 1.0.3 | Safe patch of a build-time transitive | `pgrx-pg-config → cargo_toml → toml 0.9.12+spec-1.1.0 → toml_parser 1.1.2 → winnow 1.0` |
| `pgrx` | 0.16.1 → 0.18.0 (held) | Pinned root (E-006) — not eligible under current `Cargo.toml` constraint `0.16` | direct |
| `pgrx-tests` | 0.16.1 → 0.18.0 (held) | Pinned root (E-006) — same | dev-dep |

The single eligible bump (`winnow` 1.0.2 → 1.0.3) is a patch on the
inner parser used by `cargo_toml` at build time — no runtime crate
touched, no `serde_json`/`serde_core`/`tokio`/-sys edge in scope, no
pinned root moves. Under a v0.4-cycle policy this would land
automatically alongside a `just test-regression` re-run.

**Decision: skip and defer to the v0.4 hygiene cycle.** Rationale:
the lock is reproducing cleanly, the only eligible bump is a single
transitive patch with zero behavioural surface, and the v0.3 tag is
imminent. Intentional churn against `Cargo.lock` this close to a
release tag adds risk (new regression-test pass required, new
artifact hash, new container layer) without proportionate benefit.
The held bumps on `pgrx` 0.16 → 0.18 are gated by ERRATA E-006 and
will move only when E-006 resolves; that's a v0.4 work item already
on the roadmap, and `winnow` will ride along on the same `cargo
update` invocation at that point.

This closes the hygiene group (slices #54 → #46, 9 entries). Lock
is fresh, reproducible, and audited; the next intentional refresh
is owed in the v0.4 cycle.

### Hygiene — lints allowlist review (slice #47)

Audited every `#![allow(...)]` / `#[allow(...)]` attribute in `src/`
plus the `[lints.rust]` block in `Cargo.toml`. Procedure: comment each
entry, run `cargo check --no-default-features --features pg17 --tests`
(rustc 1.95.0 via rustup-installed stable, since the Homebrew rustc on
this workstation is 1.88 — below the declared MSRV), then restore.
For each entry, classify the lint as still firing (keep), masking a
single site only (narrow candidate), or no longer firing (trim
candidate).

| Allow | Location | Scope | Lint still fires? | Disposition |
| --- | --- | --- | --- | --- |
| `unreachable_patterns` | `src/lib.rs:14` | crate | Yes — 6 sites (`reasonable.rs:247`, `loader.rs:161`, `executor.rs:1861`, `executor.rs:1902`, `parser.rs:161`, `parser.rs:212`) | Keep. Rationale ("future-proof against upstream `#[non_exhaustive]` variant additions") is crate-wide design intent. |
| `clippy::doc_lazy_continuation` | `src/lib.rs:19` | crate | Yes — 4 doc sites (`lib.rs:35`, `lib.rs:36`, `executor.rs:450`, `executor.rs:451`) | Keep. Rationale ("vertically-aligned ASCII continuation lines"). |
| `clippy::useless_conversion` | `src/lib.rs:25` | crate | Yes — 1 site (`executor.rs:156` `SetOfIterator::new(rows.into_iter())`) | Keep. Single-site narrowing would invert the rationale ("don't litter call sites with annotations"). |
| `unreachable_patterns` | `src/inference/reasonable.rs:246` | item | Redundant under crate-level allow above, but documents intent at the call site | Keep. |
| `unreachable_patterns` | `src/storage/loader.rs:160` | item | Same as above. | Keep. |
| `[lints.rust] unexpected_cfgs check-cfg = ["cfg(feature, values(\"pg13\", \"pg18\"))"]` | `Cargo.toml:53` | crate | Yes — 9 `pg13` + 9 `pg18` sites under rustc 1.95 (pgrx 0.16.1's `pg_shmem_init!` / per-PG `pg_guard` shims expand cfg branches for every PG major they know about regardless of which `feature = "pgN"` we select) | Keep. |

**Result:** zero trims. Every allow on disk currently suppresses a
real lint that fires under rustc 1.95. The two item-level
`unreachable_patterns` allows are redundant under the crate-level one
but document intent at the call site, so they stay. Cargo accepts
`[lints.rust]` as written (the IDE schema linter flags it as invalid
under its TOML schema; `cargo check` parses it without complaint).
This audit re-establishes the baseline: a future slice that drops or
narrows an allow can point back here as the prior-state record.

### Hygiene — ERRATA E-006 pgrx-upstream re-check (slice #48)

Refreshed `specs/ERRATA.v0.2.md` E-006 against today's upstream state.
`crates.io` reports `pgrx.max_stable_version = "0.18.0"` (unchanged
since 2026-04-17); `develop` is one commit ahead (PR #2280, an
aarch64 `-Wl,--no-gc-sections` link-flag fix). Upstream README now
documents "pgrx supports Postgres 13 through Postgres 18" — PG 18
support has officially landed at the 0.18.0 line. Local-compile
blockers from the 2026-05-13 saga are unchanged: 0.17.0's
`non_null_from_ref` E0658 and 0.18.0's `impl_table_iter` E0716 still
reproduce on every Rust stable/nightly we tested, and `develop` has
not touched the relevant macro since the release. Additionally,
0.18.0 carries a hard breaking migration (PR #2264 /
`v18.0-MIGRATION.md`): `pgrx_embed` binary removed, `crate-type` must
drop `"lib"`, manual `SqlTranslatable` impls move from methods to
associated `const`s. pgRDF still ships `src/bin/pgrx_embed.rs` and
`crate-type = ["cdylib", "lib"]`, so the bump is non-trivial.

**Disposition:** E-006 stays open. Classification B — partially
resolved at the upstream layer (PG 18 support exists), still blocked
at the consumption layer (E0716 + breaking-migration scope). Hold
pgrx 0.16.1 + PG 14–17 matrix for v0.3; defer pgrx-0.18 migration to
v0.4 as a planned work item. README + `docs/10-roadmap.md` updated
to reflect the partial-resolution framing. Next re-check trigger:
any pgrx publish above 0.18.0 OR an E0716 fix landing on `develop`.

### Hygiene — MSRV declared (slice #49)

Added `rust-version = "1.91"` to `[package]` in `Cargo.toml`. The value
matches the CI build container (`compose/builder.Containerfile` →
`FROM rust:1.91-bookworm`), which is the only Rust version pgRDF
artifacts are actually produced against. The existing `[lints.rust]`
block already assumes Rust 1.91+'s strict `check-cfg` behavior (see
the inline comment introduced in an earlier slice), so declaring a
lower MSRV would misadvertise support. pgrx 0.16.1's
`resolver = "3"` independently imposes a 1.84 floor; this declaration
tightens that to the value CI verifies. `rust-toolchain.toml` stays
on `channel = "stable"` — pinning a specific minor for an active
project trades health for false stability.

Verification: `cargo check --no-default-features --features pg17
--ignore-rust-version` clean on the dev workstation (rustc 1.88.0
Homebrew); the `--ignore-rust-version` is required only because the
workstation toolchain is older than the declared MSRV — CI's 1.91
container is unaffected. Bump the `rust-version` in lockstep with
the Containerfile when upgrading the build floor.

### Hygiene — cargo tree duplicate-version audit (slice #50)

Ran `cargo tree --duplicates --no-default-features --features pg17`
against `Cargo.lock`. Workspace currently resolves to **182 crates**
(normal + build edges); first-order direct deps are seven: `oxrdf`,
`oxttl`, `pgrx`, `reasonable`, `serde_json`, `spargebra` (normal)
plus `pgrx-tests` (dev). Nine crates appear at two distinct versions:

| Crate | Versions | Sources | Fix attempted |
| --- | --- | --- | --- |
| `byteorder` | 0.5.3 / 1.5.0 | `reasonable → roaring 0.5.2` (0.5) vs `pgrx-tests → tokio-postgres → postgres-protocol` (1.5) | No. `reasonable` is pinned (E-009); `roaring 0.5.2`'s old `byteorder 0.5` is structural until `reasonable` bumps. |
| `getrandom` | 0.3.4 / 0.4.2 | `oxrdf/oxttl/spargebra → rand 0.9 → rand_core 0.9 → getrandom 0.3` vs `pgrx → uuid + tempfile + rand 0.10 → getrandom 0.4` | No. Both roots pinned (oxrdf/oxttl/spargebra semantic-stability; pgrx 0.16 E-006). |
| `hashbrown` | 0.15.5 / 0.17.1 | `pgrx-sql-entity-graph → petgraph 0.8.3 → hashbrown 0.15` AND same `petgraph 0.8.3 → indexmap 2.14 → hashbrown 0.17`. `petgraph 0.8.3` itself pulls two hashbrowns. | No. Internal to `petgraph 0.8.3`; not fixable downstream. |
| `itertools` | 0.8.2 / 0.13.0 | `reasonable` (0.8) vs `pgrx-bindgen → bindgen 0.71.1` (0.13) | No. Both roots pinned. |
| `rand` | 0.9.4 / 0.10.1 | `oxrdf/oxttl/spargebra` (0.9) vs `pgrx-tests → tokio-postgres → postgres-protocol` + `pgrx → uuid + tempfile` (0.10) | No. Both roots pinned. |
| `rand_core` | 0.9.5 / 0.10.1 | Follows the `rand` split. | No. Same as `rand`. |
| `thiserror` | 1.0.69 / 2.0.18 | `reasonable` + `cargo_metadata 0.18.1 (via clap-cargo via pgrx-tests)` (1.x) vs `oxrdf/oxttl/spargebra/pgrx/pgrx-pg-config/pgrx-sql-entity-graph/pgrx-tests` (2.x) | No. `thiserror` 1↔2 is an intentional major; reasonable+cargo_metadata cannot move to 2 without their own bumps. |
| `thiserror-impl` | 1.0.69 / 2.0.18 | Mirrors `thiserror`. | No. Same as `thiserror`. |
| `winnow` | 0.7.15 / 1.0.2 | `pgrx-pg-config → cargo_toml → toml 0.9.12+spec-1.1.0` — same `toml` crate uses `winnow 0.7` (top-level parser) AND `winnow 1.0` (via the inner `toml_parser` 1.1.2 helper crate). | No. Internal to `toml 0.9.12`; not fixable downstream. |

Plus six crates that `cargo tree --duplicates` flags but Cargo.lock
shows at exactly one version (`bitflags 2.11.1`, `memchr 2.8.0`,
`peg-runtime 0.8.6`, `percent-encoding 2.3.2`, `serde_core 1.0.228`,
`serde_json 1.0.149`) — these are single-version crates pulled in
through multiple distinct dep chains, which the `--duplicates` view
also surfaces. Nothing to fix on those.

**Disposition:** zero code or `Cargo.lock` changes. Every actual
duplicate roots in a pinned dep (`reasonable` 0.4.1 / `pgrx` 0.16 /
`oxttl`+`oxrdf`+`spargebra` semantic-stability) or in a transitive
internal split (`petgraph` 0.8.3, `toml` 0.9.12). No SemVer-safe
`cargo update --precise` collapse exists today. The duplicate budget
is in line with what a Rust workspace of this composition produces
and is purely informational — recorded so a future audit can diff
against the same picture once `reasonable` and `pgrx 0.16` are
unpinned (E-006, E-009).

### Hygiene — cargo audit (slice #51)

Ran `cargo audit` (v0.22.1, advisory-db 1088 advisories loaded
2026-05-14) against `Cargo.lock` (287 crate deps). **Zero security
vulnerabilities.** Four informational warnings, all in pinned-dep
subtrees (`pgrx 0.16` / `reasonable 0.4.1`) — none has a
SemVer-compatible fix without violating the pinned-core-dep
constraint (E-006 / pgrx 0.16, `reasonable` 0.4.1 RDF 1.2 saga in
E-009). Deferred to the cuts that bump those upstreams.

| ID | Kind | Crate | Source | Disposition |
| --- | --- | --- | --- | --- |
| RUSTSEC-2024-0375 | unmaintained | `atty 0.2.14` | `reasonable 0.4.1 → env_logger 0.7.1 → atty` | Defer. Fix requires `reasonable` to bump `env_logger` past 0.7; `reasonable` is pinned (see ERRATA E-009). |
| RUSTSEC-2021-0145 | unsound | `atty 0.2.14` | same path as above | Defer. Same root cause; unaligned-read CVE in `atty`'s Windows path. Unreachable on the Linux/macOS targets pgRDF builds for, but the advisory still trips on `Cargo.lock`. |
| RUSTSEC-2024-0436 | unmaintained | `paste 1.0.15` | `pgrx-tests 0.16.1 → paste` (dev-dep only) | Defer. Test-only proc-macro dep of pgrx-tests. Resolves when pgrx is unpinned (E-006). |
| RUSTSEC-2021-0127 | unmaintained | `serde_cbor 0.11.2` | `pgrx 0.16.1 → serde_cbor` | Defer. Hard transitive of pgrx 0.16. Resolves when pgrx is unpinned (E-006). |

Counts: Critical 0 / High 0 / Medium 0 / Low 0 / Yanked 0 /
Informational 4. No code or `Cargo.lock` changes — the advisories
are real but structurally unfixable in v0.3 without breaking the
pinned-dep contract. New ERRATA entry **E-010** records the
pinned-dep advisory ledger so future audits can diff cleanly.

### Hygiene — stale-docstring sweep

Audited every public `#[pg_extern]` and module-level docstring under
`src/` against actual signatures and behavior, looking for prose that
lied about current code (wrong return types, missing JSONB fields,
"Phase N backlog" claims for features that have since shipped, "still
unsupported" lists that the executor now handles). Eleven category-A
fixes landed across 7 files:

| file:line | drift | fix |
| --- | --- | --- |
| `src/storage/hexastore.rs:46` | `add_graph` SQL surface doc claimed `→ VOID` | corrected to `→ BOOLEAN`, documented return semantics (TRUE = created) |
| `src/storage/loader.rs:320-321` | `load_turtle_verbose` JSONB-field list missed `shmem_cache_hits` | added the field to the doc |
| `src/storage/stats.rs:17-29` | `stats()` JSON example showed only `shmem_*` keys; `plan_cache_*` keys (added in Phase 3 step 2) were missing | extended the example with all four `plan_cache_*` fields + explanatory note |
| `src/storage/mod.rs:3-5` | module doc said "Implementation status: skeleton" | replaced with a submodule index reflecting the v0.3 reality |
| `src/query/parser.rs:9-21` | "Today's scope" claimed OPTIONAL/UNION/non-BGP get flagged in `unsupported_algebra`; code walks through them | rewrote scope list to reflect the actual `unsupported_algebra` rejection set + added v0.4 cross-refs |
| `src/query/plan_cache.rs:103` | `plan_cache_clear` SQL surface doc said `→ integer` | corrected to `→ BIGINT` (matches `i64` return) |
| `src/query/executor.rs:14-17` | module doc claimed "dynamic SQL only carries integer constants" — incorrect post-Phase-3-step-2 (placeholders, not inlined constants) | rewrote to reflect `$N` positional parameter binding |
| `src/query/executor.rs:20-21` | "Scope today: SELECT only (no CONSTRUCT/ASK/DESCRIBE)" | ASK ships — fixed; CONSTRUCT/DESCRIBE marked with v0.4-FUTURE §6 pointer |
| `src/query/executor.rs:61-69` | scope said "HAVING and `GROUP_CONCAT` / `SAMPLE` are Phase 3 backlog" and "BIND remain unsupported" — all three landed | rewrote aggregates + BIND blocks to reflect implementation; v0.4-FUTURE pointers on the still-unsupported set |
| `src/query/executor.rs:498-499` | `parse_aggregate` doc listed only COUNT/SUM/AVG/MIN/MAX | added GROUP_CONCAT and SAMPLE; noted Custom IRI panic |
| `src/query/executor.rs:1391-1404` | `translate_filter` doc said numeric ordering, IN, REGEX were "not yet supported" — they are | rewrote both lists; left `EXISTS` + conditional `IF` as still-unsupported |
| `src/query/executor.rs:1535-1539` | `expr_to_id_sql` doc said constants → "inlined integer literal" | corrected to "`$N` parameter placeholder bound to the resolved dict id" |
| `src/query/executor.rs:2107-2109` | test docstring `sparql_unknown_predicate_returns_zero_rows` said translator "inlines `-1`" | corrected to "binds `-1` as the parameterised dict id sentinel" |
| `src/validation/shacl.rs:38-52` | `validate` JSONB schema doc listed `data_graph_exists` + `shapes_graph_exists` fields the body does not emit | removed the two fields from the doc (the stub doesn't emit existence flags — only counts) |

No behavior changes. `cargo check --features pg17` is green. Test bar
unchanged (39 pg_regress + 93 pgrx + 23 W3C + 3 LUBM = 158).

### Licensing — explicit attribution surface

`LICENSE` carries the resolved Apache 2.0 copyright notice
("Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;" + project URL) in
place of the upstream `[yyyy] [name of copyright owner]`
placeholders. A new `NOTICE` file at the repo root carries the
Apache convention header — distributions that bundle pgRDF
should preserve it per Apache 2.0 §4(d). `Cargo.toml` gains an
`authors = ["Peter Styk &lt;peter@styk.tv&gt;"]` field and a
`homepage` mirror of the `repository` URL. `README.md`'s License
section is fleshed out to name the copyright holder and link
both `LICENSE` and `NOTICE`. No code or test changes.

### Spec — SPEC.pgRDF.LLD.v0.4-FUTURE draft landed (forward-looking)

New `specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md` is a draft, forward-looking
target spec for the next cut; v0.3 remains the authoritative
shipped contract until v0.4 actually lands. The draft scopes five
new substantive tracks — named-graph scoping with an IRI ↔ graph_id
mapping table (§3), SPARQL UPDATE including the graph-scoped
variants (§4), graph-level lifecycle UDFs over the LIST-partitioned
quads table (§5), CONSTRUCT returning triple-shaped JSONB rows
(§6), and property paths `*` / `+` / `?` / `^` with
materialised-closure-aware translation (§7) — plus the v0.3-deferred
SPARQL surface backlog (multi-triple OPTIONAL, VALUES,
BIND-downstream, aggregates over UNION, DESCRIBE) which shares
enough translator machinery with §4 and §6 to ship in the same cut.
v0.3 §0 gains a one-line cross-link to the new draft.

### Coverage — error-path regression signals

New `tests/regression/sql/81-error-paths.sql` opens a sibling track
to `80-unsupported-shapes.sql`: instead of locking the failure-mode
of SPARQL translator gaps, it locks the stable error-prefix each
UDF emits when given an invalid input. The helper `_check_error`
generalises `80`'s `_check_gap` to run arbitrary SQL via `EXECUTE`
inside a plpgsql try/catch, capturing the boolean signal (`t` =
expected substring present in SQLERRM) without pinning the
volatile tail (OS-level `os error N` numbers, path strings, etc.).

This commit locks #66 of the 66 → 1 countdown toward v0.3.0:
`pgrdf.load_turtle()` against a missing path must surface the
prefix `load_turtle: failed to open` (from
`src/storage/loader.rs:315`). Downstream tooling matches that
prefix to decide retry-vs-escalate; a silent rename would break
those callers without any pgRDF-side test firing.

Locks #65 of the countdown: a syntactically invalid `base_iri`
argument must surface the prefix `load_turtle: invalid base IRI`
(from `src/storage/loader.rs::ingest_turtle_with_stats`'s
`with_base_iri().unwrap_or_else(...)`). The check fires through
`pgrdf.parse_turtle('...', 9982, 'not an iri at all')` — using
`parse_turtle` keeps the regression file fixture-free while
exercising the same shared ingest path. The panic message is
prefixed `load_turtle:` even when triggered via `parse_turtle`;
that cross-UDF prefix invariance is itself part of the contract
(downstream callers route on one substring regardless of which
UDF parses the Turtle). Empty-string `base_iri` continues to
short-circuit before `with_base_iri()` runs, so callers can
safely pass `''` to mean "no base"; only a non-empty value that
fails oxiri's IRI grammar trips the prefix.

Locks #64 of the countdown: syntactically malformed Turtle bytes
must surface the prefix `load_turtle: turtle parse error` (from
`src/storage/loader.rs:256`'s `triple_result.expect(...)` inside
the parser-iterator loop). The check fires through
`pgrdf.parse_turtle(':alice :name "Alice"', 9964)` — the
fragment uses the default `:` prefix without declaring it, so
oxttl rejects at byte 0 with `The prefix : has not been
declared`; that specific complaint is tail / volatile, the
locked substring is just the `load_turtle: turtle parse error`
prefix. The same cross-UDF prefix invariance as error-65
applies: the panic text says `load_turtle:` regardless of
whether bytes entered via `load_turtle()` or `parse_turtle()`,
so downstream tooling routes on one substring. Any malformed
Turtle variant (missing trailing dot, undeclared prefix, bad
IRI ref, RDF-star in default mode) trips the same prefix.

Locks #63 of the countdown: a syntactically malformed SPARQL
query handed to `pgrdf.sparql()` must surface the prefix
`sparql: parse error:` (from `src/query/executor.rs:142`'s
`SparqlParser::new().parse_query(sql).unwrap_or_else(...)`).
The check fires through
`SELECT * FROM pgrdf.sparql('this is not sparql at all')` —
spargebra rejects at byte 10 with `expected CONSTRUCT`; that
specific complaint plus the line:col coordinates are tail /
volatile across spargebra versions, the locked substring is
just the `sparql: parse error:` prefix. This is the user-facing
contract surface for query-parse failure, distinct from the
translator-gap prefix locked across `80-unsupported-shapes`
(`sparql: …not supported yet`, `sparql: aggregates on top of
UNION…`, etc.) and from the RDF-ingest prefixes locked in
error-66/65/64. The sibling introspection UDF
`pgrdf.sparql_parse()` routes through its own panic site with
prefix `sparql_parse:` instead — a deliberate distinction so
callers can tell which entry point the bytes came in through;
that path is covered by the `#[pg_test]`
`sparql_parse_syntax_error_panics` in `src/query/parser.rs`
and is not pinned by this regression slice.

Test bar: **93 pgrx + 34 pg_regress + 23 W3C-shape + 3 LUBM-shape
= 153 tests**, green locally.

### Coverage — edge-case correctness regression signals

New `tests/regression/sql/62-materialize-empty.sql` opens a sibling
track to the error-path file (`81-error-paths.sql`): instead of
locking the prefix a UDF emits when given an *invalid* input, it
locks the *correctness contract* on **edge-case but valid** inputs
the engine must handle without surprise. The countdown shifts from
66→63 (error-path locks) into 62→onward (edge-case locks).

Locks #55 — the final entry in the 66→1 coverage countdown — promotes
the W3C-shape and LUBM-shape harnesses to first-class Justfile
recipes and adds a cold-compose smoke that exercises every
compose-based test layer end-to-end. New recipes: `just test-w3c`
(wraps `bash tests/w3c-sparql/run.sh`), `just test-lubm` (wraps
`bash tests/perf/lubm-shape/run.sh`), `just test-conformance` (the
three compose-based layers: regression + W3C-shape + LUBM-shape),
`just test-everything` (pgrx integration + test-conformance — the
broadest sweep), and `just smoke-cold` (`compose-down` →
`build-ext` → `compose-up` → `CREATE EXTENSION` → test-conformance,
the cold-compose discipline gate). `just test-all` keeps its
original narrow shape (`test` + `test-regression`) for back-compat;
`docs/08-testing.md` and `README.md`'s Tests block point at
`test-everything` and `smoke-cold` as the new entry points. The
shift matters because two of the three compose-based harnesses
(W3C-shape, LUBM-shape) were previously discoverable only by
knowing the bash paths — a contributor running `just --list`
saw nothing about them, and `just test-all` silently skipped
them. Cold-compose smoke is the verification half: it tears the
compose stack down with `compose-down` first (no shortcuts to a
warm `compose-up`), rebuilds the extension artefacts, brings the
stack back, recreates `CREATE EXTENSION pgrdf`, and runs all
three compose-based layers against the fresh state — catching the
class of bugs that pass on a warm compose because some prior
DROP/CREATE left state behind, and break on the next cold boot.
This is the final coverage-countdown slice before the hygiene
phase opens.

Locks #62 of the 66→1 countdown: `pgrdf.materialize(N)` on a graph
with zero base triples MUST NOT panic and MUST return a JSONB stats
object with `base_triples = 0`. The UDF still emits OWL 2 RL
**axiomatic triples** (per `reasonable 0.4`, four self-statements
over `owl:Thing` / `rdfs:Class` / etc. on the empty input) — that
count is upstream-defined and NOT pinned by this slice; only
`inferred_triples_written ≥ 0` is part of the locked contract.
Idempotency carries across the empty case: a second
`materialize(N)` call wipes its own prior `is_inferred=TRUE` rows
before re-deriving, so run 2's `previous_inferred_dropped` equals
run 1's `inferred_triples_written` exactly. Both invariants
project as booleans (`base_is_zero`, `inferred_nonneg`,
`first_run_dropped_zero`, `idempotent`) so the expected output
stays `t` regardless of axiomatic-set churn from upstream
`reasonable` releases.

Locks #61 of the countdown: `pgrdf.shmem_reset()` MUST actually
invalidate the process-wide shmem dict cache. The implementation
in `src/storage/shmem_cache.rs::reset()` bumps a single
`PgAtomic<AtomicU64>` `GENERATION` counter; `lookup()` reads slots
as cold whenever `slot.generation != current`. A refactor that
drops the generation bump would silently leave stale dict ids
visible across a `DROP EXTENSION; CREATE EXTENSION` cycle (where
the dict id space resets), so the regression must catch the
omission. New `tests/regression/sql/63-shmem-reset-invalidation.sql`
warms shmem with three terms, snapshots `(shmem_hits,
shmem_inserts)` via `\gset`, re-parses the same Turtle and asserts
hits went up (sanity — cache is hot), then calls `shmem_reset()`,
re-parses one more time, and asserts (a) `shmem_hits` stayed flat
across the post-reset parse and (b) `shmem_inserts` strictly
increased. Counter VALUES are not pinned — each assertion projects
a single boolean comparing deltas, so the expected output stays
`t`-flat across cumulative-counter drift from prior tests in the
same psql session.

Locks #60 of the countdown: `pgrdf.plan_cache_clear()` MUST return
the literal count of prepared statements drained from THIS
backend's `thread_local!` plan-cache HashMap — NOT zero, NOT a
constant, NOT the cumulative shmem `plan_cache_inserts` counter.
The implementation in `src/query/plan_cache.rs::plan_cache_clear`
reads `m.len()` BEFORE calling `m.clear()` and returns that as
`i64`; a refactor that swaps `m.len()` for a constant, or hoists
the `len()` call to AFTER `m.clear()` (always returning 0), would
corrupt operator-facing telemetry. New
`tests/regression/sql/64-plan-cache-clear.sql` locks four
invariants: (a) fresh backend → `clear()` returns 0 (nothing to
drop); (b) after one `parse_turtle` + three structurally distinct
SPARQL shapes, the drained count matches the pre-clear
`plan_cache_local_size` snapshot; (c) `plan_cache_local_size = 0`
immediately after the clear; (d) a second consecutive clear
returns 0 (idempotent at zero). Empirically `size_before = 4` on
the current pgrx 0.16 / PG 17 build (1 ingest-side `flush_batch`
INSERT plan + 3 SELECT plans), but the test locks the RELATION
`drained = size_before AND size_after = 0 AND idempotent_clear =
0 AND size_before > 0` rather than the literal — an ingest-path
refactor that takes `flush_batch` off the plan-cache path leaves
the test still passing as long as the contract holds. Bare-row
`SELECT count(*) FROM pgrdf.sparql(...)` calls are the cleanest
way to drive distinct plans into the cache; `\gset` captures the
snapshots without polluting the expected-output stream.

Locks #59 of the countdown: `pgrdf.parse_turtle()` MUST accept
*triple-free* Turtle input without panicking and MUST return `0`
as the inserted-triple count. The parser path in
`src/storage/loader.rs::ingest_turtle_with_stats` drives an oxttl
`TurtleParser` iterator whose for-loop body — the only site that
interns dict ids and pushes onto `batch_s/p/o` — runs ONCE PER
TRIPLE. Inputs that contain no triples (empty string,
whitespace-only, Turtle comment lines, bare `@prefix` declaration)
yield zero iterator items: the loop body never executes,
`stats.triples` stays `0`, the trailing `flush_batch()` flushes
empty vectors (no SQL is emitted to `_pgrdf_quads`), and the
function returns `0`. New `tests/regression/sql/65-parse-turtle-empty.sql`
locks six invariants in one go: each of the four
zero-triple inputs returns `0` (four booleans), `_pgrdf_quads`
for the test graph stays empty (one boolean), and
`_pgrdf_dictionary` stays empty across all four parses (one
boolean — interning is loop-body-only, so the `@prefix` IRI in
case 4 is parser-scope state, not a dict write). This is the
orthogonal correct-path companion to the malformed-input case
noted in `81-error-paths.sql` (where the parser panics with the
literal `load_turtle: turtle parse error: …` prefix): an EMPTY
parser iterator is NOT a parse error — it returns `0` cleanly.
Guards against a refactor that wraps the loop in a "fast-path"
panicking on empty input, that seeds a placeholder dict/quad row,
or that mishandles `flush_batch()` of zero-length arrays. The
whitespace-only case uses an `E''` extended-string literal so
`\n` / `\t` reach the parser as actual whitespace rather than
literal backslash-n (which the Turtle grammar correctly rejects).

Locks #58 of the countdown: the smoke-ontologies set MUST keep
parsing AND each ontology's triple count MUST stay stable. The
existing `tests/perf/smoke-ontologies.sh` loads every `*.ttl`
under `fixtures/ontologies/` through `pgrdf.load_turtle` into its
own graph and prints `<filename>: <triples>` per file; today's
snapshot is **24 ontologies, 17,134 triples** (workflow.ttl held
out per ERRATA E-007). Slice #58 captures that snapshot as
`tests/perf/smoke-ontologies.expected.tsv` (alphabetically-sorted
`filename<TAB>triples` rows) and adds a `--check` mode to the
smoke script: it re-runs the smoke, regenerates the TSV from the
live output, and `diff -u`'s it against the lock-file, exiting
non-zero on any drift. The diff catches two regression classes
the bare smoke can't: (a) an ontology that used to parse stops
parsing — the row disappears from the actual side; (b) the
parser silently drops or duplicates triples and the count moves
even though parsing nominally succeeds. The check is NOT yet
wired into CI (the fetched ontology payloads under
`fixtures/ontologies/*.ttl` are gitignored, so CI can't run the
smoke without a fetch step that doesn't yet exist in the
workflow). Landing the lock-file + the opt-in `--check` mode
now means a future Phase 6 slice can wire `--check` once the
ontology-fetch step is added to CI. The default behaviour
(no flag → pretty-print results, exit 0) is unchanged so
existing manual runs still work. Updating the lock-file is a
deliberate maintenance step — when an upstream ontology updates
and the new count is intentional, regenerate the TSV from a
fresh smoke run and commit the delta as one explicit move; no
`--accept`-style automatic refresh.

Locks #57 of the countdown: the end-to-end round-trip from
`pgrdf.parse_turtle` ingest through `pgrdf.sparql` query MUST
preserve every triple the parser saw, across all four
object-term kinds AND the blank-node-subject case. New
`tests/regression/sql/66-parse-sparql-roundtrip.sql` parses a
single 5-shape Turtle fragment and asserts five
`bool_and(EXISTS (SELECT 1 FROM pgrdf.sparql(…) WHERE …))`
booleans, one per shape: (1) IRI object —
`ex:alice foaf:knows ex:bob` resolves with the bob IRI as the
lexical projection of `?o`; (2) plain literal —
`foaf:name "Alice"`; (3) typed literal —
`ex:age "30"^^xsd:integer` projects `"30"`; (4) lang-tagged
literal — `ex:bio "Engineer"@en` projects `"Engineer"`; (5)
blank-node subject — the anonymous `[ a foaf:Person ;
foaf:name "Anon" ]` is keyed via a sibling-property join
`?s foaf:name "Anon" . ?s foaf:name ?n` so the
parser-allocated bnode id stays out of the assertion and the
contract is "queryable via sibling property", not "this
specific bnode id". Sibling to `61-materialize-then-sparql.sql`
which locks the materialize→sparql edge; together they pin
both ends of the storage layer's visibility contract to the
SPARQL surface. Datatype URI and lang-tag echo policy are
NOT pinned by this slice — the `pgrdf.sparql` projection
emits the lexical value only; the storage-side datatype-URI
contract is locked separately by `21-typed-literals.sql` and
the lang-tag contract by `22-lang-tags.sql`. Guards against a
refactor that loses a triple from the dict→quads write path
for any one of these five term kinds.

Locks #56 of the countdown: the `pgrdf.stats()` JSONB shape MUST
NOT silently gain, lose, rename, or `null` a field — the canonical
key set is closed at the 10 keys emitted by
`src/storage/stats.rs::stats()` today (`shmem_ready`, `shmem_slots`,
`shmem_hits`, `shmem_misses`, `shmem_inserts`, `shmem_evictions`,
`plan_cache_hits`, `plan_cache_misses`, `plan_cache_inserts`,
`plan_cache_local_size`). Extends the existing `82-stats-shape.sql`
in-place (no new pg_regress file — the file is explicitly scoped to
schema-shape contract and these three new invariants are schema
shape too) with three appended assertion blocks: (a) exact field
count — `count(*) FROM jsonb_object_keys(stats()) = 10`, the
deliberate-update tripwire that fires the moment any new field
lands without a corresponding test update; (b) keys-match-canonical
— `array_agg(k ORDER BY k) = ARRAY[…literal 10-element list…]`,
catches both silent additions and silent renames in one assertion
(an addition makes the array longer; a rename swaps an element);
(c) no-null-fields — `bool_and(jsonb_typeof(value) != 'null')`,
catches a refactor that defaults an uninitialised counter to JSON
`null` rather than `0` (the type-contract block above would not
fire on a null since `jsonb_typeof(null) = 'null'` is checked
positively only on the seven existing-key assertions, not on
unknown keys). The existing "fields-that-should-be-there are
there" assertions are sibling to these "fields-that-shouldn't-be-
there ARE NOT there" assertions; together they pin the closed-set
shape contract that downstream operator tooling (CloudNativePG
operators, CI dashboards, client telemetry parsers) wires against.

Test bar: **93 pgrx + 39 pg_regress + 23 W3C-shape + 3 LUBM-shape
= 158 tests**, green locally. Slice #58 doesn't add a pg_regress
file — the smoke is a separate harness, so its lock-file (24
rows / 17,134 triples) lives alongside the script and is
enforced by `tests/perf/smoke-ontologies.sh --check`. Slice #57
adds the 39th pg_regress file (`66-parse-sparql-roundtrip.sql`).
Slice #56 extends `82-stats-shape.sql` in-place — three new
assertion blocks, three new rows in the expected baseline — no
test count bump (still 39 pg_regress files).

### Translator fix — type-aware `MIN` / `MAX`

`src/query/executor.rs::translate_aggregate` for `MIN` / `MAX`
previously emitted

    MIN(lexical_value)

which sorts lexicographically — so over the four `xsd:integer`
literals `10, 2, 100, 20` it returned `"10"` (since
`"10" < "100" < "2" < "20"` as strings). Now emits

    COALESCE(MIN(numeric_cast_subselect)::text, MIN(lexical_value))

so when any row in the group has an `xsd:numeric` datatype the
numeric MIN/MAX wins (matches the SUM/AVG path that has been
type-aware since Phase 2.2). Pure-string groups fall back to
lexicographic ordering. Mixed-type groups prefer numeric — the
SPARQL spec (§17.4) leaves mixed-type ordering
implementation-defined.

Coverage: new `tests/w3c-sparql/23-min-max-numeric/` — fixture's
`xsd:integer` literals `10/2/100/20` produce `MIN=2, MAX=100`
(would have been `MIN="10", MAX="20"` lexicographically).

Test bar: **93 pgrx + 33 pg_regress + 23 W3C-shape + 3 LUBM-shape
= 152 tests**, green locally. v0.4 deferred SPARQL surface
shrinks by one entry.

### Translator fix — inline `HAVING(SUM(?v) > c)` now supported

`src/query/executor.rs::AggregateSpec` gains a `synth_aliases:
Vec<String>` field that preserves spargebra's synthetic
intermediate-variable name even after `Extend` renames
`output_var` to the user's AS-alias.

Why: spargebra emits algebra of the form
```
Project { Filter(HAVING) { Extend(?total = $synth) {
  Group(aggregates=[($synth, SUM(?p))]) } } }
```
The `Extend` visitor previously rewrote `output_var` from `$synth`
to `?total` and dropped the `$synth` mapping. The HAVING filter
`Filter(Greater(Variable($synth), Literal(15)))` then couldn't
find its aggregate during the filter-migration step and fell
through to the non-aggregate-aware FILTER translator, producing
`sparql: FILTER expression not translatable`.

The fix:
- `AggregateSpec.synth_aliases` is initialised by
  `parse_aggregate` with the original `$synth` name and never
  modified by `Extend`.
- The filter-migration step's `agg_names` is now the union of
  every aggregate's `output_var` AND its `synth_aliases`.
- `translate_filter_with_aggregates`'s lookup helper (`find_agg`)
  consults both fields.

Effects:
- `tests/regression/sql/80-unsupported-shapes.sql::gap-1` removed
  (was negative-locked; no longer a gap).
- New positive coverage:
  `tests/w3c-sparql/22-having-inline-aggregate/` — same shape as
  `08-aggregates-having` but with the inline `HAVING(SUM(?p)>15)`
  form. Hand-computed expected output verified.
- Both forms are now first-class. `08`'s description.md updated
  to note the companion test.
- v0.4 SPARQL-surface deferred list shrinks by one entry.

Test bar: **93 pgrx + 33 pg_regress + 22 W3C-shape + 3 LUBM-shape
= 151 tests**, green locally.

### Translator-gap regression signals + Phase 6 step 3 scaffolding

Two adjacent additions, motivated by a real translator gap I hit
while expanding the W3C-shape harness (W3C 08 — inline `HAVING(SUM
(?v) > c)` falls through with `FILTER expression not translatable`
when spargebra synthesises a fresh aggregate node for the HAVING).

**1. `tests/regression/sql/80-unsupported-shapes.sql`** —
regression signals locking the failure-mode contract for every
known unsupported SPARQL shape. Each gap drives a query that MUST
fail, and asserts via plpgsql `EXCEPTION WHEN OTHERS` that
`SQLERRM` contains a stable error-prefix substring. The check
helper outputs a clean boolean (`t` = expected substring present)
rather than the raw error message — so the baseline isn't pinned
to spargebra's algebra-dump format, synthetic variable hashes, or
upstream `dataset` / `base_iri` internals.

Gaps locked in:
- `gap-1` — `HAVING(SUM(?v) > c)` inline (vs the supported
  alias form `HAVING(?total > c)`).
- `gap-2` — multi-triple OPTIONAL.
- `gap-3` — VALUES inline data block.
- `gap-4` — GRAPH named-graph clause.
- `gap-5` — CONSTRUCT query form.
- `gap-6` — DESCRIBE query form.
- `gap-7` — property path with `*` repetition.
- `gap-8` — aggregates over UNION.

If pgRDF accidentally starts producing wrong results for any of
these shapes (translator regression), the baseline diff fires
with `unexpected success`. If we genuinely add support for a
shape, this file is the single place to flip the assertion to a
positive test.

**2. `tests/perf/lubm-shape/`** — Phase 6 step 3 scaffolding.
Three hand-authored LUBM-shape queries (`Q1` class membership,
`Q2` `teacherOf`, `Q3` `takesCourse` aggregate) against a small
LUBM-shape fixture. Same directory-per-test layout + bash runner
shape as `tests/w3c-sparql/`; runs alongside the W3C harness in
the CI `regression` job. Real LUBM-1/10/100 with the Java
generator + cross-engine comparison vs Apache Jena TDB and
Apache AGE remains v0.4 work (see `tests/perf/README.md`).

Test bar: **93 pgrx + 31 pg_regress + 18 W3C-shape + 3 LUBM-shape
= 145 tests**, green locally.

### Phase 6 step 2 starter — W3C-shape SPARQL harness

- `tests/w3c-sparql/` ships a directory-per-test harness with **13
  hand-authored W3C-shape conformance tests** covering common
  spec patterns:
  - `01-basic-bgp` — §5 Basic Graph Pattern.
  - `02-distinct` — §15.4 `SELECT DISTINCT` multiset → set.
  - `03-union-disjoint` — §18.2.4 `UNION` with disjoint variables
    (unbound → null in cross-branch rows).
  - `04-optional-chain` — §6 `OPTIONAL` keeps the row when the
    optional pattern fails to match.
  - `05-minus-no-shared` — §8.3.2 `MINUS` with no shared variables
    is a no-op (the translator elides the WHERE NOT EXISTS).
  - `06-filter-isiri` — §17.4.2.1 `isIRI` term-type filter.
  - `07-aggregates-count` — §11 `COUNT(?v) GROUP BY ?s`.
  - `08-aggregates-having` — §11.5 `HAVING(?alias > c)` after `SUM`.
  - `09-order-by-desc` — §15.1 `ORDER BY DESC(?v)`.
  - `10-limit-offset` — §15.2 / §15.3 `LIMIT 2 OFFSET 2`.
  - `11-bind-concat` — §10.1 `BIND` + §17.4.3.2 `CONCAT(...)`.
  - `12-ask-true` — §16.2 `ASK` returning `true`.
  - `13-ask-false` — §16.2 `ASK` returning `false`.
  - `14-filter-regex` — §17.4.3.14 `REGEX(?v, "^A")`.
  - `15-filter-in` — §17.4.1.9 `FILTER(?v IN (...))`.
  - `16-strlen` — §17.4.3.3 `STRLEN(?v)`.
  - `17-lang-tag` — §17.4.2.4 `LANG(?v)` over language-tagged literals.
  - `18-ucase` — §17.4.3.8 `UCASE(?v)`.
- `tests/w3c-sparql/run.sh` is a bash runner: for each test it
  drops + recreates the extension, loads `data.ttl`, runs
  `query.rq` via `pgrdf.sparql`, sorts both sides
  lexicographically (bag-equivalent comparison; SPARQL solutions
  are unordered absent ORDER BY), and `diff -u`s against
  `expected.jsonl`. `ACCEPT=1` regenerates expected; every
  baseline must be hand-verified against the W3C spec.
- Each test ships a `description.md` quoting the spec section
  exercised + the hand-computed expected JSONL — load-bearing for
  reviewers and for the "never ACCEPT=1 blind" rule from v0.3 §6.2.
- Wired into CI's `regression` job (right after the pg_regress
  suite, using the same compose Postgres). The W3C harness is
  gated PR-on / push-on like the rest of the regression suite.
- `regression-w3c.yml` nightly workflow stays gated `if: false`
  — it's the destination shape for the **full W3C TTL-manifest
  runner** (`pgrdf-w3c-sparql` Rust binary parsing
  `w3c/rdf-tests/sparql/sparql11/manifest.ttl` against the
  ratcheting coverage targets `≥ 30 % → ≥ 70 % → ≥ 95 %`).
  v0.4 work item; not blocking the v0.3 release.

### Phase 6 step 1 — regression suite in CI

- `.github/workflows/ci.yml`: new `regression` job runs the
  compose-based pg_regress suite on every PR + push to main. The
  job:
  - Builds `pgrdf.so` via `compose/builder.Containerfile`
    (BuildKit, same path as the local dev loop).
  - Boots `postgres:17.4-bookworm` via `docker compose up -d` with
    the artifacts bind-mounted at the canonical paths.
  - Waits on the compose healthcheck, then drives
    `tests/regression/sql/NN-*.sql` via
    `PGRDF_RUNTIME=docker bash tests/regression/run.sh`.
  - Captures `docker logs pgrdf-postgres` on failure for triage.
  - Tears the stack down with `compose down -v` on `always()`.
- Pinned to PG 17 today (compose pin per ERRATA E-006). Widens to
  the full matrix when the PG-18 / pgrx issue clears.
- `tests/regression/run.sh` already honoured `PGRDF_RUNTIME` so no
  runner changes needed.

**Deferred (still placeholders):**
- W3C SPARQL 1.1 + SHACL conformance runners live in
  `.github/workflows/regression-w3c.yml` gated `if: false`. Need a
  Rust runner binary that reads the manifest TTL, materialises each
  test's data graph, runs the query, and diffs against the expected
  result. v0.4 work item.
- LUBM-10 / LUBM-100 perf comparison vs Jena TDB and Apache AGE.
  Needs `tests/perf/run-lubm.sh` + a normalised reporting layer.
  v0.4 work item.
- Release workflow (`release.yml`) is wired but only fires on
  `v*` tags. Tag the first release once Phase 6 step 2 (the
  conformance runners) lands.

### Phase 5 — SHACL `pgrdf.validate` ships as a STUB

- `src/validation/shacl.rs`: `pgrdf.validate(data_graph_id,
  shapes_graph_id) → JSONB` is wired with a stable response shape
  but a `{"status": "stub", "reason": "...", …}` body. The UDF
  echoes both graph IDs and reports the actual triple count in
  each — enough for downstream tooling (CloudNativePG operators,
  client libraries, CI jobs) to integrate the SQL surface today.
- 2 new pgrx tests (`validate_stub_shape`,
  `validate_stub_unknown_graphs`) lock the JSONB schema.
- New regression `70-validate-stub.sql` asserts: status = "stub",
  `data_graph_id` / `shapes_graph_id` echoed, triple counts
  matched, `conforms` is `null`, `results` is an empty array,
  `reason` field present. Hand-computed; never ACCEPT=1 baselined.
- Test bar: **91 → 93 pgrx + 29 → 30 regression**, green.

**Why a stub, not a real impl.** New ERRATA entry
[`E-009`](specs/ERRATA.v0.2.md). Briefly:
- `shacl_validation 0.2.x` (latest 0.2.12) ships an unfinished
  `iri_s` → `rudof_iri` migration; `shacl_ast 0.2.9` fails to
  compile against the resolved tree
  (`expected rudof_iri::IriS, found iri_s::IriS`).
- `shacl_validation 0.1.149` compiles in isolation but its
  transitives turn on `oxrdf`'s `rdf-12` feature, which adds
  `TermRef::Triple(_)` — a variant `reasonable 0.4.1`'s pattern
  match doesn't handle. Cargo feature unification means we can't
  have both crates in one workspace until either upstream catches
  up.
- We chose to ship Phase 4 (inference) first because it's
  load-bearing; Phase 5's real implementation is a v0.4 follow-up
  the moment upstream unblocks. The stub keeps the surface
  available so nothing downstream gets blocked on a missing UDF.
- `Cargo.toml` carries the `shacl_validation = "0.2"` line
  commented out with the full reason inline.

### Phase 4 — OWL 2 RL materialization via `reasonable`

- `Cargo.toml`: `reasonable = "0.4"` (0.4.1, 2026-05-10 publish).
  Pulls in `datafrog 2`, `disjoint-sets 0.4`, `roaring 0.5`,
  `rio_api / rio_turtle 0.7`, `farmhash 1`, `serde_sexpr 0.1`. The
  oxrdf version requirement (`^0.3.3`) matches our existing pin so
  triple types unify cleanly across the codebase.
- `src/inference/reasonable.rs`: full implementation of
  `pgrdf.materialize(graph_id BIGINT) → JSONB`. Flow:
  1. Idempotency — wipe every `is_inferred = TRUE` row in this
     graph via a single `DELETE … RETURNING 1` + count aggregate.
  2. Bulk-rehydrate base triples — one `SELECT … JOIN
     _pgrdf_dictionary × 3 + LEFT JOIN dt` round-trip builds
     `Vec<oxrdf::Triple>` directly. Datatype + language tag both
     carried; blank-node subjects + object IRIs / literals all
     supported.
  3. `Reasoner::new().load_triples(base).reason()` — OWL 2 RL
     forward chain.
  4. Set-diff against the base `HashSet<Triple>` to isolate
     entailed-but-not-asserted triples (filters out the base AND
     the OWL 2 RL axiomatic triples that match the input).
  5. Each new triple's terms intern via `put_term_full` (shmem-
     warm path from Phase 3 step 1) and INSERT with
     `is_inferred = TRUE`.
- Stats JSONB:
  `base_triples / inferred_triples_written /
  previous_inferred_dropped / reasoner_errors[] / elapsed_ms`.
- 3 new pgrx tests:
  `materialize_subclass_chain` (verifies
  `?a a :Engineer ⇒ ?a a :Person`),
  `materialize_is_idempotent` (two calls produce the same row count
  and drop the prior output),
  `materialize_pure_data_preserves_input` (base survives).
- New regression `60-materialize-owl-rl.sql` covers:
  - 2-hop subClassOf chain
    (`Engineer ⊑ Person ⊑ Agent` plus assertions →
     `alice a Person`, `alice a Agent`, `bob a Agent`).
  - Idempotence — `previous_inferred_dropped` equals the prior
    `inferred_triples_written`.
  - `owl:inverseOf` entailment
    (`:owner :owns :store` ⇒ `:store :ownedBy :owner`).
- Test bar: **88 → 91 pgrx + 28 → 29 regression**, green.

Scope honest. `reasonable` implements OWL 2 RL only. OWL 2 EL/QL
and arbitrary Datalog beyond RL are NOT covered. Pre-existing
ERRATA E-002 (LLD §2 → "reasonable Datalog reasoner") remains
correct; v0.3 LLD §5.2 already restricts the slice to RL.

### Phase 3 step 3 — bulk-ingest prepared INSERT (LLD §4.3 phase A)

- `src/storage/loader.rs`: the batch-flush SQL is a constant
  string; the per-backend `plan_cache` from Phase 3 step 2 stashes
  the prepared `INSERT … SELECT FROM unnest(…)` exactly once.
  Every flush across every load in the same backend reuses the
  cached `OwnedPreparedStatement`. Saves one parse+plan per batch
  (typically ~100–500 µs each on PG 17).
- `flush_batch` now runs inside `Spi::connect_mut(|c| {…})` and
  binds arguments as `Vec<DatumWithOid>` (three `INT8ARRAY` + one
  `INT8`), driving the cached plan via `client.update`.
- `tests/regression/sql/52-bulk-ingest-perf.sql` + new
  `fixtures/regression/synth-10k.{sh,ttl}` fixture (10 000
  triples = ≥ 10 flushes per load). Asserts:
  - Load 1 produces exactly one `plan_cache_misses` += 1 and one
    `plan_cache_inserts` += 1 (the cold prepare).
  - Two loads together produce ≥ 19 `plan_cache_hits` (the other
    flushes all hit).
  - Load 3 produces zero new inserts (cache fully warm).
  Hand-computed; never `ACCEPT=1` baselined.
- Test bar: **88 → 88 pgrx + 27 → 28 regression**, green.

**Honest framing — wall-clock target.** LLD §4.3 calls for *"ingest
throughput at least 2× the current batched-INSERT baseline"*. The
prepared-INSERT cache saves a few hundred µs per batch but the
batched-INSERT executor walk (`SELECT … FROM unnest(…)` per-tuple
construction + partition routing) still dominates per-batch wall
clock. Observed: synth-100 unchanged within noise; synth-10k
~85 ms steady-state on both before/after.

To hit the 2× bar the next slice has to bypass the executor —
either `pg_sys::heap_multi_insert` directly (skips per-tuple
projection and the partition tuple-router uses heap-bulk paths) or
the proper `BeginCopyFrom` + binary COPY-protocol feed. Both are
FFI-heavy. Tracked as **Phase 3 step 3b (deferred)** — does NOT
block Phase 4 (Inference) start.

### Phase 3 step 2 — prepared-plan cache (LLD §4.2)

- `src/query/plan_cache.rs`: per-backend `thread_local!`
  `HashMap<String, OwnedPreparedStatement>`. Cumulative
  `plan_cache_hits / misses / inserts` counters live in shmem
  (alongside the dict-cache counters) so a multi-backend view is
  available through `pgrdf.stats()`. Per-backend cache size is
  surfaced as `plan_cache_local_size`.
- `src/query/executor.rs`: every dict-id constant that used to be
  inlined into the dynamic SQL (`bind_subject/predicate/object`,
  `expr_to_id_sql`, `translate_in`, `numeric_datatype_id_list`,
  …) now becomes a `$N` positional placeholder. A thread-local
  `PARAM_BUF` collects the resolved i64s in declaration order;
  `translate()` snapshots it into `ExecPlan { sql, params }`.
  The SQL string itself is the canonical cache key — identical
  algebra shape → identical key by construction.
- `execute()` consults the per-backend cache before paying for
  parse + plan. Miss path uses `client.prepare(sql, &[INT8OID; n])`
  followed by `.keep()` to promote to `'static`-lifetime
  `OwnedPreparedStatement`. Hit path reuses the stashed statement
  with a fresh `Vec<DatumWithOid>` built from `plan.params`.
- `pgrdf.plan_cache_clear() -> bigint` returns the number of
  plans dropped from THIS backend's cache. Useful for diagnostics
  and tear-down; production workloads never need it.
- `pgrdf.stats()` JSONB now includes four new fields:
  `plan_cache_hits`, `plan_cache_misses`, `plan_cache_inserts`,
  `plan_cache_local_size`.
- **Perf regression**: new `tests/regression/sql/51-plan-cache.sql`
  exercises three blocks:
  - 5 identical queries → 1 miss + 4 hits (single shape, repeat).
  - 2 queries with same parametric shape but different IRI
    constants → 1 miss + 1 hit (parameterisation works — SQL
    string stays byte-identical despite constant change).
  - 1 structurally distinct query (FILTER added) → 1 miss + 0 hits.
  Also asserts `plan_cache_clear() >= 2` and
  `plan_cache_local_size == 0` post-clear. **All deltas
  hand-computed**.
- 2 new pgrx integration tests: `plan_cache_repeats_hit`,
  `plan_cache_clear_returns_count`.
- Test bar: **86 → 88 pgrx + 26 → 27 regression** tests, green.

### Phase 3 step 1 — shmem dict cache (LLD §4.1)

- `src/storage/shmem_cache.rs`: process-wide, cross-backend
  dictionary cache backed by `pgrx::PgLwLock<[Slot; 16_384]>` (~512
  KiB shmem). Slot carries a u128 fingerprint (two SipHash variants)
  plus dict_id + generation. Open-addressed with 8-deep linear
  probing; canonical-slot eviction on full streak.
- `_PG_init` gates `pg_shmem_init!` on
  `pg_sys::process_shared_preload_libraries_in_progress` so hook
  registration only happens in the postmaster scan. Lazy-loaded
  backends short-circuit every lookup and fall back to the per-call
  HashMap path. Compose already sets
  `shared_preload_libraries=pgrdf`; the pgrx-test harness's
  `postgresql_conf_options` now does too.
- `put_term_full` consults shmem before SELECT. On both SELECT-hit
  and INSERT it **stages** the (key → dict_id) mapping in a
  per-backend pending list; pgrx's `register_xact_callback` flushes
  to shmem on `XACT_EVENT_COMMIT` and discards on
  `XACT_EVENT_ABORT`. The deferred publish keeps shmem in lockstep
  with the dictionary table — a rolled-back INSERT never leaves an
  orphan id in the cache.
- `pgrdf.stats() -> JSONB` exposes cumulative shmem counters
  (`shmem_ready`, `shmem_slots`, `shmem_hits`, `shmem_misses`,
  `shmem_inserts`, `shmem_evictions`) — observability target for
  the LLD §4.1 acceptance criterion.
- `pgrdf.shmem_reset() -> void` atomically bumps a shmem
  generation counter so every previously-cached entry reads as
  cold on next lookup. Required after
  `DROP EXTENSION pgrdf; CREATE EXTENSION pgrdf;` (the dict id
  space resets but the cache survives) — also useful in regression
  setup. Slot generation is part of the slot record; mismatch on
  lookup is silent and equivalent to a miss.
- Per-call `load_turtle_verbose` stats gain `shmem_cache_hits` —
  the count of term references that fell through the per-call
  HashMap and were satisfied by the cross-backend shmem cache
  without touching `_pgrdf_dictionary`. Loader snapshots the
  global HITS counter around each `put_term_full` call to
  attribute hits.
- **Perf regression**: new `tests/regression/sql/50-shmem-dict-cache.sql`
  loads `fixtures/regression/synth-100.ttl` (100 triples, 115
  distinct terms) three times into successive graphs and asserts:
  load 1 has 115 db calls + 0 shmem hits; loads 2–3 have 0 db
  calls + 115 shmem hits each. Cumulative counter deltas asserted
  via `pgrdf.stats()` ≥ 230 shmem hits / ≥ 115 inserts vs pre-test
  snapshot. **All expected values hand-computed**, never
  autobaselined.
- 6 new pgrx integration tests cover the cache primitive
  (`shmem_ready_in_test`, `shmem_roundtrip_via_committed`,
  `shmem_disambiguates_keys`, `shmem_datatype_in_key`,
  `shmem_counters_advance`, `shmem_reset_invalidates_slots`).
- Test bar: **85 → 86 pgrx + 25 → 26 regression** tests, all green.

### LLD v0.3 — Refocus

- [`specs/SPEC.pgRDF.LLD.v0.3.md`](specs/SPEC.pgRDF.LLD.v0.3.md)
  shipped. Supersedes v0.2 at the contract level; v0.2 LLD is now
  historical (still referenced for §4.1–4.3 internals that haven't
  changed). INSTALL spec (`SPEC.pgRDF.INSTALL.v0.2.md`) unchanged.
- The v0.3 LLD acknowledges Phase 3 steps 1–12 (SPARQL surface)
  as substantively complete (BGP + FILTER + OPTIONAL + UNION +
  MINUS + DISTINCT/LIMIT/OFFSET/ORDER BY + aggregates + HAVING +
  GROUP_CONCAT/SAMPLE + expression richness + BIND + multi-triple
  MINUS + ASK) and re-bins forward work:
  - **Phase 3 (NEW): Storage Performance** — shmem dict cache
    (v0.2 LLD §4.1), prepared-plan cache (§4.2), COPY BINARY
    ingestion (§4.3). The single biggest remaining LLD gap.
  - **Phase 4**: Inference engine (OWL 2 RL via `reasonable`).
  - **Phase 5**: Validation engine (SHACL via `shacl_validation`).
  - **Phase 6**: W3C SPARQL/SHACL conformance + LUBM + release
    artifacts + CI matrix.
- v0.4 deferral list (none block Phase 3):
  - GRAPH `{ … }` named-graph clause (needs storage schema work)
  - VALUES inline tables
  - Property paths beyond simple sequence (`*`, `+`, `?`, `^`)
  - Multi-triple OPTIONAL (needs LATERAL refactor)
  - CONSTRUCT, DESCRIBE (different output shape)
  - Aggregates over UNION
  - BIND output referenced in later FILTER / BGP
  - Type-aware ORDER BY / MIN / MAX
- v0.3 also formalises the **empirical-verification rule**: new
  regression fixtures hand-compute their expected output; no
  `ACCEPT=1` autobaselining of new query coverage.
- Per-call `pgrdf.load_turtle_verbose` stats will gain
  `shmem_cache_hits` + `plan_cache_hits` in Phase 3 to support
  perf regression tests on the synth-100 fixture.
- Cross-references updated: `README.md`, `docs/README.md`,
  `docs/10-roadmap.md`, `specs/ERRATA.v0.2.md`.

### Phase 3 step 12 — ASK query form

- `pgrdf.sparql('ASK { … }')` now works. Returns a single JSONB
  row `{"_ask": "true"}` or `{"_ask": "false"}` reflecting whether
  the pattern has at least one solution.
- The pattern walk reuses `parse_select` so ASK transparently
  supports FILTER, OPTIONAL, UNION, MINUS, and any combination
  the SELECT executor handles. `build_ask_probe_sql` emits a
  `SELECT 1 FROM …` probe wrapped in `EXISTS(…)` in the outer
  query.
- `pgrdf.sparql_parse` now reports `form: "ASK"` with the same
  `bgp_pattern_count` / `bgp_patterns` / `unsupported_algebra`
  shape it gives SELECT, rather than `supported: false`.
- 2 new pg_tests: ASK match/no-match, ASK with FILTER.
- `tests/regression/sql/44-sparql-ask.sql` covers 6 query shapes:
  match, no-match, FILTER pass/fail, ASK with OPTIONAL, ASK with
  UNION.
- `README.md` pills: 77+24 → 79+25.
- `CONSTRUCT` and `DESCRIBE` change the output shape (triples
  instead of solutions) and are **deferred to v0.4**.

Test bar:
  pg_test:    79 passed; 0 failed  (was 77)
  regression: 25 passed; 0 failed  (was 24)

### Phase 3 step 11 — Multi-triple MINUS

- `MINUS { ?s :p ?o . ?s :q ?r . … }` now accepts arbitrary
  N-triple sub-patterns. `ParsedSelect.minuses` changed from
  `Vec<TriplePattern>` to `Vec<Vec<TriplePattern>>`; same for
  `UnionBranch.minuses`.
- `translate_minus` rewrites to emit one `NOT EXISTS (SELECT 1
  FROM q_min_1, q_min_2, … WHERE …)` per MINUS block. Each
  triple in the sub-pattern gets its own quad alias; shared
  variables with the outer query AND shared-inside-the-MINUS
  emit equality predicates automatically via `pattern_clauses`.
- SPARQL spec's "no shared variables → MINUS is identity" rule
  still applies: the translator unions all variables in the
  sub-pattern, checks intersection with outer anchors, and
  elides the block if empty.
- Single-triple MINUS continues to work (it's the
  `triples.len() == 1` case of the multi-triple path).
- 1 new pg_test: `sparql_minus_multi_triple` (alice+eve have
  both mbox+age → dropped, bob/carol/dave survive).
- `tests/regression/sql/43-sparql-minus-multi.sql` covers 4
  query shapes: 2-triple AND, 3-triple AND, chained multi-triple
  MINUSes, single-triple back-compat.
- `README.md` pills: 76+23 → 77+24.
- Multi-triple OPTIONAL is **deferred to v0.4** — the LATERAL
  refactor it needs is bigger than the MINUS rewrite (OPTIONAL
  has to EXPOSE its new bindings to the outer query, while MINUS
  is just a boolean check). Workaround: chain single-triple
  OPTIONALs.

Test bar:
  pg_test:    77 passed; 0 failed  (was 76)
  regression: 24 passed; 0 failed  (was 23)

### Phase 3 step 10 — BIND (non-aggregate)

- `BIND(expr AS ?v)` (and the equivalent `SELECT (expr AS ?v)` form
  on non-aggregate expressions) now adds a virtual column. `walk_select`'s
  Extend handler falls through to a `BindSpec` when the expression
  isn't a Variable-rename of an existing aggregate.
- Projection in `build_single_branch_outer` checks `ps.binds` before
  falling back to the BGP anchor lookup, emitting the translated
  expression with the BIND var as the column alias.
- `translate_bind_expression` covers Literal / NamedNode / Variable,
  STR / LANG / DATATYPE / UCASE / LCASE, arithmetic, STRLEN, and
  `CONCAT(?a, ?b, …)` via Postgres `concat`. All values surface as
  text in the JSONB row.
- Today's restriction: a BIND output variable referenced in a later
  FILTER / BGP isn't yet supported (would need expression substitution
  during translation). Filtering on BIND output is Phase 3 backlog.
- 3 new pg_tests + `tests/regression/sql/42-sparql-bind.sql`
  (6 query shapes: UCASE, arithmetic, CONCAT, literal-constant,
  STRLEN, two-BINDs in one query).
- `README.md` pills: 73+22 → 76+23.

Test bar:
  pg_test:    76 passed; 0 failed  (was 73)
  regression: 23 passed; 0 failed  (was 22)

### Phase 3 step 9 — Expression richness in FILTER

- `pgrdf.sparql` FILTER translator gains a much wider expression
  surface:
  - **Arithmetic**: `?a + ?b`, `?a - ?b`, `?a * ?b`, `?a / ?b`
    (with NULLIF-guarded divide-by-zero), unary `-`, unary `+`.
    All built on top of `expr_to_numeric_sql`'s CASE-cast so
    non-numeric operands NULL-propagate instead of erroring.
  - **String predicates**: `CONTAINS`, `STRSTARTS`, `STRENDS` —
    Postgres `strpos`, `left`, `right` against `lexical_value`.
  - **String-valued functions** usable inside other expressions:
    `LANG(?v)`, `DATATYPE(?v)`, `UCASE(?v)`, `LCASE(?v)`,
    `STR(?v)` (was passthrough, formalised). LANG / DATATYPE use
    chained dict lookups (datatype IRI ids → IRI lexical).
  - **`STRLEN(?v)`** is numeric-valued, plugged into
    `expr_to_numeric_sql`.
- Equality fallback: when either side of `=` / `sameTerm` is a
  function call (or otherwise can't resolve to a dict id), the
  translator falls back to lexical comparison. Lets `STR(?v) =
  "x"`, `LANG(?v) = "en"`, `DATATYPE(?v) = xsd:integer` etc.
  translate cleanly.
- `expr_to_lexical_sql` learned to emit a SQL string for
  `NamedNode` (the IRI's lexical form), making the fallback work
  for IRI constants on the right of equality.
- 6 new pg_tests: arithmetic add, mul/div, STRLEN, CONTAINS/
  STRSTARTS/STRENDS, LANG/DATATYPE equality, UCASE/LCASE case
  folding.
- `tests/regression/sql/41-sparql-expressions.sql` covers 11
  query shapes (4 arithmetic, STRLEN, 4 string predicates,
  LANG, DATATYPE).
- `README.md` pills: 67+21 → 73+22.

Test bar:
  pg_test:    73 passed; 0 failed  (was 67)
  regression: 22 passed; 0 failed  (was 21)

### Phase 3 step 8 — HAVING + GROUP_CONCAT + SAMPLE

- `pgrdf.sparql` now translates `HAVING (expr)` clauses on
  aggregate queries. `parse_select` post-processes the collected
  filters: any filter referencing an aggregate output variable
  becomes a HAVING predicate (the rest stay as WHERE).
- `translate_filter_with_aggregates` is the HAVING-aware translator:
  variable references resolve to (a) the underlying SQL aggregate
  function for aggregate-output vars, (b) the group-by expression
  for group vars, (c) literals are used directly. Supports
  identity, numeric ordering (`<`/`>`/`<=`/`>=`), boolean composition.
- `GROUP_CONCAT(?v [; SEPARATOR = "…"])` → Postgres `STRING_AGG`,
  default separator a single space per SPARQL spec.
- `SAMPLE(?v)` → `MIN(lexical_value)` as a deterministic surrogate
  (SPARQL spec says "implementation-defined element"; MIN is one
  conformant choice).
- 4 new pg_tests: HAVING with COUNT, HAVING with SUM, GROUP_CONCAT
  with custom separator, SAMPLE.
- `tests/regression/sql/40-sparql-having.sql` covers 9 query
  shapes (HAVING > N, HAVING = 1, HAVING composite, GROUP_CONCAT
  custom + default separator, SAMPLE, SUM-HAVING on non-numeric
  strings — demonstrates the numeric-awareness rule — and
  SUM-HAVING on real numeric data across two graphs).
- `README.md` pills: 63+20 → 67+21.

Test bar:
  pg_test:    67 passed; 0 failed  (was 63)
  regression: 21 passed; 0 failed  (was 20)

### Phase 3 step 7 — Aggregates + GROUP BY

- `pgrdf.sparql` handles SPARQL aggregates with or without
  `GROUP BY`:
  - `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT ?v)`.
  - `SUM(?v)`, `AVG(?v)` — numeric-aware via the same XSD-numeric
    CASE cast as FILTER ordering. Non-numeric values contribute
    `NULL` (skipped by SUM/AVG per SQL semantics, no Postgres
    cast error).
  - `MIN(?v)`, `MAX(?v)` — lexicographic on the term's
    `lexical_value`. Type-aware MIN/MAX queued.
- `GROUP BY ?vars` translates to SQL `GROUP BY` using the same
  dict-lookup expressions that drive the SELECT clause. Multiple
  aggregates per group supported.
- Aggregate output values come back as **JSON strings** in the
  `pgrdf.sparql` row, consistent with the rest of the surface.
  Callers cast with `(j ->> 'n')::int`/`::numeric` etc.
- Algebra layout: spargebra lowers `SELECT (EXPR AS ?v)` to
  `Project → Extend → Group → BGP`. `walk_select` now handles
  Extend (renames the synthesised `$agg_N` to `?v`) and Group
  (captures group_vars + AggregateSpecs). Walk order: descend
  into inner first so Group's aggregates are populated before
  Extend tries to rename them.
- Parser walks `GraphPattern::Group` and `GraphPattern::Extend`
  rather than flagging them; tests adjusted.
- 7 new pg_tests: COUNT(*), COUNT(DISTINCT), GROUP BY counting,
  SUM numeric, AVG numeric, MIN/MAX lex, multiple aggregates
  per group.
- `tests/regression/sql/39-sparql-aggregates.sql` covers 10
  query shapes: count_all, count_o, count_distinct, sum_age,
  avg_age (rounded), min/max names, group_by predicates,
  multi-aggregate, ORDER-BY-aggregate + LIMIT.
- `README.md` pills: 56+19 → 63+20; SPARQL pill adds AGGREGATES.
- `guide/03-querying.md` gains a full "Aggregates and GROUP BY"
  section covering the JSON-string output rule, the SUM/AVG
  numeric-awareness rule, the MIN/MAX lex caveat, and the
  HAVING/GROUP_CONCAT/BIND restrictions.

Today's restrictions:
- HAVING not yet translated — post-process with regular SQL.
- BIND outside aggregate aliasing not supported.
- Aggregates on top of UNION not supported (panic with clear msg).
- `GROUP_CONCAT` / `SAMPLE` not supported.

Test bar:
  pg_test:    63 passed; 0 failed  (was 56)
  regression: 20 passed; 0 failed  (was 19)

### Phase 3 step 6 — MINUS

- `pgrdf.sparql` handles `MINUS { ?s :p ?o }` and chained MINUSes.
  Each block becomes a `WHERE NOT EXISTS (SELECT 1 FROM
  pgrdf._pgrdf_quads qMIN_K WHERE …)` sub-SELECT, keyed on shared
  variables between the outer query and the MINUS triple.
- Per SPARQL spec, MINUS with no shared variables is a no-op —
  the translator detects this at translation time and emits no
  SQL for that block (different from OPTIONAL, which always
  emits a LEFT JOIN).
- Restriction: each MINUS block must be a single triple pattern
  (mirrors OPTIONAL's current restriction).
- Inside UNION branches, MINUS works the same way (scoped to the
  branch's anchor map).
- 4 new pg_tests: basic MINUS, no-shared-vars no-op, chained
  MINUSes, MINUS + outer FILTER + REGEX.
- Parser walks `GraphPattern::Minus` rather than flagging it.
  New parser pg_test for the new state + a Path-still-flagged
  test taking its place (transitive `:a*`, not simple `:a/:b`
  which spargebra desugars to BGP).
- `tests/regression/sql/38-sparql-minus.sql` covers 6 query
  shapes (basic, no-op, chained, with-FILTER, ordered survivor,
  shared-non-subject-var).
- `30-sparql-parse.sql` baseline updated: MINUS supported, Path
  (quantified) is the new unsupported representative.
- `README.md` pills: 51+18 → 56+19, SPARQL pill adds MINUS.
- `guide/03-querying.md` gains a MINUS section covering the
  shared-vars-vs-no-op rule and the OPTIONAL-asymmetry note.

Test bar:
  pg_test:    56 passed; 0 failed  (was 51)
  regression: 19 passed; 0 failed  (was 18)

### Phase 3 step 5 — UNION

- `pgrdf.sparql` handles `{ A } UNION { B }` and chained
  `A UNION B UNION C`. Each branch is its own complete sub-SELECT
  (own BGP / FILTERs / OPTIONALs / per-branch dict-id anchors).
  Branches are combined with SQL `UNION ALL`; the outer SELECT
  layers `DISTINCT` / `ORDER BY` / `LIMIT` / `OFFSET`.
- Variables bound in only some branches come back as `null` from
  the other branches (each branch SELECTs `NULL::TEXT` for vars
  it doesn't bind, so row shapes line up across `UNION ALL`).
- ORDER BY on UNION may only reference projected variables — the
  outer SELECT can't see branch-local alias columns. Executor
  panics with a clear message otherwise.
- Refactor: extracted `build_from_and_where` (shared by both the
  single-branch and per-UNION-branch paths) + `build_branch_sql`
  + `build_union_sql`. The original `build_bgp_sql` is now a
  dispatcher over `ps.union_branches.is_empty()`.
- 5 new pg_tests: basic UNION over same var, different-var
  UNION with NULL pad, three-way chain, UNION + DISTINCT,
  UNION + ORDER BY + LIMIT.
- Parser walks `GraphPattern::Union` rather than flagging it.
  New parser pg_test for the new state + a new MINUS-still-flagged
  test taking its place.
- `tests/regression/sql/37-sparql-union.sql` covers 9 query shapes
  (basic, DISTINCT, different-var, two NULL-discriminator checks,
  three-way chain, ORDER BY first, LIMIT, branch-local FILTER).
- `30-sparql-parse.sql` baseline refreshed: UNION supported,
  MINUS now the unsupported representative.
- `README.md` pills: 45+17 → 51+18, SPARQL pill adds UNION.
- `guide/03-querying.md` gains a full UNION section covering
  the cross-branch null padding, ORDER-BY-must-be-projected
  rule, and the no-nesting restriction for this slice.

Test bar:
  pg_test:    51 passed; 0 failed  (was 45)
  regression: 18 passed; 0 failed  (was 17)

### Phase 3 step 4 — OPTIONAL (LeftJoin) translation

- `pgrdf.sparql` now handles `OPTIONAL { ?s :p ?o }`. Each OPTIONAL
  block emits a `LEFT JOIN pgrdf._pgrdf_quads qOPT_i ON (…)`. Variables
  introduced inside an OPTIONAL surface as NULL (JSONB `null`) when
  the LEFT JOIN didn't match.
- `OPTIONAL { … FILTER(...) }` — the inner filter lands in the LEFT
  JOIN's ON clause, so rejected matches keep the optional variable
  NULL rather than pruning the whole row.
- Multiple chained OPTIONALs each get their own LEFT JOIN, in
  left-to-right order. Per SPARQL semantics, variables introduced
  by one OPTIONAL aren't visible to another OPTIONAL's ON clause.
- `BOUND(?v)` translation tightened: now emits `qN.col IS NOT NULL`
  regardless of whether ?v is mandatory or OPTIONAL. Mandatory
  anchors are non-NULL so it's trivially TRUE there; OPTIONAL anchors
  can be NULL so this is the spec-correct semantics.
- Internal refactor: `build_bgp_sql` switched from comma-style FROM
  (`q1, q2, q3 WHERE …`) to explicit JOIN syntax
  (`q1 INNER JOIN q2 ON … INNER JOIN q3 ON …`). Same semantics for
  INNER joins; necessary for OPTIONAL's LEFT JOIN to compose.
- Parser updated: `LeftJoin` no longer flagged in
  `unsupported_algebra` — the parser walks both arms.
- 4 new pg_tests: simple OPTIONAL, OPTIONAL with inner FILTER,
  multiple chained OPTIONALs, outer FILTER(BOUND) pruning.
- `tests/regression/sql/36-sparql-optional.sql` covers 8 query
  shapes (LEFT JOIN counts, NULL/not-NULL discrimination, inner
  filter, multi-chain, outer BOUND prune, OPTIONAL + ORDER BY).
- `30-sparql-parse.sql` baseline updated: OPTIONAL no longer
  flagged; new UNION assertion replaces it.
- `README.md` pills: 40+16 → 45+17; SPARQL pill adds OPTIONAL.
- `guide/03-querying.md` gains a full OPTIONAL section covering
  inner-FILTER semantics, chained OPTIONALs, BOUND-pruning, and
  the single-triple restriction for this slice.

Test bar:
  pg_test:    45 passed; 0 failed  (was 40)
  regression: 17 passed; 0 failed  (was 16)

### Phase 3 step 3 — Solution modifiers (DISTINCT / LIMIT / OFFSET / ORDER BY)

- The four classic SPARQL solution modifiers now land in the
  generated SQL instead of being silently stripped from the AST:
  - `SELECT DISTINCT ?vars` → `SELECT DISTINCT` in SQL.
  - `SELECT REDUCED ?vars` → also `SELECT DISTINCT` (REDUCED is a
    "dups may or may not be removed" hint per spec; over-approxing
    with DISTINCT is conformant).
  - `LIMIT N` / `OFFSET N` → `LIMIT N` / `OFFSET N`.
  - `ORDER BY ?var`, `ORDER BY ASC(?var)`, `ORDER BY DESC(?var)`,
    multi-key — sorted by the term's `lexical_value` with
    `NULLS LAST`. If the var is projected the existing column is
    reused; otherwise an extra hidden column is appended and ORDER
    BY references it by ordinal (so the JSONB output stays clean).
- ORDER BY today is **lexicographic on string form**, not SPARQL's
  full type-aware ordering. Numeric ordering through ORDER BY lands
  in step 4+; for now use FILTER for numeric range + post-SQL
  `ORDER BY (sparql->>'n')::numeric`.
- Refactor: `unwrap_select` → `parse_select` returning a richer
  `ParsedSelect` struct (projected, bgp, filters, distinct,
  order_by, limit, offset). Single recursive walk replaces the
  old two-pass extract_bgp_and_filters / unwrap_select split.
- 6 new pg_tests: distinct dedups, LIMIT caps, OFFSET skips,
  ORDER BY ASC + DESC, DISTINCT + ORDER BY interaction.
- `tests/regression/sql/35-sparql-modifiers.sql` covers 10 query
  shapes (raw count, DISTINCT, REDUCED, LIMIT 2, ORDER ASC first,
  ORDER DESC first, OFFSET 3 LIMIT 2 window, DISTINCT + ORDER,
  ORDER BY on non-projected var, LIMIT 0).
- `README.md` pills: 34+15 → 40+16, SPARQL pill adds
  DISTINCT/ORDER/LIMIT.
- `guide/03-querying.md` gains a full "Solution modifiers" section
  covering ORDER BY's lexicographic-vs-type-aware caveat, the
  DISTINCT-with-non-projected-order-by panic case, and a worked
  example.

Test bar:
  pg_test:    40 passed; 0 failed  (was 34)
  regression: 16 passed; 0 failed  (was 15)

### Phase 3 step 2 — FILTER numeric ordering + REGEX + IN

- `pgrdf.sparql` FILTER translator gains three new shapes:
  - **Numeric ordering** (`<`, `>`, `<=`, `>=`): operand resolves to
    `NUMERIC` via a CASE-guarded subselect on `_pgrdf_dictionary`.
    Only XSD numeric datatypes (integer, decimal, double, float,
    sized + unsigned + constraint subtypes — 16 IRIs total)
    contribute; everything else compares NULL → row dropped. This
    matches SPARQL's "type error → unbound" semantics without ever
    raising a Postgres cast error.
  - **`REGEX(?v, "pat" [, "flags"])`**: Postgres `~` (case-sensitive)
    or `~*` (with `i` flag) against the term's `lexical_value`.
    Pattern + flags are SPARQL literals at translation time;
    single quotes in the pattern are escaped. `STR(?v)` inside
    REGEX is a passthrough.
  - **`?term IN (e1, e2, …)`**: dict-id set membership.
- 6 new pg_tests: numeric `>` / range / non-numeric drop, regex
  case-sensitive / case-insensitive with STR(), and IN.
- `tests/regression/sql/34-sparql-filter-advanced.sql` covers 10
  query shapes (numeric `>`, range, `<` with non-numeric mixed in,
  `>= 0` over a typed-decimal row, regex `^A`, regex `ar` case-i,
  regex+STR wrap, IN over IRIs, IN over a literal, and a cross-BGP
  composition).
- `README.md` pills: tests 28+14 → 34+15, SPARQL pill adds REGEX.
- `guide/03-querying.md` gains full sections for numeric ordering,
  REGEX (with the POSIX-vs-PCRE caveat), and IN. Capability matrix
  refreshed.

Test bar:
  pg_test:    34 passed; 0 failed  (was 28)
  regression: 15 passed; 0 failed  (was 14)

### Phase 3 step 1 — FILTER expressions over BGPs

- `pgrdf.sparql` now walks `GraphPattern::Filter { expr, inner }`
  and translates a useful subset of `Expression` into SQL WHERE
  predicates appended after the BGP joins:
  - **Identity**: `=`, `!=`, `sameTerm` — both operands resolved to
    dictionary ids, compared as BIGINT. Sound because the dictionary
    deduplicates by `(term_type, lexical, datatype, language)`.
  - **Boolean**: `&&`, `||`, `!`.
  - **Term-type predicates**: `isIRI`, `isLiteral`, `isBlank` — emit
    a correlated subselect on `_pgrdf_dictionary.term_type`.
  - **`BOUND`**: trivially `TRUE` for any anchored BGP variable.
  - Untranslatable shapes (numeric `<`/`>`/`<=`/`>=`, `regex`, `str`,
    `lang`, arithmetic, `IN`, `EXISTS`) panic with a clear message
    rather than silently dropping the filter.
- `pgrdf.sparql_parse` no longer flags `Filter` in
  `unsupported_algebra` — it walks into the inner BGP. OPTIONAL,
  UNION, MINUS, Group, Path, Values, Extend (BIND), Service still
  flagged.
- 6 new pg_tests: literal equality, `!=`, `isIRI`, boolean AND
  composition, var-equals-var (self-loop), `BOUND` trivially-true.
- 1 new parser pg_test: OPTIONAL replaces the FILTER-flagged baseline.
- `tests/regression/sql/33-sparql-filter.sql` covers 9 query shapes
  end-to-end (literal eq, neg, isIRI, isLiteral, self-loop,
  boolean AND, negated isIRI, BOUND, unknown-literal-zero-rows).
- `tests/regression/sql/30-sparql-parse.sql` baseline updated: Filter
  no longer reported as unsupported; new OPTIONAL assertion added.
- `guide/03-querying.md` adds a full FILTER section with examples,
  including the `=` ↔ sameTerm-vs-value-equality caveat and how
  filters interact with multi-pattern BGPs.
- `README.md`: status pill → `phase 3 start`, test pill 21+13 → 28+14,
  SPARQL pill `SELECT/BGP` → `SELECT/BGP/FILTER`.

Test bar:
  pg_test:        28 passed; 0 failed  (was 21)
  regression:     14 passed; 0 failed  (was 13)

### Phase 2.2 step 8 — Node.js + Go client guides

- `guide/clients/typescript.md` — `pg` (node-postgres) + `postgres.js`
  + `pg-cursor` streaming + strongly-typed binding helpers. Covers
  `load_turtle`, `parse_turtle`, `load_turtle_verbose`, and the
  full `pgrdf.sparql` JSONB result shape with type narrowing.
- `guide/clients/go.md` — `pgx` v5 + `pgxpool` + sqlc integration
  + bulk-ingest pattern + the constant-time graph-drop idiom.
- `guide/README.md` index lists both new client pages.
- `README.md` clients section now points at all 4 supported clients
  (Python, Rust, TypeScript, Go).

### Phase 2.2 step 7 — User guide for SPARQL surface

- New `guide/03-querying.md`: full walkthrough of `pgrdf.sparql`
  (single + multi-pattern BGPs, constants in any position, JSONB
  output, combining with regular SQL, `pgrdf.sparql_parse` for
  introspection) plus what works / doesn't / why, and a worked
  example of the SQL translation.
- `README.md` promoted the SPARQL surface from "coming soon" to a
  live code example, bumped the test pill from 9+10 to 21+13,
  added a SPARQL pill, refreshed the status row.
- `guide/README.md` index entry for `03-querying.md`.

### Phase 2.2 step 6 — Multi-pattern BGP joins

- `pgrdf.sparql` now handles N-pattern Basic Graph Patterns. Each
  pattern becomes a `_pgrdf_quads qN` clause; shared variables across
  patterns are tracked by first-occurrence anchors and emit equality
  predicates (`q2.subject_id = q1.subject_id`) that fold into INNER
  joins.
- 2 new pg_tests: two-pattern shared-subject BGP (Alice + Carol have
  both `foaf:name` and `foaf:mbox`, Bob doesn't), three-pattern chain
  following `foaf:knows`.
- `tests/regression/sql/32-sparql-multipattern.sql` covers 5 shapes:
  shared-subject BGP, three-pattern chain, self-loop pattern (?s ?p ?s),
  bound-subject multi-pattern, and bound-predicate + bound-literal.

Test bar:
  pg_test:        21 passed; 0 failed  (was 19)
  regression:     13 passed; 0 failed  (was 12)

### Phase 2.2 step 5 — SPARQL execution: BGP → SQL

- `pgrdf.sparql(q TEXT) → SETOF JSONB` — first user-visible SPARQL
  surface. Parses via spargebra, translates a single Basic Graph
  Pattern into a dynamic SQL SELECT over `_pgrdf_quads` joined to
  `_pgrdf_dictionary`, returns one JSONB row per solution keyed by
  the projected variable names.

  ```sql
  SELECT * FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n WHERE { ?s foaf:name ?n }'
  );
  --  → {"s": "http://example.com/alice", "n": "Alice"}
  --  → {"s": "http://example.com/bob",   "n": "Bob"}
  ```

  Scope today (intentionally narrow — multi-pattern joins land in
  step 6):
  - SELECT only.
  - Exactly one BGP triple per query.
  - Constants in any position (subject IRI, predicate IRI, object
    IRI or literal). Unknown constants resolve to `-1` so the query
    correctly returns zero rows rather than erroring.
  - Variables in any position.
  - Distinct / Reduced / Slice / OrderBy wrappers are passed through.
- 4 new pg_tests covering all-three-vars BGP, bound-predicate filter,
  bound-subject filter, and unknown-predicate-returns-empty.
- `tests/regression/sql/31-sparql-bgp.sql` exercises 7 query shapes
  end-to-end through the compose Postgres.

Infrastructure:

- `compose/builder.Containerfile` rewritten with BuildKit cache
  mounts. The builder image dropped from 7.73 GB → 3.35 GB; cargo
  registry + target/ now live in build-scoped cache volumes that
  persist across rebuilds without bloating image layers.
- `Justfile build-ext` now invokes `DOCKER_BUILDKIT=1 docker build`
  so the `# syntax=docker/dockerfile:1.4` directive activates.
- `.dockerignore` excludes `target/`, `.target-linux/`,
  `compose/pg-data/`, `compose/extensions/lib|share`,
  `fixtures/ontologies/`, `.git/`. Build context dropped accordingly.

### Phase 2.2 step 4 — SPARQL parser surface

- `spargebra = "0.4"` (0.4.6 resolved). Pins `oxrdf = "=0.3.3"`, the
  same version oxttl 0.2.3 uses, so no graph split.
- New module `src/query/parser.rs`.
- `pgrdf.sparql_parse(q TEXT) -> JSONB` parses a SPARQL query via
  `spargebra::SparqlParser` and returns the high-level shape:
  - `form` — SELECT / CONSTRUCT / ASK / DESCRIBE
  - `variables` — projected vars (SELECT only)
  - `bgp_pattern_count`, `bgp_patterns` — BGP triples with
    s/p/o each rendered as `{var: …}`, `{iri: …}`, `{bnode: …}`,
    or `{literal: …, datatype/lang: …}`
  - `unsupported_algebra` — flags Filter / Union / OPTIONAL /
    Property paths / Aggregates / VALUES / SERVICE / etc., so
    callers see the AST has shape the translator doesn't yet
    cover.
- 5 new pg_tests covering basic SELECT, predicate-as-IRI BGP,
  two-pattern BGP, FILTER detection, and a syntax-error panic path.
- New regression `tests/regression/sql/30-sparql-parse.sql` asserts
  the JSONB extraction over 6 query forms.

### Phase 2.2 step 3 — Batched ingestion

(landed alongside docs split + README pills.)

- `src/storage/loader.rs`: per-call HashMap dict cache + buffered
  multi-row INSERTs via `unnest($1::bigint[], $2::bigint[], $3::bigint[])`.
  BATCH_SIZE = 1000. Reduces SPI calls from ~7/triple to roughly
  `distinct_terms + ceil(triples/1000)`.
- `pgrdf.load_turtle_verbose(path, graph_id, base_iri)` and the
  matching `pgrdf.parse_turtle_verbose(content, graph_id, base_iri)`
  return JSONB stats: `triples`, `dict_cache_hits`, `dict_db_calls`,
  `quad_batches`, `elapsed_ms`. Used to assert the cache is firing.
- `fixtures/regression/synth-100.sh` + `synth-100.ttl`: deterministic
  100-triple synthetic fixture (10 subjects × 5 predicates × 100
  objects). 115 distinct terms, 185 expected cache hits.
- `tests/regression/sql/25-bulk-ingest.sql` asserts exact stat values
  on the synth-100 fixture and verifies dict dedup across two graphs.
- One new pg_test (`parse_turtle_verbose_cache_fires`) asserts cache
  behavior at the Rust level.
- `serde_json = "1"` added as a direct dependency for the verbose UDFs.

### Phase 2.1 — Turtle ingest

- `pgrdf.load_turtle(path, graph_id, base_iri)` and
  `pgrdf.parse_turtle(content, graph_id, base_iri)` parse Turtle via
  `oxttl 0.2` and stream triples through the dictionary +
  partitioned hexastore. `base_iri` resolves relative IRIs like
  `<#>` (needed for W3C PROV).
- Internal `put_term_full(value, type, datatype_id, lang)` honours
  the full dictionary key with `IS NOT DISTINCT FROM` lookups so
  NULL datatype + language columns participate in dedup.
- Compose: read-only `./fixtures:/fixtures:ro,z` bind mount so the
  postgres process can reach test + ontology fixtures by path.
- 24 W3C / Apache Jena / ConceptKernel / ValueFlows ontologies fetch
  cleanly via `fixtures/ontologies.sh`; `tests/perf/smoke-ontologies.sh`
  loads each one through `pgrdf.load_turtle` and prints triple
  counts. 17,134 triples across the set on the 2026-05-13 fetch.
- Four checked-in regression fixtures (`typed-literals.ttl`,
  `lang-tags.ttl`, `blank-nodes.ttl`, `rdf-list.ttl`) under
  `fixtures/regression/` exercise XSD datatypes, language tags,
  blank-node dedup, and `rdf:List` desugaring. All assertions are
  scoped strictly by graph_id so prior smoke loads don't pollute
  results.
- `workflow.ttl` excluded from the iteration set: source uses
  `<ckp://Name:v0.1>` IRI form (colon in path segment, not RFC 3986
  compliant). To be re-added when the CKP source is fixed.

### Phase 2.0 — Storage CRUD UDFs

- `pgrdf.put_term(value, term_type)`,
  `pgrdf.get_term(id)`,
  `pgrdf.put_quad(s, p, o, g)`,
  `pgrdf.count_quads(g)`,
  `pgrdf.add_graph(g)` — all backed by SPI against the
  `_pgrdf_dictionary` + `_pgrdf_quads` schema declared in
  `sql/schema_v0_2_0.sql`.
- 7 `#[pg_test]` integration tests + 3 regression files.
- Justfile: `just test` runs `cargo pgrx test` inside the linux
  builder container; `just test-regression` runs pg_regress-style
  SQL fixtures against the compose Postgres. Both gate the same
  thing CI will.

### Phase 1 — Scaffold + runtime

- pgrx 0.16 extension scaffolding (PG 14-17 feature matrix,
  `pgrx_embed` bin target for schema generation).
- Compose-based local runtime: stock `postgres:17.4-bookworm` with
  per-file bind mounts at `$libdir` / `$sharedir/extension`. No init
  script, no entrypoint wrapper.
- Linux builder container (`compose/builder.Containerfile`) that
  produces glibc-bookworm artifacts on macOS hosts. Two-VM topology:
  Colima for builds (100 GB), podman for the compose stack (avoids
  filling the user's other container state).
- 10-doc engineering set under `docs/` (architecture, storage, query,
  inference, validation, install, dev, testing, release, roadmap).
- `specs/SPEC.pgRDF.LLD.v0.2.md` + `specs/SPEC.pgRDF.INSTALL.v0.2.md`
  captured verbatim alongside `specs/ERRATA.v0.2.md` cataloguing
  deltas found during implementation.
- CI / release workflow placeholders for the
  {pg14..pg17}×{amd64, arm64} matrix.

### Errata against v0.2 specs

- `shacl-rust` → `shacl_validation` (E-001).
- `reasonable` is OWL 2 RL only, not arbitrary Datalog (E-002).
- PG 18 forward path blocked on pgrx 0.17/0.18 not building on
  current Rust (E-006). Compose targets PG 17 until upstream lands
  a fix.
- See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) for the full set.
