#!/usr/bin/env bash
# TDD K6-1/E6 (PL-5) · the fd1 collision pair: 4-cycle vs two 2-cycles of bnodes.
# Non-isomorphic; first-degree signatures collide (proven offline, PASS-2 PL-5).
# TARGET  pgrdf.structural_digest exists, pair COLLIDES under it (SAME = evidence,
#         never identity), method label pgrdf-fd1-sha256 in the emission
# PREDICTED TODAY  function absent
. "$(dirname "$0")/../lib.sh"
have="$(scalar "SELECT coalesce(to_regprocedure('pgrdf.structural_digest(bigint)')::text, 'ABSENT')")"
if [ "$have" = "ABSENT" ] || [ -z "$have" ]; then
  red "pgrdf.structural_digest(bigint) does not exist — E6 fd1 half unimplemented"
fi
GA='urn:tdd:cycle4' ; GB='urn:tdd:cycle2x2'
drop_g "$GA"; drop_g "$GB"
scalar "SELECT pgrdf.add_graph('$GA')" >/dev/null
scalar "SELECT pgrdf.add_graph('$GB')" >/dev/null
scalar "SELECT pgrdf.parse_turtle('@prefix e: <http://e/> . _:x1 e:p _:x2 . _:x2 e:p _:x3 . _:x3 e:p _:x4 . _:x4 e:p _:x1 .', pgrdf.graph_id('$GA'))" >/dev/null
scalar "SELECT pgrdf.parse_turtle('@prefix e: <http://e/> . _:y1 e:p _:y2 . _:y2 e:p _:y1 . _:y3 e:p _:y4 . _:y4 e:p _:y3 .', pgrdf.graph_id('$GB'))" >/dev/null
same="$(scalar "SELECT pgrdf.structural_digest(pgrdf.graph_id('$GA')) = pgrdf.structural_digest(pgrdf.graph_id('$GB'))")"
drop_g "$GA"; drop_g "$GB"
if [ "$same" = "t" ]; then
  green "fd1 collides on the canonical automorphic pair — conformant first-degree behaviour (SAME is evidence, not identity)"
else
  broken "structural_digest SEPARATES the fd1 collision pair — the algorithm is NOT fleet-fd1; method mismatch (L7 all over again)"
fi
