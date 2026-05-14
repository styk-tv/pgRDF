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
| E-006 | LLD §4, §6 | "Rust framework: pgrx" — implicitly any recent version. | Reality on 2026-05-13: pgrx **0.17.0** uses `NonNull::from_mut` without enabling the `non_null_from_ref` feature flag, so the crate root fails to compile on Rust ≤ 1.91.1 stable AND on nightly 1.97 (the flag is required for any nightly use). pgrx **0.18.0** fails with 33 errors on Rust 1.95.0 stable and 1.97.0 nightly — `E0716` (borrow checker, temporary value freed) in the `impl_table_iter` macro plus residual `E0658`. cargo-pgrx (the CLI) installs fine in both cases because it depends on different sub-crates than those that fail. | **Pin pgrx 0.16**. Support matrix: PG 14–17. Compose pins `postgres:17.4-bookworm`. Bump matrix to include PG 18 once pgrx publishes a fixed 0.17.x or 0.18.x. Track upstream at https://github.com/pgcentralfoundation/pgrx/issues. |
| E-007 | INSTALL §7 | Preferred forward path is PG 18 with `extension_control_path` GUC. | True in principle; blocked in practice by E-006. | Compose uses **per-file bind mounts** at canonical `$libdir`/`$sharedir/extension` paths on PG 17 — same observable end-state, no entrypoint wrapper, no init script. Switch to the GUC path the day E-006 clears. |
| E-008 | LLD §6.1 / INSTALL builder pattern | Implicit: native macOS dev. | macOS host produces a `.dylib`, not the glibc `.so` the Linux postgres container can load. | Local builds happen inside a `rust:1.91-bookworm` builder container (`compose/builder.Containerfile`). Output lands in `compose/extensions/`, then the postgres container picks it up via bind mounts. macOS native `cargo pgrx run` is still available for fast iteration but only against pgrx's bundled PG, not the compose stack. |
| E-009 | LLD §2 / Phase 5 | `shacl_validation` is the production-grade SHACL processor (per E-001 supersession of `shacl-rust`). | As of 2026-05-14: `shacl_validation 0.2.x` ships an unfinished `iri_s` → `rudof_iri` migration; `shacl_ast 0.2.9` no longer compiles against the resolved tree (`expected rudof_iri::IriS, found iri_s::IriS`). The 0.1.x line builds in isolation, but its transitives enable `oxrdf`'s `rdf-12` feature, which adds `TermRef::Triple(_)` — a variant unhandled by `reasonable 0.4.1`'s pattern match. Feature-unification means we can't have both crates in the workspace until one upstream catches up. | `pgrdf.validate(data, shapes) → JSONB` ships as a **stub** (`src/validation/shacl.rs`) that returns `{"status": "stub", …}` and echoes the input triple counts. The SQL surface is stable; clients and tooling can wire against it now. Re-enable the dep when (a) `shacl_validation 0.2.x` lands a release that compiles cleanly against a single `iri_s` major OR (b) `reasonable` ships a version that handles RDF 1.2 triple terms. Tracked in `docs/10-roadmap.md` Phase 5. |

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
