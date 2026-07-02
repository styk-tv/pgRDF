#!/usr/bin/env bash
#
# tests/w3c-sparql/run.sh — W3C-shape SPARQL conformance harness.
#
# ALL orchestration lives in the Rust binary `pgrdf-oracle run`
# (tests/oracle/ — issue #17 course-correction 2; the earlier bash
# per-fixture wiring was a spike and is eliminated). This script only
# resolves the container/env context, builds the binary, and invokes
# it once.
#
# Each subdirectory of tests/w3c-sparql/ is one test:
#   <NN>-<name>/
#     data.ttl       — Turtle loaded into a fresh graph (single-graph
#                      default; `setup.sql` is the multi-graph
#                      alternative for §13.3 GRAPH fixtures)
#     query.rq       — SPARQL query executed via pgrdf.sparql
#                      (`kind` file routes `construct` / `describe`
#                      fixtures through their entry points)
#     expected.jsonl — one JSONB row per line, byte-sorted (the
#                      blocking golden gate; bag-equivalent)
#     oracle         — differential-oracle marker: `eligible`,
#                      `ineligible: <reason>`, or
#                      `known-divergence: <issue-ref> — <reason>`
#                      (absent = ineligible). Eligible fixtures are
#                      additionally evaluated with spareval
#                      (Oxigraph's W3C SPARQL 1.1 evaluator) and
#                      diffed engine-vs-oracle under canonicalization
#                      + blank-node isomorphism; a divergence on an
#                      `eligible` fixture FAILS the run.
#
# The `fixtures/` subdirectory is reserved for the official W3C
# rdf-tests suite (issue #17 course-correction 1, manifest-driven).
#
# Usage:
#   bash tests/w3c-sparql/run.sh                  # all tests
#   bash tests/w3c-sparql/run.sh 01-basic-bgp     # one test
#   ACCEPT=1 bash tests/w3c-sparql/run.sh ...     # regenerate expected.jsonl
#                                                  (the oracle still
#                                                  second-opinions each
#                                                  regenerated golden)

set -u

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
TESTS_DIR="${REPO_ROOT}/tests/w3c-sparql"
CONTAINER="${PGRDF_CONTAINER:-pgrdf-pgrdf-postgres}"
RUNTIME="${PGRDF_RUNTIME:-podman}"
PSQL_USER="${POSTGRES_USER:-pgrdf}"
PSQL_DB="${POSTGRES_DB:-pgrdf}"
ACCEPT="${ACCEPT:-0}"

if ! cargo build --quiet --release \
    --manifest-path "${REPO_ROOT}/tests/oracle/Cargo.toml"; then
  echo "run.sh: pgrdf-oracle failed to build" >&2
  exit 2
fi
ORACLE_BIN="${REPO_ROOT}/tests/oracle/target/release/pgrdf-oracle"

# The engine command receives each fixture's SQL stream on stdin and
# emits psql rows on stdout. Single-quoted user/db (resolved here) so
# the binary can hand the whole string to `sh -c`.
ENGINE_CMD="${RUNTIME} exec -i ${CONTAINER} psql -U '${PSQL_USER}' -d '${PSQL_DB}' -X -A -t -q -v ON_ERROR_STOP=1"

args=(run --fixtures "${TESTS_DIR}" --engine-cmd "${ENGINE_CMD}")
if [ "${ACCEPT}" = "1" ]; then
  args+=(--accept)
fi
if [ -n "${1:-}" ]; then
  args+=(--filter "$1")
fi

exec "${ORACLE_BIN}" "${args[@]}"
