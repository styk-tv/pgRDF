#!/usr/bin/env bash
# gh-watch.sh <tag> — wait for the GitHub Actions release chain for <tag>,
# print the outcome, fire a macOS notification.
#
# Run it BACKGROUNDED from a Claude Code Bash call (run_in_background: true):
# the harness re-invokes the agent with this script's output when the chain
# settles. No hooks, no temp files, no settings.json. Pattern verified in
# pgCK (SPEC.CLAUDE.GH-WATCH.v0.2); adapted for pgRDF's three-hop chain.
#
# Chain (per PROVENANCE.md): release.yml -> oci-publish.yml -> update-latest-md.yml.
# A release is "in" only when update-latest-md.yml has rewritten LATEST.md
# (Rule 2 + Rule 3). SHA-keyed on the entry hop so parallel pushes of
# different tags never cross.
#
#   scripts/gh-watch.sh v0.5.18
#   scripts/gh-watch.sh            # most recent local tag (git describe)

set -euo pipefail

tag="${1:-$(git describe --tags --abbrev=0 2>/dev/null || true)}"
[ -z "$tag" ] && { echo "Usage: $0 <tag>" >&2; exit 2; }

notify() {
  command -v osascript >/dev/null 2>&1 || return 0
  osascript -e "display notification \"$2\" with title \"pgRDF release\" sound name \"$1\"" 2>/dev/null || true
}
trap 'echo "✗ Release FAILED: $tag"; notify Sosumi "$tag chain failed"' ERR

case "$tag" in
  v*) wf="release.yml" ;;
  *) echo "Unknown tag pattern: $tag" >&2; exit 2 ;;
esac

echo "▶ Watching release chain for $tag ($wf -> oci-publish.yml -> update-latest-md.yml)"
sha=$(git rev-list -n1 "$tag" 2>/dev/null || true)
[ -z "$sha" ] && { echo "Cannot resolve $tag SHA (pushed yet?)" >&2; exit 2; }
echo "  SHA ${sha:0:12}"

# Find the entry-hop run (SHA-keyed; anti-race for parallel pushes).
find_run_by_sha() {
  local workflow="$1" run=""
  for _ in $(seq 1 10); do
    run=$(gh run list --workflow="$workflow" --limit 20 --json databaseId,headSha \
      --jq ".[] | select(.headSha == \"$sha\") | .databaseId" | head -1)
    [ -n "$run" ] && { echo "$run"; return 0; }
    sleep 3
  done
  return 1
}

# Find a downstream-hop run (repository_dispatch triggered; SHA matches main,
# not tag, so anchor by created_at >= start of upstream run instead).
find_dispatch_run() {
  local workflow="$1" anchor="$2" run=""
  for _ in $(seq 1 20); do
    run=$(gh run list --workflow="$workflow" --limit 20 --json databaseId,event,createdAt \
      --jq ".[] | select(.event == \"repository_dispatch\" and .createdAt >= \"$anchor\") | .databaseId" | head -1)
    [ -n "$run" ] && { echo "$run"; return 0; }
    sleep 3
  done
  return 1
}

# Hop 1: release.yml (event=push, SHA-keyed).
initial=$(find_run_by_sha "$wf") || { echo "✗ no $wf run for $sha after 30s"; exit 1; }
echo "  release.yml run $initial"
anchor=$(gh run view "$initial" --json createdAt --jq '.createdAt')
gh run watch "$initial" --exit-status

# Hop 2: oci-publish.yml (event=repository_dispatch, anchored to release start).
oci=$(find_dispatch_run oci-publish.yml "$anchor") || { echo "✗ oci-publish.yml run not seen after 60s"; exit 1; }
echo "  oci-publish.yml run $oci"
gh run watch "$oci" --exit-status

# Hop 3: update-latest-md.yml (event=repository_dispatch, anchored to release start).
chain=$(find_dispatch_run update-latest-md.yml "$anchor") || { echo "✗ update-latest-md.yml run not seen after 60s"; exit 1; }
echo "  update-latest-md.yml run $chain"
gh run watch "$chain" --exit-status

echo "✓ Release in: $tag"
sed -n '1,20p' LATEST.md
notify Glass "$tag chain landed"
