# Latest published OCI bundle

Latest = **0.5.0** (4 PostgreSQL majors × 2 architectures, behind one
aggregate index)

|  |  |
|---|---|
| Pull URI | `ghcr.io/styk-tv/pgrdf-bundle:0.5.0` |
| Also tagged | `v0.5.0` |
| Index digest | `sha256:d25560197296f396718e8bf997d851cbcad5599bf2cb11d52e4e1e06e958f00f` |
| Created (UTC) | 2026-05-16 19:48:26 |
| PostgreSQL majors | 14, 15, 16, 17 |
| Architectures | linux/amd64, linux/arm64 |
| Leaf artifacts | 8 (`0.5.0-pg{14,15,16,17}-{amd64,arm64}`) |
| GHCR view | <https://github.com/users/styk-tv/packages/container/package/pgrdf-bundle> |
| Repo packages view | <https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle> |

## Per-PG × arch leaf artifacts

| Tag | Digest (short) |
|---|---|
| `0.5.0-pg17-arm64` | `sha256:733cd…` |
| `0.5.0-pg17-amd64` | `sha256:e0484…` |
| `0.5.0-pg16-arm64` | `sha256:19141…` |
| `0.5.0-pg16-amd64` | `sha256:8369c…` |
| `0.5.0-pg15-arm64` | `sha256:e0868…` |
| `0.5.0-pg15-amd64` | `sha256:42eab…` |
| `0.5.0-pg14-arm64` | `sha256:ed9dc…` |
| `0.5.0-pg14-amd64` | `sha256:5a9e6…` |

Pull a specific PG-major + arch directly:

```bash
oras pull ghcr.io/styk-tv/pgrdf-bundle:0.5.0-pg17-arm64
```

Or pull the aggregate index and let your client pick:

```bash
oras pull ghcr.io/styk-tv/pgrdf-bundle:0.5.0
```

## Notes on `Latest` lag

The most recent git tag at the time of writing is **v0.5.8**; the OCI
bundle above is **v0.5.0**. `.github/workflows/oci-publish.yml` runs on
the `release: [published]` event but has fired exactly once
(v0.5.0 ⇒ 2026-05-16). v0.5.1 → v0.5.8 are tagged + have GitHub releases
but did not trigger the OCI publish. Follow-up tracker noted separately;
once the trigger condition is fixed, this file updates to whatever the
new latest is.
