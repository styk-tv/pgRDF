#!/usr/bin/env bash
# TDD E5-2/K9-1/K9-2 · every live export is classified; no stale rows.
# The manifest is authored judgment at src/surface_manifest.tsv, shipped
# in-crate (include_str!) and served by pgrdf.surface(); this case checks
# the FILE against the live surface, the in-engine K9-2 pg_test checks
# surface() against pg_proc. GREEN = complete coverage both directions.
. "$(dirname "$0")/../lib.sh"
HERE="$(cd "$(dirname "$0")" && pwd)"
live="$(bash "$HERE/../surface/gen-surface.sh" | cut -f1 | sort)"
classified="$(grep -v '^#' "$HERE/../../../src/surface_manifest.tsv" | cut -f1 | sed '/^$/d' | sort)"
unclassified="$(comm -23 <(printf '%s\n' "$live") <(printf '%s\n' "$classified"))"
stale="$(comm -13 <(printf '%s\n' "$live") <(printf '%s\n' "$classified"))"
if [ -n "$unclassified" ]; then
  broken "live exports missing from classification.tsv (K9-2 would fail the build): $(printf '%s' "$unclassified" | tr '\n' ' ')"
fi
if [ -n "$stale" ]; then
  broken "classification.tsv carries rows for exports that no longer exist: $(printf '%s' "$stale" | tr '\n' ' ')"
fi
n="$(printf '%s\n' "$live" | sed '/^$/d' | wc -l | tr -d ' ')"
green "all $n live exports classified, no stale rows (classification is DRAFT — operator review owed)"
