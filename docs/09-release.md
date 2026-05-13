# 09 — Release pipeline

Tag-based. Push a tag matching `v*` to trigger
`.github/workflows/release.yml`, which produces the release artifact
matrix specified in INSTALL spec §3.

## The matrix

```
{14, 15, 16, 17, 18}   ×   {amd64, arm64}   =   10 tarballs per release
```

ARM64 builds run on `ubuntu-24.04-arm` (native, no QEMU). AMD64 on
`ubuntu-22.04`.

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

The `release` job downloads all per-arch tarballs and emits a top-level
`SHA256SUMS` covering every artifact in the release. INSTALL spec
§3 calls for a detached GPG signature (`SHA256SUMS.asc`) — that lands
in v0.3 (tracked under INSTALL OQ4).

## Trigger

```bash
git tag v0.2.0
git push origin v0.2.0
```

## Manual re-runs

If a single matrix cell fails, `workflow_dispatch` is enabled. Re-run
only the failing cell from the Actions UI rather than reissuing a tag.

## Pre-release vs release

Tags matching `v*-alpha.*`, `v*-beta.*`, `v*-rc.*` are treated as
pre-releases by `softprops/action-gh-release@v2` and are not picked up
by the `latest` symlink in INSTALL spec §5.1's URL template. Consumers
on `latest` only see stable tags.

## Verification after release

Run the conformance check from a clean K8s namespace using INSTALL
spec §5 manifest with the newly-tagged version. CI doesn't do this
yet — Phase 4 deliverable per `docs/10-roadmap.md`.

## Release notes

GitHub auto-generates from PR titles by virtue of `generate_release_notes: true`.
Update `CHANGELOG.md` before tagging so the human-readable summary
exists alongside.
