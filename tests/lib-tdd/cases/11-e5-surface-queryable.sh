#!/usr/bin/env bash
# TDD E5-1 · the surface is queryable from the engine (capability detection as a
# query, not an inference from absence — the L8 cure)
# TARGET  pgrdf.surface() returns (name, args, class, since) rows
# PREDICTED TODAY  absent
. "$(dirname "$0")/../lib.sh"
have="$(scalar "SELECT coalesce(to_regprocedure('pgrdf.surface()')::text,'ABSENT')")"
if [ "$have" = "ABSENT" ] || [ -z "$have" ]; then
  red "pgrdf.surface() does not exist — capability detection stays to_regprocedure guesswork (L8)"
fi
n="$(scalar "SELECT count(*) FROM pgrdf.surface()")"
# Extension OWNERSHIP, not schema — the pgrdf schema is writable and test
# helpers legitimately live beside the extension (measured 2026-08-25).
total="$(scalar "SELECT count(*) FROM pg_proc p JOIN pg_depend d ON d.classid='pg_proc'::regclass AND d.objid=p.oid AND d.deptype='e' JOIN pg_extension e ON e.oid=d.refobjid AND e.extname='pgrdf'")"
if [ "$n" = "$total" ]; then
  green "pgrdf.surface() exists and covers all $total exports"
else
  broken "pgrdf.surface() exists but covers $n of $total exports — an unclassified export escaped K9-2"
fi
