#!/usr/bin/env python3
"""Render pgRDF benchmark history (target/perf-history/runs.jsonl)
into a self-contained HTML report at target/perf-history/index.html.

Pure stdlib — no pip installs required. Chart.js loaded inline via
the jsdelivr CDN; the rendered HTML works offline once the page is
open + Chart.js is cached, otherwise needs internet for the first
view.

Usage:
    python3 tests/perf/render-history.py \\
        --history target/perf-history/runs.jsonl \\
        --out     target/perf-history/index.html

Companion to `tests/perf/benchmark-runner.sh` (which appends to the
JSONL after each benchmark run).
"""

from __future__ import annotations

import argparse
import html
import json
import sys
from pathlib import Path


def load_runs(history_path: Path) -> list[dict]:
    if not history_path.exists():
        return []
    runs = []
    with history_path.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                runs.append(json.loads(line))
            except json.JSONDecodeError as e:
                print(f"WARN: malformed line skipped: {e}", file=sys.stderr)
    return runs


def fmt_ms(v):
    if v is None:
        return "—"
    return f"{v:.1f}"


def fmt_int(v):
    if v is None:
        return "—"
    return f"{int(v):,}"


def short_sha(sha: str) -> str:
    return (sha or "")[:8] or "—"


METRICS = [
    ("ingest.elapsed_ms",   "Ingest total (ms)",      "ingest_elapsed"),
    ("ingest.dict_ms",      "Ingest dict_ms",         "ingest_dict"),
    ("ingest.parse_ms",     "Ingest parse_ms",        "ingest_parse"),
    ("ingest.insert_ms",    "Ingest insert_ms",       "ingest_insert"),
    ("materialize.rdfs.elapsed_ms",   "Materialize RDFS (ms)",      "mat_rdfs"),
    ("materialize.owl_rl.elapsed_ms", "Materialize OWL-RL (ms)",    "mat_owl_rl"),
    ("q14.elapsed_ms_median", "Q14 median (ms)",      "q14"),
]


def get_nested(d: dict, dotted: str):
    cur = d
    for part in dotted.split("."):
        if not isinstance(cur, dict):
            return None
        cur = cur.get(part)
    return cur


def build_chart_series(runs: list[dict]) -> dict:
    """Build a {metric_id: [{x, y, run_idx, sha, ts}, ...]} mapping."""
    series = {chart_id: [] for _, _, chart_id in METRICS}
    for idx, r in enumerate(runs):
        ts = r.get("ts", "")
        sha = short_sha(r.get("git_sha", ""))
        for dotted, _, chart_id in METRICS:
            v = get_nested(r, dotted)
            if v is None:
                continue
            series[chart_id].append({
                "x": idx,
                "y": float(v),
                "ts": ts,
                "sha": sha,
            })
    return series


def render_html(runs: list[dict]) -> str:
    runs_sorted = sorted(runs, key=lambda r: r.get("ts_unix", 0))
    latest = runs_sorted[-1] if runs_sorted else None
    series = build_chart_series(runs_sorted)
    series_json = json.dumps(series, indent=2)

    # Table rows (most recent first, top 20).
    table_rows = []
    for r in reversed(runs_sorted[-20:]):
        ingest = r.get("ingest", {}) or {}
        mat = r.get("materialize", {}) or {}
        rdfs = (mat.get("rdfs") or {})
        owl = (mat.get("owl_rl") or {})
        q14 = r.get("q14", {}) or {}
        table_rows.append(
            "<tr>"
            f"<td class=\"ts\">{html.escape(r.get('ts',''))}</td>"
            f"<td class=\"sha\">{html.escape(short_sha(r.get('git_sha','')))}</td>"
            f"<td>{html.escape(r.get('pgrdf_version','—'))}</td>"
            f"<td class=\"num\">{r.get('lubm_size','—')}</td>"
            f"<td class=\"num\">{fmt_int(r.get('triples'))}</td>"
            f"<td class=\"num\">{fmt_ms(ingest.get('elapsed_ms'))}</td>"
            f"<td class=\"num\">{fmt_ms(ingest.get('parse_ms'))}</td>"
            f"<td class=\"num\">{fmt_ms(ingest.get('dict_ms'))}</td>"
            f"<td class=\"num\">{fmt_ms(ingest.get('insert_ms'))}</td>"
            f"<td class=\"num\">{fmt_ms(rdfs.get('elapsed_ms'))}</td>"
            f"<td class=\"num\">{fmt_int(rdfs.get('triples_inferred'))}</td>"
            f"<td class=\"num\">{fmt_ms(owl.get('elapsed_ms'))}</td>"
            f"<td class=\"num\">{fmt_int(owl.get('triples_inferred'))}</td>"
            f"<td class=\"num\">{fmt_ms(q14.get('elapsed_ms_median'))}</td>"
            "</tr>"
        )

    # Charts wrapper.
    chart_canvases = []
    for _, label, chart_id in METRICS:
        chart_canvases.append(
            f"<div class=\"chart\">"
            f"<h3>{html.escape(label)}</h3>"
            f"<canvas id=\"chart_{chart_id}\" height=\"160\"></canvas>"
            f"</div>"
        )

    latest_summary = ""
    if latest:
        ingest = latest.get("ingest", {}) or {}
        mat = latest.get("materialize", {}) or {}
        rdfs = (mat.get("rdfs") or {})
        owl = (mat.get("owl_rl") or {})
        q14 = latest.get("q14", {}) or {}
        latest_summary = f"""
<section class="latest">
  <h2>Latest run — {html.escape(latest.get('ts',''))}</h2>
  <dl>
    <dt>Commit</dt><dd><code>{html.escape(short_sha(latest.get('git_sha','')))}</code> on <code>{html.escape(latest.get('git_branch','?'))}</code> · pgRDF <code>{html.escape(latest.get('pgrdf_version','?'))}</code> · pg{latest.get('postgres_major','?')} · host <code>{html.escape(latest.get('host','?'))}</code></dd>
    <dt>LUBM size</dt><dd>{latest.get('lubm_size','—')} ({fmt_int(latest.get('triples'))} triples)</dd>
    <dt>Ingest</dt><dd>{fmt_ms(ingest.get('elapsed_ms'))} ms (parse {fmt_ms(ingest.get('parse_ms'))} · dict {fmt_ms(ingest.get('dict_ms'))} · insert {fmt_ms(ingest.get('insert_ms'))})</dd>
    <dt>Materialize RDFS</dt><dd>{fmt_ms(rdfs.get('elapsed_ms'))} ms · {fmt_int(rdfs.get('triples_inferred'))} inferred</dd>
    <dt>Materialize OWL-RL</dt><dd>{fmt_ms(owl.get('elapsed_ms'))} ms · {fmt_int(owl.get('triples_inferred'))} inferred</dd>
    <dt>Q14 (median of 3 warm)</dt><dd>{fmt_ms(q14.get('elapsed_ms_median'))} ms · {fmt_int(q14.get('result_count'))} rows</dd>
  </dl>
</section>"""

    chart_js_init = []
    for _, label, chart_id in METRICS:
        chart_js_init.append(f"""
    new Chart(document.getElementById('chart_{chart_id}'), {{
      type: 'line',
      data: {{
        datasets: [{{
          label: {json.dumps(label)},
          data: SERIES[{json.dumps(chart_id)}],
          borderWidth: 1.5,
          pointRadius: 2.5,
          tension: 0.15,
        }}]
      }},
      options: {{
        responsive: true,
        plugins: {{ legend: {{display: false}}, tooltip: {{
          callbacks: {{ label: (ctx) => `${{ctx.parsed.y.toFixed(1)}} (${{ctx.raw.sha}} · ${{ctx.raw.ts}})` }}
        }} }},
        scales: {{
          x: {{ type: 'linear', title: {{display: true, text: 'run index (time-ordered)'}} }},
          y: {{ title: {{display: true, text: {json.dumps(label)}}} }}
        }}
      }}
    }});""")

    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>pgRDF benchmark history</title>
<style>
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; max-width: 1180px; margin: 1.5rem auto; padding: 0 1rem; color: #1a1a1a; }}
  h1 {{ margin-bottom: 0.25rem; }}
  .subtitle {{ color: #666; margin-top: 0; }}
  section.latest {{ background: #f6f8fa; border: 1px solid #d0d7de; border-radius: 8px; padding: 1rem 1.25rem; margin: 1rem 0 2rem; }}
  section.latest dl {{ display: grid; grid-template-columns: max-content 1fr; gap: 0.3rem 1rem; margin: 0; }}
  section.latest dt {{ font-weight: 600; color: #57606a; }}
  section.latest dd {{ margin: 0; }}
  section.latest code {{ background: #eaeef2; padding: 1px 4px; border-radius: 4px; font-size: 0.95em; }}
  .charts {{ display: grid; grid-template-columns: repeat(2, 1fr); gap: 1.5rem; }}
  .chart {{ background: #fff; border: 1px solid #d0d7de; border-radius: 6px; padding: 0.5rem 0.75rem; }}
  .chart h3 {{ margin: 0.25rem 0 0.5rem; font-size: 0.95rem; color: #57606a; }}
  table {{ border-collapse: collapse; width: 100%; margin: 1rem 0; font-size: 0.85rem; }}
  th, td {{ border-bottom: 1px solid #d0d7de; padding: 0.4rem 0.6rem; text-align: left; vertical-align: top; }}
  th {{ background: #f6f8fa; color: #57606a; font-weight: 600; }}
  td.num {{ text-align: right; font-variant-numeric: tabular-nums; }}
  td.ts {{ white-space: nowrap; color: #57606a; }}
  td.sha {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }}
  footer {{ margin-top: 2rem; color: #666; font-size: 0.85rem; }}
</style>
<script src="https://cdn.jsdelivr.net/npm/chart.js@4.4.0/dist/chart.umd.min.js"></script>
</head>
<body>
<h1>pgRDF benchmark history</h1>
<p class="subtitle">{len(runs_sorted)} run{'s' if len(runs_sorted) != 1 else ''} recorded · self-contained · regenerate via <code>just benchmark</code> or via the CI workflow's nightly artifact.</p>

{latest_summary}

<h2>Run-over-run charts</h2>
<div class="charts">
{''.join(chart_canvases)}
</div>

<h2>Recent runs (most-recent first, top 20)</h2>
<table>
  <thead>
    <tr>
      <th>Timestamp (UTC)</th>
      <th>Commit</th>
      <th>Version</th>
      <th>LUBM</th>
      <th>Triples</th>
      <th>Ingest ms</th>
      <th>parse</th>
      <th>dict</th>
      <th>insert</th>
      <th>RDFS ms</th>
      <th>RDFS inferred</th>
      <th>OWL-RL ms</th>
      <th>OWL-RL inferred</th>
      <th>Q14 ms</th>
    </tr>
  </thead>
  <tbody>
    {''.join(table_rows)}
  </tbody>
</table>

<footer>
Generated by <code>tests/perf/render-history.py</code>.
Source: <code>target/perf-history/runs.jsonl</code> (gitignored — accumulates per local <code>just benchmark</code> run).
</footer>

<script>
const SERIES = {series_json};
window.addEventListener('DOMContentLoaded', () => {{
  if (typeof Chart === 'undefined') {{
    document.querySelector('.charts').innerHTML = '<p>Chart.js failed to load — charts unavailable; the table above still has the numbers.</p>';
    return;
  }}
{''.join(chart_js_init)}
}});
</script>
</body>
</html>
"""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--history", required=True, type=Path,
                    help="Path to runs.jsonl (one JSON object per line).")
    ap.add_argument("--out", required=True, type=Path,
                    help="Output HTML path.")
    args = ap.parse_args()

    runs = load_runs(args.history)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(render_html(runs))
    print(f"render-history: wrote {args.out} ({len(runs)} run(s))", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
