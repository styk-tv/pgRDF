#!/usr/bin/env bash
# lib-tdd case helpers. Sourced by every case; PGRDF_TDD_PSQL is set by run.sh.
#
# Exit protocol (M1 three-state):
#   green "msg"  -> exit 0   target behaviour met
#   red   "msg"  -> exit 44  fails exactly as predicted today (a suite PASS)
#   broken "msg" -> exit 1   fails for an unstated reason (the only alarm)
set -u
PSQL="${PGRDF_TDD_PSQL:?cases run via run.sh (or export PGRDF_TDD_PSQL)}"

# Run N statements in ONE session (VERBOSITY verbose so errors carry SQLSTATE);
# stdout+stderr combined.
sql() {
  local args=(-c '\set VERBOSITY verbose') s
  for s in "$@"; do args+=(-c "$s"); done
  $PSQL "${args[@]}" 2>&1
}

# SQLSTATE of the first error across the statements, or NONE.
sqlstate() {
  local out st
  out="$(sql "$@")"
  st="$(printf '%s\n' "$out" | sed -n 's/^ERROR:  \([0-9A-Z]\{5\}\):.*/\1/p' | head -1)"
  if [ -n "$st" ]; then printf '%s' "$st"; else printf 'NONE'; fi
}

# Single scalar, errors suppressed (pair with sqlstate for the error path).
scalar() { $PSQL -tA -c "$1" 2>/dev/null | head -1; }

# Best-effort teardown of a urn:tdd:* graph (strict-NULL makes absent a no-op).
drop_g() { scalar "SELECT pgrdf.drop_graph(pgrdf.graph_id('$1'))" >/dev/null; }

green()  { echo "STATUS GREEN — $1";  exit 0; }
red()    { echo "STATUS RED — $1";    exit 44; }
broken() { echo "STATUS BROKEN — $1"; exit 1; }
