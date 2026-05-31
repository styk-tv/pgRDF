# pgRDF release notes

Per-release notes have moved to two surfaces, each more reliable than this file ever was:

- **GitHub Releases** — every tagged release at <https://github.com/styk-tv/pgRDF/releases>. As of v0.5.29, the release body is rendered by `release.yml` from the **annotated tag message** the maintainer wrote at `git tag -a -F`. That body is per-release-current by construction; a `pre-publish` gate in the workflow refuses to publish if the body doesn't even mention the tag.
- **`CHANGELOG.md`** at the repo root — the cumulative record, Keep a Changelog format. The `[Unreleased]` section accumulates work in flight; each tag annotation reflects whatever was Unreleased at tag time.

## Why this file is just a pointer now

Before v0.5.29 the release pipeline used `body_path: RELEASE_NOTES.md` in `release.yml`. That file was rewritten per release by hand and **drifted** — it was last edited for v0.5.1 (2026-05-23). Every subsequent release (v0.5.10..v0.5.28, 19 advertised + orphaned tags) inherited the same stale "v0.5.1 — PGXN, artifact parity, and MIT cleanup" body on GitHub. OCI-GERMINATION flagged this on 2026-05-31; v0.5.29 ships the fix.

## What the fix is

`release.yml` now extracts the body from `git for-each-ref refs/tags/<tag> --format='%(contents)'` (the annotated tag message), then fails the publish if that body doesn't contain the tag's version string. The gate is described in the workflow comments and lives at `.github/workflows/release.yml`, step "Pre-publish — assert Release body mentions tag".

## Where this file is still referenced

- `LATEST.md` footer carries a link to this file (rendered by `tools/render-latest-md.py`). The link resolves to this page (which says "see GitHub Releases or CHANGELOG.md") rather than to stale 2026-05-23 content.

## Historic snapshots

The pre-2026-05-31 v0.5.1 body of this file is preserved in git history. To see what shipped at any particular tag prior to v0.5.29, prefer the GitHub Release page for that tag — though note that pages for v0.5.10..v0.5.28 inherited this file's stale text and are being backfilled as a one-time clean-up step with the actual tag annotation content (see v0.5.29 commit messages for the `gh release edit` steps).
