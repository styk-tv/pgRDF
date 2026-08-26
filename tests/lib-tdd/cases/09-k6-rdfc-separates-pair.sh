#!/usr/bin/env bash
# TDD K6-2 (PL-5, RETENTION) · RDFC-1.0 must conclusively separate the exact
# pair fd1 cannot. Measured GREEN 2026-08-25; must stay green.
# TARGET = TODAY  graph_digest(4-cycle) ≠ graph_digest(2×2), within budget
. "$(dirname "$0")/../lib.sh"
GA='urn:tdd:cycle4b' ; GB='urn:tdd:cycle2x2b'
drop_g "$GA"; drop_g "$GB"
scalar "SELECT pgrdf.add_graph('$GA')" >/dev/null
scalar "SELECT pgrdf.add_graph('$GB')" >/dev/null
scalar "SELECT pgrdf.parse_turtle('@prefix e: <http://e/> . _:x1 e:p _:x2 . _:x2 e:p _:x3 . _:x3 e:p _:x4 . _:x4 e:p _:x1 .', pgrdf.graph_id('$GA'))" >/dev/null
scalar "SELECT pgrdf.parse_turtle('@prefix e: <http://e/> . _:y1 e:p _:y2 . _:y2 e:p _:y1 . _:y3 e:p _:y4 . _:y4 e:p _:y3 .', pgrdf.graph_id('$GB'))" >/dev/null
st="$(sqlstate "SELECT pgrdf.graph_digest(pgrdf.graph_id('$GA'))")"
diff="$(scalar "SELECT pgrdf.graph_digest(pgrdf.graph_id('$GA')) <> pgrdf.graph_digest(pgrdf.graph_id('$GB'))")"
drop_g "$GA"; drop_g "$GB"
[ "$st" = "NONE" ] || broken "RDFC raised '$st' on the automorphic pair — complexity budget moved?"
if [ "$diff" = "t" ]; then
  green "RDFC-1.0 separates the fd1 collision pair — the two-digest design holds"
else
  broken "RDFC digests EQUAL on non-isomorphic graphs — conclusiveness broken, stop everything"
fi
