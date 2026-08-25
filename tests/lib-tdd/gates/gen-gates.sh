#!/usr/bin/env bash
# T-GATE-SWEEP groundwork (K10-1): mechanical census of deliberate refusal gates —
# panic! sites whose message reads as a gate, not an invariant break. Output:
# file:line<TAB>message-head, sorted, for gates.tsv. Regenerate on every sweep;
# a diff against the checked-in snapshot = gates added/removed without ledger update.
# Heuristic is the TDD §2 one; the sweep itself will hand-review each row into
# an E0 code + test. Run from the repo root.
set -u
cd "$(dirname "$0")/../../.." || exit 1
grep -rn 'panic!' src/ --include='*.rs' \
  | grep -viE '^\s*//|#\[should_panic' \
  | grep -iE 'refus|locked|must |cannot |not allowed|exceed|budget|unsupported|mismatch|invalid|requires' \
  | sed -E 's/^([^:]+:[0-9]+):.*panic!\(\s*"?([^"]{0,90}).*/\1\t\2/' \
  | sort
