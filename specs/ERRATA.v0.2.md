# ERRATA.v0.2

> **Status (2026-05-14):** [`SPEC.pgRDF.LLD.v0.3.md`](SPEC.pgRDF.LLD.v0.3.md)
> has shipped and supersedes `SPEC.pgRDF.LLD.v0.2.md` at the contract
> level. This errata document remains authoritative for the deltas
> below — v0.3 folds the architectural facts into the body but does
> NOT void the entries here. INSTALL spec (`SPEC.pgRDF.INSTALL.v0.2.md`)
> is unchanged in v0.3; errata against it still apply.

Corrections to v0.2 specs discovered during implementation. Each entry
states the **claim in the spec**, the **observed reality**, and the
**resolution** applied in this repository.

| # | Source | Claim | Reality | Resolution |
|---|---|---|---|---|
| E-001 | LLD §2 | "Validates RDF graphs against SHACL shape graphs (shacl-rust)" | A crate named `shacl-rust` is not a production-grade SHACL validator. `shacl_validation` (crates.io) and `oxirs-shacl` are the actively maintained options. | Use `shacl_validation` as the default; revisit in v0.3 after benchmark against `oxirs-shacl`. |
| E-002 | LLD §2 | "reasonable Datalog reasoner" | `reasonable` implements **OWL 2 RL only**, not arbitrary Datalog. Sufficient for the LLD's stated scope, but worth being explicit. | Documented in [`docs/04-inference.md`](../docs/04-inference.md). Out-of-scope: OWL 2 EL/QL and Datalog-beyond-RL. |
| E-003 | INSTALL §5 | Example uses `postgres:17.4-bookworm`. | PG 18 GA shipped 2025-10. PG 18 enables the GUC-based drop-in path (INSTALL §7), which is the spec-preferred forward path. | Long-term: local compose will target PG 18 with `extension_control_path`. Today it targets PG 17 — see E-006 — because pgrx doesn't yet build at versions that support PG 18. |
| E-004 | LLD §5.2 | Example init script copies into `/`. | When the spec's preferred §7 path is used (PG 18+), no init script is required at all; the GUCs handle path resolution. | Local compose has **no init script**. K8s manifests on PG 17 retain the wrapper per INSTALL §4.3. |
| E-005 | Repo URL | `github.com/my-org/pgRDF` (placeholder). | Repo lives at `github.com/styk-tv/pgRDF`. | `Cargo.toml.repository`, README and release workflow point at `styk-tv/pgRDF`. |
| E-006 | LLD §4, §6 | "Rust framework: pgrx" — implicitly any recent version. | **Re-checked 2026-05-14** (slice #48). No new pgrx release since 0.18.0 (2026-04-17) — `max_stable_version` on crates.io is still `0.18.0`; only one commit on `develop` since (an aarch64 linker fix, PR #2280). Upstream README now states "pgrx supports Postgres 13 through Postgres 18" — i.e. PG 18 support is **landed** at the 0.18.0 line. However the local-compile blockers we recorded on 2026-05-13 still stand: 0.17.0's `NonNull::from_mut`/`non_null_from_ref` E0658 and 0.18.0's E0716 `impl_table_iter` macro errors are unchanged in `develop`. Additionally, 0.18.0 carries a **breaking migration** (PR #2264 / `v18.0-MIGRATION.md`): `pgrx_embed` binary removed, `crate-type` must drop `"lib"`, manual `SqlTranslatable` impls move from methods to associated `const`s. pgRDF still ships `src/bin/pgrx_embed.rs` + `crate-type = ["cdylib", "lib"]`, so the bump is non-trivial. A new known 0.18+ bug (#2281) leaks Postgres symbols (`CurrentMemoryContext`, `ErrorContext`, `BufferBlocks`) into `cargo test` / `cargo llvm-cov` binaries via `typetag::serde` ctor registration. Dep-wise: 0.18.0 still pulls `serde_cbor 0.11.2` (RUSTSEC-2021-0127, see E-010) and has no `oxrdf` edge of its own, so `reasonable 0.4.1` / `oxrdf 0.3.3` feature-unification (E-009) is orthogonal. | **Hold pgrx 0.16.1 for v0.3.** Support matrix: PG 14–17. Compose pins `postgres:17.4-bookworm`. **v0.4 work item:** plan a pgrx-0.18 migration once (a) the `impl_table_iter` E0716 lands in a 0.18.x point release AND (b) we have bandwidth for the `pgrx_embed` removal + manual-`SqlTranslatable` (none today, but verify) audit. Next re-check trigger: any pgrx publish > 0.18.0 OR an E0716 fix lands in `develop`. Track at https://github.com/pgcentralfoundation/pgrx/issues. |
| E-007 | INSTALL §7 | Preferred forward path is PG 18 with `extension_control_path` GUC. | True in principle; blocked in practice by E-006. | Compose uses **per-file bind mounts** at canonical `$libdir`/`$sharedir/extension` paths on PG 17 — same observable end-state, no entrypoint wrapper, no init script. Switch to the GUC path the day E-006 clears. |
| E-008 | LLD §6.1 / INSTALL builder pattern | Implicit: native macOS dev. | macOS host produces a `.dylib`, not the glibc `.so` the Linux postgres container can load. | Local builds happen inside a `rust:1.91-bookworm` builder container (`compose/builder.Containerfile`). Output lands in `compose/extensions/`, then the postgres container picks it up via bind mounts. macOS native `cargo pgrx run` is still available for fast iteration but only against pgrx's bundled PG, not the compose stack. |
| E-009 | LLD §2 / Phase 5 | `shacl_validation` is the production-grade SHACL processor (per E-001 supersession of `shacl-rust`). | As of 2026-05-14: `shacl_validation 0.2.x` ships an unfinished `iri_s` → `rudof_iri` migration; `shacl_ast 0.2.9` no longer compiles against the resolved tree (`expected rudof_iri::IriS, found iri_s::IriS`). The 0.1.x line builds in isolation, but its transitives enable `oxrdf`'s `rdf-12` feature, which adds `TermRef::Triple(_)` — a variant unhandled by `reasonable 0.4.1`'s pattern match. Feature-unification means we can't have both crates in the workspace until one upstream catches up. | `pgrdf.validate(data, shapes) → JSONB` ships as a **stub** (`src/validation/shacl.rs`) that returns `{"status": "stub", …}` and echoes the input triple counts. The SQL surface is stable; clients and tooling can wire against it now. Re-enable the dep when (a) `shacl_validation 0.2.x` lands a release that compiles cleanly against a single `iri_s` major OR (b) `reasonable` ships a version that handles RDF 1.2 triple terms. Tracked in `docs/10-roadmap.md` Phase 5. |
| E-010 | `cargo audit` ledger | Implicit: a clean `cargo audit` is achievable in v0.3. | **Re-checked 2026-05-16** at tag v0.5.0-rc1 (gate-prep). `cargo audit` reports **0 vulnerabilities** and **2 informational** advisories (down from 4) — both unmaintained-crate notices in subtrees of the pinned core deps: `paste 1.0.15` (RUSTSEC-2024-0436, no longer maintained — now reached via `rudof_rdf 0.3.1 → shacl 0.3.1` for the real SHACL engine **and** dev-dep `pgrx-tests 0.16.1`); `serde_cbor 0.11.2` (RUSTSEC-2021-0127, unmaintained, via `pgrx 0.16.1`). The two earlier `atty 0.2.14` advisories (RUSTSEC-2024-0375 / RUSTSEC-2021-0145, via `reasonable 0.4.1 → env_logger 0.7.1`) **cleared**: the v0.4 patched `reasonable` fork (E-011) bumped `env_logger`, dropping the `atty` edge entirely. Both remaining items are unmaintained notices, **not vulnerabilities**, with no SemVer-compatible bump available within the pinned set. | Accept the 2 informational warnings for v0.5 — no vulnerabilities, informational only. `paste`/`serde_cbor` clear automatically when E-006 (pgrx unpin) and the upstream `rudof`/`shacl` line refresh resolve. Audit slice #51 (CHANGELOG [Unreleased]) records the original ledger; this v0.5.0-rc1 re-check supersedes the 4-advisory count. Re-run on every major dep refresh. |

## Forward-looking notes

- **v0.3 LLD** should fold E-001 through E-008 into the body. Bump
  version field. Explicitly state minimum pgrx version + Rust MSRV.
- **v0.3 INSTALL** should make the PG 18 GUC path canonical and demote
  the entrypoint wrapper to "legacy PG ≤ 17" appendix once E-006 clears.
- Consider publishing pre-built artifacts as both GitHub-Release
  tarballs AND OCI artifacts at `ghcr.io/styk-tv/pgrdf-bundle:<ver>`
  (INSTALL §11 OQ1). The OCI variant is the easier consumption surface
  for CloudNativePG / StackGres.

## Tracking the pgrx version saga (E-006)

Observed sequence on 2026-05-13:

| Attempt | pgrx | Rust | Result |
|---|---|---|---|
| 1 | 0.17.0 | 1.88.0 stable (Homebrew) | `E0658` `non_null_from_ref` |
| 2 | 0.17.0 | 1.91.1 stable (rustup) | `E0658` `non_null_from_ref` |
| 3 | 0.17.0 | 1.93.0-nightly | `E0658` `non_null_from_ref` (feature still gated on nightly without `#![feature(…)]`) |
| 4 | 0.18.0 | 1.91.1 stable | 33 errors: `E0658` + `E0716` in `impl_table_iter` macro |
| 5 | 0.18.0 | 1.95.0 stable | same 33 errors |
| 6 | 0.18.0 | 1.97.0-nightly | same 33 errors |
| 7 | 0.17.0 | 1.97.0-nightly | `E0658` `non_null_from_ref` |
| 8 | 0.16.x | 1.95.0 stable | **Compiles cleanly.** Re-evaluate higher pgrx versions periodically. |

### Re-check 2026-05-14 (hygiene slice #48)

Upstream snapshot:

- `crates.io` reports `pgrx.max_stable_version = "0.18.0"` (unchanged since 2026-04-17).
- `develop` is one commit ahead of `v0.18.0` (PR #2280, aarch64 `-Wl,--no-gc-sections` link-flag fix); no changes touching the `impl_table_iter` macro or `NonNull::from_mut` site.
- Upstream README now documents "pgrx supports Postgres 13 through Postgres 18" — PG 18 support is officially in 0.18.0.
- Open follow-ups affecting our consumption: #2281 (`typetag::serde` leaking pg_sys symbols into test binaries on 0.18+); #2287 (README range cleanup).

Status decision: **E-006 stays open, classification B (partially resolved).** PG 18 support has landed upstream, but local-compile blockers from attempts 4–6 are unfixed, AND 0.18.0 carries a hard breaking change (`pgrx_embed` removal, `crate-type` change) per `v18.0-MIGRATION.md` that we would need to absorb. Defer the bump to **v0.4** as a planned migration item rather than a v0.3 hygiene action.

Next re-check trigger: any pgrx publish above 0.18.0 OR an E0716 / `impl_table_iter` fix landing on `develop`.
