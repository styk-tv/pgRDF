# scripts/

Release tooling. None of it is needed to build or use pgRDF.

| Script | Used by | What it does |
|---|---|---|
| `pre-tag-check.sh <version>` | maintainer, before pushing a tag | Checks that every file carrying the release version (`Cargo.toml`, `Cargo.lock`, `pgrdf.control`, `META.json`, …) says `<version>`. Exit 0 means it is safe to tag. |
| `gh-watch.sh [tag]` | maintainer, after pushing a tag | Waits for the release chain for that tag (`release.yml` → `oci-publish.yml` → `update-latest-md.yml`) and prints the outcome. Defaults to the most recent local tag. Needs `gh`. |
| `render-latest-md.py` | CI (`update-latest-md.yml`) | Renders `LATEST.md` from the published, attestation-verified GHCR digests. Reads `VER` from the environment and calls `gh api`. |
| `validate-bundle.sh <dir\|tarball> [version]` | anyone holding a release bundle | Checks that a bundle's `MANIFEST.json` agrees with its control file and SQL files, and optionally with an expected version. Needs `jq`. |

## Cutting a release

```sh
git checkout main && git pull
scripts/pre-tag-check.sh 0.6.35            # on the commit you are about to tag
git tag -a v0.6.35 -m "…" && git push origin v0.6.35
scripts/gh-watch.sh v0.6.35                # done when LATEST.md is refreshed
```

## Checking a downloaded bundle

```sh
scripts/validate-bundle.sh pgrdf-0.6.34-pg18-glibc-amd64.tar.gz 0.6.34
```
