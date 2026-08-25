#!/usr/bin/env bash
# TDD T-BENCH-TRIPLE (PASS-2 §1.2, RETENTION) · the bench must prove what it runs
# before any suite result is citable: version == extversion, build_id populated.
# This case caught a stale pre-tag .so on the compose bench's first boot.
. "$(dirname "$0")/../lib.sh"
v="$(scalar "SELECT pgrdf.version()")"
b="$(scalar "SELECT pgrdf.build_id()")"
e="$(scalar "SELECT extversion FROM pg_extension WHERE extname='pgrdf'")"
[ -n "$v" ] || broken "pgrdf.version() returned nothing — extension not loaded?"
if [ "$v" != "$e" ]; then
  broken "version ($v) != extversion ($e) — ALTER EXTENSION pending or wrong .so mapped"
fi
case "$b" in
  unknown) red "build_id is 'unknown' — built without PGRDF_BUILD_ID (workstation self-confession); rebuild with the seam populated" ;;
  *-dirty) red "build_id is '$b' — dirty-tree build; fine for iteration, never citable" ;;
  "")      broken "build_id empty" ;;
  *)       green "triple holds: $v == $e, build_id $b" ;;
esac
