#!/usr/bin/env bash
#
# tests/regression/scripts/pg-dump-dict-roundtrip.sh — #35 regression.
#
# Proves `_pgrdf_dictionary` survives a plain `pg_dump` + restore. Before the
# `pg_extension_config_dump('_pgrdf_dictionary','')` registration (added to
# sql/schema_v0_4_0_graphs.sql), `pg_dump` skipped the dict's ROW DATA — the
# runtime `add_graph` quad partitions are standalone tables and dumped fine, so
# a restore rebuilt all the quads pointing at an EMPTY dictionary: a silently
# broken graph (every subject/predicate/object id dangling). This asserts:
#   1. the dict data is present in a plain pg_dump (canary on a known term),
#   2. a restore into a fresh DB keeps dict + quads counts with ZERO orphan
#      subjects (every quad id still resolves in the restored dict).
#
# The graphs-seed-row duplicate-key + redundant-partition-index errors on
# restore are KNOWN, non-fatal pg_dump-compat wrinkles (#35: plain pg_dump is
# the lower-trust path; the robust portable export is `pgrdf.export_graph`,
# #36). The dict/quads DATA lands correctly regardless — that is what this
# verifies.
#
# Defaults match tests/regression/run.sh. Override via
#   PGRDF_CONTAINER=… PGRDF_RUNTIME=… POSTGRES_USER=… POSTGRES_DB=…
# Idempotent: re-creates a fresh extension on exit so a follow-on run is clean.

set -eu

CONTAINER="${PGRDF_CONTAINER:-pgrdf-pgrdf-postgres}"
RUNTIME="${PGRDF_RUNTIME:-podman}"
USR="${POSTGRES_USER:-pgrdf}"
DB="${POSTGRES_DB:-pgrdf}"
RDB="pgrdf_dict_rt_restore"
FIX="/fixtures/regression/multiload-dedup-sample.nt"

x() { "${RUNTIME}" exec -i "${CONTAINER}" "$@"; }

echo "[dict-rt] seeding dict + quads (graph 550) ..."
x psql -U "${USR}" -d "${DB}" -q -v ON_ERROR_STOP=1 <<SQL
DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.add_graph(550);
SELECT pgrdf.load_turtle('${FIX}', 550);
SQL

DICT0="$(x psql -U "${USR}" -d "${DB}" -At -c "SELECT count(*) FROM pgrdf._pgrdf_dictionary")"
QUAD0="$(x psql -U "${USR}" -d "${DB}" -At -c "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id=550")"
echo "[dict-rt] pre-dump: dict=${DICT0} quads=${QUAD0}"

echo "[dict-rt] pg_dump + dict canary ..."
x bash -c "pg_dump -U '${USR}' -d '${DB}' > /tmp/dict-rt.sql"
if ! x bash -c "grep -q 'COPY pgrdf._pgrdf_dictionary' /tmp/dict-rt.sql && grep -q 'example.org/alice' /tmp/dict-rt.sql"; then
    echo "[dict-rt] FAIL: dictionary data absent from pg_dump" >&2
    echo "[dict-rt]   cause: pg_extension_config_dump('_pgrdf_dictionary') not registered" >&2
    exit 1
fi

echo "[dict-rt] restore into fresh DB ${RDB} (non-fatal seed/index NOTICEs expected) ..."
x psql -U "${USR}" -d postgres -q -c "DROP DATABASE IF EXISTS ${RDB}"
x psql -U "${USR}" -d postgres -q -c "CREATE DATABASE ${RDB}"
x bash -c "psql -U '${USR}' -d '${RDB}' < /tmp/dict-rt.sql > /tmp/dict-rt.restore.log 2>&1 || true"

DICT1="$(x psql -U "${USR}" -d "${RDB}" -At -c "SELECT count(*) FROM pgrdf._pgrdf_dictionary")"
QUAD1="$(x psql -U "${USR}" -d "${RDB}" -At -c "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id=550")"
ORPH="$(x psql -U "${USR}" -d "${RDB}" -At -c "SELECT count(*) FROM pgrdf._pgrdf_quads q WHERE NOT EXISTS (SELECT 1 FROM pgrdf._pgrdf_dictionary d WHERE d.id = q.subject_id)")"
echo "[dict-rt] restored:  dict=${DICT1} quads=${QUAD1} orphan_subjects=${ORPH}"

echo "[dict-rt] cleanup ..."
x psql -U "${USR}" -d postgres -q -c "DROP DATABASE IF EXISTS ${RDB};" || true
x bash -c "rm -f /tmp/dict-rt.sql /tmp/dict-rt.restore.log" || true
x psql -U "${USR}" -d "${DB}" -q -c "DROP EXTENSION IF EXISTS pgrdf CASCADE; CREATE EXTENSION pgrdf;" >/dev/null

[ "${DICT1}" = "${DICT0}" ] || { echo "[dict-rt] FAIL: dict count ${DICT0} -> ${DICT1} (dictionary lost)" >&2; exit 1; }
[ "${QUAD1}" = "${QUAD0}" ] || { echo "[dict-rt] FAIL: quad count ${QUAD0} -> ${QUAD1}" >&2; exit 1; }
[ "${ORPH}" = "0" ] || { echo "[dict-rt] FAIL: ${ORPH} orphan subjects — dictionary did not round-trip" >&2; exit 1; }
echo "[dict-rt] OK"
