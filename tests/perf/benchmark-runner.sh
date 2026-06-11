#!/usr/bin/env bash
#
# tests/perf/benchmark-runner.sh — pgRDF benchmark harness with
# persistent run history + HTML report.
#
# Boots an isolated postgres+pgrdf sidecar, runs LUBM-N ingest +
# LUBM Q1-Q14 at THREE reasoning profiles (none, rdfs, owl-rl):
#
#   profile=none   : queries against the raw ABox (no inference)
#   profile=rdfs   : queries after pgrdf.materialize(g, 'rdfs')
#   profile=owl-rl : queries after pgrdf.materialize(g, 'owl-rl')
#
# Per (profile, query) the runner captures result_count + the
# median-of-3-warm elapsed_ms. Counts are compared against
# `tests/perf/lubm/queries/expected-counts.json`; null entries get
# written back on first observation so future runs detect drift.
#
# DOCKER ONLY (per [[docker-only-pgrdf-prefix]]). Sidecar name is
# `pgrdf-bench-pg-<pid>` (separate from `run-lubm.sh`'s
# `pgrdf-perf-pg-<pid>`).
#
# Pre-flight:
#   1. `just lubm-build` has produced the UBA generator image.
#   2. `just lubm-gen <N>` has populated the `pgrdf-lubm-data` volume.
#   3. `just build-ext` has populated `compose/extensions/`.
#
# Usage:
#   bash tests/perf/benchmark-runner.sh             # LUBM-10 (default)
#   bash tests/perf/benchmark-runner.sh 1           # LUBM-1
#   PROFILES=none,rdfs bash tests/perf/benchmark-runner.sh 10
#
# Outputs:
#   target/perf-history/runs.jsonl        appended one line per run
#   target/perf-history/index.html        re-rendered each run
#   target/perf-history/last-run.json     full output of the most recent run
#   tests/perf/lubm/queries/expected-counts.json   updated with observed counts where null

set -euo pipefail

UNIV_COUNT="${1:-10}"
PROFILES="${PROFILES:-none,rdfs,owl-rl}"
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
REPO_ROOT="$(cd "${REPO_ROOT}/.." && pwd)"
HISTORY_DIR="${HISTORY_DIR:-${REPO_ROOT}/target/perf-history}"
QUERIES_DIR="${QUERIES_DIR:-${REPO_ROOT}/tests/perf/lubm/queries}"
EXPECTED_FILE="${EXPECTED_FILE:-${QUERIES_DIR}/expected-counts.json}"
RUNTIME="${PGRDF_RUNTIME:-docker}"
LUBM_VOLUME="${LUBM_VOLUME:-pgrdf-lubm-data}"
EXTENSIONS_DIR="${EXTENSIONS_DIR:-${REPO_ROOT}/compose/extensions}"
SIDECAR_NAME="pgrdf-bench-pg-$$"
PG_IMAGE="${PG_IMAGE:-docker.io/library/postgres:17.4-bookworm}"
# Baked-image mode: when PGRDF_BAKED_IMAGE is set, the sidecar runs a
# self-contained image with pgrdf (.so/.control/.sql) + the LUBM Tbox
# fixtures already baked in, and the host bind-mounts for those are
# skipped. This makes the benchmark portable to a daemon that does NOT
# share the repo's host filesystem (e.g. a separate colima VM reached
# via DOCKER_HOST). Default (unset) = the original host-bind-mount path.
BAKED_IMAGE="${PGRDF_BAKED_IMAGE:-}"
[ -n "${BAKED_IMAGE}" ] && PG_IMAGE="${BAKED_IMAGE}"
# Optional Postgres tuning: PGRDF_PG_ARGS is appended verbatim to the
# postgres server command (e.g. "-c work_mem=256MB -c shared_buffers=8GB").
# PGRDF_SHM_SIZE sets the container --shm-size (needed when shared_buffers
# / parallel workers are bumped). Both default to empty (= stock postgres
# config, the out-of-the-box baseline). The effective values are echoed
# at boot + recorded in the run JSON so each pass's config is traceable.
read -r -a PG_TUNE_ARGS <<< "${PGRDF_PG_ARGS:-}"
PG_SHM_SIZE="${PGRDF_SHM_SIZE:-}"
PG_CONFIG_LABEL="${PGRDF_CONFIG_LABEL:-$([ -n "${PGRDF_PG_ARGS:-}" ] && echo tuned || echo default)}"
PG_USER="${POSTGRES_USER:-pgrdf}"
PG_PASS="${POSTGRES_PASSWORD:-pgrdf}"
PG_DB="${POSTGRES_DB:-pgrdf}"
DATA_GID="${DATA_GID:-92000}"

fail() { printf '[bench] FATAL: %s\n' "$*" >&2; exit "${2:-1}"; }

[ "${RUNTIME}" = "docker" ] || fail "this runner requires docker (got '${RUNTIME}')" 1
${RUNTIME} volume inspect "${LUBM_VOLUME}" >/dev/null 2>&1 \
  || fail "${LUBM_VOLUME} volume not present — run 'just lubm-gen ${UNIV_COUNT}' first" 1
if [ -z "${BAKED_IMAGE}" ]; then
  [ -f "${EXTENSIONS_DIR}/lib/pgrdf.so" ] \
    || fail "${EXTENSIONS_DIR}/lib/pgrdf.so missing — run 'just build-ext' first" 1
fi

# DEFAULT_VERSION is read from the host control file in both modes (the
# host always has compose/extensions even when the sidecar daemon does
# not). PGRDF_VERSION can override it for baked mode if the image was
# built elsewhere.
DEFAULT_VERSION="${PGRDF_VERSION:-$(sed -n "s/^default_version = '\\(.*\\)'/\\1/p" \
  "${EXTENSIONS_DIR}/share/extension/pgrdf.control")}"
SQL_FILE="${EXTENSIONS_DIR}/share/extension/pgrdf--${DEFAULT_VERSION}.sql"
if [ -z "${BAKED_IMAGE}" ]; then
  [ -f "${SQL_FILE}" ] || fail "missing ${SQL_FILE}" 1
fi

DATA_PATH="/lubm-data/lubm-${UNIV_COUNT}/nt/lubm-${UNIV_COUNT}.nt"
${RUNTIME} run --rm -v "${LUBM_VOLUME}:/lubm-data:ro" alpine:3.20 \
  test -f "${DATA_PATH}" \
  || fail "data file ${DATA_PATH} missing inside ${LUBM_VOLUME} — run 'just lubm-gen ${UNIV_COUNT}' first" 1

mkdir -p "${HISTORY_DIR}"

# Count the 14 query files; runner expects q01..q14.
QUERY_FILES=()
for n in 01 02 03 04 05 06 07 08 09 10 11 12 13 14; do
  f="${QUERIES_DIR}/q${n}.rq"
  [ -f "${f}" ] || fail "missing query file ${f}" 1
  QUERY_FILES+=("${f}")
done

# ── sidecar boot ────────────────────────────────────────────────────
cleanup() { ${RUNTIME} rm -f "${SIDECAR_NAME}" >/dev/null 2>&1 || true; }
trap cleanup EXIT INT TERM

printf '[bench] booting %s (pg %s, pgrdf %s, lubm-%s, profiles=%s, config=%s)\n' \
  "${SIDECAR_NAME}" "${PG_IMAGE##*:}" "${DEFAULT_VERSION}" "${UNIV_COUNT}" "${PROFILES}" "${PG_CONFIG_LABEL}"
[ "${#PG_TUNE_ARGS[@]}" -gt 0 ] && printf '[bench] pg tuning: %s\n' "${PG_TUNE_ARGS[*]}"

# Mount args: the LUBM data volume is always mounted. In default mode
# the ext (.so/.control/.sql) + Tbox fixtures are bind-mounted from the
# host; in baked mode they're already inside the image, so we skip them.
MOUNT_ARGS=(-v "${LUBM_VOLUME}:/lubm-data:ro")
if [ -z "${BAKED_IMAGE}" ]; then
  MOUNT_ARGS+=(
    -v "${REPO_ROOT}/tests/perf/lubm/fixtures:/fixtures:ro"
    -v "${EXTENSIONS_DIR}/lib/pgrdf.so:/usr/lib/postgresql/17/lib/pgrdf.so:ro"
    -v "${EXTENSIONS_DIR}/share/extension/pgrdf.control:/usr/share/postgresql/17/extension/pgrdf.control:ro"
    -v "${SQL_FILE}:/usr/share/postgresql/17/extension/pgrdf--${DEFAULT_VERSION}.sql:ro"
  )
fi

SHM_ARGS=()
[ -n "${PG_SHM_SIZE}" ] && SHM_ARGS=(--shm-size "${PG_SHM_SIZE}")
# `${arr[@]+"${arr[@]}"}` expands to nothing when the array is empty,
# which is safe under `set -u` across bash versions (an empty array is
# the default-config path: no shm override, no tuning flags).
${RUNTIME} run --rm -d \
  --name "${SIDECAR_NAME}" \
  ${SHM_ARGS[@]+"${SHM_ARGS[@]}"} \
  -e POSTGRES_USER="${PG_USER}" \
  -e POSTGRES_PASSWORD="${PG_PASS}" \
  -e POSTGRES_DB="${PG_DB}" \
  "${MOUNT_ARGS[@]}" \
  "${PG_IMAGE}" ${PG_TUNE_ARGS[@]+"${PG_TUNE_ARGS[@]}"} >/dev/null

for i in 1 2 3 4 5 6 7 8 9 10; do
  ${RUNTIME} exec "${SIDECAR_NAME}" pg_isready -U "${PG_USER}" >/dev/null 2>&1 && break
  sleep 1
done

psql_exec() {
  ${RUNTIME} exec -i "${SIDECAR_NAME}" \
    psql -U "${PG_USER}" -d "${PG_DB}" -X -A -t -q -v ON_ERROR_STOP=1
}

echo "CREATE EXTENSION pgrdf; SELECT pgrdf.add_graph(${DATA_GID});" | psql_exec >/dev/null \
  || fail "extension/graph setup failed" 2

# ── ingest ────────────────────────────────────────────────────────────
INGEST_JSON=$(psql_exec <<SQL
SELECT pgrdf.load_turtle_verbose('${DATA_PATH}', ${DATA_GID})::text;
SQL
) || fail "ingest failed" 3

extract_num() {
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

# ── Tbox ingest (LUBM univ-bench ontology) ──────────────────────────
# pgrdf.materialize(g, 'rdfs'|'owl-rl') only fires inferences over the
# Tbox+ABox CO-LOCATED in the same graph. UBA's generated .nt files
# are ABox-only; the Tbox is shipped separately. We load tests/perf/
# lubm/fixtures/univ-bench.ttl (the rewritten univ-bench.owl with
# `file:///opt/lubm/univ-bench.owl#` namespace matching the ABox) into
# the same graph before materialize so the reasoner sees the
# subClassOf / subPropertyOf chains the LUBM queries depend on.
TBOX_JSON=$(psql_exec <<SQL || true
SELECT pgrdf.load_turtle_verbose('/fixtures/univ-bench.ttl', ${DATA_GID})::text;
SQL
)
TBOX_TRIPLES=$(extract_num "${TBOX_JSON}" "triples")
TBOX_ELAPSED=$(extract_num "${TBOX_JSON}" "elapsed_ms")
if [ -n "${TBOX_TRIPLES}" ]; then
  printf '[bench] tbox:   triples=%s elapsed_ms=%s\n' "${TBOX_TRIPLES}" "${TBOX_ELAPSED}"
else
  printf '[bench] tbox:   load skipped (file missing or ingest failed)\n'
  TBOX_TRIPLES=0
fi

# ── per-profile loop: optionally materialize, then run all 14 queries ─
run_query() {
  # $1 = query file path; $2 = scratch out var name (return string)
  # Returns "<count>;<median_ms>" via echo.
  local qfile="$1"
  local qtext
  qtext=$(cat "${qfile}" | sed -e 's/^#.*//' | tr '\n' ' ' | tr -s ' ')
  # Escape single quotes for the inner SPARQL literal.
  local qesc="${qtext//\'/\'\'}"

  # Wrap to count rows + capture warm timing. We run COUNT(*) over the
  # SETOF JSONB the SPARQL SELECT projects; pgrdf.sparql returns one row
  # per solution.
  local sql="\\timing on
SELECT count(*) FROM pgrdf.sparql('${qesc}');"

  local raw_warm
  raw_warm=$(printf '%s\n' "${sql}" | ${RUNTIME} exec -i "${SIDECAR_NAME}" \
    psql -U "${PG_USER}" -d "${PG_DB}" -X -A -t -q -v ON_ERROR_STOP=1 2>&1 || true)

  local times=()
  for i in 1 2 3; do
    local raw
    raw=$(printf '%s\n' "${sql}" | ${RUNTIME} exec -i "${SIDECAR_NAME}" \
      psql -U "${PG_USER}" -d "${PG_DB}" -X -A -t -q -v ON_ERROR_STOP=1 2>&1 || true)
    local t
    t=$(printf '%s' "${raw}" | grep -oE "Time: [0-9]+(\\.[0-9]+)? ms" | grep -oE "[0-9]+(\\.[0-9]+)?" | head -1)
    [ -n "${t}" ] || t="0"
    times+=("${t}")
  done

  local count
  count=$(printf '%s' "${raw_warm}" | grep -oE "^[0-9]+$" | head -1 || true)
  [ -n "${count}" ] || count="0"

  local median
  median=$(printf '%s\n' "${times[@]}" | LC_ALL=C sort -n | awk 'NR==2')

  echo "${count};${median}"
}

profiles_json=""
IFS=',' read -ra MAT_PROFS <<< "${PROFILES}"
for prof in "${MAT_PROFS[@]}"; do
  printf '[bench] profile=%s\n' "${prof}"

  mat_elapsed="null"
  mat_inferred="null"
  if [ "${prof}" != "none" ]; then
    MAT_JSON=$(psql_exec <<SQL
SELECT pgrdf.materialize(${DATA_GID}, '${prof}')::text;
SQL
    ) || fail "materialize(${prof}) failed" 4
    mat_elapsed=$(extract_num "${MAT_JSON}" "elapsed_ms")
    [ -n "${mat_elapsed}" ] || mat_elapsed="null"
    mat_inferred=$(extract_num "${MAT_JSON}" "inferred_triples_written")
    [ -n "${mat_inferred}" ] || mat_inferred="null"
    printf '[bench]   materialize: elapsed_ms=%s triples_inferred=%s\n' "${mat_elapsed}" "${mat_inferred}"
  fi

  q_block=""
  for n in 01 02 03 04 05 06 07 08 09 10 11 12 13 14; do
    qfile="${QUERIES_DIR}/q${n}.rq"
    result=$(run_query "${qfile}")
    count="${result%%;*}"
    median="${result##*;}"
    printf '[bench]   q%s: count=%s median_ms=%s\n' "${n}" "${count}" "${median}"
    q_block="${q_block}\"q${n}\":{\"count\":${count},\"elapsed_ms_median\":${median}},"
  done
  q_block="${q_block%,}"

  profiles_json="${profiles_json}\"${prof//-/_}\":{\"materialize_ms\":${mat_elapsed},\"triples_inferred\":${mat_inferred},\"queries\":{${q_block}}},"
done
profiles_json="${profiles_json%,}"

# ── emit single-line JSON record ────────────────────────────────────
TS_UTC=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
TS_UNIX=$(date +%s)
HOST=$(hostname -s)
GIT_SHA=$(cd "${REPO_ROOT}" && git rev-parse HEAD 2>/dev/null || echo "")
GIT_BRANCH=$(cd "${REPO_ROOT}" && git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")
PG_MAJOR=$(${RUNTIME} exec "${SIDECAR_NAME}" psql -U "${PG_USER}" -d "${PG_DB}" \
  -X -A -t -q -c "SHOW server_version_num;" | awk '{print int($1/10000)}')

RUN_JSON=$(cat <<JSON
{"ts":"${TS_UTC}","ts_unix":${TS_UNIX},"host":"${HOST}","git_sha":"${GIT_SHA}","git_branch":"${GIT_BRANCH}","pgrdf_version":"${DEFAULT_VERSION}","postgres_major":${PG_MAJOR},"lubm_size":${UNIV_COUNT},"triples":${INGEST_TRIPLES},"tbox_triples":${TBOX_TRIPLES:-0},"ingest":{"elapsed_ms":${INGEST_ELAPSED:-null},"parse_ms":${INGEST_PARSE_MS:-null},"dict_ms":${INGEST_DICT_MS:-null},"insert_ms":${INGEST_INSERT_MS:-null},"dict_cache_hits":${INGEST_DICT_HITS:-null},"shmem_cache_hits":${INGEST_SHMEM_HITS:-null},"dict_db_calls":${INGEST_DB_CALLS:-null},"quad_batches":${INGEST_BATCHES:-null}},"profiles":{${profiles_json}}}
JSON
)

echo "${RUN_JSON}" > "${HISTORY_DIR}/last-run.json"
echo "${RUN_JSON}" >> "${HISTORY_DIR}/runs.jsonl"
printf '[bench] appended to %s/runs.jsonl (now %s runs total)\n' \
  "${HISTORY_DIR}" "$(wc -l < "${HISTORY_DIR}/runs.jsonl" | tr -d ' ')"

# ── update expected-counts.json (null → observed) + flag drift ──────
if command -v python3 >/dev/null 2>&1 && [ -f "${EXPECTED_FILE}" ]; then
  python3 "${REPO_ROOT}/tests/perf/lubm/queries/update-expected.py" \
    --expected "${EXPECTED_FILE}" \
    --run "${HISTORY_DIR}/last-run.json" \
    --lubm-size "${UNIV_COUNT}" \
    && printf '[bench] expected-counts.json reconciled\n'
fi

if command -v python3 >/dev/null 2>&1; then
  python3 "${REPO_ROOT}/tests/perf/render-history.py" \
    --history "${HISTORY_DIR}/runs.jsonl" \
    --out "${HISTORY_DIR}/index.html" \
    && printf '[bench] rendered %s/index.html\n' "${HISTORY_DIR}"
fi
