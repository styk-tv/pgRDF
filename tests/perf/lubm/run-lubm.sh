#!/usr/bin/env bash
#
# tests/perf/lubm/run-lubm.sh — TF-10 LUBM runner.
#
# Boots an isolated postgres+pgrdf sidecar, ingests LUBM-N from the
# `pgrdf-lubm-data` docker named volume, runs a small SPARQL query,
# and emits `target/perf-report.json` per the
# `tests/perf/lubm/schema/baseline.schema.json` JSON Schema 2020-12
# contract.
#
# DOCKER ONLY — never podman (per [[docker-only-pgrdf-prefix]]). All
# container / volume / network names are `pgrdf-perf-*`-prefixed so
# this runner is safe to invoke alongside parallel agents on the same
# Colima daemon.
#
# Pre-flight:
#   1. `just lubm-build` has produced the `pgrdf-lubm-generator:latest`
#      image, and
#   2. `just lubm-gen N` has populated the `pgrdf-lubm-data` named
#      volume with `/data/lubm-<N>/nt/lubm-<N>.nt`.
#   3. `just build-ext` has populated `compose/extensions/` with the
#      current pgrdf .so / .control / pgrdf--<ver>.sql files.
#
# Usage:
#   bash tests/perf/lubm/run-lubm.sh                # LUBM-10 (default)
#   bash tests/perf/lubm/run-lubm.sh 10             # explicit
#   OUTFILE=tests/perf/lubm/baseline.lubm-10.json \
#       bash tests/perf/lubm/run-lubm.sh 10         # baseline capture
#
# Exit codes:
#   0   success — report written
#   1   pre-flight failure (missing image / volume / extension files)
#   2   sidecar boot failure
#   3   ingest failure
#   4   query failure
#   5   schema-validation failure (only when JSON_SCHEMA_VALIDATE=1)
set -euo pipefail

UNIV_COUNT="${1:-10}"
REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
OUTFILE="${OUTFILE:-${REPO_ROOT}/target/perf-report.json}"
# Always docker per [[docker-only-pgrdf-prefix]]; honour an override for
# debugging only, never for the supported flow.
RUNTIME="${PGRDF_RUNTIME:-docker}"
LUBM_VOLUME="${LUBM_VOLUME:-pgrdf-lubm-data}"
EXTENSIONS_DIR="${EXTENSIONS_DIR:-${REPO_ROOT}/compose/extensions}"
SIDECAR_NAME="pgrdf-perf-pg-$$"
SIDECAR_PORT="${SIDECAR_PORT:-0}"   # 0 = random host port
PG_IMAGE="${PG_IMAGE:-docker.io/library/postgres:17.4-bookworm}"
PG_USER="${POSTGRES_USER:-pgrdf}"
PG_PASS="${POSTGRES_PASSWORD:-pgrdf}"
PG_DB="${POSTGRES_DB:-pgrdf}"
DATA_GID="${DATA_GID:-92000}"

# A small UBA-friendly SPARQL — count GraduateStudent instances. The
# UBA-emitted ontology IRI is `file:///opt/lubm/univ-bench.owl#…`
# (the `-onto file://…` we passed UBA at generation time becomes the
# concrete class IRI prefix in the .owl output). Stable for a given
# `-seed 0` run.
GRAD_STUDENT_IRI="file:///opt/lubm/univ-bench.owl#GraduateStudent"

# ── pre-flight ───────────────────────────────────────────────────────
fail() { printf '[run-lubm] FATAL: %s\n' "$*" >&2; exit "${2:-1}"; }

[ "${RUNTIME}" = "docker" ] || \
  fail "this runner requires docker (got '${RUNTIME}'); LUBM is docker-only per workstation discipline" 1

${RUNTIME} volume inspect "${LUBM_VOLUME}" >/dev/null 2>&1 || \
  fail "${LUBM_VOLUME} volume not present — run 'just lubm-gen ${UNIV_COUNT}' first" 1

[ -f "${EXTENSIONS_DIR}/lib/pgrdf.so" ] || \
  fail "${EXTENSIONS_DIR}/lib/pgrdf.so missing — run 'just build-ext' first" 1
[ -f "${EXTENSIONS_DIR}/share/extension/pgrdf.control" ] || \
  fail "pgrdf.control missing under ${EXTENSIONS_DIR}/share/extension/" 1

# Read default_version out of the .control to figure out which
# pgrdf--<ver>.sql to mount; matches the compose pattern.
DEFAULT_VERSION="$(sed -n "s/^default_version = '\\(.*\\)'/\\1/p" \
  "${EXTENSIONS_DIR}/share/extension/pgrdf.control")"
[ -n "${DEFAULT_VERSION}" ] || fail "could not read default_version from .control" 1
SQL_FILE="${EXTENSIONS_DIR}/share/extension/pgrdf--${DEFAULT_VERSION}.sql"
[ -f "${SQL_FILE}" ] || fail "missing ${SQL_FILE}" 1

mkdir -p "$(dirname "${OUTFILE}")"

# Verify the data file actually exists in the volume.
DATA_PATH="/lubm-data/lubm-${UNIV_COUNT}/nt/lubm-${UNIV_COUNT}.nt"
${RUNTIME} run --rm -v "${LUBM_VOLUME}:/lubm-data:ro" alpine:3.20 \
  test -f "${DATA_PATH}" \
  || fail "data file ${DATA_PATH} missing inside ${LUBM_VOLUME} — run 'just lubm-gen ${UNIV_COUNT}' first" 1

# ── sidecar boot ────────────────────────────────────────────────────
cleanup() {
  ${RUNTIME} rm -f "${SIDECAR_NAME}" >/dev/null 2>&1 || true
}
trap cleanup EXIT INT TERM

printf '[run-lubm] booting sidecar %s (pg %s, pgrdf %s)\n' \
  "${SIDECAR_NAME}" "${PG_IMAGE##*:}" "${DEFAULT_VERSION}"

PORT_ARG=()
if [ "${SIDECAR_PORT}" != "0" ]; then
  PORT_ARG=(-p "${SIDECAR_PORT}:5432")
fi

# Per-file extension mounts mirror compose.yml exactly (so the same
# parity contract holds). LUBM volume mounts read-only at /lubm-data.
${RUNTIME} run -d --name "${SIDECAR_NAME}" \
  "${PORT_ARG[@]}" \
  -e POSTGRES_USER="${PG_USER}" \
  -e POSTGRES_PASSWORD="${PG_PASS}" \
  -e POSTGRES_DB="${PG_DB}" \
  -v "${LUBM_VOLUME}:/lubm-data:ro" \
  -v "${EXTENSIONS_DIR}/lib/pgrdf.so:/usr/lib/postgresql/17/lib/pgrdf.so:ro" \
  -v "${EXTENSIONS_DIR}/share/extension/pgrdf.control:/usr/share/postgresql/17/extension/pgrdf.control:ro" \
  -v "${SQL_FILE}:/usr/share/postgresql/17/extension/pgrdf--${DEFAULT_VERSION}.sql:ro" \
  "${PG_IMAGE}" \
  postgres -c shared_preload_libraries=pgrdf \
  >/dev/null \
  || fail "sidecar failed to start" 2

# Wait for pg_isready (up to 30s). On failure dump logs.
for _ in $(seq 1 30); do
  if ${RUNTIME} exec "${SIDECAR_NAME}" pg_isready -U "${PG_USER}" -d "${PG_DB}" >/dev/null 2>&1; then
    break
  fi
  sleep 1
done
if ! ${RUNTIME} exec "${SIDECAR_NAME}" pg_isready -U "${PG_USER}" -d "${PG_DB}" >/dev/null 2>&1; then
  ${RUNTIME} logs "${SIDECAR_NAME}" | tail -50 >&2
  fail "sidecar did not become healthy within 30s" 2
fi

# ── SQL helpers ─────────────────────────────────────────────────────
psql_exec() {
  ${RUNTIME} exec -i "${SIDECAR_NAME}" \
    psql -U "${PG_USER}" -d "${PG_DB}" \
    -X -A -t -q -v ON_ERROR_STOP=1 "$@"
}

# Bootstrap extension + a graph for the data.
psql_exec <<SQL >/dev/null || fail "bootstrap CREATE EXTENSION failed" 3
CREATE EXTENSION pgrdf;
SELECT pgrdf.add_graph(${DATA_GID});
SQL

# ── ingest ──────────────────────────────────────────────────────────
printf '[run-lubm] ingesting %s into graph %s\n' "${DATA_PATH}" "${DATA_GID}"

INGEST_JSON=$(psql_exec <<SQL
SELECT pgrdf.load_turtle_verbose('${DATA_PATH}', ${DATA_GID})::text;
SQL
) || fail "ingest failed" 3

# Extract triples + elapsed_ms + dict stats from the JSONB row.
extract_num() { printf '%s' "$1" | grep -oE "\"$2\"[ ]*:[ ]*[0-9]+(\\.[0-9]+)?" \
  | grep -oE "[0-9]+(\\.[0-9]+)?$" | head -1; }

INGEST_TRIPLES=$(extract_num "${INGEST_JSON}" "triples")
INGEST_ELAPSED=$(extract_num "${INGEST_JSON}" "elapsed_ms")
INGEST_DICT_HITS=$(extract_num "${INGEST_JSON}" "dict_cache_hits")
INGEST_SHMEM_HITS=$(extract_num "${INGEST_JSON}" "shmem_cache_hits")
INGEST_DB_CALLS=$(extract_num "${INGEST_JSON}" "dict_db_calls")
[ -n "${INGEST_TRIPLES}" ] || fail "ingest report missing triples: ${INGEST_JSON}" 3

# Total dict_lookups = cache hits + db calls (a useful diagnostic).
INGEST_LOOKUPS=$(( ${INGEST_DICT_HITS:-0} + ${INGEST_SHMEM_HITS:-0} + ${INGEST_DB_CALLS:-0} ))

printf '[run-lubm]   triples=%s  elapsed_ms=%s  dict_db_calls=%s\n' \
  "${INGEST_TRIPLES}" "${INGEST_ELAPSED}" "${INGEST_DB_CALLS}"

# ── query ───────────────────────────────────────────────────────────
printf '[run-lubm] running Q14 (count GraduateStudent instances)\n'

# Warm + measured pass. We do one warm + three measured passes and
# take the median to damp out cold-cache jitter.
QUERY="SELECT (COUNT(?s) AS ?n) WHERE { ?s a <${GRAD_STUDENT_IRI}> }"
QUERY_ESC="${QUERY//\'/\'\'}"

measure_query() {
  ${RUNTIME} exec -i "${SIDECAR_NAME}" \
    psql -U "${PG_USER}" -d "${PG_DB}" \
    -X -A -t -q -v ON_ERROR_STOP=1 <<SQL
\\timing on
SELECT sparql::text FROM pgrdf.sparql('${QUERY_ESC}');
SQL
}

# psql \timing prints "Time: 12.345 ms" after each statement; collect.
extract_timing() { printf '%s\n' "$1" | grep -oE "Time: [0-9]+(\\.[0-9]+)? ms" \
  | grep -oE "[0-9]+(\\.[0-9]+)?" | head -1; }

# Warm pass — discard timing.
WARM_RAW=$(measure_query 2>&1) || fail "warm query failed: ${WARM_RAW}" 4
WARM_RESULT=$(printf '%s\n' "${WARM_RAW}" | grep -E '^\{' | head -1)

# Three measured passes.
declare -a TIMES=()
for i in 1 2 3; do
  RAW=$(measure_query 2>&1) || fail "measured query #${i} failed" 4
  T=$(extract_timing "${RAW}")
  [ -n "${T}" ] || fail "could not extract timing from psql output: ${RAW}" 4
  TIMES+=("${T}")
done

# Median of 3 — sort, take middle.
QUERY_MS=$(printf '%s\n' "${TIMES[@]}" | LC_ALL=C sort -n | awk 'NR==2')

# Extract the result count from the JSONB binding. pgrdf.sparql for
# an aggregate SELECT returns a JSONB row of shape `{"n": "24019"}`
# (variable name → literal string). We pull the first quoted integer.
QUERY_RESULT_COUNT=$(printf '%s' "${WARM_RESULT}" \
  | grep -oE '"[0-9]+"' \
  | head -1 \
  | tr -d '"')
[ -n "${QUERY_RESULT_COUNT}" ] || fail "could not extract Q14 count from result: ${WARM_RESULT}" 4

printf '[run-lubm]   Q14 GraduateStudent count = %s  median elapsed_ms=%s\n' \
  "${QUERY_RESULT_COUNT}" "${QUERY_MS}"

# ── emit report ─────────────────────────────────────────────────────
PRODUCED_AT="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
PRODUCED_BY="$(basename "$0")@$(hostname -s)"
PG_MAJOR="$(${RUNTIME} exec "${SIDECAR_NAME}" psql -U "${PG_USER}" -d "${PG_DB}" -X -A -t -q \
  -c "SHOW server_version_num;" | awk '{print int($1/10000)}')"

cat > "${OUTFILE}" <<JSON
{
  "schema_version": "v0.6",
  "produced_at": "${PRODUCED_AT}",
  "produced_by": "${PRODUCED_BY}",
  "univ_count": ${UNIV_COUNT},
  "pgrdf_version": "${DEFAULT_VERSION}",
  "postgres_major": ${PG_MAJOR},
  "fixtures": [
    {
      "name": "lubm-${UNIV_COUNT}-ingest-nt",
      "kind": "sparql-select",
      "modes": {
        "default": {
          "elapsed_ms": ${INGEST_ELAPSED:-0},
          "dict_lookups": ${INGEST_LOOKUPS:-0}
        }
      },
      "comparison_tolerance": { "elapsed_ms_pct": 50 }
    },
    {
      "name": "lubm-${UNIV_COUNT}-q14-graduate-students",
      "kind": "sparql-select",
      "modes": {
        "default": {
          "elapsed_ms": ${QUERY_MS}
        }
      },
      "comparison_tolerance": { "elapsed_ms_pct": 50 }
    }
  ]
}
JSON

printf '[run-lubm] wrote %s\n' "${OUTFILE}"

# Optional schema-validation pass. Off by default (jsonschema/python
# not assumed on every dev box); CI flips on with JSON_SCHEMA_VALIDATE=1.
if [ "${JSON_SCHEMA_VALIDATE:-0}" = "1" ]; then
  python3 -c "
import json, sys
import jsonschema
with open('${REPO_ROOT}/tests/perf/lubm/schema/baseline.schema.json') as f:
    schema = json.load(f)
with open('${OUTFILE}') as f:
    doc = json.load(f)
jsonschema.validate(doc, schema)
print('[run-lubm] schema-validation OK')
" || fail "schema validation failed" 5
fi

# Stash side-info for human eyeballing.
printf '[run-lubm] ingest summary: triples=%s elapsed_ms=%s\n' "${INGEST_TRIPLES}" "${INGEST_ELAPSED}"
printf '[run-lubm] query  summary: q14_count=%s elapsed_ms=%s (median of 3, warm)\n' \
  "${QUERY_RESULT_COUNT}" "${QUERY_MS}"

exit 0
