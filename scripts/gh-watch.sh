#!/usr/bin/env bash
# gh-watch.sh — wait for a GitHub Actions release chain, notify on completion.
#
# Three modes selected by first arg (called by Claude Code hooks or shell):
#   hook         PostToolUse filter (reads tool JSON from stdin; spawns watcher)
#   surface      UserPromptSubmit hook (prepends completed logs to next prompt)
#   watch <tag>  the watcher itself (also runnable from shell)
#
# Wiring in .claude/settings.json:
#   { "hooks": {
#     "PostToolUse":      [{"matcher":"Bash","hooks":[{"type":"command","command":"scripts/gh-watch.sh hook"}]}],
#     "UserPromptSubmit": [{"hooks":[{"type":"command","command":"scripts/gh-watch.sh surface"}]}]
#   } }
#
# Chain (per PROVENANCE.md): release.yml → oci-publish.yml → update-latest-md.yml.
# release.yml is on tag push (headSha == tag SHA, so SHA-keyed). The next two
# hops are triggered by repository_dispatch, which carries the default-branch
# headSha — they are correlated forward by "first matching run created after
# the previous run started". SHA-keyed on the entry hop is enough to keep
# parallel pushes from different shells from racing onto the same --limit 1
# lookup; the chain hops are then deterministic per release.

set -euo pipefail
mode="${1:-}"; shift || true

case "$mode" in

hook)
  cmd=$(jq -r '.tool_input.command // empty' 2>/dev/null || true)
  [ -z "$cmd" ] && exit 0
  case "$cmd" in *"git push"*"origin"*"v"*) ;; *) exit 0 ;; esac
  tag=$(printf '%s' "$cmd" | grep -oE 'origin v[0-9][^ ]+' | awk '{print $NF}' | head -1)
  [ -z "$tag" ] && exit 0
  self="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
  safe="${tag//\//_}"
  nohup "$self" watch "$tag" >"/tmp/gh-watch-${safe}.log" 2>&1 &
  ;;

surface)
  mkdir -p /tmp/gh-watch-seen
  n=0
  for log in /tmp/gh-watch-*.log; do
    [ -f "$log" ] || continue
    safe="$(basename "$log" .log | sed 's/^gh-watch-//')"
    marker="/tmp/gh-watch-seen/$safe"
    [ -f "$marker" ] && [ ! "$log" -nt "$marker" ] && continue
    grep -qE '^(✓ Release in:|✗ Release FAILED:)' "$log" || continue
    [ $n -eq 0 ] && echo "──────── gh-watch ────────"
    grep -E '^(▶ Tag:|  Tag SHA:|✓ Release in:|✗ Release FAILED:)' "$log" || true
    echo
    touch "$marker"
    n=$((n + 1))
  done
  [ $n -gt 0 ] && echo "(full: /tmp/gh-watch-<tag>.log)"
  exit 0
  ;;

watch)
  tag="${1:-$(git describe --tags --abbrev=0 2>/dev/null || true)}"
  [ -z "$tag" ] && { echo "Usage: $0 watch <tag>" >&2; exit 2; }

  notify() {
    command -v osascript >/dev/null 2>&1 || return 0
    osascript -e "display notification \"$2\" with title \"release\" sound name \"$1\"" 2>/dev/null || true
  }
  trap 'echo "✗ Release FAILED: $tag" >&2; notify Sosumi "$tag chain failed"' ERR

  case "$tag" in
    v*) ;;
    *) echo "Unknown tag pattern: $tag (expected v*)" >&2; exit 2 ;;
  esac

  echo "▶ Tag: $tag"
  sha=$(git rev-list -n1 "$tag" 2>/dev/null || true)
  [ -z "$sha" ] && { echo "  Cannot resolve $tag SHA" >&2; exit 2; }
  echo "  Tag SHA: ${sha:0:12}"

  # Entry hop: release.yml is on tag push, so headSha == tag SHA.
  # SHA-keyed lookup means parallel pushes from different shells never race.
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

  # Chain hops: oci-publish.yml and update-latest-md.yml are both triggered
  # by repository_dispatch (event_type=oci-publish-release and
  # event_type=latest-md-refresh respectively). repository_dispatch runs
  # carry the default-branch headSha, NOT the tag SHA — so we correlate
  # forward by "first dispatch-triggered run created at or after $after".
  # Polls up to 60s before giving up.
  find_run_after() {
    local workflow="$1" after="$2" run=""
    for _ in $(seq 1 20); do
      run=$(gh run list --workflow="$workflow" --limit 20 \
        --json databaseId,event,createdAt \
        --jq "[.[] | select(.event == \"repository_dispatch\" and .createdAt >= \"$after\")] | sort_by(.createdAt) | .[0].databaseId" 2>/dev/null)
      [ -n "$run" ] && [ "$run" != "null" ] && { echo "$run"; return 0; }
      sleep 3
    done
    return 1
  }

  # Capture the dispatch-anchor timestamp BEFORE waiting on release.yml,
  # so a fast oci-publish.yml that fires while release.yml is still
  # finalising still gets picked up.
  anchor=$(date -u +%Y-%m-%dT%H:%M:%SZ)

  initial=$(find_run_by_sha release.yml) || { echo "  No release.yml run for $sha after 30s" >&2; exit 1; }
  echo "  ▶ release.yml: $initial"
  gh run watch "$initial" --exit-status

  oci_run=$(find_run_after oci-publish.yml "$anchor") || { echo "  oci-publish run for $tag not seen after 60s" >&2; exit 1; }
  echo "  ▶ oci-publish.yml: $oci_run"
  gh run watch "$oci_run" --exit-status

  latest_run=$(find_run_after update-latest-md.yml "$anchor") || { echo "  update-latest-md run for $tag not seen after 60s" >&2; exit 1; }
  echo "  ▶ update-latest-md.yml: $latest_run"
  gh run watch "$latest_run" --exit-status

  echo
  echo "✓ Release in: $tag"
  echo
  sed -n '1,20p' LATEST.md
  notify Glass "$tag chain landed"
  ;;

*)
  echo "Usage: $0 {hook|surface|watch <tag>}" >&2
  exit 2
  ;;

esac
