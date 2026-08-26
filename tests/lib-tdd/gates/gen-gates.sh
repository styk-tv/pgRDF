#!/usr/bin/env bash
# T-GATE-SWEEP (K10-1): mechanical census of refusal gates.
#
# Two populations, one file:
#   refuse:  crate::refuse(CODE, ...) sites — RECLASSIFIED gates, code named
#   panic:   panic! sites whose message reads as a gate — sweep REMAINDER
#
# Test modules are excluded: scanning stops at the first #[cfg(test)] /
# #[cfg(any(test, ...))] line of each file (test mods sit at file end in this
# crate). The first census predated this and counted 9 path.rs test
# assertions as gates — measured 2026-08-26, corrected here.
# Run from the repo root; regenerate on every sweep and diff the snapshot.
set -u
cd "$(dirname "$0")/../../.." || exit 1
for f in $(find src -name '*.rs' | sort); do
  awk -v F="$f" '
    /#\[cfg\((any\()?test/ { exit }
    /crate::refuse\(/ {
      line=$0
      if (getline nxt > 0) { line = line " " nxt }
      match(line, /ERRCODE_[A-Z_]+/)
      code = (RSTART ? substr(line, RSTART, RLENGTH) : "code-on-later-line")
      printf "%s:%d\trefuse\t%s\n", F, FNR, code
      next
    }
    /panic!/ {
      if ($0 ~ /refus|locked|must |cannot |not allowed|exceed|budget|unsupported|mismatch|invalid|requires|unknown |out of scope|not yet supported|non-empty|bound to a different/ \
          && $0 !~ /failed|corrupt|internal|vanished|expect\(/) {
        msg = $0
        sub(/^[ \t]+/, "", msg)
        printf "%s:%d\tpanic\t%.90s\n", F, FNR, msg
      }
    }
  ' "$f"
done
