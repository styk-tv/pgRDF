#!/usr/bin/env bash
# TDD E7-1/K5/K8 · graph_manifest: every digest carries its method (K5),
# not_carried is present and non-empty (K8), and the bytes digest
# recomputes from the export content — the offline-verification contract.
# TARGET  manifest complete and honest    PREDICTED pre-0.6.34: absent
. "$(dirname "$0")/../lib.sh"
have="$(scalar "SELECT coalesce(to_regprocedure('pgrdf.graph_manifest(bigint)')::text,'ABSENT')")"
if [ "$have" = "ABSENT" ] || [ -z "$have" ]; then
  red "pgrdf.graph_manifest does not exist — E7 unimplemented (clients reinvent package manifests)"
fi
G='urn:tdd:man17'
drop_g "$G"
scalar "SELECT pgrdf.add_graph('$G')" >/dev/null
scalar "SELECT pgrdf.parse_turtle('<urn:m:s> <urn:m:p> \"v\" . _:x <urn:m:q> \"w\" .', pgrdf.graph_id('$G'))" >/dev/null
methods="$(scalar "SELECT count(*) FROM jsonb_each(pgrdf.graph_manifest(pgrdf.graph_id('$G'))->'digests') d WHERE d.value->>'method' IS NULL OR d.value->>'method' = ''")"
ncarried="$(scalar "SELECT jsonb_array_length(pgrdf.graph_manifest(pgrdf.graph_id('$G'))->'not_carried')")"
# offline-verification contract: bytes digest == sha256 of the export file form
# psql -tA prints each row followed by \n — exactly the export file form.
recomputed="$($PSQL -tA -c "SELECT l FROM pgrdf.export_graph(pgrdf.graph_id('$G')) l" 2>/dev/null | shasum -a 256 | cut -d' ' -f1)"
declared="$(scalar "SELECT pgrdf.graph_manifest(pgrdf.graph_id('$G'))->'digests'->'bytes'->>'value'")"
drop_g "$G"
[ "$methods" = "0" ] || broken "a digest without a method label (K5): $methods bare values"
[ "${ncarried:-0}" -ge 1 ] 2>/dev/null || broken "not_carried empty or absent (K8)"
if [ "$declared" = "$recomputed" ]; then
  green "manifest honest: every digest method-labelled, not_carried=$ncarried entries, bytes digest recomputes offline"
else
  broken "bytes digest does not recompute from the export (declared=$declared recomputed=$recomputed)"
fi
