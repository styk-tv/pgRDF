#!/usr/bin/env bash
# TDD T-NULL-2 (PL-2) · an EMPTY graph digests (an answer); an ABSENT graph refuses.
# TARGET  empty→digest AND absent→42704    PREDICTED TODAY  both return the same hash
. "$(dirname "$0")/../lib.sh"
G='urn:tdd:empty06'
drop_g "$G"
scalar "SELECT pgrdf.add_graph('$G')" >/dev/null
d_empty="$(scalar "SELECT pgrdf.graph_digest(pgrdf.graph_id('$G'))")"
st_absent="$(sqlstate "SELECT pgrdf.graph_digest(999999999)")"
d_absent="$(scalar "SELECT pgrdf.graph_digest(999999999)")"
drop_g "$G"
[ -n "$d_empty" ] || broken "empty graph produced no digest at all"
if [ "$st_absent" = "42704" ]; then
  green "empty digests ($d_empty), absent refuses — the two are distinguishable"
fi
if [ "$d_empty" = "$d_absent" ]; then
  red "empty and absent graphs produce the IDENTICAL digest ($d_empty) — indistinguishable in the identity plane"
else
  broken "empty=$d_empty absent=$d_absent st=$st_absent — neither predicted state"
fi
