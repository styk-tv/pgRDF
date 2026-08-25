#!/usr/bin/env bash
# lib-tdd three-state runner. GREEN = target met · RED = fails exactly as
# predicted (a PASS) · BROKEN = fails for an unstated reason (the only alarm).
# Exit: non-zero iff any BROKEN.
#
# Bench selection mirrors tests/regression/run.sh conventions (container exec)
# unless PGRDF_TDD_PSQL provides a full psql command for any other instance.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"

if [ -z "${PGRDF_TDD_PSQL:-}" ]; then
  CONTAINER="${PGRDF_CONTAINER:-pgrdf-postgres}"
  RUNTIME="${PGRDF_RUNTIME:-docker}"
  PSQL_USER="${POSTGRES_USER:-pgrdf}"
  PSQL_DB="${POSTGRES_DB:-pgrdf}"
  export PGRDF_TDD_PSQL="${RUNTIME} exec -i ${CONTAINER} psql -U ${PSQL_USER} -d ${PSQL_DB} -X -q"
fi

filter="${1:-}"
declare -a cases=()
for c in "${HERE}"/cases/*.sh; do
  [ -f "$c" ] || continue
  name="$(basename "$c" .sh)"
  if [ -z "$filter" ] || [ "$name" = "$filter" ]; then cases+=("$c"); fi
done
[ "${#cases[@]}" -gt 0 ] || { echo "no cases matched"; exit 2; }

pass_green=0; pass_red=0; fail_broken=0
printf '%-34s %-7s %s\n' "CASE" "STATE" "NOTE"
for c in "${cases[@]}"; do
  name="$(basename "$c" .sh)"
  out="$(bash "$c" 2>&1)"; rc=$?
  note="$(printf '%s\n' "$out" | sed -n 's/^STATUS [A-Z]* — //p' | tail -1)"
  case "$rc" in
    0)  state=GREEN;  pass_green=$((pass_green+1));;
    44) state=RED;    pass_red=$((pass_red+1));;
    *)  state=BROKEN; fail_broken=$((fail_broken+1))
        [ -n "$note" ] || note="$(printf '%s' "$out" | tail -2 | tr '\n' ' ')";;
  esac
  printf '%-34s %-7s %s\n' "$name" "$state" "$note"
done
echo
echo "green ${pass_green} · red-as-predicted ${pass_red} · BROKEN ${fail_broken}"
[ "$fail_broken" -eq 0 ]
