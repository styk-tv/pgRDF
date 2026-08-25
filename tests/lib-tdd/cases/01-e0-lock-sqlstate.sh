#!/usr/bin/env bash
# TDD C1-1/E0 · lock gate SQLSTATE
# TARGET  55P03 lock_not_available          PREDICTED TODAY  XX000 (PL-6, lock.rs:55)
. "$(dirname "$0")/../lib.sh"
G='urn:tdd:e0lock'
drop_g "$G"
scalar "SELECT pgrdf.add_graph('$G')" >/dev/null
scalar "SELECT pgrdf.parse_turtle('<urn:tdd:s> <urn:tdd:p> <urn:tdd:o> .', pgrdf.graph_id('$G'))" >/dev/null
scalar "SELECT pgrdf.lock_graph(pgrdf.graph_id('$G'), 'lib-tdd 01')" >/dev/null
st="$(sqlstate "SELECT pgrdf.clear_graph(pgrdf.graph_id('$G'))")"
scalar "SELECT pgrdf.unlock_graph(pgrdf.graph_id('$G'), 'lib-tdd 01 done')" >/dev/null
drop_g "$G"
case "$st" in
  55P03) green "lock refusal raises 55P03 lock_not_available" ;;
  XX000) red   "lock refusal is XX000 internal_error — E0 unimplemented" ;;
  *)     broken "lock refusal raised '$st', neither today's XX000 nor target 55P03" ;;
esac
