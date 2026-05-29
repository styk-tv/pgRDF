#!/usr/bin/env python3
"""Compare a perf-report.json against tests/perf/lubm/baseline.lubm-N.json.

TF-7 dev-gate and TF-8 nightly-cron use this script to fail-fast on
regression. Compares fixture-by-fixture:

- **Correctness fields** (``conforms``, ``violations``, ``dict_lookups``)
  are EXACT-MATCH where present in both records. Missing fields in the
  actual aren't an error (forward-compat); missing fields in the
  baseline are a baseline-out-of-date warning, not a hard fail.
- **Timing fields** (``elapsed_ms``) are tolerance-compared per
  fixture using ``comparison_tolerance.elapsed_ms_pct`` from the
  baseline. Actual outside ``baseline ± tolerance%`` is a regression.

Exit codes:
  0  every fixture matched within tolerance
  1  one or more regressions surfaced
  2  invocation error (missing file, malformed JSON, fixture-name
     mismatch)

Usage:
    python3 tests/perf/lubm/compare-to-baseline.py \\
        --actual target/perf-report.json \\
        --baseline tests/perf/lubm/baseline.lubm-10.json

Honest-reporting discipline: every regression prints
``REGRESS: <name> <field> baseline=… actual=… tolerance=…`` to
stderr so CI logs surface the specific drift.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path


EXACT_FIELDS = ("conforms", "violations", "dict_lookups", "plan_cache_hits")


def load(p: Path) -> dict:
    try:
        return json.loads(p.read_text())
    except FileNotFoundError:
        print(f"compare-to-baseline: {p}: file not found", file=sys.stderr)
        sys.exit(2)
    except json.JSONDecodeError as e:
        print(f"compare-to-baseline: {p}: malformed JSON: {e}", file=sys.stderr)
        sys.exit(2)


def index_fixtures(report: dict) -> dict[str, dict]:
    out: dict[str, dict] = {}
    for fx in report.get("fixtures", []):
        name = fx.get("name")
        if not name:
            print("compare-to-baseline: fixture without `name`", file=sys.stderr)
            sys.exit(2)
        if name in out:
            print(f"compare-to-baseline: duplicate fixture {name!r}", file=sys.stderr)
            sys.exit(2)
        out[name] = fx
    return out


def compare_mode(name: str, mode: str, b: dict, a: dict, tol_pct: float) -> list[str]:
    """Return a list of regression messages for one (fixture, mode) pair."""
    regress: list[str] = []
    for field in EXACT_FIELDS:
        if field in b and field in a:
            if b[field] != a[field]:
                regress.append(
                    f"REGRESS: {name}/{mode} {field} baseline={b[field]} actual={a[field]} (exact-match required)"
                )
        elif field in b and field not in a:
            # Baseline had it; actual dropped it. Likely a runner
            # change, not a perf regression per se — warn but don't
            # fail (per the spec, missing-in-actual is forward-compat).
            print(
                f"WARN: {name}/{mode} {field} dropped from actual report (baseline had {b[field]})",
                file=sys.stderr,
            )

    if "elapsed_ms" in b and "elapsed_ms" in a:
        bms = float(b["elapsed_ms"])
        ams = float(a["elapsed_ms"])
        if bms <= 0:
            # Zero baseline is nonsensical; avoid division by zero by
            # only failing if actual is also zero (no work happened).
            if ams == 0:
                pass
            else:
                regress.append(
                    f"REGRESS: {name}/{mode} elapsed_ms baseline=0 actual={ams} (zero baseline)"
                )
        else:
            drift_pct = abs(ams - bms) / bms * 100.0
            if drift_pct > tol_pct:
                direction = "slower" if ams > bms else "faster"
                regress.append(
                    f"REGRESS: {name}/{mode} elapsed_ms baseline={bms:.3f} actual={ams:.3f} "
                    f"drift={drift_pct:.1f}% (>{tol_pct:.1f}% tolerance, {direction})"
                )
    elif "elapsed_ms" in b and "elapsed_ms" not in a:
        regress.append(
            f"REGRESS: {name}/{mode} elapsed_ms missing in actual (baseline had {b['elapsed_ms']})"
        )
    return regress


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--actual", required=True, type=Path)
    ap.add_argument("--baseline", required=True, type=Path)
    args = ap.parse_args()

    baseline = load(args.baseline)
    actual = load(args.actual)

    b_idx = index_fixtures(baseline)
    a_idx = index_fixtures(actual)

    regressions: list[str] = []

    # Every baseline fixture MUST be present in actual (otherwise a
    # silent runner regression is being masked as "we just stopped
    # measuring this thing").
    missing = sorted(set(b_idx.keys()) - set(a_idx.keys()))
    if missing:
        for name in missing:
            regressions.append(f"REGRESS: {name!r} present in baseline but missing from actual")

    # Extra fixtures in actual are NOT a regression (forward-compat —
    # the runner can add new measurements that haven't been baselined).
    extras = sorted(set(a_idx.keys()) - set(b_idx.keys()))
    for name in extras:
        print(f"INFO: {name!r} present in actual but not baselined yet", file=sys.stderr)

    for name in sorted(set(b_idx.keys()) & set(a_idx.keys())):
        b_fx = b_idx[name]
        a_fx = a_idx[name]
        tol_pct = float(b_fx.get("comparison_tolerance", {}).get("elapsed_ms_pct", 50))
        b_modes = b_fx.get("modes", {})
        a_modes = a_fx.get("modes", {})
        for mode in sorted(set(b_modes.keys()) & set(a_modes.keys())):
            regressions.extend(compare_mode(name, mode, b_modes[mode], a_modes[mode], tol_pct))

    if regressions:
        print("compare-to-baseline: FAIL", file=sys.stderr)
        for r in regressions:
            print(r, file=sys.stderr)
        print(
            f"compare-to-baseline: {len(regressions)} regression(s) across "
            f"{len(b_idx)} baselined fixture(s)",
            file=sys.stderr,
        )
        return 1

    print(
        f"compare-to-baseline: PASS — {len(b_idx)} baselined fixture(s) within tolerance",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
