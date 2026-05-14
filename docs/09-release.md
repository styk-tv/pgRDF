# 09 — Release pipeline

Tag-based. Push a tag matching `v*` to trigger
`.github/workflows/release.yml`, which produces the release artifact
matrix specified in INSTALL spec §3.

No release has been cut yet — the first cut is `v0.3.0` (in progress).
Cargo.toml still reads `version = "0.2.0"`; bump-to-`0.3.0` happens as
part of the cut. See `CHANGELOG.md` for the running set of `[Unreleased]`
entries that will land in that release.

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
