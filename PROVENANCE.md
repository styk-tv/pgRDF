# Build provenance & release policy

## Hard rules

1. **All builds and all GHCR pushes run on GitHub Actions only.** Workstation `oras push`, `docker push`, `gh release create`, or any equivalent local-credential publish is prohibited at every tier.
2. **`LATEST.md` MUST NOT carry any version that was published manually or that lacks a verifiable SLSA Build Provenance v1 attestation.** If `gh attestation verify` rejects (or has no record of) the digest in question, that digest is not "the latest" — the file stays where it was. There is no manual-edit exception to this rule, not even to seed initial state. When no attested release has been produced yet, `LATEST.md` says so plainly.
3. **The only allowed write to `LATEST.md` is from `.github/workflows/update-latest-md.yml`,** which renders the file only after `gh attestation verify` accepts every digest it is about to advertise. Any other write is treated as drift and will be reverted by the next workflow run.
4. **A new version tag MUST NOT be pushed unless the previous tag of the same series is already advertised in `LATEST.md`.** Concretely: do not tag `v0.5.11` until `v0.5.10` shows up in `LATEST.md`. This guarantees the previous release went through the attestation gate end-to-end. Tagging ahead of the gate breaks the chain and creates orphan releases that the policy cannot retroactively verify.
5. **Release often, in small groups of 1–3 closed task IDs.** Single-task releases are explicitly fine. Larger groupings only when the tasks are inherently coupled (a `feat()` and its paired test, a Rust hook plus its SQL fixture, etc.). The roadmap §10 grouping table is a suggestion; the rule is the cadence, not the bundle size.
6. **Report task counts every release turn.** When a tag is pushed (or proposed), the user-facing turn summary MUST state:
   - **This turn:** N task IDs closed (list them, e.g. `TH-4`, `TF-12`)
   - **Total closed:** M / T (= X%) — running count across every release since the roadmap was opened
   - **Total tasks:** T — current size of the per-track countdown (changes only when the roadmap is amended; deletions or additions must be called out)

   Source of truth: `_WIP/SPEC.ROADMAP.TRACK.TASKS.v1.0-devel.md`. Counts are sanity-checked against the per-track tables; the user should never have to open the roadmap to see where things stand.

7. **Internal version label MUST match the tag.** Four sources of truth must align (bump them together when cutting a release):

   - `Cargo.toml` `version` — drives the SQL install filename (`pgrdf--<ver>.sql`) via `cargo pgrx package`.
   - repo-root `pgrdf.control` `default_version` — copied verbatim into the install dir; what PostgreSQL reads when a consumer says `CREATE EXTENSION pgrdf` (no version pin).
   - `compose/compose.yml` per-file bind-mount line (one explicit `pgrdf--<ver>.sql:.../pgrdf--<ver>.sql` line). The compose stack is how every CI regression-suite postgres comes up; if this line points at a non-existent file the stack mounts an empty directory and CI fails with `extension has no installation script for version <ver>` — exactly the v0.5.1-stuck failure mode.
   - The git tag itself (the SOURCE of all three above).

   A consumer running `CREATE EXTENSION pgrdf VERSION '<tag>'` MUST succeed; `SELECT extversion FROM pg_extension WHERE extname='pgrdf'` MUST return that tag. Enforced at four layers:

   - **Gate 0 — `ci.yml` regression on every push.** The compose Postgres boot step runs `CREATE EXTENSION IF NOT EXISTS pgrdf` against the freshly-built `compose/extensions/` artifacts. If `compose.yml`'s bind-mount line points at a non-existent `pgrdf--<X>.sql`, or if Cargo.toml-derived SQL filename disagrees with pgrdf.control's `default_version`, this step fails. Always-on; fires on every PR + every push to main, never just at release time. Caught the v0.5.25 dual-source-of-truth mistake on commit 02769bb / ac4e74a before any tag got cut.
   - **Gate 1 — `release.yml` pre-build assertion.** Before `cargo pgrx package` runs, asserts BOTH `Cargo.toml` version AND `pgrdf.control` `default_version` equal `${GITHUB_REF_NAME#v}`. Either drift ⇒ build aborts.
   - **Gate 2 — `release.yml` post-build assertion.** After the Repack step, verifies the tarball contains `share/extension/pgrdf--<TAG>.sql` and that `pgrdf.control`'s `default_version` equals `<TAG>`. Aborts upload otherwise.
   - **Gate 3 — `oci-publish.yml` boot+version gate, BEFORE attestation.** The `gate` job runs first in `oci-publish.yml`: it boots a clean `postgres:17.4-bookworm`, installs the exact release-tarball bytes the leaf job is about to push, runs `CREATE EXTENSION pgrdf VERSION '<TAG>'`, and asserts `pg_extension.extversion == TAG` and `pgrdf.version() == TAG`. The leaf publish+attest job declares `needs: gate`, so **no SLSA attestation is ever issued unless a container started and reported the matching version** — attestation is gated on a working, version-correct install, not merely on a pushed digest. A second, post-publish consumer-style smoke (pull the published artifact via ORAS, boot `postgres:17.4-bookworm`, re-run the same `CREATE EXTENSION`) then runs in the index job before `update-latest-md.yml` is triggered — defence-in-depth on the round-tripped OCI bytes. Either gate failing is fail-fast: nothing downstream fires, the GitHub release stays a draft (Rule 8), and `LATEST.md` stays at the prior version. A wrong-labeled tag exists at most as an orphan GHCR digest — never attested-and-advertised. (`postgres:17.4-bookworm` is the contractual base — see Rule 8.)

   `compose/compose.yml` doesn't get its own pre-build assertion because the Gate-0 regression-on-push enforcement is structurally stronger: the gate runs on EVERY push, not just at release time, and validates the file by actually using it (the compose stack must boot postgres + load the extension). A future cycle may refactor compose.yml to env-interpolation (`pgrdf--${PGRDF_VERSION}.sql`) so the per-release bump becomes a single Cargo.toml edit + a single pgrdf.control edit, but the gate stack catches the four-source-of-truth drift today.

   **Rule 7 takes effect from v0.5.25 onward.** Releases v0.5.1 through v0.5.23 all shipped with `Cargo.toml`'s version field stuck at `0.5.1` (the field was never bumped after the initial declaration). Their internal label reads `0.5.1` regardless of the GHCR tag — a consumer pinning `CREATE EXTENSION pgrdf VERSION '0.5.X'` for any `X != 1` would have failed at install. The .so itself was current per-release; the bug was label-only. v0.5.25 introduces the bump + the three enforcement layers above. We do NOT retroactively re-cut v0.5.2..v0.5.23 (per `[[only-forward-never-revert]]`); the no-op upgrade script `sql/pgrdf--0.5.1--0.5.25.sql` ships with v0.5.25 so anyone with a 0.5.1-labeled install can `ALTER EXTENSION pgrdf UPDATE TO '0.5.25'` cleanly. OCI-GERMINATION surfaced the bug at v0.5.23 — fleet feedback that should have come from CI, not from a downstream consumer. The three enforcement layers above are the rule.

8. **Attestation is gated on a live boot+version check, and a code release is never advertised without a verified OCI bundle.** Two coupled invariants:

   - **Attest only past the gateway.** `oci-publish.yml`'s `gate` job boots a clean `postgres:17.4-bookworm`, installs the exact release-tarball bytes about to be pushed, runs `CREATE EXTENSION pgrdf VERSION '<TAG>'`, and asserts both `pg_extension.extversion` and `pgrdf.version()` equal `<TAG>`. The publish+attest job declares `needs: gate`, so **no SLSA attestation is ever issued unless a container started and reported the matching version** (Gate 3 above). A pushed digest is not enough — a *working, version-correct install* is the precondition for attestation.
   - **Draft until verified.** The `release` workflow creates the GitHub release as a **draft** (`draft: true`, `make_latest: false`). It is flipped to public + Latest only by `update-latest-md.yml`'s final step (`gh release edit --draft=false --latest`), which runs after the bundle is boot+version-gated, attested, attestation-verified, and recorded in `LATEST.md`. So a publicly visible GitHub release ⇔ a verified, advertised OCI bundle — there is no window where a code release is advertised with a missing or broken OCI. (Drafts stay downloadable by the workflow token, so `oci-publish`'s `gh release download` keeps working; the `--draft=false` flip is performed under `GITHUB_TOKEN`, which does not cascade to re-trigger `oci-publish`.)

9. **Base image / ABI: the published `.so` targets Debian `bookworm`, and pgRDF owns that choice for the whole downstream bundle chain.** The release binary is built for and validated against `postgres:17.4-bookworm` (glibc); the artifact name encodes it (`pgrdf-<ver>-pg17-glibc-<arch>.tar.gz`), and every gate (`ci.yml`, `compose/compose.yml`, the `oci-publish` `gate` + post-publish smoke) boots exactly that image. Per the standing agreement with downstream consumers (`pgrdf-owns-pg-base-release`, oci-germination NOTIFY 2026-05-31/06-05): pgRDF's compile target dictates the Debian release the entire `pg_base` → bundle chain inherits — consumers must not choose it independently. **A move off `bookworm` (e.g. to `trixie`) is SONAME-breaking** — a `.so` linked against one Debian release fails to load (`undefined symbol` at `CREATE EXTENSION`, or `libicuuc.so.<N>: cannot open` at `initdb`) on base PostgreSQL binaries from another. So any base-release move MUST (a) bump the validation image in all gates in lockstep, and (b) emit a heads-up NOTIFY to downstream consumers *before* the tag, so their `pg_base` can regenerate its lib closure ahead of time.

10. **Every bundle ships a self-describing `MANIFEST.json`.** Each release tarball carries a `MANIFEST.json` (next to `SHA256SUMS`) declaring `version`, `extversion`, `runtime` (`pgrdf.version()` value + that it is `env!(CARGO_PKG_VERSION)`, compile-time), the install/upgrade SQL filenames, platform (`pg_major`/`arch`/`base`), and build (`tag`/`git_sha`/`built`). It is generated in `release.yml`'s repack step **from the actual repacked artifact** (control `default_version` + SQL filenames read back), so it cannot disagree with what shipped; the post-build step asserts `version == extversion == runtime.value == tag`, and the `oci-publish` gate boots the published `.so` and cross-checks `MANIFEST.json`'s `runtime.value` against the *actual* `pgrdf.version()` (validate the manifest against the artifact, not the source). This is the one place a downstream consumer reads a component's version — the gap that let a consumer pin (oci-germination `versions.yaml`) rot stale. `tools/validate-bundle.sh` is the cheap static gate a consumer runs on a pulled artifact. (Rule 7 keeps the *labels* honest; Rule 10 makes them *self-describing + machine-checkable*.)

Everything else in this document explains how those rules are enforced.

### One-time bootstrap (Rule 4 transition — closed)

Rule 4 takes effect from the **first attested release** onward. Releases that predate the attestation wiring on `main` (`v0.5.0` through `v0.5.9`, the entire pre-attestation portion of the v0.5 cycle) do not appear in `LATEST.md` once the attestation gate is live, and never will — re-publishing them with attestations would change their digests and break the immutability promise. **v0.5.10** was the bootstrap: that workflow run issued an attestation for the first time and `LATEST.md` was hand-seeded to point at it; the first workflow-rendered `LATEST.md` commit came from the **v0.5.13** chain. From v0.5.14 onward, Rule 4 is strict — no tag pushed without the prior tag advertised in a workflow-rendered `LATEST.md`.

Bootstrap exception is one-time and now closed. Rules 2 and 3 are tooling-enforced from v0.5.13 onward.

---

Every artifact this repo publishes — the pgRDF extension OCI artifacts and the GitHub release tarballs — is built and pushed **exclusively** by GitHub Actions. Workstation pushes are not permitted at any tier.

## What's enforced

Aligned with fleet-wide spec SPEC.OCI.BUNDLE.v0.3 §2.3 (LATEST.md attestation gate).

| Surface | Build / push performed by | Provenance |
|---|---|---|
| `ghcr.io/styk-tv/pgrdf-bundle:<ver>-pg<PG>-<arch>` (per-PG×arch leaf, 8 per release) | `oci-publish` workflow on `repository_dispatch: oci-publish-release` (chained from `release` workflow) | [SLSA Build Provenance v1](https://slsa.dev/spec/v1.0/provenance) via [`actions/attest-build-provenance@v1`](https://github.com/actions/attest-build-provenance), pushed as an OCI referrer |
| `ghcr.io/styk-tv/pgrdf-bundle:<ver>` + `:v<ver>` (aggregate index manifests) | Same `oci-publish` workflow's `oras manifest index create` step | Attestation covers the leaf digests the index references |
| `https://github.com/styk-tv/pgRDF/releases/tag/v<ver>` (tarballs + PGXN source archive) | `release` workflow on `v*` tag push (uses `softprops/action-gh-release@v2`), created as a **draft** | Tarballs come from the same workflow run as the OCI artifacts; the OCI attestation covers the binary bytes that landed in both surfaces. The release is **not advertised** (draft, not Latest) until the OCI chain completes — see the visibility row below |
| GitHub release **visibility** (`v<ver>` draft → public + Latest) | Born a draft in `release`; flipped to public + Latest by `update-latest-md` as its **final** step | Advertised only after the bundle is boot+version-gated (Gate 3), attested, attestation-verified, and recorded in `LATEST.md`. A publicly visible release therefore implies a verified, advertised bundle — no "advertised code, missing OCI" window |
| `LATEST.md` at the repo root | `update-latest-md` workflow on successful `workflow_run` of `oci-publish` | Refuses to advance unless `gh attestation verify` accepts every digest it's about to publish |

If `gh attestation verify` rejects an artifact, `LATEST.md` stays where it was. That's how a workstation push gets caught — it can't produce a valid GitHub-issued OIDC attestation.

## Verifying a release locally (post-attestation)

```sh
# Aggregate index (multi-arch, multi-PG-major)
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.5.10 \
  --repo styk-tv/pgRDF

# A specific PG×arch leaf
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.5.10-pg17-amd64 \
  --repo styk-tv/pgRDF
```

A successful verify means:

- Signed by GitHub's Fulcio CA against the OIDC token of a specific workflow run
- That workflow run is in `styk-tv/pgRDF`
- The signature is recorded in Sigstore's Rekor transparency log
- The subject digest matches the artifact you pulled

## Development & tracking rituals

These keep the public trail self-documenting and the branch list clean. They are not provenance-critical (a release is valid without them) but they are the standing convention for **every** change on this repo.

### Issues & PRs — the trail is the documentation

- **Every change is a GitHub issue + a PR — features as well as fixes,** not only the "negatives." The issue states scope (what + why, with refs to the spec / tracking issue); the PR states what changed, the root cause or design, the verification (tests RED→GREEN, suite green), and cross-links the issue (`Fixes #N` closes it on merge, `Refs #N` links it). The merged PR and its commits are the permanent record — browsable forever at `/pull/<n>`.
- **GitHub uses ONE shared number counter for issues AND PRs.** A gap in the PR list (e.g. `#11 → #24`) is issue numbers, NOT deleted PRs — issues and PRs interleave in the same sequence, and nothing is ever removed from history. (Unlike GitLab, where issues and MRs have independent counters — a common source of "where did my MRs go?" confusion.)

### Labels — so the tracker reads as a roadmap, not a pile of bugs

- **Type:** `feature` · `bug` · `enhancement` · `test` · `tracking` · `release` · `documentation`. **Area:** `carve` · `loader` · `query`.
- Every issue and PR carries one type label and, where it applies, one area label. `is:issue label:feature` is the roadmap; `label:bug` is the (rare) defect list.

### Milestones — the version roadmap

- **Every issue/PR carries a version milestone** = the release it targets (`v0.6.16`, `v0.6.17`, … `v0.7.0`). The milestone list IS the roadmap.
- **One code merge → one version.** Each code/artifact MR carries its own version bump (the Rule 7 source set) and produces a release; a **pure-documentation MR** (PROVENANCE, README, or the release PR's own CHANGELOG flip) merges with **no** version bump and **no** release. An urgent fix inserted into the sequence shifts the later milestones forward — version-number gaps are fine, never reuse a number, count forward (`only-forward-never-revert`).

### Branch hygiene — delete after merge

- **After a PR merges, delete the branch — remote AND local.** The merge commit copies every one of the branch's commits onto `main`, so the branch ref is redundant; deleting it removes only the pointer, **never history**. `gh pr merge --delete-branch` handles the remote; prune the local with `git branch -d <branch>`, or `git worktree remove <path>` if the branch lived in a worktree. This keeps the branch list to in-flight work only — the PR and its commits stay permanently browsable.

## Cutting a release (the only allowed flow)

pgRDF cuts each release as a **PR** (so the version bump + CHANGELOG flip are reviewed before the tag), then an annotated tag on the merge commit drives the attested pipeline. The version source is the four-way Rule 7 reconciliation, NOT the bare tag.

1. Confirm the previous release shows up in `LATEST.md` (Rule 4 — strict from v0.5.14 onward).
2. On a `release-v<new>` branch off `main`, reconcile the version across the **Rule 7 source set** (`Cargo.toml` + the `Cargo.lock` `pgrdf` entry + `pgrdf.control` `default_version` + `META.json` + the `compose/compose.yml` bind-mount + `tests/regression/expected/00-smoke.out`), and `git mv sql/pgrdf--0.5.1--<old>.sql sql/pgrdf--0.5.1--<new>.sql` (version-string rename; the upgrade path carries no DDL unless a schema change shipped).
3. Flip `CHANGELOG.md` `[Unreleased]` → `[<new>]` with the per-task entries that close the release — **at the cut, in this PR; never a separate post-merge changelog PR.** Refresh README / guide / compose install version refs (leave historical bench / feature attributions at their original version).
4. Open the release PR — label `release`, milestone `v<new>`. CI is the boot+version reconciliation gate (the regression suite asserts `pgrdf.version() == <new>`). Merge on green (the merge is a human gate).
5. Tag the merge commit (tagger `Peter Styk <peter@styk.tv>`): `git tag -a v<new> -F <annotated-message-file> <merge-sha>`; `git push origin v<new>`. Then delete the merged `release-v<new>` branch (branch-hygiene ritual above).

GitHub Actions takes over:

- `release.yml` triggers on `push: tags: v*`. Builds 8 per-PG×arch tarballs, generates the PGXN source archive, computes aggregate SHA256SUMS, creates the GitHub release via `softprops/action-gh-release@v2`, and POSTs `repository_dispatch: oci-publish-release` carrying the tag in `client_payload.tag`.
- `oci-publish.yml` fires on the dispatch. Downloads the release tarballs, pushes 8 per-PG×arch OCI artifacts to `ghcr.io/styk-tv/pgrdf-bundle`, builds the `:<ver>` and `:v<ver>` aggregate index manifests, **and generates SLSA Build Provenance v1 attestations for every digest pushed**.
- `update-latest-md.yml` fires on the `repository_dispatch: latest-md-refresh` POSTed by `oci-publish.yml`. Pulls the just-published digests, runs `gh attestation verify` on every one, and — only on full-pass — renders the new `LATEST.md` and commits it back to `main`.

There is no step in this flow that requires `oras push`, `docker push`, `gh release create`, or any local-token credential.

## When is a release "in"?

A release is "in" only when `LATEST.md` advertises the new digest (Rule 2). The full chain after `git push origin <tag>` is:

1. `release.yml` (on `v*` tag push) — builds 8 per-PG×arch tarballs, generates the PGXN source archive, creates the GitHub release, and POSTs `repository_dispatch: oci-publish-release` carrying the tag in `client_payload.tag`.
2. `oci-publish.yml` (on the dispatch) — pushes the 8 leaves to GHCR, builds the `:<ver>` + `:v<ver>` aggregate index manifests, attests every digest, and POSTs `repository_dispatch: latest-md-refresh` carrying the tag forward one more hop.
3. `update-latest-md.yml` (on the second dispatch) — verifies the attestations with `gh attestation verify` against the aggregate index + the pg17 leaves, then renders + commits `LATEST.md` on full-pass.

Wait for the `docs(auto): refresh LATEST.md to v<ver>` commit to appear on `main`, or use the helper:

```sh
scripts/gh-watch.sh watch v0.5.10        # specific tag
scripts/gh-watch.sh watch                # most recent local tag (git describe)
```

The helper is **SHA-keyed on the entry hop**: it resolves `git rev-list -n1 <tag>` and filters `gh run list` by `headSha` for `release.yml`, so two simultaneous pushes from different shells never race onto the same `--limit 1` lookup. The two downstream hops (`oci-publish.yml`, `update-latest-md.yml`) are correlated forward by anchor-timestamp + `event == repository_dispatch` because `repository_dispatch` runs carry the default-branch `headSha`, not the tag SHA. The helper exits zero only after all three workflow runs report success — non-zero surfaces any chain failure so a CI script, shell watcher, or agent can act on it. Per-tag log at `/tmp/gh-watch-<safe-tag>.log`.

The helper can also be auto-fired after every tag push by wiring a `PostToolUse` hook in a Claude Code session's `.claude/settings.json`. That wiring is Claude-Code-specific; the watcher script itself is the same one used here, called as `scripts/gh-watch.sh watch <tag>`. SPEC.OCI.BUNDLE.v0.3 §2.3 binds `LATEST.md` to the same attestation-verify gate `update-latest-md.yml` runs, so the helper's "in" answer matches the spec's "advertised" answer — by construction.

## Why `repository_dispatch` not `release.published`

GitHub blocks the `release: [published]` event from firing downstream workflows when the release is created by another workflow using the default `GITHUB_TOKEN` (a security boundary against workflow recursion). pgRDF's `release.yml` is one such case (via `softprops/action-gh-release@v2`), so all v0.5.0 through v0.5.8 releases tagged and ci-passed but did NOT auto-trigger `oci-publish.yml`. v0.5.0 was published only because someone manually clicked "Run workflow" in the Actions UI; v0.5.1–v0.5.8 were backfilled via `gh workflow run oci-publish.yml -f tag=vX.Y.Z`.

The forward fix landed in commit `dd55afb` (2026-05-28): `release.yml` ends with a `gh api ... /dispatches` step that POSTs `repository_dispatch: oci-publish-release` with the tag in `client_payload.tag`. `oci-publish.yml` listens on that event. The first auto-published release using this chain was **v0.5.9** (no human in the loop between `git push origin v0.5.9` and the bundle landing at `ghcr.io/styk-tv/pgrdf-bundle:0.5.9`).

## Hooks that block accidental local pushes

The repo's `.gitignore` keeps OCI credentials out of the tree, and the release Justfile recipes do not have `oras push` or `docker push` lines — only the build-side `docker build` (for `pgrdf-builder-rust` and `pgrdf-lubm-generator`, neither of which push anywhere). If you find yourself reaching for `oras push`: stop, push the tag instead, and let CI publish.

## Audit trail

- Workflow source: `.github/workflows/{release,oci-publish,update-latest-md,ci}.yml`.
- Release-chain watcher: `scripts/gh-watch.sh` — SHA-keyed on `release.yml`, dispatch-correlated through `oci-publish.yml` and `update-latest-md.yml`. Used by both the shell and any Claude Code `PostToolUse` hook.
- Attestation generator: `actions/attest-build-provenance@v1` (Sigstore-backed).
- Verifier: `gh attestation verify` (built into `gh` 2.49+).
- Renderer: `tools/render-latest-md.py`.

## What's pending the wire-up

The chain is live and proven. SLSA attestations landed in `oci-publish.yml` (commit `8b7e01e`); v0.5.10 was the first release to exercise them end-to-end. `update-latest-md.yml` + `tools/render-latest-md.py` landed at `c32c5b5`, and the chain has produced bot-authored `docs(auto): refresh LATEST.md to v<ver>` commits for v0.5.13, v0.5.14, v0.5.15, and v0.5.16. Rule 3 is tooling-enforced from v0.5.13 onward; Rule 4 (previous tag must be advertised in `LATEST.md` before the next tag is pushed) is strict from v0.5.14 onward.

What remains open is the **renderer's §2.2 surface coverage**:

- `tools/render-latest-md.py` does not yet emit the optional SPEC.OCI.BUNDLE.v0.3 §2.2 fields **Also tagged**, **Built by**, **Built from commit**, and **Release notes** in the `LATEST.md` table. The current output is correct and verifiable but does not yet advertise the additional provenance breadcrumbs the spec calls for. This is a renderer extension — no workflow / attestation change needed — and is tracked as a follow-up against pgRDF Track G hygiene.

The bootstrap window stays closed: v0.5.0–v0.5.9 are the pre-attestation cycle and never appear in `LATEST.md`; every release from v0.5.10 onward is attested, and every release from v0.5.13 onward has its `LATEST.md` entry workflow-rendered.

## Why this matters

The trust gap that surfaced in the v0.5 cycle is the entire reason this document exists. v0.5.1 through v0.5.8 tagged green, CI green, GitHub release green — but for 12 days none of them had a downstream artifact. The user reasonably asked "what the fuck were you lying to me about" because from outside the maintainer's view, the green checkmarks did not equal a shipped release. The `repository_dispatch` chain in commit `dd55afb` closed the publishing gap; SLSA attestations close the trust gap. Together they make "green CI" mean "verifiably shipped."
