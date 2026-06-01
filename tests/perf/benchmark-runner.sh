#!/usr/bin/env bash
#
# tests/perf/benchmark-runner.sh — pgRDF benchmark harness with
# persistent run history + HTML report.
#
# Boots an isolated postgres+pgrdf sidecar, runs an LUBM-N ingest
# + RDFS materialization + OWL-RL materialization + Q14 query, and
# appends a JSON line per run to `target/perf-history/runs.jsonl`.
# `render-history.py` reads that file and emits a self-contained
# `target/perf-history/index.html` with run-over-run charts.
#
# DOCKER ONLY (per [[docker-only-pgrdf-prefix]]). Sidecar name is
# `pgrdf-bench-pg-<pid>` to stay clear of `pgrdf-perf-pg-<pid>` used
# by the older `run-lubm.sh` dev-gate runner.
#
# Pre-flight:
#   1. `just lubm-build` has produced the UBA generator image.
#   2. `just lubm-gen <N>` has populated the `pgrdf-lubm-data` volume.
#   3. `just build-ext` has populated `compose/extensions/`.
#
# Usage:
#   bash tests/perf/benchmark-runner.sh             # LUBM-10 (default)
#   bash tests/perf/benchmark-runner.sh 1           # LUBM-1
#   PROFILES=rdfs bash tests/perf/benchmark-runner.sh 10
#   PROFILES=rdfs,owl-rl bash tests/perf/benchmark-runner.sh 10
#
# Outputs:
#   target/perf-history/runs.jsonl        appended one line per run
#   target/perf-history/index.html        re-rendered each run
#   target/perf-history/last-run.json     full output of the most recent run
#
# Exit codes:
#   0   success — history line appended + report re-rendered
#   1   pre-flight failure
#   2   sidecar boot failure
#   3   ingest failure
#   4   materialize failure
#   5   query failure

set -euo pipefail

UNIV_COUNT="${1:-10}"
PROFILES="${PROFILES:-rdfs,owl-rl}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"
HISTORY_DIR="${HISTORY_DIR:-${REPO_ROOT}/target/perf-history}"
RUNTIME="${PGRDF_RUNTIME:-docker}"
LUBM_VOLUME="${LUBM_VOLUME:-pgrdf-lubm-data}"
EXTENSIONS_DIR="${EXTENSIONS_DIR:-${REPO_ROOT}/compose/extensions}"
SIDECAR_NAME="pgrdf-bench-pg-$$"
PG_IMAGE="${PG_IMAGE:-docker.io/library/postgres:17.4-bookworm}"
PG_USER="${POSTGRES_USER:-pgrdf}"
PG_PASS="${POSTGRES_PASSWORD:-pgrdf}"
PG_DB="${POSTGRES_DB:-pgrdf}"
DATA_GID="${DATA_GID:-92000}"
GRAD_STUDENT_IRI="file:///opt/lubm/univ-bench.owl#GraduateStudent"

fail() { printf '[bench] FATAL: %s\n' "$*" >&2; exit "${2:-1}"; }

[ "${RUNTIME}" = "docker" ] || fail "this runner requires docker (got '${RUNTIME}')" 1
${RUNTIME} volume inspect "${LUBM_VOLUME}" >/dev/null 2>&1 \
  || fail "${LUBM_VOLUME} volume not present — run 'just lubm-gen ${UNIV_COUNT}' first" 1
[ -f "${EXTENSIONS_DIR}/lib/pgrdf.so" ] \
  || fail "${EXTENSIONS_DIR}/lib/pgrdf.so missing — run 'just build-ext' first" 1
[ -f "${EXTENSIONS_DIR}/share/extension/pgrdf.control" ] \
  || fail "pgrdf.control missing under ${EXTENSIONS_DIR}/share/extension/" 1

DEFAULT_VERSION="$(sed -n "s/^default_version = '\\(.*\\)'/\\1/p" \
  "${EXTENSIONS_DIR}/share/extension/pgrdf.control")"
SQL_FILE="${EXTENSIONS_DIR}/share/extension/pgrdf--${DEFAULT_VERSION}.sql"
[ -f "${SQL_FILE}" ] || fail "missing ${SQL_FILE}" 1

DATA_PATH="/lubm-data/lubm-${UNIV_COUNT}/nt/lubm-${UNIV_COUNT}.nt"
${RUNTIME} run --rm -v "${LUBM_VOLUME}:/lubm-data:ro" alpine:3.20 \
  test -f "${DATA_PATH}" \
  || fail "data file ${DATA_PATH} missing inside ${LUBM_VOLUME} — run 'just lubm-gen ${UNIV_COUNT}' first" 1

mkdir -p "${HISTORY_DIR}"

# ── sidecar boot ────────────────────────────────────────────────────
cleanup() { ${RUNTIME} rm -f "${SIDECAR_NAME}" >/dev/null 2>&1 || true; }
trap cleanup EXIT INT TERM

printf '[bench] booting %s (pg %s, pgrdf %s, lubm-%s, profiles=%s)\n' \
  "${SIDECAR_NAME}" "${PG_IMAGE##*:}" "${DEFAULT_VERSION}" "${UNIV_COUNT}" "${PROFILES}"

${RUNTIME} run --rm -d \
  --name "${SIDECAR_NAME}" \
  -e POSTGRES_USER="${PG_USER}" \
  -e POSTGRES_PASSWORD="${PG_PASS}" \
  -e POSTGRES_DB="${PG_DB}" \
  -v "${LUBM_VOLUME}:/lubm-data:ro" \
  -v "${EXTENSIONS_DIR}/lib/pgrdf.so:/usr/lib/postgresql/17/lib/pgrdf.so:ro" \
  -v "${EXTENSIONS_DIR}/share/extension/pgrdf.control:/usr/share/postgresql/17/extension/pgrdf.control:ro" \
  -v "${SQL_FILE}:/usr/share/postgresql/17/extension/pgrdf--${DEFAULT_VERSION}.sql:ro" \
  "${PG_IMAGE}" >/dev/null

for i in 1 2 3 4 5 6 7 8 9 10; do
  ${RUNTIME} exec "${SIDECAR_NAME}" pg_isready -U "${PG_USER}" >/dev/null 2>&1 && break
  sleep 1
done

psql_exec() {
  ${RUNTIME} exec -i "${SIDECAR_NAME}" \
    psql -U "${PG_USER}" -d "${PG_DB}" -X -A -t -q -v ON_ERROR_STOP=1
}

# Install + bind graph.
echo "CREATE EXTENSION pgrdf; SELECT pgrdf.add_graph(${DATA_GID});" | psql_exec >/dev/null \
  || fail "extension/graph setup failed" 2

# ── ingest ────────────────────────────────────────────────────────────
INGEST_JSON=$(psql_exec <<SQL
SELECT pgrdf.load_turtle_verbose('${DATA_PATH}', ${DATA_GID})::text;
SQL
) || fail "ingest failed" 3

extract_num() {
  # `set -euo pipefail` would propagate grep's no-match-exit-1 through
  # the pipe; wrap in `|| true` so absent keys cleanly return empty.
  printf '%s' "$1" \
    | grep -oE "\"$2\"[ ]*:[ ]*[0-9]+(\\.[0-9]+)?" 2>/dev/null \
    | grep -oE "[0-9]+(\\.[0-9]+)?$" 2>/dev/null \
    | head -1 \
    || true
}
INGEST_TRIPLES=$(extract_num "${INGEST_JSON}" "triples")
INGEST_ELAPSED=$(extract_num "${INGEST_JSON}" "elapsed_ms")
INGEST_PARSE_MS=$(extract_num "${INGEST_JSON}" "parse_ms")
INGEST_DICT_MS=$(extract_num "${INGEST_JSON}" "dict_ms")
INGEST_INSERT_MS=$(extract_num "${INGEST_JSON}" "insert_ms")
INGEST_DICT_HITS=$(extract_num "${INGEST_JSON}" "dict_cache_hits")
INGEST_SHMEM_HITS=$(extract_num "${INGEST_JSON}" "shmem_cache_hits")
INGEST_DB_CALLS=$(extract_num "${INGEST_JSON}" "dict_db_calls")
INGEST_BATCHES=$(extract_num "${INGEST_JSON}" "quad_batches")
[ -n "${INGEST_TRIPLES}" ] || fail "ingest report missing triples: ${INGEST_JSON}" 3

printf '[bench] ingest: triples=%s elapsed_ms=%s\n' "${INGEST_TRIPLES}" "${INGEST_ELAPSED}"

# ── materialize per profile ──────────────────────────────────────────
mat_block=""
IFS=',' read -ra MAT_PROFS <<< "${PROFILES}"
for prof in "${MAT_PROFS[@]}"; do
  printf '[bench] materializing %s ...\n' "${prof}"
  MAT_JSON=$(psql_exec <<SQL
SELECT pgrdf.materialize(${DATA_GID}, '${prof}')::text;
SQL
  ) || fail "materialize(${prof}) failed" 4
  MAT_ELAPSED=$(extract_num "${MAT_JSON}" "elapsed_ms")
  # The actual key from src/inference/reasonable.rs is
  # `inferred_triples_written`. Older builds may have used
  # `triples_inferred` / `inferred`; check all three for robustness.
  MAT_INFERRED=$(extract_num "${MAT_JSON}" "inferred_triples_written")
  if [ -z "${MAT_INFERRED}" ]; then MAT_INFERRED=$(extract_num "${MAT_JSON}" "triples_inferred"); fi
  if [ -z "${MAT_INFERRED}" ]; then MAT_INFERRED=$(extract_num "${MAT_JSON}" "inferred"); fi
  printf '[bench]   %s: elapsed_ms=%s triples_inferred=%s\n' \
    "${prof}" "${MAT_ELAPSED:-?}" "${MAT_INFERRED:-?}"
  key="${prof//-/_}"
  mat_block="${mat_block}\"${key}\":{\"elapsed_ms\":${MAT_ELAPSED:-null},\"triples_inferred\":${MAT_INFERRED:-null}},"
done
mat_block="${mat_block%,}"

# ── Q14 (warm median of 3) ──────────────────────────────────────────
printf '[bench] running Q14 (3 warm passes, median)\n'
Q14_RESULT_COUNT=""
declare -a TIMES=()

# Warm pass first (discard timing).
psql_warm=$(${RUNTIME} exec -i "${SIDECAR_NAME}" \
  psql -U "${PG_USER}" -d "${PG_DB}" -X -A -t -q -v ON_ERROR_STOP=1 <<SQL || true
\\timing on
SELECT sparql::text FROM pgrdf.sparql('PREFIX : <http://x/> SELECT (COUNT(?s) AS ?n) WHERE { ?s a <${GRAD_STUDENT_IRI}> }');
SQL
)
Q14_RESULT_COUNT=$(printf '%s' "${psql_warm}" | grep -oE '"[0-9]+"' | head -1 | tr -d '"')

for i in 1 2 3; do
  raw=$(${RUNTIME} exec -i "${SIDECAR_NAME}" \
    psql -U "${PG_USER}" -d "${PG_DB}" -X -A -t -q -v ON_ERROR_STOP=1 <<SQL || true
\\timing on
SELECT sparql::text FROM pgrdf.sparql('PREFIX : <http://x/> SELECT (COUNT(?s) AS ?n) WHERE { ?s a <${GRAD_STUDENT_IRI}> }');
SQL
  )
  t=$(printf '%s' "${raw}" | grep -oE "Time: [0-9]+(\\.[0-9]+)? ms" | grep -oE "[0-9]+(\\.[0-9]+)?" | head -1)
  [ -n "${t}" ] || fail "Q14 timing extract failed (run ${i}): ${raw}" 5
  TIMES+=("${t}")
done
Q14_MS=$(printf '%s\n' "${TIMES[@]}" | LC_ALL=C sort -n | awk 'NR==2')
printf '[bench] Q14: count=%s median_ms=%s\n' "${Q14_RESULT_COUNT:-?}" "${Q14_MS}"

# ── emit single-line JSON record ────────────────────────────────────
TS_UTC=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
TS_UNIX=$(date +%s)
HOST=$(hostname -s)
GIT_SHA=$(cd "${REPO_ROOT}" && git rev-parse HEAD 2>/dev/null || echo "")
GIT_BRANCH=$(cd "${REPO_ROOT}" && git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")
PG_MAJOR=$(${RUNTIME} exec "${SIDECAR_NAME}" psql -U "${PG_USER}" -d "${PG_DB}" \
  -X -A -t -q -c "SHOW server_version_num;" | awk '{print int($1/10000)}')

RUN_JSON=$(cat <<JSON
{"ts":"${TS_UTC}","ts_unix":${TS_UNIX},"host":"${HOST}","git_sha":"${GIT_SHA}","git_branch":"${GIT_BRANCH}","pgrdf_version":"${DEFAULT_VERSION}","postgres_major":${PG_MAJOR},"lubm_size":${UNIV_COUNT},"triples":${INGEST_TRIPLES},"ingest":{"elapsed_ms":${INGEST_ELAPSED:-null},"parse_ms":${INGEST_PARSE_MS:-null},"dict_ms":${INGEST_DICT_MS:-null},"insert_ms":${INGEST_INSERT_MS:-null},"dict_cache_hits":${INGEST_DICT_HITS:-null},"shmem_cache_hits":${INGEST_SHMEM_HITS:-null},"dict_db_calls":${INGEST_DB_CALLS:-null},"quad_batches":${INGEST_BATCHES:-null}},"materialize":{${mat_block}},"q14":{"elapsed_ms_median":${Q14_MS:-null},"result_count":${Q14_RESULT_COUNT:-null}}}
JSON
)

echo "${RUN_JSON}" > "${HISTORY_DIR}/last-run.json"
echo "${RUN_JSON}" >> "${HISTORY_DIR}/runs.jsonl"
printf '[bench] appended to %s/runs.jsonl (now %s runs total)\n' \
  "${HISTORY_DIR}" "$(wc -l < "${HISTORY_DIR}/runs.jsonl" | tr -d ' ')"

# ── render HTML report ──────────────────────────────────────────────
if command -v python3 >/dev/null 2>&1; then
  python3 "${REPO_ROOT}/tests/perf/render-history.py" \
    --history "${HISTORY_DIR}/runs.jsonl" \
    --out "${HISTORY_DIR}/index.html" \
    && printf '[bench] rendered %s/index.html\n' "${HISTORY_DIR}"
else
  printf '[bench] python3 missing — skipping HTML render\n'
fi
