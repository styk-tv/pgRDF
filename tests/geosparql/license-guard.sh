#!/usr/bin/env bash
# License guard for tests/geosparql.
#
#   bash tests/geosparql/license-guard.sh
#
# pgRDF is MIT and this directory is PUBLISHED IN A PUBLIC REPOSITORY.
# Everything here is pgRDF's own work: no third-party material is vendored.
# This guard keeps it that way.
#
# A failure here is a CRITICAL STOP, not a warning. Do not commit, do not
# push, do not override. See LICENSE.md.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
F="$HERE/fixtures"
fail=0
red()  { printf '\033[31m%s\033[0m\n' "$*"; }
green(){ printf '\033[32m%s\033[0m\n' "$*"; }

[ -d "$F" ] || { red "FAIL: $F does not exist"; exit 1; }

# 1. Nothing third-party is vendored. These paths existed in an earlier
#    revision and were removed once the fixtures were re-authored; the guard
#    names them so they cannot quietly return.
for d in upstream vendor third_party third-party; do
  if [ -e "$HERE/$d" ]; then
    red "FAIL: vendored third-party directory present: $d/"
    red "      This tree vendors nothing. See LICENSE.md."
    fail=1
  fi
done
for bad in validator.ttl geo.ttl demo-dataset.ttl DIGESTS.txt; do
  found=$(find "$HERE" -name "$bad" 2>/dev/null || true)
  [ -n "$found" ] && { red "FAIL: refused artefact present: $found"; fail=1; }
done
if find "$HERE" -name 'moreton-island.*' 2>/dev/null | grep -q .; then
  red "FAIL: refused artefact present: moreton-island.*"; fail=1
fi

# 2. Every fixture declares MIT.
missing=0
while IFS= read -r f; do
  head -3 "$f" | grep -q 'SPDX-License-Identifier: MIT' || {
    red "FAIL: fixture missing SPDX MIT header: ${f#$HERE/}"; missing=1; fail=1; }
done < <(find "$F" -name '*.ttl')
[ "$missing" -eq 0 ] || red "      Every fixture is pgRDF's own work and must say so."

# 3. No foreign licensing declaration anywhere.
hits=$(grep -rniE \
  'dcterms:license|schema:license|sdo:license|dc:rights|dcterms:rights|ogc\.org/license|creativecommons\.org' \
  "$F" 2>/dev/null || true)
if [ -n "$hits" ]; then
  red "FAIL: foreign licensing declaration in a fixture:"
  echo "$hits" | sed 's/^/    /'; fail=1
fi

# 4. No copyleft or otherwise incompatible terms, ever.
hits=$(grep -rniE \
  '\bGPL\b|GNU General Public|LGPL|AGPL|Mozilla Public|Eclipse Public|\bCDDL\b|SSPL|Commons Clause|non-commercial|noncommercial' \
  "$F" 2>/dev/null || true)
if [ -n "$hits" ]; then
  red "FAIL: incompatible license term in a fixture:"
  echo "$hits" | sed 's/^/    /'; fail=1
fi

# 5. Only .ttl under fixtures/.
unexpected=$(find "$F" -type f ! -name '*.ttl' 2>/dev/null || true)
if [ -n "$unexpected" ]; then
  red "FAIL: unexpected non-.ttl file under fixtures/:"
  echo "$unexpected" | sed 's/^/    /'; fail=1
fi

n=$(find "$F" -name '*.ttl' | wc -l | tr -d ' ')
r=$(find "$F" -mindepth 1 -maxdepth 1 -type d | wc -l | tr -d ' ')

if [ "$fail" -eq 0 ]; then
  green "license-guard: PASS — $n pgRDF-authored fixtures over $r rules, all MIT, nothing vendored"
  exit 0
fi
red ""
red "==================================================================="
red " ⛔ CRITICAL STOP — license problem in a PUBLIC repository tree"
red "==================================================================="
red " Do NOT commit. Do NOT push. Do NOT override."
red ""
red " This tree is 100% pgRDF's own work, MIT. Nothing third-party is"
red " vendored. Neither .gitignore nor a container is a license boundary."
red " See LICENSE.md."
red "==================================================================="
exit 1
