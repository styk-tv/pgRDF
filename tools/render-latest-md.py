#!/usr/bin/env python3
"""Regenerate LATEST.md from the GHCR head of `ghcr.io/styk-tv/pgrdf-bundle`.

Called by `.github/workflows/update-latest-md.yml` AFTER SLSA Build Provenance
v1 attestation verification succeeds for the head digest(s). Re-renders the
whole LATEST.md every time — pgRDF ships a single OCI surface so there is no
"preserve the other side" logic the pgCK sibling renderer needs.

Env:
  VER                      pgRDF version (no ``v`` prefix; e.g. ``0.5.10``)
  GH_TOKEN                 GitHub token with ``packages:read`` + ``actions:read``
                           (the latter for `Built by` resolution)
  GITHUB_REPOSITORY_OWNER  GH owner slug (defaults to ``styk-tv``)
  BUILT_BY_URL             (optional) workflow-run HTML URL for the
                           `Built by` field. If unset, the renderer resolves
                           it via the GitHub API (latest ``release.yml`` run
                           against the tag SHA). If both fail it falls back
                           to the repo actions index and tags the fallback in
                           a renderer comment in the output.
  BUILT_FROM_SHA           (optional) 40-char commit SHA for the tag. If
                           unset, resolved via ``git rev-list -n 1 v<VER>``
                           then ``gh api .../git/ref/tags/v<VER>``.

Output: full LATEST.md content on stdout.

Bootstrap discipline: this renderer trusts the workflow's `gh attestation
verify` step. v0.5.0–v0.5.9 will never be passed in as ``VER`` because their
digests do not verify — the workflow exits before reaching this renderer.
The bootstrap exception (PROVENANCE.md) is enforced upstream, not here.

SPEC.OCI.BUNDLE.v0.3 §2.2 fields emitted in addition to v0.2 set:
- per-arch ``Also tagged`` column (renders ``—`` for Shape B leaves: pgRDF
  does not currently apply aliases to per-arch leaves; only the aggregate
  index carries ``v<VER>`` as an alias of ``<VER>``).
- ``Built by`` row (workflow-run URL of the ``release.yml`` invocation that
  kicked off ``oci-publish.yml``).
- ``Built from commit`` row (linked abbreviated SHA → repo commit URL).
- ``Release notes`` row (GitHub release URL; kept in addition to the
  existing ``Tarball mirror`` row — v0.3 §2.2 lists them as distinct
  semantic fields even if the URL happens to coincide for pgRDF).
"""

from __future__ import annotations

import datetime
import json
import os
import subprocess
import sys
from typing import Tuple


def gh_api(path: str) -> list[dict]:
    r = subprocess.run(
        ["gh", "api", path, "--paginate"],
        capture_output=True,
        text=True,
        check=True,
    )
    return json.loads(r.stdout)


def gh_api_one(path: str, jq: str | None = None) -> str:
    """Call `gh api` for a single response (no --paginate, no list wrap).

    Returns stdout as a stripped string. Used for `/git/ref/tags/...` and
    `/actions/runs?...` lookups where the renderer wants either a single
    object's field or a small list to filter in-process. ``jq`` is applied
    raw via ``--jq`` so callers can extract a specific subfield.

    Returns empty string on ANY failure (network, 404, auth) — callers
    are expected to handle "couldn't resolve" by falling back, not by
    bubbling the exception up.
    """
    cmd = ["gh", "api", path]
    if jq is not None:
        cmd += ["--jq", jq]
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, check=True)
    except (subprocess.CalledProcessError, FileNotFoundError):
        return ""
    return r.stdout.strip()


def find_version(packages: list[dict], tag_filter: str) -> Tuple[str, str]:
    """Find the GHCR version whose tag list contains ``tag_filter``.

    Returns (digest, created_at_iso). Raises SystemExit if not found — that
    means the workflow's bookkeeping is wrong (asked us to render a VER for
    which no GHCR version exists).
    """
    for v in packages:
        tags = v.get("metadata", {}).get("container", {}).get("tags", [])
        if tag_filter in tags:
            digest = v["name"]
            if not digest.startswith("sha256:"):
                raise SystemExit(f"unexpected digest format: {digest}")
            return digest, v.get("created_at", "")
    raise SystemExit(f"no GHCR version found with tag {tag_filter!r}")


def fmt_ts(iso: str) -> str:
    return iso.replace("T", " ").replace("Z", "").split(".")[0]


def resolve_built_from_sha(owner: str, repo: str, ver: str) -> str:
    """Resolve the 40-char commit SHA for tag ``v<ver>``.

    Order: env override → local ``git rev-list -n 1 v<ver>`` → ``gh api
    /repos/<owner>/<repo>/git/ref/tags/v<ver>``. Returns empty string if
    all three paths fail (caller renders an unlinked placeholder).
    """
    sha = os.environ.get("BUILT_FROM_SHA", "").strip()
    if sha:
        return sha
    try:
        r = subprocess.run(
            ["git", "rev-list", "-n", "1", f"v{ver}"],
            capture_output=True,
            text=True,
            check=True,
        )
        sha = r.stdout.strip()
        if sha and len(sha) == 40:
            return sha
    except (subprocess.CalledProcessError, FileNotFoundError):
        pass
    sha = gh_api_one(
        f"repos/{owner}/{repo}/git/ref/tags/v{ver}", jq=".object.sha"
    )
    return sha


def resolve_built_by_url(owner: str, repo: str, sha: str) -> Tuple[str, bool]:
    """Resolve the `Built by` workflow-run HTML URL for tag SHA.

    Returns (url, resolved_cleanly). When ``resolved_cleanly`` is False,
    the caller (renderer) should annotate the output that the URL is a
    fallback to the repo's generic actions index.

    Env override: ``BUILT_BY_URL`` short-circuits the API call.
    """
    url = os.environ.get("BUILT_BY_URL", "").strip()
    if url:
        return url, True
    if sha:
        # Filter to release.yml runs at the tag's SHA; pick most recent.
        # gh's --jq is jq-syntax; we sort descending by created_at.
        jq = (
            "[.workflow_runs[] | "
            'select(.path == ".github/workflows/release.yml")] '
            "| sort_by(.created_at) | reverse | .[0].html_url // \"\""
        )
        url = gh_api_one(
            f"repos/{owner}/{repo}/actions/runs?event=push&head_sha={sha}",
            jq=jq,
        )
        if url:
            return url, True
    return f"https://github.com/{owner}/{repo}/actions", False


def render(owner: str, ver: str) -> str:
    repo = "pgRDF"  # publish-side repo slug; constant for this renderer
    pkgs = gh_api(f"/users/{owner}/packages/container/pgrdf-bundle/versions")
    amd_d, amd_t = find_version(pkgs, f"{ver}-pg17-amd64")
    arm_d, arm_t = find_version(pkgs, f"{ver}-pg17-arm64")
    agg_d, _ = find_version(pkgs, ver)  # the bare ``X.Y.Z`` aggregate tag

    # SPEC.OCI.BUNDLE.v0.3 §2.2 fields.
    built_from_sha = resolve_built_from_sha(owner, repo, ver)
    built_by_url, built_by_clean = resolve_built_by_url(
        owner, repo, built_from_sha
    )
    if built_from_sha:
        built_from_cell = (
            f"[`{built_from_sha[:12]}`]"
            f"(https://github.com/{owner}/{repo}/commit/{built_from_sha})"
        )
    else:
        built_from_cell = "_unresolved_"
    if built_by_clean:
        built_by_cell = f"[Workflow run]({built_by_url})"
    else:
        # Fallback: linked but flagged as approximate. The renderer
        # leaves an HTML comment trail so a reviewer can spot the
        # degraded case without parsing the URL.
        built_by_cell = (
            f"[Actions]({built_by_url}) "
            "<!-- fallback: could not resolve specific release.yml run -->"
        )

    # SPEC.OCI.BUNDLE.v0.3 §2.2 per-arch ``Also tagged`` column. pgRDF
    # currently applies no aliases to Shape B per-arch leaves (only the
    # aggregate index carries ``v<VER>`` alongside ``<VER>``). Render
    # ``—`` for now; if aliases are introduced later, replace with a
    # comma-separated tag list per leaf.
    also_tagged_amd = "—"
    also_tagged_arm = "—"

    now = datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")
    return f"""<!--
  This file is auto-generated by .github/workflows/update-latest-md.yml after a
  successful oci-publish.yml run AND after SLSA Build Provenance v1
  attestations have been verified against every GHCR digest below. Do NOT edit
  by hand — the next workflow run will overwrite your changes. Last refresh:
  {now} (version: v{ver}).
-->

# pgRDF — latest published artifacts

One publishable surface ships from this repo: the PostgreSQL **extension** (oras-pulled OCI artifact). This file tracks the head on **PostgreSQL 17**. Builds for pg14 / pg15 / pg16 are PAUSED during a stabilization window (see [CHANGELOG.md "Changed (stabilization window)"](./CHANGELOG.md) for context) — they will resume once the multi-PG matrix is stable again. The [Repo packages view](https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle) shows everything currently published.

## pgRDF extension — `v{ver}` (PostgreSQL 17)

Every digest below carries a verifiable SLSA Build Provenance v1 attestation per [`PROVENANCE.md`](./PROVENANCE.md). v0.5.0–v0.5.9 predate the attestation wiring and never appear here.

`oras pull ghcr.io/styk-tv/pgrdf-bundle:{ver}-pg17-<arch>` → drop `lib/pgrdf.so` + `share/extension/{{pgrdf.control, pgrdf--{ver}.sql}}` next to your `postgres:17` install.

| arch  | Pull URI                                             | Also tagged | Digest                                                                  | Created (UTC)       |
|-------|------------------------------------------------------|-------------|-------------------------------------------------------------------------|---------------------|
| amd64 | `ghcr.io/styk-tv/pgrdf-bundle:{ver}-pg17-amd64`     | {also_tagged_amd}           | `{amd_d}` | {fmt_ts(amd_t)} |
| arm64 | `ghcr.io/styk-tv/pgrdf-bundle:{ver}-pg17-arm64`     | {also_tagged_arm}           | `{arm_d}` | {fmt_ts(arm_t)} |

|                       |                                                                                                |
|-----------------------|------------------------------------------------------------------------------------------------|
| Artifact type         | `application/vnd.styk.pgrdf.bundle.v1+tar`                                                     |
| Aggregate index       | `ghcr.io/styk-tv/pgrdf-bundle:{ver}` (also tagged `v{ver}`)                                  |
| Aggregate digest      | `{agg_d}` |
| Provenance            | SLSA Build Provenance v1, Sigstore-backed, pushed as OCI referrer                              |
| Built by              | {built_by_cell}                                                                                |
| Built from commit     | {built_from_cell}                                                                              |
| Verify (CLI)          | `gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:{ver} --repo styk-tv/pgRDF`         |
| Release notes         | https://github.com/styk-tv/pgRDF/releases/tag/v{ver}                                          |
| Tarball mirror        | https://github.com/styk-tv/pgRDF/releases/tag/v{ver}                                          |
| Repo packages view    | https://github.com/styk-tv/pgRDF/pkgs/container/pgrdf-bundle                                   |
| Older PG majors       | PAUSED during the stabilization window — pg14 / pg15 / pg16 leaves are NOT published for v{ver}. Resumes per CHANGELOG.md once matrix is stable.   |

## Verifying any artifact above

```sh
# Aggregate index (multi-arch, multi-PG-major)
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:{ver} \\
  --repo styk-tv/pgRDF

# A specific PG×arch leaf
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:{ver}-pg17-amd64 \\
  --repo styk-tv/pgRDF
```

A successful verify means: signed by GitHub's Fulcio CA against the OIDC token of the v{ver} `oci-publish` workflow run, recorded in Sigstore's Rekor transparency log, subject digest matches the pulled artifact.

## Pin policy

- There is **no `latest` synonym** on the extension OCI artifact — pin by `pg`×`arch` explicitly (e.g. `{ver}-pg17-amd64`).
- Tagged versions are immutable on GHCR.
- The aggregate `vX.Y.Z` / `X.Y.Z` index references all 8 per-PG×arch leaves for that release; pull it to let your client pick.
- Per [`PROVENANCE.md`](./PROVENANCE.md) Rule 2: do not consider an artifact "shipped" if its digest does not verify under `gh attestation verify`.

See [`CHANGELOG.md`](./CHANGELOG.md) and [`RELEASE_NOTES.md`](./RELEASE_NOTES.md) for what changed per version.
"""


def main() -> None:
    owner = os.environ.get("GITHUB_REPOSITORY_OWNER", "styk-tv")
    ver = os.environ.get("VER", "").strip()
    if not ver:
        raise SystemExit("VER environment variable is required (e.g. 0.5.10)")
    if ver.startswith("v"):
        ver = ver[1:]
    sys.stdout.write(render(owner, ver))


if __name__ == "__main__":
    main()
