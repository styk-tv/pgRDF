# pgRDF — latest published artifacts

One publishable surface ships from this repo: the PostgreSQL **extension** (oras-pulled OCI artifact). This file tracks the head on **PostgreSQL 17**. Older PG majors (14, 15, 16) are still built per release — see [Repo packages view](https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle) for the full matrix.

## pgRDF extension — `v0.5.9` (PostgreSQL 17)

`oras pull ghcr.io/styk-tv/pgrdf-bundle:0.5.9-pg17-<arch>` → drop `lib/pgrdf.so` + `share/extension/{pgrdf.control, pgrdf--0.5.9.sql}` next to your `postgres:17` install.

| arch  | Pull URI                                            | Digest                                                                  | Created (UTC)       |
|-------|-----------------------------------------------------|-------------------------------------------------------------------------|---------------------|
| amd64 | `ghcr.io/styk-tv/pgrdf-bundle:0.5.9-pg17-amd64`     | `sha256:cc0a9166f1eca5adcf133a996b2d65e7b0289eddad8ca3c70e3f609a9aa7a8a4` | 2026-05-28 17:41:46 |
| arm64 | `ghcr.io/styk-tv/pgrdf-bundle:0.5.9-pg17-arm64`     | `sha256:fcd713f85fa02e1082ab7d952fb564710ce91d5bccbf190c656e6f827ab4bbcf` | 2026-05-28 17:41:49 |

|                       |                                                                          |
|-----------------------|--------------------------------------------------------------------------|
| Artifact type         | `application/vnd.styk.pgrdf.bundle.v1+tar`                               |
| Aggregate index       | `ghcr.io/styk-tv/pgrdf-bundle:0.5.9` (also tagged `v0.5.9`)              |
| Tarball mirror        | https://github.com/styk-tv/pgRDF/releases/tag/v0.5.9                     |
| Repo packages view    | https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle             |
| Older PG majors       | `0.5.9-pg{14,15,16}-{amd64,arm64}` published alongside; same v0.5.9 tag  |

## Pin policy

- There is **no `latest` synonym** on the extension OCI artifact — pin by `pg`×`arch` explicitly (e.g. `0.5.9-pg17-amd64`).
- Tagged versions are immutable on GHCR.
- The aggregate `vX.Y.Z` / `X.Y.Z` index references all 8 per-PG×arch leaves for that release; pull it to let your client pick.

See [`CHANGELOG.md`](./CHANGELOG.md) and [`RELEASE_NOTES.md`](./RELEASE_NOTES.md) for what changed per version.
