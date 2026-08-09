#!/usr/bin/env bash
#
# Pre-tag gate — run this BEFORE `git push origin v<ver>`.
#
# It replicates release.yml's pre-build assertions locally, against the
# working tree you are about to tag. `release.yml` refuses a mismatched tag,
# which is correct but expensive: the tag is already pushed by then, and a
# pushed tag is never reused (only-forward-never-revert). v0.6.24 was burned
# exactly this way — META.json still read 0.6.22 while the tag said 0.6.24.
#
# META.json is the reason this script exists. It is a documented member of
# the Rule 7 source set (PROVENANCE.md, "Cutting a release" step 2), and it
# is the one member a glob over *.toml / *.control / *.yml never reaches --
# so sweeping for version strings by filename silently skips it.
#
# Run it on the MERGE COMMIT, not the release branch: the merge commit is
# what gets tagged, and it is not always what you last tested.
#
#   git checkout main && git pull
#   scripts/pre-tag-check.sh 0.6.25
#
# Exit 0 = safe to tag. Non-zero = do not tag; fix and re-run.
set -uo pipefail

TAG_VER="${1:-}"
if [ -z "$TAG_VER" ]; then
  echo "usage: scripts/pre-tag-check.sh <version>   e.g. 0.6.25 (no leading v)" >&2
  exit 2
fi
case "$TAG_VER" in v*) echo "pass the bare version, not the tag name (0.6.25, not v0.6.25)" >&2; exit 2;; esac

FAIL=0
ok(){   printf '  OK    %-24s %s\n' "$1" "$2"; }
bad(){  printf '  FAIL  %-24s %s\n' "$1" "$2"; FAIL=1; }
chk(){  if [ "$2" = "$TAG_VER" ]; then ok "$1" "$2"; else bad "$1" "$2 != $TAG_VER"; fi; }

echo "pre-tag gate · v${TAG_VER} · $(git rev-parse --short HEAD) on $(git branch --show-current)"
echo

# --- Rule 7 source set (PROVENANCE.md "Cutting a release" step 2) ----------
chk "Cargo.toml"         "$(grep -m1 '^version'         Cargo.toml    | cut -d'"' -f2)"
chk "Cargo.lock (pgrdf)" "$(grep -A1 '^name = "pgrdf"$' Cargo.lock    | tail -1 | cut -d'"' -f2)"
chk "pgrdf.control"      "$(grep -m1 '^default_version' pgrdf.control | cut -d\' -f2)"
chk "META.json top"      "$(python3 -c "import json;print(json.load(open('META.json'))['version'])" 2>/dev/null)"
chk "META.json provides" "$(python3 -c "import json;print(json.load(open('META.json'))['provides']['pgrdf']['version'])" 2>/dev/null)"
chk "smoke golden"       "$(head -1 tests/regression/expected/00-smoke.out)"

# --- artifacts the version implies ----------------------------------------
if compgen -G "sql/pgrdf--*--${TAG_VER}.sql" >/dev/null; then
  ok "upgrade scripts" "$(ls sql/pgrdf--*--${TAG_VER}.sql | xargs -n1 basename | tr '\n' ' ')"
else
  bad "upgrade scripts" "no sql/pgrdf--*--${TAG_VER}.sql"
fi

# Every bridge must land a COMPLETE release. Postgres takes the SHORTEST
# update path, so an install on 0.5.1 reads the 0.5.1 bridge and never sees
# the newer script -- a bridge renamed forward without replaying the delta
# lands version-correct but function-incomplete.
NEWEST=$(ls sql/pgrdf--*--${TAG_VER}.sql 2>/dev/null | grep -v -- '--0\.5\.1--' | head -1)
BRIDGE=$(ls sql/pgrdf--0.5.1--${TAG_VER}.sql 2>/dev/null | head -1)
if [ -n "$NEWEST" ] && [ -n "$BRIDGE" ]; then
  MISSING=""
  while read -r fn; do
    grep -q "$fn" "$BRIDGE" || MISSING="$MISSING $fn"
  done < <(grep -oE 'CREATE (OR REPLACE )?FUNCTION "[a-z_]+"' "$NEWEST" | sort -u)
  [ -z "$MISSING" ] && ok "0.5.1 bridge complete" "carries the newer delta" \
                    || bad "0.5.1 bridge" "missing:$MISSING"
fi

grep -q "pgrdf--${TAG_VER}.sql" compose/compose.yml \
  && ok  "compose mount" "pgrdf--${TAG_VER}.sql" \
  || bad "compose mount" "not bumped"

grep -q "^## \[${TAG_VER}\]" CHANGELOG.md \
  && ok  "CHANGELOG" "[${TAG_VER}] section present" \
  || bad "CHANGELOG" "no [${TAG_VER}] section — the tag body renders from Unreleased at tag time"

make check-meta >/dev/null 2>&1 && ok "make check-meta" "pass" || bad "make check-meta" "fail"

# --- Rule 4: the PREVIOUS release must be advertised before tagging a new one
PREV=$(grep -m1 -oE 'v0\.[0-9]+\.[0-9]+' LATEST.md 2>/dev/null | head -1)
[ -n "$PREV" ] && ok "LATEST.md advertises" "$PREV (Rule 4 — must be the prior release)" \
               || bad "LATEST.md" "could not read a version"

# --- tree state -----------------------------------------------------------
D=$(git status --short | wc -l | tr -d ' '); U=$(git log @{u}.. --oneline 2>/dev/null | wc -l | tr -d ' ')
[ "$D" = "0" ] && ok "tree clean" "0 uncommitted" || bad "tree dirty" "$D uncommitted"
[ "$U" = "0" ] && ok "pushed" "0 unpushed"        || bad "unpushed" "$U commit(s)"

echo
if [ $FAIL -eq 0 ]; then
  echo "  PASS — safe to: git tag -a v${TAG_VER} -F <annotation> $(git rev-parse --short HEAD)"
else
  echo "  FAIL — do NOT tag. A pushed tag is never reused; fix and re-run."
fi
exit $FAIL
