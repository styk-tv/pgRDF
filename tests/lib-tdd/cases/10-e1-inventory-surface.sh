#!/usr/bin/env bash
# TDD C3-1/E1-1 · a supported graph-inventory surface exists (Q1: graphs with iri+id)
# TARGET  a stable inventory verb answers Q1 with parity against the private table
# PREDICTED TODAY  no candidate exists; consumers are forced into pgrdf._pgrdf_graphs
. "$(dirname "$0")/../lib.sh"
found=""
for cand in 'pgrdf.graph_inventory()' 'pgrdf.graphs()' 'pgrdf.inventory()' 'pgrdf.list_graphs()'; do
  have="$(scalar "SELECT coalesce(to_regprocedure('$cand')::text,'')")"
  if [ -n "$have" ]; then found="$cand"; break; fi
done
if [ -z "$found" ]; then
  red "no inventory verb exists under any candidate name — E1 unimplemented (54 private-table sites remain forced)"
fi
# Parity Q1 the day it lands: public count == private count, same transaction.
pub="$(scalar "SELECT count(*) FROM ${found%??}()")"
priv="$(scalar "SELECT count(*) FROM pgrdf._pgrdf_graphs")"
if [ "$pub" = "$priv" ]; then
  green "$found exists and Q1 parity holds ($pub graphs both ways)"
else
  broken "$found exists but disagrees with the private table (public=$pub private=$priv)"
fi
