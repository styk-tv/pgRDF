# ERRATA.v0.4

Spec deltas accumulated during the v0.4 cycle. v0.2-era entries that
remain live are cross-linked to [`ERRATA.v0.2.md`](ERRATA.v0.2.md)
rather than duplicated.

## v0.2 entries still live in v0.4

These rows in [`ERRATA.v0.2.md`](ERRATA.v0.2.md) remain open at the
start of the v0.4 cycle. See that file for the full claim / reality /
resolution text; status notes below are deltas only.

| Entry | One-line status in v0.4 |
|---|---|
| [E-006 — pgrx 0.18 / PG 18 deferred](ERRATA.v0.2.md) | Re-check trigger unchanged: any pgrx publish > 0.18.0, or an E0716 `impl_table_iter` fix on `develop`. Planned v0.4 migration item. |
| [E-007 — `extension_control_path` GUC blocked by E-006](ERRATA.v0.2.md) | Tracks E-006; clears the day E-006 clears. |
| [E-008 — Linux builder container vs native macOS](ERRATA.v0.2.md) | Engineering arrangement, no upstream blocker. Stays as documented through v0.4. |
| [E-009 — SHACL real integration](ERRATA.v0.2.md) | **Partially resolved upstream.** The `iri_s → rudof_iri` half cleared in rudof 0.3.1 (2026-05-12) which consolidated `shacl_ast` + `shacl_validation` into a single `shacl 0.3.1` crate. The remaining `rdf-12 / TermRef::Triple` half is tracked below as **E-011**. |
| [E-010 — `cargo audit` informational advisories](ERRATA.v0.2.md) | Unchanged. Clears with E-006 + E-009. Re-run on every major dep refresh. |

## v0.4 entries

### E-011 — Upstream `reasonable` patch for RDF 1.2 coexistence

| Field | Value |
|---|---|
| Filed | 2026-05-15 |
| Status | in-progress (fork branch pushed; upstream PR not yet filed) |
| Resolves | The remaining `rdf-12 / TermRef::Triple` half of [E-009](ERRATA.v0.2.md) |
| Fork branch | https://github.com/styk-tv/reasonable/tree/rdf12-passthrough |
| Upstream target | https://github.com/gtfierro/reasonable (PR not yet filed) |
| Local PR draft | `/Users/neoxr/git_styk/reasonable/PR-DRAFT.md` |

#### Context

[`rudof 0.3.1`](https://github.com/rudof-project/rudof) (released
2026-05-12) consolidated `shacl_ast` and `shacl_validation` into a
single `shacl 0.3.1` crate. The half-finished `iri_s → rudof_iri`
migration cited in [`ERRATA.v0.2`](ERRATA.v0.2.md) E-009 is resolved
upstream.

The remaining half of E-009 — `rudof_rdf 0.3.1` hard-enabling
`oxrdf`'s `rdf-12` feature (workspace `Cargo.toml` lines 284-294,
non-optional), which adds `TermRef::Triple(_)` unhandled by
`reasonable 0.4.1`'s match in `lib/src/common.rs:140` — is a small
upstream patch.

The `reasonable` maintainer (`gtfierro`) is active: 3 PRs merged in
the past two weeks, last release v0.4.1 on 2026-05-10.

#### Patch summary

Two strictly additive changes to `reasonable`:

1. New `rdf-12` feature in `lib/Cargo.toml` that forwards to
   `oxrdf/rdf-12`.
2. `#[cfg(feature = "rdf-12")] TermRef::Triple(_) => panic!(...)` arm
   added to `oxrdf_to_rio` in `lib/src/common.rs` (the only
   non-exhaustive `TermRef` match in `lib/src/` after a full sweep).

Behaviour mirrors the existing `panic!("no rdf*")` arms in
`rio_to_oxrdf` for unsupported variants — `reasonable` does not
implement RDF-star reasoning and `rio-api` has no equivalent term
shape today. Strictly additive when off; panics with a clear message
when on.

Diff: 2 files changed, 24 insertions(+). Commit
[`f0659da`](https://github.com/styk-tv/reasonable/commit/f0659da) on
fork branch `rdf12-passthrough`.

#### Smoke results (2026-05-15)

Verified on the patched fork:

| Command | Result |
|---|---|
| `cargo check` (default features) | clean |
| `cargo check -p reasonable --features rdf-12` | clean |
| `cargo test -p reasonable` (default) | 73 unit + 8 doc tests pass, 0 failed, 1 ignored |
| `cargo test -p reasonable --features rdf-12` | 73 unit + 8 doc tests pass, 0 failed, 1 ignored |

The existing suite continues to pass in both feature configurations.

#### PR draft

Held in `/Users/neoxr/git_styk/reasonable/PR-DRAFT.md` pending
review. Will be posted to `gtfierro/reasonable` once the patch is
confirmed to unblock pgRDF locally via `[patch.crates-io]`
(separate slice).

#### Next steps for pgRDF

1. Wire `[patch.crates-io] reasonable = { git = "https://github.com/styk-tv/reasonable", branch = "rdf12-passthrough" }` in pgRDF's `Cargo.toml` (with `features = ["rdf-12"]` on the dep).
2. Add `shacl 0.3.x` to pgRDF's deps; verify the dep tree resolves cleanly.
3. Replace the `pgrdf.validate` stub in `src/validation/shacl.rs` with a real `shacl::GraphValidation`-backed body.
4. File the upstream PR using the held draft.
5. Once upstream merges, drop the `[patch.crates-io]` line and pin `reasonable = "0.4.2"` (or whatever lands).

This entry is updated as work progresses; final state is **resolved**
once upstream merges and pgRDF pins the released `reasonable` version.

## Forward-looking notes

- **v0.4 LLD** should fold E-011's outcome into the body once SHACL
  validation goes from stub to real implementation. The
  `pgrdf.validate()` JSONB return shape is stable per E-009; v0.4
  populates the body without changing the signature.
- **v0.4 pgrx migration (E-006)** remains the largest deferred work
  item; consider sequencing it after the SHACL real-body slice to
  keep the validation surface stable across the pgrx bump.
- Once E-011 resolves, re-run `cargo audit` (E-010); the
  `reasonable → env_logger 0.7.1 → atty 0.2.14` chain may clear if
  upstream simultaneously bumps `env_logger`.
