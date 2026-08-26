#!/usr/bin/env bash
# TDD C1-1/E0 · path-truncation under on_path_truncation=error
# TARGET  54000 program_limit_exceeded      PREDICTED TODAY  XX000 (PL-6, executor.rs:7043)
. "$(dirname "$0")/../lib.sh"
G='urn:tdd:deep04'
drop_g "$G"
scalar "SELECT pgrdf.add_graph('$G')" >/dev/null
scalar "SELECT pgrdf.parse_turtle((SELECT string_agg('<http://d/n'||i||'> <http://d/next> <http://d/n'||(i+1)||'> .', E'\n') FROM generate_series(1,70) i), pgrdf.graph_id('$G'))" >/dev/null
st="$(sqlstate \
  "SET pgrdf.on_path_truncation = 'error'" \
  "SELECT count(*) FROM pgrdf.sparql('SELECT ?e WHERE { GRAPH <$G> { <http://d/n1> <http://d/next>+ ?e } }')")"
drop_g "$G"
case "$st" in
  54000) green "fail-closed truncation raises 54000 program_limit_exceeded" ;;
  XX000) red   "fail-closed truncation is XX000 internal_error — E0 unimplemented" ;;
  NONE)  broken "70-hop chain under default cap 64 did not truncate — cap changed? re-derive" ;;
  *)     broken "fail-closed truncation raised '$st'" ;;
esac
