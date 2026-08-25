#!/usr/bin/env bash
# E5/K9 groundwork: dump the LIVE surface as "name(identity_args)<TAB>kind<TAB>volatility<TAB>strict"
# sorted, for diffing against classification.tsv. The unit is (proname, identity_args)
# — PL-1: Rust names collapse into SQL overloads; pg_proc is the only truthful source.
#
# Membership is EXTENSION OWNERSHIP (pg_depend deptype 'e'), NOT schema: the pgrdf
# schema is writable, and anything else may live beside the extension — measured
# 2026-08-25 when the regression suite's own plpgsql helpers (_check_error,
# _carve_raises) appeared in a namespace-based dump and correctly broke case 12.
set -u
PSQL="${PGRDF_TDD_PSQL:?export PGRDF_TDD_PSQL (full psql command)}"
$PSQL -tA -F $'\t' -c "
  SELECT p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')',
         p.prokind, p.provolatile, p.proisstrict
  FROM pg_proc p
  JOIN pg_depend d ON d.classid = 'pg_proc'::regclass
                  AND d.objid = p.oid AND d.deptype = 'e'
  JOIN pg_extension e ON e.oid = d.refobjid AND e.extname = 'pgrdf'
  ORDER BY 1;"
