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

Everything else in this document explains how those rules are enforced.

### One-time bootstrap (Rule 4 transition)

Rule 4 takes effect from the **first attested release** onward. Releases that predate the attestation wiring on `main` (`v0.5.0` through `v0.5.9`, the entire v0.5 cycle to date) do not appear in `LATEST.md` once the attestation gate is live, and never will — re-publishing them with attestations would change their digests and break the immutability promise. The next tag (`v0.5.10`) is the bootstrap: that workflow run will issue an attestation for the first time, `update-latest-md.yml` will verify and populate `LATEST.md`, and from that point Rule 4 holds for every successor.

Bootstrap exception is one-time. Once the gate has fired once, "previous tag must be in `LATEST.md`" is strict.

Until then — and explicitly in the current state of this repository — the `LATEST.md` you see is a **hand-maintained** snapshot of the pre-attestation v0.5 cycle. Rules 2 and 3 are aspirational while we cross over; the workflow tooling that enforces them is tracked as Track G hygiene items (`TG-3.attestation`, `TG-3.update-latest-md`).

---

Every artifact this repo publishes — the pgRDF extension OCI artifacts and the GitHub release tarballs — is built and pushed **exclusively** by GitHub Actions. Workstation pushes are not permitted at any tier.

## What's enforced

| Surface | Build / push performed by | Provenance (target state) |
|---|---|---|
| `ghcr.io/styk-tv/pgrdf-bundle:<ver>-pg<PG>-<arch>` (per-PG×arch leaf, 8 per release) | `oci-publish` workflow on `repository_dispatch: oci-publish-release` (chained from `release` workflow) | [SLSA Build Provenance v1](https://slsa.dev/spec/v1.0/provenance) via [`actions/attest-build-provenance@v1`](https://github.com/actions/attest-build-provenance), pushed as an OCI referrer **— pending wire-up** |
| `ghcr.io/styk-tv/pgrdf-bundle:<ver>` + `:v<ver>` (aggregate index manifests) | Same `oci-publish` workflow's `oras manifest index create` step | Attestation covers the leaf digests the index references — pending wire-up |
| `https://github.com/styk-tv/pgRDF/releases/tag/v<ver>` (tarballs + PGXN source archive) | `release` workflow on `v*` tag push (uses `softprops/action-gh-release@v2`) | Tarballs come from the same workflow run as the OCI artifacts; the OCI attestation covers the binary bytes that landed in both surfaces |
| `LATEST.md` at the repo root | `update-latest-md` workflow on successful `workflow_run` of `oci-publish` — pending wire-up | Refuses to advance unless `gh attestation verify` accepts every digest it's about to publish |

If `gh attestation verify` rejects an artifact (post-wire-up), `LATEST.md` stays where it was. That's how a workstation push gets caught — it can't produce a valid GitHub-issued OIDC attestation.

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

## Cutting a release (the only allowed flow)

The release-cutting flow is simpler than pgCK's because pgRDF's version source is the git tag — there's no per-release file bump.

1. Confirm the previous release shows up in `LATEST.md` (Rule 4 — once Rule 4 is live; ignored during the bootstrap window).
2. Update `CHANGELOG.md` with the per-task entries that close the release.
3. Commit.
4. Tag: `git tag -a v<new> -F <annotated-message-file>`.
5. Push the tag: `git push origin v<new>`.

GitHub Actions takes over:

- `release.yml` triggers on `push: tags: v*`. Builds 8 per-PG×arch tarballs, generates the PGXN source archive, computes aggregate SHA256SUMS, creates the GitHub release via `softprops/action-gh-release@v2`, and POSTs `repository_dispatch: oci-publish-release` carrying the tag in `client_payload.tag`.
- `oci-publish.yml` fires on the dispatch. Downloads the release tarballs, pushes 8 per-PG×arch OCI artifacts to `ghcr.io/styk-tv/pgrdf-bundle`, builds the `:<ver>` and `:v<ver>` aggregate index manifests, **and (pending wire-up) generates SLSA Build Provenance v1 attestations for every digest pushed**.
- `update-latest-md.yml` (pending wire-up) fires on successful completion of `oci-publish.yml`. Pulls the just-published digests, runs `gh attestation verify` on every one, and — only on full-pass — renders the new `LATEST.md` and commits it back to `main`.

There is no step in this flow that requires `oras push`, `docker push`, `gh release create`, or any local-token credential.

## Why `repository_dispatch` not `release.published`

GitHub blocks the `release: [published]` event from firing downstream workflows when the release is created by another workflow using the default `GITHUB_TOKEN` (a security boundary against workflow recursion). pgRDF's `release.yml` is one such case (via `softprops/action-gh-release@v2`), so all v0.5.0 through v0.5.8 releases tagged and ci-passed but did NOT auto-trigger `oci-publish.yml`. v0.5.0 was published only because someone manually clicked "Run workflow" in the Actions UI; v0.5.1–v0.5.8 were backfilled via `gh workflow run oci-publish.yml -f tag=vX.Y.Z`.

The forward fix landed in commit `dd55afb` (2026-05-28): `release.yml` ends with a `gh api ... /dispatches` step that POSTs `repository_dispatch: oci-publish-release` with the tag in `client_payload.tag`. `oci-publish.yml` listens on that event. The first auto-published release using this chain was **v0.5.9** (no human in the loop between `git push origin v0.5.9` and the bundle landing at `ghcr.io/styk-tv/pgrdf-bundle:0.5.9`).

## Hooks that block accidental local pushes

The repo's `.gitignore` keeps OCI credentials out of the tree, and the release Justfile recipes do not have `oras push` or `docker push` lines — only the build-side `docker build` (for `pgrdf-builder-rust` and `pgrdf-lubm-generator`, neither of which push anywhere). If you find yourself reaching for `oras push`: stop, push the tag instead, and let CI publish.

## Audit trail

- Workflow source: `.github/workflows/{release,oci-publish,ci}.yml`. Pending: `update-latest-md.yml`.
- Attestation generator (target): `actions/attest-build-provenance@v1` (Sigstore-backed).
- Verifier: `gh attestation verify` (built into `gh` 2.49+).
- Renderer (target): `tools/render-latest-md.py` or equivalent.

## What's pending the wire-up

The SLSA attestation half went live in `oci-publish.yml`'s matrix refactor (commit `8b7e01e`); v0.5.10 was the first release to exercise it end-to-end. Every per-PG×arch leaf and the aggregate index now carry verifiable provenance.

The `LATEST.md` auto-rendering half went in next:

- **`.github/workflows/update-latest-md.yml`** — committed at `c32c5b5`. Triggers on `workflow_run: oci-publish completed`; resolves the head version from the GHCR API; runs the attestation-verify gate against the aggregate + both pg17 leaf digests; renders + commits only on full-pass. Refuses to advance if any digest fails to verify.
- **`tools/render-latest-md.py`** — committed at the same SHA. Reads the three head digests via `gh api` and emits the full `LATEST.md` content. Adapted from the pgCK sibling-repo renderer; trimmed because pgRDF ships a single OCI surface.

**Verification state: unproven by this point in the doc.** The workflow files exist on `main` but no tagged release has fired the `release.yml` → `oci-publish.yml` → `update-latest-md.yml` chain end-to-end yet. The first tagged release after `c32c5b5` is the verification gate. Until that run lands a bot-authored auto-rendered `LATEST.md` commit, Rule 3 ("only `update-latest-md.yml` writes `LATEST.md`") remains a discipline + scaffold rather than a tooling-enforced gate.

The bootstrap window stays open across this transition: v0.5.0–v0.5.9 are the pre-attestation cycle and never appear in `LATEST.md`; v0.5.10 was attested but its `LATEST.md` entry is hand-written. The first auto-rendered entry will come from the next tagged release's chain. Once that lands successfully, Rule 4 becomes strict — no tag pushed without the prior tag advertised in a workflow-rendered `LATEST.md`.

## Why this matters

The trust gap that surfaced in the v0.5 cycle is the entire reason this document exists. v0.5.1 through v0.5.8 tagged green, CI green, GitHub release green — but for 12 days none of them had a downstream artifact. The user reasonably asked "what the fuck were you lying to me about" because from outside the maintainer's view, the green checkmarks did not equal a shipped release. The `repository_dispatch` chain in commit `dd55afb` closed the publishing gap; SLSA attestations close the trust gap. Together they make "green CI" mean "verifiably shipped."
