# pgRDF — latest published artifacts

One publishable surface ships from this repo: the PostgreSQL **extension** (oras-pulled OCI artifact). This file tracks the head on **PostgreSQL 17**. Older PG majors (14, 15, 16) are still built per release — see [Repo packages view](https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle) for the full matrix.

## pgRDF extension — `v0.5.10` (PostgreSQL 17)

**First SLSA-attested release.** Per [`PROVENANCE.md`](./PROVENANCE.md), v0.5.10 is the bootstrap of the attestation gate: every digest below verifies under `gh attestation verify oci://… --repo styk-tv/pgRDF`. v0.5.0–v0.5.9 predate the attestation wiring and never appear here (re-publishing them would change digests and break the immutability promise).

`oras pull ghcr.io/styk-tv/pgrdf-bundle:0.5.10-pg17-<arch>` → drop `lib/pgrdf.so` + `share/extension/{pgrdf.control, pgrdf--0.5.10.sql}` next to your `postgres:17` install.

| arch  | Pull URI                                             | Digest                                                                  | Created (UTC)       |
|-------|------------------------------------------------------|-------------------------------------------------------------------------|---------------------|
| amd64 | `ghcr.io/styk-tv/pgrdf-bundle:0.5.10-pg17-amd64`     | `sha256:a862700b46c3678f156820e96aa8c3c7963f96d1045f32fcc8e747c472a6d40f` | 2026-05-28 18:56:13 |
| arm64 | `ghcr.io/styk-tv/pgrdf-bundle:0.5.10-pg17-arm64`     | `sha256:7c7ff676f0deea81e1b5fe9012768d2e65d5e1da58c202999be0be398da81966` | 2026-05-28 18:56:13 |

|                       |                                                                                                |
|-----------------------|------------------------------------------------------------------------------------------------|
| Artifact type         | `application/vnd.styk.pgrdf.bundle.v1+tar`                                                     |
| Aggregate index       | `ghcr.io/styk-tv/pgrdf-bundle:0.5.10` (also tagged `v0.5.10`)                                  |
| Aggregate digest      | `sha256:715d5917f6f1cd4a7df4fba2d9fe6737a9d2c45b039679c236feebb4c1387193`                       |
| Provenance            | SLSA Build Provenance v1, Sigstore-backed, pushed as OCI referrer                              |
| Verify (CLI)          | `gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.5.10 --repo styk-tv/pgRDF`         |
| Tarball mirror        | https://github.com/styk-tv/pgRDF/releases/tag/v0.5.10                                          |
| Repo packages view    | https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle                                   |
| Older PG majors       | `0.5.10-pg{14,15,16}-{amd64,arm64}` published alongside; same v0.5.10 tag; each individually attested |

## Verifying any artifact above

```sh
# Aggregate index (multi-arch, multi-PG-major)
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.5.10 \
  --repo styk-tv/pgRDF

# A specific PG×arch leaf
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.5.10-pg17-amd64 \
  --repo styk-tv/pgRDF
```

A successful verify means: signed by GitHub's Fulcio CA against the OIDC token of the v0.5.10 `oci-publish` workflow run, recorded in Sigstore's Rekor transparency log, subject digest matches the pulled artifact.

## Pin policy

- There is **no `latest` synonym** on the extension OCI artifact — pin by `pg`×`arch` explicitly (e.g. `0.5.10-pg17-amd64`).
- Tagged versions are immutable on GHCR.
- The aggregate `vX.Y.Z` / `X.Y.Z` index references all 8 per-PG×arch leaves for that release; pull it to let your client pick.
- Per [`PROVENANCE.md`](./PROVENANCE.md) Rule 2: do not consider an artifact "shipped" if its digest does not verify under `gh attestation verify`.

See [`CHANGELOG.md`](./CHANGELOG.md) and [`RELEASE_NOTES.md`](./RELEASE_NOTES.md) for what changed per version.
