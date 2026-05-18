#!/usr/bin/env bash
#
# tests/perf/lubm-shape/run.sh — LUBM-shape correctness gate.
#
# Mirrors the W3C-shape harness exactly (per-test data.ttl + query.rq
# + expected.jsonl); the only LUBM-specific bit is that data.ttl in
# every Q*/ directory is a symlink to the shared parent data.ttl.
#
# Real LUBM-1/10/100 with the Java generator and cross-engine
# comparison vs Apache Jena TDB / Apache AGE is deferred — see
# tests/perf/README.md and v0.3 LLD §5.4 Phase 6 step 3.
#
# Usage:
#   bash tests/perf/lubm-shape/run.sh                  # all queries
#   bash tests/perf/lubm-shape/run.sh Q1-class-membership
#   ACCEPT=1 bash tests/perf/lubm-shape/run.sh ...
set -u

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TESTS_DIR="${REPO_ROOT}/tests/perf/lubm-shape"
CONTAINER="${PGRDF_CONTAINER:-pgrdf-pgrdf-postgres}"
RUNTIME="${PGRDF_RUNTIME:-podman}"
PSQL_USER="${POSTGRES_USER:-pgrdf}"
PSQL_DB="${POSTGRES_DB:-pgrdf}"
ACCEPT="${ACCEPT:-0}"

filter="${1:-}"
declare -a tests=()
for dir in "${TESTS_DIR}"/Q*/; do
  [ -d "${dir}" ] || continue
  name="$(basename "${dir}")"
  [ -f "${dir}data.ttl" ] || continue
  [ -f "${dir}query.rq" ] || continue
  if [ -z "${filter}" ] || [ "${name}" = "${filter}" ]; then
    tests+=("${name}")
  fi
done

if [ "${#tests[@]}" -eq 0 ]; then
  echo "no lubm-shape tests matched"
  exit 1
fi

pass=0
fail=0
baselined=0

graph_id_for() {
  local name="$1"
  local hash
  hash=$(printf '%s' "${name}" | sha1sum | tr -d -c '0-9' | head -c 4)
  if [ -z "${hash}" ] || [ "${hash}" = "0000" ]; then
    hash="1234"
  fi
  echo "20${hash}"
}

run_one() {
  local data="$1" query="$2" gid="$3"
  local content q content_esc q_esc
  content=$(< "${data}")
  q=$(< "${query}")
  content_esc="${content//\'/\'\'}"
  q_esc="${q//\'/\'\'}"
  "${RUNTIME}" exec -i "${CONTAINER}" \
    psql -U "${PSQL_USER}" -d "${PSQL_DB}" \
    -X -A -t -q -v ON_ERROR_STOP=1 <<SQL
DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
SELECT pgrdf.add_graph(${gid});
SELECT pgrdf.parse_turtle('${content_esc}', ${gid});
SELECT sparql::text FROM pgrdf.sparql('${q_esc}');
SQL
}

for name in "${tests[@]}"; do
  data="${TESTS_DIR}/${name}/data.ttl"
  query="${TESTS_DIR}/${name}/query.rq"
  expected="${TESTS_DIR}/${name}/expected.jsonl"
  gid="$(graph_id_for "${name}")"

  raw="$(run_one "${data}" "${query}" "${gid}")"
  actual="$(printf '%s\n' "${raw}" | grep -E '^\{|^\[' | LC_ALL=C sort || true)"

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

printf '\nlubm-shape summary: %d pass, %d fail, %d new baselines\n' \
  "${pass}" "${fail}" "${baselined}"
[ "${fail}" -eq 0 ]
