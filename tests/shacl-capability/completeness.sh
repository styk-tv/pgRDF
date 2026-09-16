#!/usr/bin/env bash
# SHACL Core completeness gate.
#
#   bash tests/shacl-capability/completeness.sh
#
# Asks one question the capability document cannot ask of itself:
#
#     is every feature of the SHACL Core surface MEASURED AT ALL?
#
# `CAPABILITY.json` reports what the probes found. It cannot report what no
# probe looked at. Until this gate existed, the harness measured 17 of 46
# features and reported `not_enforced: []` -- true of the seventeen, read by
# everyone downstream as a clean bill of health for the surface.
#
#     A green suite is not coverage. Coverage is a fraction, and the
#     denominator must not be computable from the numerator.
#
# So the denominator is SHACL-CORE-SURFACE.tsv, enumerated from the W3C
# Recommendation. Adding a probe cannot extend it. Only a change to the
# specification, or a corrected reading of it, can -- and that is a reviewable
# edit rather than a side effect of writing a test.
#
# This gate asserts a probe EXISTS per feature. It does not assert the probe is
# correct: a wrong probe passes it. See the verdict caveats in CAPABILITY.json.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SURFACE="$HERE/SHACL-CORE-SURFACE.tsv"
FIX="$HERE/fixtures"

red()   { printf '\033[31m%s\033[0m\n' "$*"; }
green() { printf '\033[32m%s\033[0m\n' "$*"; }

[ -f "$SURFACE" ] || { red "FAIL: $SURFACE missing — no denominator, no coverage claim"; exit 1; }
[ -d "$FIX" ]     || { red "FAIL: $FIX missing"; exit 1; }

total=0; measured=0; missing=()
while IFS=$'\t' read -r feature section kind; do
  case "$feature" in ''|\#*) continue ;; esac
  total=$((total+1))
  if [ -f "$FIX/${feature}.shapes.ttl" ]; then
    measured=$((measured+1))
  else
    missing+=( "$(printf '%-30s §%-9s %s' "$feature" "$section" "$kind")" )
  fi
done < "$SURFACE"

# The reverse direction: a probe with no row in the surface is either a
# feature missing from the enumeration or a probe named wrong. Both matter.
orphans=()
for f in "$FIX"/*.shapes.ttl; do
  n="$(basename "$f" .shapes.ttl)"
  grep -qE "^${n}"$'\t' "$SURFACE" || orphans+=( "$n" )
done

echo "SHACL Core coverage: ${measured}/${total} features probed"
echo "  denominator: $(basename "$SURFACE") (W3C SHACL Recommendation)"
echo

fail=0
if [ ${#missing[@]} -gt 0 ]; then
  red "UNMEASURED — ${#missing[@]} feature(s) have no probe:"
  printf '    %s\n' "${missing[@]}"
  red "  Add fixtures/<feature>.{shapes,control,violating}.ttl for each."
  fail=1
fi
if [ ${#orphans[@]} -gt 0 ]; then
  red "UNLISTED — ${#orphans[@]} probe(s) have no row in the surface:"
  printf '    %s\n' "${orphans[@]}"
  red "  Either the enumeration is incomplete, or the probe is misnamed."
  fail=1
fi

if [ "$fail" -eq 0 ]; then
  green "completeness: PASS — every SHACL Core feature has a probe (${measured}/${total})"
  exit 0
fi
red ""
red "completeness: FAIL"
red "  A release must not claim a surface it has not measured."
exit 1
