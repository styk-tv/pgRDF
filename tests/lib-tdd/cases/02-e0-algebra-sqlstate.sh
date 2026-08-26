#!/usr/bin/env bash
# TDD C1-1/E0 · unsupported-algebra gate SQLSTATE
# TARGET  0A000 feature_not_supported       PREDICTED TODAY  XX000 (PL-6, executor.rs:3759)
. "$(dirname "$0")/../lib.sh"
st="$(sqlstate "SELECT * FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o . SERVICE <http://remote/> { ?s ?p2 ?o2 } }')")"
case "$st" in
  0A000) green "unsupported algebra raises 0A000 feature_not_supported" ;;
  XX000) red   "unsupported algebra is XX000 internal_error — E0 unimplemented" ;;
  NONE)  broken "SERVICE query did not error — engine grew SERVICE support? re-derive the case" ;;
  *)     broken "unsupported algebra raised '$st'" ;;
esac
