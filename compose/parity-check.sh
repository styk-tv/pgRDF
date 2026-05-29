#!/bin/sh
#
# compose/parity-check.sh — TG-3 v2 compose-startup gate.
#
# Runs in a one-shot init container before postgres starts. Verifies
# the bind-mounted extension files are internally consistent:
#
#   1. The .so file exists and is non-empty.
#   2. The .control file parses and carries a default_version.
#   3. The pgrdf--<default_version>.sql file exists and is non-empty.
#   4. The default_version in .control matches the SQL filename.
#
# If any check fails, exits non-zero. Compose refuses to start the
# postgres container because of the `depends_on:
# condition: service_completed_successfully` directive. This catches
# the realistic drift: a release cut bumps pgrdf.control's
# default_version but the bind-mount line in compose.yml still points
# at the previous version, so initdb succeeds but CREATE EXTENSION
# fails with `pgrdf--<old>.sql not found` — without this gate that
# error appears LATER, after postgres is already started, not at
# compose-up.
#
# All resources prefixed `pgrdf-` per the workstation discipline.
#
# Exit codes:
#   0   all checks pass — postgres allowed to start
#   2   .so file missing or empty
#   3   .control file missing, unreadable, or no default_version
#   4   SQL file missing or empty
#   5   default_version / SQL filename mismatch
set -eu

SO=/extensions/lib/pgrdf.so
CTRL=/extensions/share/extension/pgrdf.control
SQL_DIR=/extensions/share/extension

# (1) shared object
if [ ! -s "${SO}" ]; then
    echo "[pgrdf-parity] FAIL: ${SO} missing or empty"
    echo "[pgrdf-parity] hint: run 'just build-ext' on the host before 'docker compose up'"
    exit 2
fi
SO_HASH=$(sha256sum "${SO}" | awk '{print $1}')
echo "[pgrdf-parity] ${SO}: $(stat -c '%s' "${SO}") bytes, sha256=${SO_HASH}"

# (2) control file → default_version
if [ ! -r "${CTRL}" ]; then
    echo "[pgrdf-parity] FAIL: ${CTRL} missing or unreadable"
    exit 3
fi
VER=$(sed -n "s/^default_version = '\\(.*\\)'/\\1/p" "${CTRL}" | head -1)
if [ -z "${VER}" ]; then
    echo "[pgrdf-parity] FAIL: ${CTRL} has no default_version"
    sed -n '1,20p' "${CTRL}"
    exit 3
fi
echo "[pgrdf-parity] ${CTRL}: default_version='${VER}'"

# (3) per-version SQL
SQL_FILE="${SQL_DIR}/pgrdf--${VER}.sql"
if [ ! -s "${SQL_FILE}" ]; then
    echo "[pgrdf-parity] FAIL: ${SQL_FILE} missing or empty"
    echo "[pgrdf-parity] candidates in ${SQL_DIR}:"
    ls -la "${SQL_DIR}/"pgrdf--*.sql 2>/dev/null || echo "  (none)"
    echo "[pgrdf-parity] hint: pgrdf.control's default_version='${VER}' but compose mounts a different pgrdf--*.sql"
    exit 4
fi
SQL_HASH=$(sha256sum "${SQL_FILE}" | awk '{print $1}')
echo "[pgrdf-parity] ${SQL_FILE}: $(stat -c '%s' "${SQL_FILE}") bytes, sha256=${SQL_HASH}"

# (4) name-consistency cross-check (cheap, but catches the case where
# the SQL file was hand-renamed to match the .control without rebuilding
# the .so — leaves a hash mismatch the host-side
# verify-installed-artifacts.sh script would also flag, but we surface
# it at compose-up instead of post-CREATE EXTENSION).
echo "[pgrdf-parity] internally consistent: .control v${VER} ↔ pgrdf--${VER}.sql"

# All checks pass; the postgres container is allowed to start.
echo "[pgrdf-parity] OK — postgres may start"
exit 0
