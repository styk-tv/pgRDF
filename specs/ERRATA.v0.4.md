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
| [E-009 — SHACL real integration](ERRATA.v0.2.md) | **Resolved in v0.4** (pending upstream `reasonable` PR merge). The `iri_s → rudof_iri` half cleared in rudof 0.3.1 (2026-05-12). The `rdf-12 / TermRef::Triple` half is unblocked locally via the patched `reasonable` fork tracked below as **E-011**; `pgrdf.validate` now ships a real W3C-shape SHACL report. Final upstream close-out depends on E-011 landing upstream. |
| [E-010 — `cargo audit` informational advisories](ERRATA.v0.2.md) | Unchanged. Clears with E-006 + E-009. Re-run on every major dep refresh. |

## v0.4 entries

### E-011 — Upstream `reasonable` patch for RDF 1.2 coexistence

| Field | Value |
|---|---|
| Filed | 2026-05-15 |
| Status | verified locally + upstream PR open (awaiting maintainer review/merge) |
| Resolves | The remaining `rdf-12 / TermRef::Triple` half of [E-009](ERRATA.v0.2.md) |
| Fork branch | https://github.com/styk-tv/reasonable/tree/rdf12-passthrough |
| Upstream PR | <https://github.com/gtfierro/reasonable/pull/50> (filed 2026-05-15) |
| Local PR draft | `PR-DRAFT.md` in the `styk-tv/reasonable` fork workspace (now reflected upstream) |

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

Verified on the patched fork in isolation:

| Command | Result |
|---|---|
| `cargo check` (default features) | clean |
| `cargo check -p reasonable --features rdf-12` | clean |
| `cargo test -p reasonable` (default) | 73 unit + 8 doc tests pass, 0 failed, 1 ignored |
| `cargo test -p reasonable --features rdf-12` | 73 unit + 8 doc tests pass, 0 failed, 1 ignored |

The existing suite continues to pass in both feature configurations.

#### Verified locally in pgRDF (2026-05-15)

The patched fork unblocks SHACL end-to-end. `pgrdf.validate(data, shapes)`
now ships as a real W3C-shape SHACL Core validator. Test bar after
the slice landed:

| Layer | Count | Notes |
|---|---|---|
| `cargo check --no-default-features --features pg17` | clean | reasonable + shacl 0.3.1 + rudof_rdf 0.3.1 resolve unanimously |
| `cargo pgrx test pg17` | 94 pass / 0 fail | +3 new tests in `validation::shacl::tests` (conforming, violations, unknown graphs) |
| `just test-regression` | 40 pass / 0 fail | +1 new file (`71-shacl-real.sql`); `70-validate-stub.sql` repurposed for the real-impl shape |
| `just test-w3c` | 23 pass / 0 fail | unchanged surface |
| `just test-lubm` | 3 pass / 0 fail | unchanged surface |

Sample violation output from the regression fixture (Alice missing
required `ex:age`):

```json
{
  "conforms": false,
  "results": [{
    "focusNode": "http://example.org/alice",
    "resultPath": "http://example.org/age",
    "sourceShape": "_:b887c79907df332dbd793b0bc80edbd5",
    "resultMessage": "MinCount(1) not satisfied",
    "resultSeverity": "sh:Violation",
    "sourceConstraintComponent": "http://www.w3.org/ns/shacl#MinCountConstraintComponent",
    "value": null
  }],
  "data_graph_id": 8971,
  "shapes_graph_id": 8972,
  "data_triples": 5,
  "shapes_triples": 10,
  "elapsed_ms": 1.68
}
```

#### Shipped in v0.4.0 (2026-05-15)

pgRDF v0.4.0 cut with the `[patch.crates-io]` block in place
([commit `d6fc261`](https://github.com/styk-tv/pgRDF/commit/d6fc261),
[release page](https://github.com/styk-tv/pgRDF/releases/tag/v0.4.0)).
The release ships 8 prebuilt tarballs (PG 14-17 × {amd64, arm64})
and 160 automated tests green. Users `cargo build`ing from source
pull the `styk-tv/reasonable@rdf12-passthrough` fork transparently
via Cargo's `[patch.crates-io]` resolution. v0.4.1 will drop the
patch once gtfierro/reasonable#50 merges; until then, the patch
is a load-bearing part of the v0.4.0 build graph.

#### PR draft

Held in the `styk-tv/reasonable` fork workspace as `PR-DRAFT.md`
pending user authorisation. The fork is now confirmed to unblock pgRDF
locally via `[patch.crates-io]`; the PR can be filed.

#### Next steps for pgRDF

1. ~~Wire `[patch.crates-io] reasonable = { git = "...", branch = "rdf12-passthrough" }` in pgRDF's `Cargo.toml` (with `features = ["rdf-12"]` on the dep).~~ **Done — landed in v0.4 slice.**
2. ~~Add `shacl 0.3.x` to pgRDF's deps; verify the dep tree resolves cleanly.~~ **Done — pinned at `shacl = "0.3"` + `rudof_rdf = "0.3"`.**
3. ~~Replace the `pgrdf.validate` stub in `src/validation/shacl.rs` with a real `shacl::GraphValidation`-backed body.~~ **Done — see [LLD.v0.4 §9](SPEC.pgRDF.LLD.v0.4.md).**
4. ~~File the upstream PR using the held draft.~~ **Done 2026-05-15: <https://github.com/gtfierro/reasonable/pull/50>**.
5. Once upstream merges, drop the `[patch.crates-io]` line and pin `reasonable = "0.4.2"` (or whatever lands). **Open — gated on maintainer review of (4).**
6. **2026-05-15: `cargo publish` is structurally blocked while `[patch.crates-io]` is in place.** The patch entry does not travel with the published crate (`cargo publish` rejects + downstream consumers can't reproduce the override). v0.4.1 confirmed this empirically — `publish-crate.yml` errored with `failed to select a version for reasonable. package pgrdf depends on reasonable with feature rdf-12 but reasonable does not have that feature`. Action taken: **`/.github/workflows/publish-crate.yml` renamed to `.disabled`** so the workflow doesn't fire on subsequent releases. Re-enable by renaming back when step (5) lands. Until then, pgRDF distribution is via prebuilt tarballs (`release.yml`) only; the crates.io `pgrdf` 0.3.0 entry stays as the name-claim seed but won't auto-update through 0.4.x.

This entry is updated as work progresses; final state is **resolved**
once upstream merges and pgRDF pins the released `reasonable` version.

## Forward-looking notes

- **[v0.4 LLD](SPEC.pgRDF.LLD.v0.4.md) §9** now describes the real
  SHACL surface (replacing the v0.3 stub framing). The
  `pgrdf.validate()` JSONB return shape is stable on the W3C
  `sh:ValidationReport` skeleton.
- **v0.4 pgrx migration (E-006)** remains the largest deferred work
  item; sequencing it after the SHACL real-body slice has kept the
  validation surface stable for the pgrx bump.
- Once the upstream `reasonable` PR merges, drop the
  `[patch.crates-io]` block from `Cargo.toml` and pin the released
  `reasonable` version (the `features = ["rdf-12"]` opt-in stays).
  Then re-run `cargo audit` (E-010); the
  `reasonable → env_logger 0.7.1 → atty 0.2.14` chain may clear if
  upstream simultaneously bumps `env_logger`.
