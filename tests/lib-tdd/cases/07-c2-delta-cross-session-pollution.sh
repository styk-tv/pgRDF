#!/usr/bin/env bash
# TDD C2-2 (PL-4/PL-9) · the counter-delta method attributes another session's
# truncation to a bystander. Deterministic two-session interleave:
#   B reads counter → A (separate backend) truncates → B reads counter again.
#   B ran nothing, yet B's delta is ≥1.
# TARGET  a structured per-call figure exists (delta method retired)
# PREDICTED TODAY  pollution observed AND no per-call figure — the L2 unsoundness
. "$(dirname "$0")/../lib.sh"
G='urn:tdd:deep07'
drop_g "$G"
scalar "SELECT pgrdf.add_graph('$G')" >/dev/null
scalar "SELECT pgrdf.parse_turtle((SELECT string_agg('<http://d/n'||i||'> <http://d/next> <http://d/n'||(i+1)||'> .', E'\n') FROM generate_series(1,70) i), pgrdf.graph_id('$G'))" >/dev/null

pre="$(scalar "SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint")"          # session B, read 1
sql "SELECT count(*) FROM pgrdf.sparql('SELECT ?e WHERE { GRAPH <$G> { <http://d/n1> <http://d/next>+ ?e } }')" >/dev/null  # session A truncates (default mode: warn)
post="$(scalar "SELECT (pgrdf.stats()->>'path_depth_truncations')::bigint")"         # session B, read 2
drop_g "$G"
delta=$(( post - pre ))

# Is there any structured per-call channel yet? (name candidates; all absent today)
percall="$(scalar "SELECT coalesce(to_regprocedure('pgrdf.last_call_stats()')::text, to_regprocedure('pgrdf.call_report()')::text, 'ABSENT')")"

if [ "$percall" != "ABSENT" ] && [ -n "$percall" ]; then
  green "a per-call figure exists ($percall) — the delta method can retire"
fi
if [ "$delta" -ge 1 ]; then
  red "bystander session's delta = $delta though it ran NOTHING — global-counter differencing is unsound (E2 unimplemented)"
else
  broken "expected cross-session pollution not observed (delta=$delta) — did the truncating query run?"
fi
