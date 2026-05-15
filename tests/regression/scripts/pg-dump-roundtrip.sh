#!/usr/bin/env bash
#
# tests/regression/scripts/pg-dump-roundtrip.sh — pg_dump round-trip
# regression for `_pgrdf_graphs` (LLD v0.4 §3.1 acceptance criterion:
# "`pg_dump` of a pgRDF database carrying the mapping round-trips the
# mapping verbatim").
#
# Phase A countdown slice 110.
#
# Three-step orchestration (cannot run inside `psql -c`; pg_dump is a
# separate binary):
#
#   1. Boot a clean state inside the compose Postgres: drop the
#      extension if present, re-create it, seed two known IRI bindings
#      via `pgrdf.add_graph(id::bigint, iri)`.
#   2. `pg_dump` the database to a tmpfile (inside the container);
#      grep for the seeded IRI strings as a fast canary.
#   3. Drop the extension (wipes all rows), re-create it (fresh empty
#      `_pgrdf_graphs` with only the seed row), restore from the dump,
#      then re-query `_pgrdf_graphs` and verify the two seeded rows
#      survived the round trip.
#
# Defaults match `tests/regression/run.sh`: container `pgrdf-postgres`,
# runtime `podman`, user `pgrdf`, db `pgrdf`. Override via:
#
#   PGRDF_CONTAINER=… PGRDF_RUNTIME=… POSTGRES_USER=… POSTGRES_DB=…
#
# Idempotent: the cleanup tail re-creates a fresh extension so a
# follow-on test run starts from a known clean state. Safe to re-run.
#
# Empirical verification: this script is committed worktree-local in
# parallel/slice-110; the parent merge agent runs it after both
# parallel batch-2 slices land on main. Do not run while the parallel
# agent in /tmp/pgrdf-wt-112 has a competing claim on the compose
# stack.
#
# KNOWN RISK — `pg_extension_config_dump`:
#
#   `_pgrdf_graphs` is created by the extension's install SQL
#   (`sql/schema_v0_4_0_graphs.sql` loaded via `extension_sql_file!`).
#   By default `pg_dump` treats extension-owned tables as part of the
#   extension's DDL — only the table is recreated on restore, NOT its
#   row data. The rows survive a round trip ONLY if the extension
#   registers the table via
#
#       SELECT pg_catalog.pg_extension_config_dump(
#           '_pgrdf_graphs', ''
#       );
#
#   inside its install SQL. As of slice 110 head, that call is NOT
#   present in `sql/schema_v0_4_0_graphs.sql`. If this script's
#   verification fails on `expected 2 rows, got 0`, the missing
#   `pg_extension_config_dump` registration is the cause — flagged in
#   slice 110's report for the parent agent to wire in slice 112's
#   territory (or as a follow-up patch). The IRI-string grep in
#   step 2 catches the same gap one step earlier: if pg_dump
#   produces zero matches, the rows were never serialised.

set -eu

CONTAINER="${PGRDF_CONTAINER:-pgrdf-postgres}"
RUNTIME="${PGRDF_RUNTIME:-podman}"
USR="${POSTGRES_USER:-pgrdf}"
DB="${POSTGRES_DB:-pgrdf}"
DUMP_IN_CONTAINER="/tmp/pgrdf-dump-test.sql"

trap 'rm -f /tmp/pgrdf-dump-test.local.sql' EXIT

echo "[round-trip] preparing graphs ..."
"${RUNTIME}" exec -i "${CONTAINER}" psql -U "${USR}" -d "${DB}" -v ON_ERROR_STOP=1 <<'SQL'
DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.add_graph(101::bigint, 'http://example.org/rt-1');
SELECT pgrdf.add_graph(102::bigint, 'http://example.org/rt-2');
SELECT graph_id, iri FROM pgrdf._pgrdf_graphs
 WHERE graph_id IN (101, 102)
 ORDER BY graph_id;
SQL

echo "[round-trip] dumping (data-only, scoped to pgrdf._pgrdf_graphs) ..."
# Scope the dump to the IRI-mapping table only. The LLD v0.4 §3.1
# acceptance criterion is specifically about the IRI mapping surviving
# pg_dump — not the full quad/partition graph (Postgres's partition-
# tables-under-extension-owned-parents round-trip story is a separate
# concern). --inserts --on-conflict-do-nothing handles the seed-row
# duplicate when add_graph(0,'urn:pgrdf:graph:0') re-runs at extension
# install time.
"${RUNTIME}" exec -i "${CONTAINER}" \
    bash -c "pg_dump --data-only --inserts --on-conflict-do-nothing --table=pgrdf._pgrdf_graphs -U '${USR}' -d '${DB}' > '${DUMP_IN_CONTAINER}'"

# Copy the dump out for grep + later restore.
"${RUNTIME}" exec -i "${CONTAINER}" cat "${DUMP_IN_CONTAINER}" \
    > /tmp/pgrdf-dump-test.local.sql

# Canary: the user-bound IRIs must appear somewhere in the dump.
# Without pg_extension_config_dump('_pgrdf_graphs', '') in the install
# SQL, pg_dump skips extension-owned row data and these greps fail.
if ! grep -q 'http://example.org/rt-1' /tmp/pgrdf-dump-test.local.sql; then
    echo "[round-trip] FAIL: dump missing rt-1 IRI" >&2
    echo "[round-trip]   likely cause: _pgrdf_graphs not registered" >&2
    echo "[round-trip]   via pg_extension_config_dump()" >&2
    exit 1
fi
if ! grep -q 'http://example.org/rt-2' /tmp/pgrdf-dump-test.local.sql; then
    echo "[round-trip] FAIL: dump missing rt-2 IRI" >&2
    echo "[round-trip]   likely cause: _pgrdf_graphs not registered" >&2
    echo "[round-trip]   via pg_extension_config_dump()" >&2
    exit 1
fi

echo "[round-trip] wiping user-bound IRI rows ..."
# Keep the extension installed; delete only the user-bound rows so
# the seed (graph_id=0) stays in place. The restore then re-INSERTs
# the user rows. This proves the round-trip without resetting the
# extension's install state.
"${RUNTIME}" exec -i "${CONTAINER}" psql -U "${USR}" -d "${DB}" -v ON_ERROR_STOP=1 <<'SQL'
DELETE FROM pgrdf._pgrdf_graphs WHERE graph_id IN (101, 102);
SQL

echo "[round-trip] restoring from dump ..."
"${RUNTIME}" exec -i "${CONTAINER}" psql -U "${USR}" -d "${DB}" -v ON_ERROR_STOP=1 \
    < /tmp/pgrdf-dump-test.local.sql > /dev/null

echo "[round-trip] verifying restored rows ..."
COUNT="$("${RUNTIME}" exec -i "${CONTAINER}" psql -U "${USR}" -d "${DB}" -A -t \
    -c "SELECT count(*) FROM pgrdf._pgrdf_graphs WHERE iri IN ('http://example.org/rt-1', 'http://example.org/rt-2')")"
COUNT="${COUNT//[$'\t\r\n ']}"
if [ "${COUNT}" != "2" ]; then
    echo "[round-trip] FAIL: expected 2 restored rows, got ${COUNT}" >&2
    echo "[round-trip]   likely cause: pg_dump emitted CREATE EXTENSION" >&2
    echo "[round-trip]   only, without _pgrdf_graphs row data" >&2
    exit 1
fi

# Symmetric IRI lookup must agree too — verifies the row really did
# round-trip and isn't a CREATE EXTENSION side-effect.
IRI_101="$("${RUNTIME}" exec -i "${CONTAINER}" psql -U "${USR}" -d "${DB}" -A -t \
    -c "SELECT pgrdf.graph_iri(101::bigint)")"
IRI_101="${IRI_101//[$'\t\r\n ']}"
if [ "${IRI_101}" != "http://example.org/rt-1" ]; then
    echo "[round-trip] FAIL: graph_iri(101) = '${IRI_101}', want 'http://example.org/rt-1'" >&2
    exit 1
fi

echo "[round-trip] cleanup ..."
"${RUNTIME}" exec -i "${CONTAINER}" psql -U "${USR}" -d "${DB}" -v ON_ERROR_STOP=1 \
    -c "DROP EXTENSION IF EXISTS pgrdf CASCADE; CREATE EXTENSION pgrdf;" > /dev/null
"${RUNTIME}" exec -i "${CONTAINER}" rm -f "${DUMP_IN_CONTAINER}"

echo "[round-trip] OK"
