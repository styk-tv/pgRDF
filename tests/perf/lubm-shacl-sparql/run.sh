#!/usr/bin/env bash
#
# tests/perf/lubm-shacl-sparql/run.sh — TH-4 LUBM-shape SHACL-SPARQL
# dev-gate.
#
# Loads a handcrafted ~10-university LUBM-shape ABox plus a
# SHACL-SPARQL constraint ("Course must be taught by at most one
# Professor"), runs `pgrdf.validate` under both 'sparql' (rudof) and
# 'pgrdf' (pgRDF-native) modes, prints a comparison row.
#
# Expected — 2 intentional teaching collisions (u0:CS101 and u3:CS101,
# each with 2 Professors), so 4 violations from the pgRDF-native
# evaluator (one row per Professor focus that shares a course). The
# rudof-side verdict surfaces the ERRATA.v0.6 E-014 gap as it does on
# the W3C node-sparql-001 fixture.
#
# Dev-gate (not release-gate): runs on every CI to keep the path
# warm; the real LUBM-10 / LUBM-100 release-gates land as TH-3 + the
# Java UBA generator work (deferred per SPEC.pgRDF.OPENBENCHMARK.v1.0).
#
# Usage:
#   bash tests/perf/lubm-shacl-sparql/run.sh
#   ACCEPT=1 bash tests/perf/lubm-shacl-sparql/run.sh   # baseline expected
set -u

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
DIR="${REPO_ROOT}/tests/perf/lubm-shacl-sparql"
CONTAINER="${PGRDF_CONTAINER:-pgrdf-pgrdf-postgres}"
# Docker-only per workstation discipline; CI explicit-sets to docker too.
RUNTIME="${PGRDF_RUNTIME:-docker}"
PSQL_USER="${POSTGRES_USER:-pgrdf}"
PSQL_DB="${POSTGRES_DB:-pgrdf}"
ACCEPT="${ACCEPT:-0}"

DATA="$(< "${DIR}/data.ttl")"
SHAPES="$(< "${DIR}/shapes.ttl")"
DATA_ESC="${DATA//\'/\'\'}"
SHAPES_ESC="${SHAPES//\'/\'\'}"

# Fresh extension + 2 graphs (data + shapes).
SETUP_SQL=$'DROP EXTENSION IF EXISTS pgrdf CASCADE;\n'
SETUP_SQL+=$'CREATE EXTENSION pgrdf;\n'
SETUP_SQL+=$'SELECT pgrdf.shmem_reset();\n'
SETUP_SQL+=$'SELECT pgrdf.plan_cache_clear();\n'
SETUP_SQL+=$'SELECT pgrdf.add_graph(91000);\n'
SETUP_SQL+="SELECT pgrdf.parse_turtle('${DATA_ESC}', 91000);"$'\n'
SETUP_SQL+=$'SELECT pgrdf.add_graph(91001);\n'
SETUP_SQL+="SELECT pgrdf.parse_turtle('${SHAPES_ESC}', 91001);"$'\n'

# Validation under each mode; collect conforms + violation count.
VAL_SQL=$''
for mode in sparql pgrdf; do
  VAL_SQL+="\\echo MODE: ${mode}"$'\n'
  VAL_SQL+="SELECT pgrdf.validate(91000, 91001, '${mode}')::text;"$'\n'
done

ALL_SQL="${SETUP_SQL}${VAL_SQL}"

raw=$("${RUNTIME}" exec -i "${CONTAINER}" \
  psql -U "${PSQL_USER}" -d "${PSQL_DB}" \
  -X -A -t -q -v ON_ERROR_STOP=1 <<<"${ALL_SQL}")

# Extract per-mode JSONB.
sparql_json="$(printf '%s\n' "${raw}" | awk '/^MODE: sparql/{flag=1;next}/^MODE: pgrdf/{flag=0}flag' | grep -E '^\{' | tail -1)"
pgrdf_json="$(printf '%s\n' "${raw}"  | awk '/^MODE: pgrdf/{flag=1;next}flag' | grep -E '^\{' | tail -1)"

extract() {
  local json="$1" key="$2"
  printf '%s' "${json}" | grep -oE "\"${key}\"[ ]*:[ ]*(true|false|null|[0-9]+(\\.[0-9]+)?)" \
    | grep -oE "(true|false|null|[0-9]+(\\.[0-9]+)?)$" | head -1
}
count_focus() {
  printf '%s' "$1" | grep -oE '"focusNode"' | wc -l | tr -d ' '
}

sparql_conforms="$(extract "${sparql_json}" conforms)"
sparql_elapsed="$(extract "${sparql_json}"  elapsed_ms)"
sparql_violations="$(count_focus "${sparql_json}")"
pgrdf_conforms="$(extract "${pgrdf_json}"  conforms)"
pgrdf_elapsed="$(extract "${pgrdf_json}"   elapsed_ms)"
pgrdf_violations="$(count_focus "${pgrdf_json}")"

actual="$(printf '{"sparql":{"conforms":%s,"violations":%s},"pgrdf":{"conforms":%s,"violations":%s}}\n' \
  "${sparql_conforms}" "${sparql_violations}" \
  "${pgrdf_conforms}" "${pgrdf_violations}")"

expected_file="${DIR}/expected.json"

if [ ! -f "${expected_file}" ]; then
  if [ "${ACCEPT}" = "1" ]; then
    printf '%s\n' "${actual}" > "${expected_file}"
    printf '  \033[33mBASELINE\033[0m  %s\n' "${actual}"
    exit 0
  fi
  printf '  \033[31mFAIL\033[0m no expected.json — hand-derive or run ACCEPT=1\n'
  exit 1
fi

want="$(tr -d ' \n\t' < "${expected_file}")"
got="$(printf '%s' "${actual}" | tr -d ' \n\t')"

# Per-mode elapsed_ms is for diagnostic only — never a gate criterion
# (CI runner noise floor is too high for hard timing assertions on a
# 250-triple fixture).
printf 'tests/perf/lubm-shacl-sparql — LUBM-shape SHACL-SPARQL dev-gate\n'
printf '  sparql:  conforms=%-5s violations=%s  elapsed_ms=%s\n' \
  "${sparql_conforms}" "${sparql_violations}" "${sparql_elapsed:-?}"
printf '  pgrdf:   conforms=%-5s violations=%s  elapsed_ms=%s\n' \
  "${pgrdf_conforms}"  "${pgrdf_violations}"  "${pgrdf_elapsed:-?}"

if [ "${want}" = "${got}" ]; then
  printf '  \033[32mPASS\033[0m  %s\n' "${got}"
  exit 0
fi
printf '  \033[31mFAIL\033[0m\n    expected: %s\n    actual:   %s\n' "${want}" "${got}"
exit 1
