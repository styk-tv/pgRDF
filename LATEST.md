# pgRDF — latest published artifacts

One publishable surface ships from this repo: the PostgreSQL **extension** (oras-pulled OCI artifact). This file tracks the head on **PostgreSQL 17**. Older PG majors (14, 15, 16) are still built per release — see [Repo packages view](https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle) for the full matrix.

## pgRDF extension — `v0.5.8` (PostgreSQL 17)

`oras pull ghcr.io/styk-tv/pgrdf-bundle:0.5.8-pg17-<arch>` → drop `lib/pgrdf.so` + `share/extension/{pgrdf.control, pgrdf--0.5.8.sql}` next to your `postgres:17` install.

| arch  | Pull URI                                            | Digest                                                                  | Created (UTC)       |
|-------|-----------------------------------------------------|-------------------------------------------------------------------------|---------------------|
| amd64 | `ghcr.io/styk-tv/pgrdf-bundle:0.5.8-pg17-amd64`     | `sha256:b7a5cd2bd37942c00d354f0241f1138b609bae80ab2ab2f30e35242155a1cc3b` | 2026-05-28 16:23:13 |
| arm64 | `ghcr.io/styk-tv/pgrdf-bundle:0.5.8-pg17-arm64`     | `sha256:a4e39ec274fb81fe1efdbd3b09022bda75467fdc4cd6ea75d4e4e04cc556735c` | 2026-05-28 16:23:16 |

|                       |                                                                          |
|-----------------------|--------------------------------------------------------------------------|
| Artifact type         | `application/vnd.styk.pgrdf.bundle.v1+tar`                               |
| Aggregate index       | `ghcr.io/styk-tv/pgrdf-bundle:0.5.8` (also tagged `v0.5.8`)              |
| Tarball mirror        | https://github.com/styk-tv/pgRDF/releases/tag/v0.5.8                     |
| Repo packages view    | https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle             |
| Older PG majors       | `0.5.8-pg{14,15,16}-{amd64,arm64}` published alongside; same v0.5.8 tag  |

## Pin policy

- There is **no `latest` synonym** on the extension OCI artifact — pin by `pg`×`arch` explicitly (e.g. `0.5.8-pg17-amd64`).
- Tagged versions are immutable on GHCR.
- The aggregate `vX.Y.Z` / `X.Y.Z` index references all 8 per-PG×arch leaves for that release; pull it to let your client pick.

See [`CHANGELOG.md`](./CHANGELOG.md) and [`RELEASE_NOTES.md`](./RELEASE_NOTES.md) for what changed per version.
