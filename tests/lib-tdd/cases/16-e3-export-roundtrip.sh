#!/usr/bin/env bash
# TDD E3-1 · export_graph round-trips BY IDENTITY: export → import into a
# fresh graph → identical rdfc-1.0 digest. Byte digests legitimately differ
# across the trip (bnode labels re-mint) — asserting byte-equality would be
# the category error the three-digest design exists to prevent.
# TARGET  export exists, round-trip identity holds    PREDICTED pre-0.6.34: absent
. "$(dirname "$0")/../lib.sh"
have="$(scalar "SELECT coalesce(to_regprocedure('pgrdf.export_graph(bigint)')::text,'ABSENT')")"
if [ "$have" = "ABSENT" ] || [ -z "$have" ]; then
  red "pgrdf.export_graph does not exist — E3 unimplemented (#36 open; clients keep hand-rolled serializers)"
fi
GA='urn:tdd:exp16a'; GB='urn:tdd:exp16b'
drop_g "$GA"; drop_g "$GB"
scalar "SELECT pgrdf.add_graph('$GA')" >/dev/null
scalar "SELECT pgrdf.add_graph('$GB')" >/dev/null
scalar "SELECT pgrdf.parse_turtle('@prefix e: <http://e/> . e:s e:p \"v w\" . _:m e:q e:o . e:s e:r _:m .', pgrdf.graph_id('$GA'))" >/dev/null
n="$($PSQL -tA -c "SELECT pgrdf.parse_turtle((SELECT string_agg(l, E'\n') FROM pgrdf.export_graph(pgrdf.graph_id('$GA')) l), pgrdf.graph_id('$GB'))" 2>&1 | tail -1)"
same="$(scalar "SELECT pgrdf.graph_digest(pgrdf.graph_id('$GA')) = pgrdf.graph_digest(pgrdf.graph_id('$GB'))")"
drop_g "$GA"; drop_g "$GB"
[ "$n" = "3" ] || broken "re-import parsed $n lines, expected 3"
if [ "$same" = "t" ]; then
  green "export round-trips to the identical rdfc-1.0 identity (3 triples, bnodes included)"
else
  broken "round-trip changed the graph identity — the export is not faithful"
fi
