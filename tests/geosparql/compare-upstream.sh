#!/usr/bin/env bash
# Coverage check against the OGC corpus — DEVELOPMENT-TIME ONLY.
#
#   bash tests/geosparql/compare-upstream.sh
#
# pgRDF authors its own fixtures (LICENSE.md). This script answers the fair
# question that raises: did we lose coverage?
#
# It fetches the upstream corpus to a TEMPORARY directory OUTSIDE the
# repository, prints the coverage matrix from live data, and deletes it.
# Nothing is vendored, nothing is committed, and no test run depends on it.
set -euo pipefail

PIN="f90fff13235aaffaf69dcba97f3e01888167f080"
REPO="opengeospatial/ogc-geosparql"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

echo "==> Fetching upstream corpus @ ${PIN:0:12} (temporary, not vendored)"
files=$(curl -fsSL \
  "https://api.github.com/repos/${REPO}/contents/examples/shacl?ref=${PIN}" \
  | grep -o '"name": "[^"]*\.ttl"' | sed 's/.*: "//;s/"//')
for f in $files; do
  curl -fsSL "https://raw.githubusercontent.com/${REPO}/${PIN}/examples/shacl/${f}" \
    -o "$TMP/${f}"
done
echo "    $(ls -1 "$TMP"/*.ttl | wc -l | tr -d ' ') upstream fixtures"
echo

UP="$TMP" OURS="$HERE/fixtures" python3 - <<'PY'
import os, re, collections
up = collections.defaultdict(list)
for f in sorted(os.listdir(os.environ["UP"])):
    m = re.match(r'(S\d\d)-(.*)\.ttl', f)
    if m: up[m.group(1)].append(m.group(2))
ours = collections.defaultdict(list)
for d in sorted(os.listdir(os.environ["OURS"])):
    p = os.path.join(os.environ["OURS"], d)
    if os.path.isdir(p):
        ours[d[:3]] += [f for f in os.listdir(p) if f.endswith(".ttl")]

# Rules excluded on scope, with the reason. Not gaps.
EXCLUDED = {"S08": "DGGS — out of scope", "S20": "DGGS — out of scope"}

print("%-5s %-9s %-6s %s" % ("RULE", "UPSTREAM", "OURS", "STATUS"))
print("-" * 64)
problems = []
for r in sorted(set(up) | set(ours) | set(EXCLUDED)):
    u, o = len(up.get(r, [])), len(ours.get(r, []))
    if r in EXCLUDED and not o:
        st = "excluded — " + EXCLUDED[r]
    elif u and o >= u:
        st = "covered" + (" (+%d)" % (o - u) if o > u else "")
    elif u and not o:
        st = "** DROPPED **"; problems.append(r)
    elif u and o < u:
        st = "** SHORT by %d **" % (u - o); problems.append(r)
    else:
        st = "new — upstream ships none"
    print("%-5s %-9s %-6s %s" % (r, u or "-", o or "-", st))
print("-" * 64)
print("upstream: %d   ours: %d" % (sum(map(len, up.values())), sum(map(len, ours.values()))))
if problems:
    print("\nUNEXPLAINED GAPS: %s" % ", ".join(problems))
    print("Either add fixtures, or record the exclusion in COVERAGE.md and EXCLUDED here.")
    raise SystemExit(1)
print("\nNo unexplained gaps. COVERAGE.md is consistent with upstream at this pin.")
PY
