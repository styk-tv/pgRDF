#!/usr/bin/env bash
#
# fixtures/ontologies.sh — pull every URL listed in TEST.ONTOLOGY-SET.md
# into fixtures/ontologies/<name>.ttl and emit a manifest with sha256 +
# size per file.
#
# Usage:
#   fixtures/ontologies.sh             # fetch all 17
#   fixtures/ontologies.sh --resume    # skip files already present
#
# The fixtures/ontologies/ directory is gitignored; the manifest is
# committed so other contributors get the expected sha256s without
# re-downloading.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
LIST="${REPO_ROOT}/TEST.ONTOLOGY-SET.md"
OUT_DIR="${REPO_ROOT}/fixtures/ontologies"
MANIFEST="${REPO_ROOT}/fixtures/ontologies.manifest.json"
RESUME="0"

for arg in "$@"; do
  case "${arg}" in
    --resume) RESUME="1" ;;
    -h|--help) sed -n '4,12p' "$0"; exit 0 ;;
    *) echo "unknown arg: ${arg}"; exit 1 ;;
  esac
done

mkdir -p "${OUT_DIR}"

# Derive a filesystem-safe local name from the URL. Last path segment,
# stripped of query string, lowercased. Falls back to a sha-prefixed
# name if the URL ends with `/`.
local_name() {
  local url="$1"
  local base
  base="$(basename "${url%%\?*}")"
  base="${base,,}"
  if [ -z "${base}" ] || [ "${base}" = "/" ]; then
    base="$(printf '%s' "${url}" | shasum -a 256 | cut -c1-12).ttl"
  fi
  # Make sure it's .ttl-ish (some servers return .ttl content from
  # URLs ending in .TTL or no extension).
  case "${base}" in
    *.ttl|*.TTL|*.nt|*.n3|*.rdf|*.owl) ;;
    *) base="${base}.ttl" ;;
  esac
  printf '%s' "${base}"
}

declare -a entries=()
total=0
fetched=0
skipped=0
failed=0

# `|| [ -n "${line}" ]` catches the last line of a file that doesn't
# end with a newline (which is the default state of many editors).
while IFS= read -r line || [ -n "${line}" ]; do
  case "${line}" in
    http*) ;;
    *) continue ;;
  esac
  total=$((total + 1))

  url="${line}"
  name="$(local_name "${url}")"
  out="${OUT_DIR}/${name}"

  if [ "${RESUME}" = "1" ] && [ -f "${out}" ]; then
    skipped=$((skipped + 1))
    sha="$(shasum -a 256 "${out}" | cut -d' ' -f1)"
    size="$(stat -f%z "${out}" 2>/dev/null || stat -c%s "${out}")"
    entries+=("{\"url\":\"${url}\",\"file\":\"ontologies/${name}\",\"sha256\":\"${sha}\",\"size\":${size}}")
    printf '  SKIP   %-45s (already on disk)\n' "${name}"
    continue
  fi

  printf '  FETCH  %-45s' "${name}"
  if curl -fsSL --max-time 60 -o "${out}.tmp" "${url}"; then
    mv "${out}.tmp" "${out}"
    sha="$(shasum -a 256 "${out}" | cut -d' ' -f1)"
    size="$(stat -f%z "${out}" 2>/dev/null || stat -c%s "${out}")"
    printf ' %8d bytes  sha=%s...\n' "${size}" "${sha:0:12}"
    fetched=$((fetched + 1))
    entries+=("{\"url\":\"${url}\",\"file\":\"ontologies/${name}\",\"sha256\":\"${sha}\",\"size\":${size}}")
  else
    rm -f "${out}.tmp"
    printf '   FAILED\n'
    failed=$((failed + 1))
  fi
done < "${LIST}"

# Emit the manifest. JSON pretty-printed with newlines between entries
# for readable diffs.
{
  printf '{\n  "fetched_at": "%s",\n  "entries": [\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  for i in "${!entries[@]}"; do
    if [ "$i" -lt $((${#entries[@]} - 1)) ]; then
      printf '    %s,\n' "${entries[$i]}"
    else
      printf '    %s\n' "${entries[$i]}"
    fi
  done
  printf '  ]\n}\n'
} > "${MANIFEST}"

echo
echo "Summary: total=${total}  fetched=${fetched}  skipped=${skipped}  failed=${failed}"
echo "Manifest: ${MANIFEST}"
[ "${failed}" -eq 0 ]
