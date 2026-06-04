#!/usr/bin/env python3
"""Reconcile observed Q1-Q14 counts from the latest run against the
checked-in expected-counts.json manifest.

Behaviour:
  - Null entries in the manifest get FILLED IN with the observed count.
    First-run capture.
  - Non-null entries get COMPARED to the observed count. Mismatches are
    printed to stderr (exit 0 still — drift surfaces but doesn't fail
    the benchmark; the user can lock the manifest manually).

Usage:
    python3 tests/perf/lubm/queries/update-expected.py \\
        --expected tests/perf/lubm/queries/expected-counts.json \\
        --run     target/perf-history/last-run.json \\
        --lubm-size 10
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--expected", required=True, type=Path)
    ap.add_argument("--run", required=True, type=Path)
    ap.add_argument("--lubm-size", required=True, type=int)
    args = ap.parse_args()

    expected = json.loads(args.expected.read_text())
    run = json.loads(args.run.read_text())

    size_key = f"lubm-{args.lubm_size}"
    if size_key not in expected:
        print(f"WARN: manifest has no '{size_key}' bucket; skipping reconcile",
              file=sys.stderr)
        return 0

    profiles = run.get("profiles") or {}
    drift = []
    captured = 0
    for prof_key, prof_data in profiles.items():
        # Normalize underscore back to dash for manifest key
        # (owl_rl -> owl-rl).
        mprof = prof_key.replace("_", "-")
        if mprof not in expected[size_key]:
            print(f"WARN: manifest has no '{mprof}' bucket under '{size_key}'",
                  file=sys.stderr)
            continue
        observed_queries = prof_data.get("queries") or {}
        for qid, obs in observed_queries.items():
            obs_count = obs.get("count")
            if obs_count is None:
                continue
            cur = expected[size_key][mprof].get(qid)
            if cur is None:
                expected[size_key][mprof][qid] = int(obs_count)
                captured += 1
            elif int(cur) != int(obs_count):
                drift.append((mprof, qid, cur, obs_count))

    if captured:
        # Pretty-print with stable key order to keep diffs minimal.
        args.expected.write_text(
            json.dumps(expected, indent=2, sort_keys=False) + "\n"
        )
        print(f"update-expected: captured {captured} null entries under '{size_key}'",
              file=sys.stderr)

    if drift:
        print(f"update-expected: {len(drift)} drift(s) under '{size_key}':",
              file=sys.stderr)
        for mprof, qid, cur, obs in drift:
            print(f"  {mprof}/{qid}: manifest={cur} observed={obs}",
                  file=sys.stderr)

    return 0


if __name__ == "__main__":
    sys.exit(main())
