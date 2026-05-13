#!/usr/bin/env bash
#
# tests/regression/run.sh — pg_regress-style runner against the compose
# Postgres. Each tests/regression/sql/NN-<name>.sql is piped to psql;
# stdout is diffed against tests/regression/expected/NN-<name>.out.
#
# Expectations:
#   - `podman compose up -d` has already booted the stack.
#   - The extension is either already CREATEd or the test file does it.
#
# Usage:
#   tests/regression/run.sh                # run all
#   tests/regression/run.sh 00-smoke       # run a specific test
#   ACCEPT=1 tests/regression/run.sh ...   # overwrite expected/ from actual

set -u

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SQL_DIR="${REPO_ROOT}/tests/regression/sql"
EXPECTED_DIR="${REPO_ROOT}/tests/regression/expected"
CONTAINER="${PGRDF_CONTAINER:-pgrdf-postgres}"
PSQL_USER="${POSTGRES_USER:-pgrdf}"
PSQL_DB="${POSTGRES_DB:-pgrdf}"
ACCEPT="${ACCEPT:-0}"

mkdir -p "${EXPECTED_DIR}"

filter="${1:-}"
declare -a tests=()
for sql in "${SQL_DIR}"/*.sql; do
  [ -f "${sql}" ] || continue
  name="$(basename "${sql}" .sql)"
  if [ -z "${filter}" ] || [ "${name}" = "${filter}" ]; then
    tests+=("${sql}")
  fi
done

if [ "${#tests[@]}" -eq 0 ]; then
  echo "no regression tests matched"
  exit 1
fi

pass=0
fail=0
new=0

for sql in "${tests[@]}"; do
  name="$(basename "${sql}" .sql)"
  expected="${EXPECTED_DIR}/${name}.out"

  # `-X` skips ~/.psqlrc; `-A` unaligned; `-t` tuples-only; `-q` quiet;
  # `-v ON_ERROR_STOP=1` so first error halts the script for that file.
  actual="$(podman exec -i "${CONTAINER}" \
    psql -U "${PSQL_USER}" -d "${PSQL_DB}" \
    -X -A -t -q -v ON_ERROR_STOP=1 < "${sql}" 2>&1)" || true

  if [ ! -f "${expected}" ] || [ "${ACCEPT}" = "1" ]; then
    printf '%s\n' "${actual}" > "${expected}"
    printf '  \033[33mBASELINE\033[0m %s\n' "${name}"
    new=$((new + 1))
    continue
  fi

  if diff -q <(printf '%s\n' "${actual}") "${expected}" > /dev/null 2>&1; then
    printf '  \033[32mPASS\033[0m     %s\n' "${name}"
    pass=$((pass + 1))
  else
    printf '  \033[31mFAIL\033[0m     %s\n' "${name}"
    diff <(printf '%s\n' "${actual}") "${expected}" | sed 's/^/    /' | head -30
    fail=$((fail + 1))
  fi
done

echo
printf 'regression summary: %d pass, %d fail, %d new baselines\n' "${pass}" "${fail}" "${new}"
[ "${fail}" -eq 0 ] || exit 1
