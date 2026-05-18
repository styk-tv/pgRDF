#!/usr/bin/env bash
#
# tests/regression/scripts/verify-installed-artifacts.sh — prove that
# the running compose Postgres has the intended pgRDF extension bytes
# mounted, not a stale local artifact set.
#
# Verification chain:
#   1. Build a fresh export from the current source tree into a temp dir
#      (without mutating compose/extensions/).
#   2. Byte-compare that fresh export to the host's compose/extensions/.
#   3. Verify the running container's mount sources point at THIS repo's
#      compose/extensions/ files.
#   4. Hash-compare the bytes inside the running container to the host
#      files.
#   5. Sanity-check the SQL-visible version surface (`extversion`,
#      `pgrdf.version()`) matches pgrdf.control's default_version.
#
# Requires:
#   - a running compose Postgres container
#   - `CREATE EXTENSION pgrdf;` already executed in that database
#   - build runtime access (`docker` by default)
#
# Defaults:
#   PG_MAJOR=17
#   PGRDF_BUILD_RUNTIME=docker
#   PGRDF_RUNTIME (or PGRDF_RUN_RUNTIME)=podman
#   PGRDF_CONTAINER=pgrdf-pgrdf-postgres

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../../.." && pwd -P)"
PG_MAJOR="${PG_MAJOR:-17}"
BUILD="${PGRDF_BUILD_RUNTIME:-docker}"
RUN="${PGRDF_RUNTIME:-${PGRDF_RUN_RUNTIME:-podman}}"
CONTAINER="${PGRDF_CONTAINER:-pgrdf-pgrdf-postgres}"
USR="${POSTGRES_USER:-pgrdf}"
DB="${POSTGRES_DB:-pgrdf}"

VERSION="$(sed -n "s/^default_version = '\\(.*\\)'/\\1/p" "${REPO_ROOT}/pgrdf.control")"
[ -n "${VERSION}" ] || {
    echo "[artifact-parity] FAIL: could not read default_version from pgrdf.control" >&2
    exit 1
}

HOST_SO="${REPO_ROOT}/compose/extensions/lib/pgrdf.so"
HOST_CONTROL="${REPO_ROOT}/compose/extensions/share/extension/pgrdf.control"
HOST_SQL="${REPO_ROOT}/compose/extensions/share/extension/pgrdf--${VERSION}.sql"

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/pgrdf-artifact-parity.XXXXXX")"
FRESH_DIR="${TMP_DIR}/fresh"
EXPORT_IMAGE="pgrdf-artifact-verify:pg${PG_MAJOR}-$$"
EXPORT_CID=""

cleanup() {
    [ -z "${EXPORT_CID}" ] || "${BUILD}" rm -f "${EXPORT_CID}" >/dev/null 2>&1 || true
    "${BUILD}" image rm -f "${EXPORT_IMAGE}" >/dev/null 2>&1 || true
    rm -rf "${TMP_DIR}"
}
trap cleanup EXIT

canon_path() {
    local path="$1"
    local dir
    dir="$(cd "$(dirname "${path}")" && pwd -P)"
    printf '%s/%s\n' "${dir}" "$(basename "${path}")"
}

sha256_host() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

cmp_bytes() {
    local left="$1"
    local right="$2"
    local label="$3"

    [ -f "${left}" ] || {
        echo "[artifact-parity] FAIL: missing file ${left}" >&2
        exit 1
    }
    [ -f "${right}" ] || {
        echo "[artifact-parity] FAIL: missing file ${right}" >&2
        exit 1
    }

    if ! cmp -s "${left}" "${right}"; then
        echo "[artifact-parity] FAIL: ${label} differs" >&2
        echo "  left : ${left}" >&2
        echo "  hash : $(sha256_host "${left}")" >&2
        echo "  right: ${right}" >&2
        echo "  hash : $(sha256_host "${right}")" >&2
        exit 1
    fi

    echo "[artifact-parity] OK host bytes: ${label}"
}

mount_source_for() {
    local dest="$1"
    "${RUN}" inspect "${CONTAINER}" \
        --format "{{range .Mounts}}{{if eq .Destination \"${dest}\"}}{{.Source}}{{end}}{{end}}"
}

sha256_container() {
    local path="$1"
    "${RUN}" exec "${CONTAINER}" sh -lc "sha256sum '${path}' | awk '{print \$1}'"
}

SO_DEST="/usr/lib/postgresql/${PG_MAJOR}/lib/pgrdf.so"
CONTROL_DEST="/usr/share/postgresql/${PG_MAJOR}/extension/pgrdf.control"
SQL_DEST="/usr/share/postgresql/${PG_MAJOR}/extension/pgrdf--${VERSION}.sql"

echo "[artifact-parity] building fresh export from current source (${BUILD}, pg${PG_MAJOR}) ..."
mkdir -p "${FRESH_DIR}"
DOCKER_BUILDKIT=1 "${BUILD}" build \
    -t "${EXPORT_IMAGE}" \
    --build-arg PG_MAJOR="${PG_MAJOR}" \
    -f "${REPO_ROOT}/compose/builder.Containerfile" \
    "${REPO_ROOT}" >/dev/null
# Extract directly from the image filesystem. A bind-mounted temp dir
# under macOS's /var/folders is not visible inside Docker/Colima.
EXPORT_CID="$("${BUILD}" create "${EXPORT_IMAGE}")"
"${BUILD}" cp "${EXPORT_CID}:/out/." "${FRESH_DIR}/"
"${BUILD}" rm -f "${EXPORT_CID}" >/dev/null
EXPORT_CID=""

echo "[artifact-parity] comparing fresh export to compose/extensions ..."
cmp_bytes "${HOST_SO}" "${FRESH_DIR}/lib/pgrdf.so" "pgrdf.so"
cmp_bytes "${HOST_CONTROL}" "${FRESH_DIR}/share/extension/pgrdf.control" "pgrdf.control"
cmp_bytes "${HOST_SQL}" "${FRESH_DIR}/share/extension/pgrdf--${VERSION}.sql" "pgrdf--${VERSION}.sql"

echo "[artifact-parity] inspecting running container ${CONTAINER} ..."
"${RUN}" inspect "${CONTAINER}" >/dev/null 2>&1 || {
    echo "[artifact-parity] FAIL: container not found: ${CONTAINER}" >&2
    exit 1
}

actual_so_src="$(canon_path "$(mount_source_for "${SO_DEST}")")"
actual_control_src="$(canon_path "$(mount_source_for "${CONTROL_DEST}")")"
actual_sql_src="$(canon_path "$(mount_source_for "${SQL_DEST}")")"

expected_so_src="$(canon_path "${HOST_SO}")"
expected_control_src="$(canon_path "${HOST_CONTROL}")"
expected_sql_src="$(canon_path "${HOST_SQL}")"

[ "${actual_so_src}" = "${expected_so_src}" ] || {
    echo "[artifact-parity] FAIL: wrong mount source for ${SO_DEST}" >&2
    echo "  expected: ${expected_so_src}" >&2
    echo "  actual:   ${actual_so_src}" >&2
    exit 1
}
[ "${actual_control_src}" = "${expected_control_src}" ] || {
    echo "[artifact-parity] FAIL: wrong mount source for ${CONTROL_DEST}" >&2
    echo "  expected: ${expected_control_src}" >&2
    echo "  actual:   ${actual_control_src}" >&2
    exit 1
}
[ "${actual_sql_src}" = "${expected_sql_src}" ] || {
    echo "[artifact-parity] FAIL: wrong mount source for ${SQL_DEST}" >&2
    echo "  expected: ${expected_sql_src}" >&2
    echo "  actual:   ${actual_sql_src}" >&2
    exit 1
}

echo "[artifact-parity] OK mount sources point at this repo"

echo "[artifact-parity] comparing container bytes to host bytes ..."
[ "$(sha256_container "${SO_DEST}")" = "$(sha256_host "${HOST_SO}")" ] || {
    echo "[artifact-parity] FAIL: container bytes differ for pgrdf.so" >&2
    exit 1
}
[ "$(sha256_container "${CONTROL_DEST}")" = "$(sha256_host "${HOST_CONTROL}")" ] || {
    echo "[artifact-parity] FAIL: container bytes differ for pgrdf.control" >&2
    exit 1
}
[ "$(sha256_container "${SQL_DEST}")" = "$(sha256_host "${HOST_SQL}")" ] || {
    echo "[artifact-parity] FAIL: container bytes differ for pgrdf--${VERSION}.sql" >&2
    exit 1
}

echo "[artifact-parity] OK container bytes match host bytes"

echo "[artifact-parity] checking SQL-visible version surface ..."
readarray -t versions < <("${RUN}" exec -i "${CONTAINER}" \
    psql -U "${USR}" -d "${DB}" -X -A -t -v ON_ERROR_STOP=1 \
    -c "SELECT extversion FROM pg_extension WHERE extname='pgrdf'; SELECT pgrdf.version();")

[ "${#versions[@]}" -eq 2 ] || {
    echo "[artifact-parity] FAIL: expected 2 SQL version rows, got ${#versions[@]}" >&2
    exit 1
}
[ "${versions[0]}" = "${VERSION}" ] || {
    echo "[artifact-parity] FAIL: extversion=${versions[0]}, expected ${VERSION}" >&2
    exit 1
}
[ "${versions[1]}" = "${VERSION}" ] || {
    echo "[artifact-parity] FAIL: pgrdf.version()=${versions[1]}, expected ${VERSION}" >&2
    exit 1
}

echo "[artifact-parity] OK extversion and pgrdf.version() report ${VERSION}"
echo "[artifact-parity] OK"
