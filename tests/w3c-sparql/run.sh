#!/usr/bin/env bash
#
# tests/w3c-sparql/run.sh — W3C-shape SPARQL conformance harness.
#
# Each subdirectory of tests/w3c-sparql/ is one test:
#   <NN>-<name>/
#     data.ttl       — Turtle loaded into a fresh graph
#     query.rq       — SPARQL query executed via pgrdf.sparql
#     expected.jsonl — one JSONB row per line, lexicographically sorted
#
# Compares engine output to expected.jsonl. The comparison is
# bag-equivalent — both sides are sorted before diffing so unordered
# SPARQL solution sequences match regardless of result order. Tests
# that need a specific order should use `ORDER BY` in the query.
#
# Usage:
#   bash tests/w3c-sparql/run.sh                  # all tests
#   bash tests/w3c-sparql/run.sh 01-basic-bgp     # one test
#   ACCEPT=1 bash tests/w3c-sparql/run.sh ...     # regenerate expected.jsonl

set -u

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TESTS_DIR="${REPO_ROOT}/tests/w3c-sparql"
CONTAINER="${PGRDF_CONTAINER:-pgrdf-postgres}"
RUNTIME="${PGRDF_RUNTIME:-podman}"
PSQL_USER="${POSTGRES_USER:-pgrdf}"
PSQL_DB="${POSTGRES_DB:-pgrdf}"
ACCEPT="${ACCEPT:-0}"

filter="${1:-}"
declare -a tests=()
for dir in "${TESTS_DIR}"/*/; do
  [ -d "${dir}" ] || continue
  name="$(basename "${dir}")"
  case "${name}" in
    fixtures) continue ;;        # reserved for future W3C submodule
  esac
  # Phase A slice 111: a test directory is valid if it provides EITHER
  # a `data.ttl` (the v0.3 single-graph default) OR a `setup.sql`
  # (the slice-111 multi-graph extension for §13.3 GRAPH fixtures).
  # `query.rq` is always required.
  [ -f "${dir}query.rq" ] || continue
  if [ ! -f "${dir}data.ttl" ] && [ ! -f "${dir}setup.sql" ]; then
    continue
  fi
  if [ -z "${filter}" ] || [ "${name}" = "${filter}" ]; then
    tests+=("${name}")
  fi
done

if [ "${#tests[@]}" -eq 0 ]; then
  echo "no w3c-sparql tests matched"
  exit 1
fi

pass=0
fail=0
baselined=0

# Deterministic graph id from the test name. Avoids collisions when
# tests run in parallel; uses a fixed-range slice so the IDs are
# easy to scan in pg_class output.
graph_id_for() {
  local name="$1"
  # Hash the name to a 4-digit suffix. Stable across runs.
  local hash
  hash=$(printf '%s' "${name}" | sha1sum | tr -d -c '0-9' | head -c 4)
  # Guard against an all-leading-zero hash (would yield graph_id 0
  # which clashes with the default partition).
  if [ -z "${hash}" ] || [ "${hash}" = "0000" ]; then
    hash="1234"
  fi
  echo "10${hash}"
}

# Run one query against a freshly-recreated extension. Returns the
# raw sparql output (one JSON per line) on stdout.
#
# Phase A slice 111 — optional per-test `setup.sql` is supported for
# W3C-shape conformance fixtures that need MULTIPLE named graphs (the
# default single-graph `data.ttl` path can't express §13.3 `GRAPH ?g`
# scoping). The runner builds a single SQL stream (DROP/CREATE/reset)
# → optional setup.sql → optional parse_turtle of data.ttl → query
# and feeds it through one psql invocation so semantics stay
# deterministic and the leading-scaffolding-row drop in the caller
# remains compatible (we still keep only lines starting with `{` or
# `[` — function return values from setup.sql are stripped by the
# same grep). Existing tests (01-23) have no `setup.sql` and a
# non-empty `data.ttl`; their SQL stream is unchanged.
run_one() {
  local test_dir="$1" query="$2" gid="$3"
  local data="${test_dir}/data.ttl"
  local setup="${test_dir}/setup.sql"
  local q
  q=$(< "${query}")
  # Escape single quotes for SQL string literals.
  local q_esc="${q//\'/\'\'}"

  # Assemble the SQL stream. Always: DROP / CREATE / shmem_reset /
  # plan_cache_clear. Then optionally setup.sql (for slice-111
  # multi-graph fixtures). Then add_graph(${gid}) + parse_turtle ONLY
  # if data.ttl exists AND is non-empty — keeps the 23 single-graph
  # tests behaviour-identical while letting multi-graph tests opt
  # out of the default graph entirely.
  local sql
  sql=$'DROP EXTENSION IF EXISTS pgrdf CASCADE;\n'
  sql+=$'CREATE EXTENSION pgrdf;\n'
  sql+=$'SELECT pgrdf.shmem_reset();\n'
  sql+=$'SELECT pgrdf.plan_cache_clear();\n'

  if [ -f "${setup}" ]; then
    local setup_content
    setup_content=$(< "${setup}")
    sql+="${setup_content}"$'\n'
  fi

  if [ -f "${data}" ] && [ -s "${data}" ]; then
    local content
    content=$(< "${data}")
    local content_esc="${content//\'/\'\'}"
    sql+="SELECT pgrdf.add_graph(${gid});"$'\n'
    sql+="SELECT pgrdf.parse_turtle('${content_esc}', ${gid});"$'\n'
  fi

  sql+="SELECT sparql::text FROM pgrdf.sparql('${q_esc}');"$'\n'

  "${RUNTIME}" exec -i "${CONTAINER}" \
    psql -U "${PSQL_USER}" -d "${PSQL_DB}" \
    -X -A -t -q -v ON_ERROR_STOP=1 <<<"${sql}"
}

for name in "${tests[@]}"; do
  test_dir="${TESTS_DIR}/${name}"
  query="${test_dir}/query.rq"
  expected="${test_dir}/expected.jsonl"
  gid="$(graph_id_for "${name}")"

  raw="$(run_one "${test_dir}" "${query}" "${gid}")"
  # Drop the three leading scaffolding rows (DROP EXTENSION's NOTICE,
  # plus pgrdf.shmem_reset / plan_cache_clear / add_graph / parse_turtle
  # return values — these show up because we set -q only suppresses
  # the prompt, not the row output. The query rows come last; keep
  # the JSON-looking ones.
  actual_raw="$(printf '%s\n' "${raw}" | grep -E '^\{|^\[' || true)"
  actual="$(printf '%s\n' "${actual_raw}" | LC_ALL=C sort)"

  if [ ! -f "${expected}" ] || [ "${ACCEPT}" = "1" ]; then
    printf '%s\n' "${actual}" > "${expected}"
    printf '  \033[33mBASELINE\033[0m %s\n' "${name}"
    baselined=$((baselined + 1))
    continue
  fi

  expected_sorted="$(LC_ALL=C sort < "${expected}")"
  if [ "${actual}" = "${expected_sorted}" ]; then
    printf '  \033[32mPASS\033[0m     %s\n' "${name}"
    pass=$((pass + 1))
  else
    printf '  \033[31mFAIL\033[0m     %s\n' "${name}"
    diff -u <(printf '%s\n' "${expected_sorted}") <(printf '%s\n' "${actual}") | sed 's/^/    /'
    fail=$((fail + 1))
  fi
done

printf '\nw3c-sparql summary: %d pass, %d fail, %d new baselines\n' \
  "${pass}" "${fail}" "${baselined}"
[ "${fail}" -eq 0 ]
