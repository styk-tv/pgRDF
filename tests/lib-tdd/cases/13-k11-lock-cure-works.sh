#!/usr/bin/env bash
# TDD K11-1 (PL-6, RETENTION) · the lock refusal names a cure; EXECUTE that exact
# cure and assert it resolves the condition. Measured working 2026-08-25.
# TARGET = TODAY  message names unlock_graph(id, reason); running it clears the lock.
. "$(dirname "$0")/../lib.sh"
G='urn:tdd:cure13'
drop_g "$G"
scalar "SELECT pgrdf.add_graph('$G')" >/dev/null
scalar "SELECT pgrdf.parse_turtle('<urn:tdd:s> <urn:tdd:p> <urn:tdd:o> .', pgrdf.graph_id('$G'))" >/dev/null
gid="$(scalar "SELECT pgrdf.graph_id('$G')")"
scalar "SELECT pgrdf.lock_graph($gid, 'lib-tdd 13')" >/dev/null
msg="$(sql "SELECT pgrdf.clear_graph($gid)")"
if ! printf '%s' "$msg" | grep -q "unlock_graph($gid"; then
  drop_g "$G"
  broken "lock refusal no longer names the cure unlock_graph($gid, …) — cure text drifted (K11)"
fi
# Execute the cure the message names, verbatim in shape:
scalar "SELECT pgrdf.unlock_graph($gid, 'lib-tdd 13 cure')" >/dev/null
cleared="$(scalar "SELECT pgrdf.clear_graph($gid)")"
drop_g "$G"
if [ -n "$cleared" ]; then
  green "the named cure works: unlock_graph then clear_graph succeeded ($cleared triples)"
else
  broken "executed the named cure and clear_graph STILL failed — a stated cure that does not work (K11's forbidden state)"
fi
