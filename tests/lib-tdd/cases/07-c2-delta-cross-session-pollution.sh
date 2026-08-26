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

# The structured per-call channel (E2): if present, the bystander runs a
# CLEAN query and its per-call figure must read ZERO even though the
# global counter just moved — the property the delta method cannot have.
percall="$(scalar "SELECT coalesce(to_regprocedure('pgrdf.last_call_stats()')::text, 'ABSENT')")"

if [ "$percall" != "ABSENT" ] && [ -n "$percall" ]; then
  # one psql process = one session; -c commands run sequentially in it.
  # A truncating call must read >=1; a clean call in another session
  # must read 0 — per-call and session-local, both directions.
  scalar "SELECT pgrdf.add_graph('$G')" >/dev/null
  scalar "SELECT pgrdf.parse_turtle((SELECT string_agg('<http://d/n'||i||'> <http://d/next> <http://d/n'||(i+1)||'> .', E'\n') FROM generate_series(1,70) i), pgrdf.graph_id('$G'))" >/dev/null
  hot="$($PSQL -tA \
    -c "SELECT count(*) FROM pgrdf.sparql('SELECT ?e WHERE { GRAPH <$G> { <http://d/n1> <http://d/next>+ ?e } }')" \
    -c "SELECT (pgrdf.last_call_stats()->>'path_depth_truncations')::bigint" 2>/dev/null | tail -1)"
  cold="$($PSQL -tA \
    -c "SELECT count(*) FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o } LIMIT 1')" \
    -c "SELECT (pgrdf.last_call_stats()->>'path_depth_truncations')::bigint" 2>/dev/null | tail -1)"
  drop_g "$G"
  if [ "$hot" -ge 1 ] 2>/dev/null && [ "$cold" = "0" ]; then
    green "per-call figures sound: truncating session reads $hot, clean session reads 0 (global still races: delta=$delta)"
  else
    broken "last_call_stats: truncating=$hot clean=$cold — per-call contract not held"
  fi
fi
if [ "$delta" -ge 1 ]; then
  red "bystander session's delta = $delta though it ran NOTHING — global-counter differencing is unsound (E2 unimplemented)"
else
  broken "expected cross-session pollution not observed (delta=$delta) — did the truncating query run?"
fi
