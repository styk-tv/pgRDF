#!/usr/bin/env bash
#
# tests/w3c-shacl/run.sh — W3C SHACL conformance harness.
#
# v0.5-FUTURE §6. The third correctness gate, alongside the W3C
# SPARQL-shape harness (tests/w3c-sparql/run.sh) and pg_regress.
#
# Mirrors the W3C-SPARQL harness shape: a vendored set of W3C SHACL
# test fixtures (hermetic — checked into the repo, NOT fetched at
# test time) plus a hand-derived expected file per test. Each fixture
# is a self-contained W3C `data-shapes-test-suite` `.ttl` (data +
# shapes + an `mf:Manifest` whose `mf:result` carries the
# spec-authoritative `sh:ValidationReport`). Per the W3C suite
# convention the test's `sht:dataGraph <>` and `sht:shapesGraph <>`
# both point at the file itself, so the harness loads the whole `.ttl`
# into ONE pgRDF graph and validates it against itself: the SHACL
# engine acts only on `sh:*` shapes + their targets and ignores the
# `mf:` / `sht:` manifest triples (they declare no SHACL constraint).
#
# Comparison invariant — `{conforms}`:
#   * `conforms` — the headline W3C `sh:conforms` boolean, hand-derived
#                  from each fixture's `mf:result` block (the
#                  W3C-authoritative answer, NEVER auto-blessed from
#                  validator output).
# Focus-node IRIs are intentionally NOT compared, and the violation
# count is shown for diagnostics only. The W3C fixtures use blank-node
# and typed-literal focus nodes that do not survive an N-Triples
# re-parse byte-stable, so blank-node relabelling can drift the
# printed count by ±1 without flipping conformance. `conforms` is the
# stable gate signal; the count is not.
#
# Modes:
#   (default)   — validate via `pgrdf.validate(g, g)` ('native').
#                 The W3C SHACL **Core** suite. Must be FULL-PASS for
#                 the v0.5 gate (§6.1 #1).
#   --sparql    — validate via `pgrdf.validate(g, g, 'sparql')`.
#                 ERRATA.v0.5 E-012 was RESOLVED on 2026-05-28 by
#                 `shacl 0.3.2` + pgRDF TH-14 (guard delete). The
#                 pre-0.3.2 short-circuit returned `conforms:null`
#                 for every fixture; with the guard gone, 'sparql'
#                 mode dispatches into rudof's working SparqlEngine
#                 and the per-fixture verdict depends on which Core
#                 constraints rudof's `SparqlValidator` trait covers
#                 upstream (Class / NodeKind / Pattern / MinLength /
#                 MaxLength / value-range bounds are covered;
#                 MinCount / MaxCount are NOT). This sub-run
#                 therefore asserts ONLY the pgRDF-side contract:
#                 `conforms` is a real Boolean (true or false, NOT
#                 JSON null), the run completes without panic, and
#                 Per-fixture conforms verdict is compared against
#                 hand-derived expected.json mirroring the W3C
#                 `mf:result`. **TH-7 (v0.5.8) and onward**: vendoring
#                 under tests/w3c-shacl/fixtures/sparql/ is incremental
#                 — fixtures whose SPARQL features pgRDF does not yet
#                 translate (e.g. FILTER NOT EXISTS, advanced builtins
#                 like isLiteral/langMatches) are deferred until the
#                 corresponding Track A work lands; the sub-run gates
#                 only on the fixtures that ship in the directory.
#                 Some fixtures may report `conforms:true` under
#                 rudof's SparqlValidator even when their W3C
#                 `mf:result` says false — rudof's MinCount/MaxCount
#                 coverage gap surfaces as a per-fixture mismatch
#                 surfaced and tracked, not silently suppressed.
#
# Usage:
#   bash tests/w3c-shacl/run.sh                      # Core, native
#   bash tests/w3c-shacl/run.sh --sparql             # sparql sub-run
#   bash tests/w3c-shacl/run.sh node-datatype-001    # one fixture
#   ACCEPT=1 bash tests/w3c-shacl/run.sh             # (re)derive — see
#                                                    # NOTE below

set -u

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CORE_DIR="${REPO_ROOT}/tests/w3c-shacl/fixtures/core"
SPARQL_DIR="${REPO_ROOT}/tests/w3c-shacl/fixtures/sparql"
CONTAINER="${PGRDF_CONTAINER:-pgrdf-pgrdf-postgres}"
# Default to docker (Colima). Multi-agent workstation discipline: this
# repo is docker-only; podman belongs to parallel agents we must not
# touch. The earlier `${PGRDF_RUNTIME:-podman}` default was a v0.4-era
# leftover; CI also runs docker (compose spins under Colima).
RUNTIME="${PGRDF_RUNTIME:-docker}"
PSQL_USER="${POSTGRES_USER:-pgrdf}"
PSQL_DB="${POSTGRES_DB:-pgrdf}"
ACCEPT="${ACCEPT:-0}"

# NOTE on ACCEPT: the expected/ files here are hand-derived from each
# fixture's W3C `mf:result` block (committed alongside the fixtures),
# NOT auto-blessed. ACCEPT=1 exists only for the rare case of adding a
# brand-new fixture whose expected has been hand-written first; it
# refuses to overwrite an existing expected.json (so a regression can
# never be silently re-baselined). To intentionally re-derive, delete
# the expected.json by hand first.

SPARQL_MODE=0
filter=""
for arg in "$@"; do
  case "${arg}" in
    --sparql) SPARQL_MODE=1 ;;
    *)        filter="${arg}" ;;
  esac
done

if [ "${SPARQL_MODE}" -eq 1 ]; then
  # TH-7 (v0.5.8) — the `--sparql` flag now walks the W3C SHACL-SPARQL
  # vendored manifest under `fixtures/sparql/`, not the Core fixtures
  # under `fixtures/core/`. The pre-TH-7 meaning ("run Core fixtures
  # through `mode => 'sparql'`, assert pgRDF-side contract only") was
  # redundant with the default Core run — the new meaning is a real
  # SHACL-SPARQL conformance gate against per-fixture
  # W3C-authoritative `expected.json` (matching the W3C `mf:result`).
  MODE_ARG=", 'sparql'"
  MODE_LABEL="sparql"
  FIX_DIR="${SPARQL_DIR}"
else
  MODE_ARG=""
  MODE_LABEL="native"
  FIX_DIR="${CORE_DIR}"
fi

declare -a tests=()
for ttl in "${FIX_DIR}"/*.ttl; do
  [ -f "${ttl}" ] || continue
  name="$(basename "${ttl}" .ttl)"
  # Skip the `.w3c.ttl` provenance copies — those are the UNMODIFIED
  # W3C source files (kept for provenance + hand-deriving expected
  # from their `mf:result` block). They root the manifest at the
  # empty relative IRI `<>` which oxttl rejects without a base; the
  # harness loads the `<name>.ttl` data+shapes split instead.
  case "${name}" in
    *.w3c) continue ;;
  esac
  if [ -z "${filter}" ] || [ "${name}" = "${filter}" ]; then
    tests+=("${name}")
  fi
done

if [ "${#tests[@]}" -eq 0 ]; then
  echo "no w3c-shacl fixtures matched"
  exit 1
fi

# Deterministic graph id from the fixture name (same scheme as the
# w3c-sparql harness; non-colliding across parallel runs).
graph_id_for() {
  local name="$1" hash
  hash=$(printf '%s' "${name}" | shasum | tr -d -c '0-9' | head -c 4)
  if [ -z "${hash}" ] || [ "${hash}" = "0000" ]; then hash="4321"; fi
  echo "12${hash}"
}

# Load one fixture into a fresh extension + graph and return the
# `pgrdf.validate` JSONB on stdout (one line).
run_one() {
  local ttl="$1" gid="$2"
  local content content_esc sql
  content="$(< "${ttl}")"
  content_esc="${content//\'/\'\'}"
  sql=$'DROP EXTENSION IF EXISTS pgrdf CASCADE;\n'
  sql+=$'CREATE EXTENSION pgrdf;\n'
  sql+=$'SELECT pgrdf.shmem_reset();\n'
  sql+=$'SELECT pgrdf.plan_cache_clear();\n'
  sql+="SELECT pgrdf.add_graph(${gid});"$'\n'
  sql+="SELECT pgrdf.parse_turtle('${content_esc}', ${gid});"$'\n'
  sql+="SELECT pgrdf.validate(${gid}, ${gid}${MODE_ARG})::text;"$'\n'
  "${RUNTIME}" exec -i "${CONTAINER}" \
    psql -U "${PSQL_USER}" -d "${PSQL_DB}" \
    -X -A -t -q -v ON_ERROR_STOP=1 <<<"${sql}"
}

pass=0
fail=0
baselined=0

for name in "${tests[@]}"; do
  ttl="${FIX_DIR}/${name}.ttl"
  expected="${FIX_DIR}/${name}.expected.json"
  gid="$(graph_id_for "${name}")"

  raw="$(run_one "${ttl}" "${gid}")"
  # Keep only the JSON object the validate() call emits (last `{…}`
  # line; the leading rows are add_graph / parse_turtle returns).
  jline="$(printf '%s\n' "${raw}" | grep -E '^\{' | tail -1)"

  if [ -z "${jline}" ]; then
    printf '  \033[31mFAIL\033[0m     %s  (no validate JSON returned)\n' "${name}"
    printf '%s\n' "${raw}" | sed 's/^/    /' | tail -5
    fail=$((fail + 1))
    continue
  fi

  # Comparable invariant — the W3C `sh:conforms` boolean.
  #
  # `conforms` is the headline W3C SHACL conformance signal: a
  # validator that decides conformance correctly IS W3C-conformant
  # at the validation-report level. The violation COUNT is shown for
  # diagnostics but is NOT a gate criterion — the W3C fixtures use
  # blank-node focus nodes whose identity does not survive pgRDF's
  # dictionary-encoded N-Triples rehydrate byte-stable, so a
  # blank-node-focus violation can be relabelled/coalesced and the
  # count drift by ±1 WITHOUT a conformance error. This is the same
  # blank-node-relabel reason the harness already excludes
  # focus-node-IRI comparison; applying it to the count too keeps
  # the gate honest (a true missed/spurious constraint flips
  # `conforms`; a blank-node serialization artifact does not).
  conforms="$(printf '%s' "${jline}" \
    | grep -oE '"conforms"[ ]*:[ ]*(true|false|null)' \
    | grep -oE '(true|false|null)$' | head -1)"
  vcount="$(printf '%s' "${jline}" | grep -oE '"focusNode"' | wc -l | tr -d ' ')"
  actual="{\"conforms\":${conforms}}"

  if [ ! -f "${expected}" ]; then
    if [ "${ACCEPT}" = "1" ]; then
      printf '%s\n' "${actual}" > "${expected}"
      printf '  \033[33mBASELINE\033[0m %s  %s\n' "${name}" "${actual}"
      baselined=$((baselined + 1))
      continue
    fi
    printf '  \033[31mFAIL\033[0m     %s  (no expected.json — hand-derive from mf:result)\n' "${name}"
    fail=$((fail + 1))
    continue
  fi

  want="$(tr -d ' \n\t' < "${expected}")"
  got="$(printf '%s' "${actual}" | tr -d ' \n\t')"
  if [ "${want}" = "${got}" ]; then
    printf '  \033[32mPASS\033[0m     %s  %s (violations=%s)\n' "${name}" "${got}" "${vcount}"
    pass=$((pass + 1))
  else
    printf '  \033[31mFAIL\033[0m     %s\n' "${name}"
    printf '    expected: %s\n' "${want}"
    printf '    actual:   %s (violations=%s)\n' "${got}" "${vcount}"
    fail=$((fail + 1))
  fi
done

printf '\nw3c-shacl summary [%s]: %d pass, %d fail, %d new baselines\n' \
  "${MODE_LABEL}" "${pass}" "${fail}" "${baselined}"
[ "${fail}" -eq 0 ]
