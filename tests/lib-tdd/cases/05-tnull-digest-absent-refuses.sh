#!/usr/bin/env bash
# TDD T-NULL-1 (PL-2 / proposed L9) · digest of an ABSENT graph must refuse
# TARGET  42704 undefined_object            PREDICTED TODAY  silent sha256("") = e3b0c44…
. "$(dirname "$0")/../lib.sh"
EMPTY_SHA='e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'
st="$(sqlstate "SELECT pgrdf.graph_digest(999999999)")"
if [ "$st" = "42704" ]; then green "digest of absent graph refuses with 42704 undefined_object"; fi
if [ "$st" != "NONE" ]; then broken "digest of absent graph raised '$st'"; fi
v="$(scalar "SELECT pgrdf.graph_digest(999999999)")"
if [ "$v" = "$EMPTY_SHA" ]; then
  red "absent graph digests to sha256 of EMPTY INPUT, silently — indistinguishable from an empty graph"
else
  broken "absent graph returned '$v' — neither a refusal nor the empty-input hash"
fi
