#!/usr/bin/env bash
# Path × value-term-type matrix.
#
#   bash tests/shacl-capability/path-matrix.sh [--print]
#
# WHY THIS EXISTS, beyond the per-feature probes next door:
#
# The per-feature probes give ONE verdict per SHACL feature. That is the wrong
# shape for a defect living in the INTERACTION between a path type and the term
# type of the value it reaches. `oneOrMorePath` and `zeroOrMorePath` were
# recorded as SILENTLY-SKIPPED on the strength of a single probe using
# `sh:nodeKind sh:IRI` over a literal -- which overstated the defect. Both path
# types ARE evaluated; literal values reached through them are not.
#
# A probe that mislabels a defect is the same class of error as a capability
# document that has gone stale: an instrument reporting confidently and wrongly.
#
# Two questions per cell, both hermetic, both generated from the table below
# rather than hand-written fixtures:
#
#   seen     -- is the value in the value set at all?  (cardinality)
#   checked  -- does a value-level constraint fire on it?  (sh:nodeKind)
#
# THREE TERM ROWS, not two. A literal at the END of a path and a literal PARTWAY
# ALONG one are different cases, and conflating them hides the sequence defect:
# `sh:path ( ex:p ex:q )` handles a literal endpoint correctly and loses the
# whole value set when a literal sits at the ex:p position -- including values
# reachable through a sibling branch that is still walkable. The first version
# of this matrix tested endpoints only and reported sequence as clean.
#
# CARDINALITY CAVEAT, and the reason `maxCount` is not fixed at 0:
# `zeroOrOnePath` and `zeroOrMorePath` include the focus node itself (the
# zero-length path, SHACL 2.3.1 via SPARQL 9.3). Their value set is therefore
# never empty, and `sh:maxCount 0` would report a violation whether or not the
# value was seen. Those two use `maxCount 1` so the verdict turns on the value.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="${HERE}/PATH-MATRIX.json"
PRINT_ONLY=0
[[ "${1:-}" == "--print" ]] && PRINT_ONLY=1

PFX='@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://ex/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .'
DPFX='@prefix ex: <http://ex/> .'

# path key | sh:path expression | data placing VALUE at a reachable position | maxCount bound
#
# `inverse` takes no literal row: the value of `^ex:p` is a SUBJECT, and a
# literal cannot be an RDF subject. That cell is `n/a` by RDF, not unmeasured.
paths=(
  "predicate|ex:p|ex:a a ex:T ; ex:p VALUE .|0"
  "sequence|( ex:p ex:q )|ex:a a ex:T ; ex:p ex:m .\nex:m ex:q VALUE .|0"
  "alternative|[ sh:alternativePath ( ex:p ex:q ) ]|ex:a a ex:T ; ex:q VALUE .|0"
  "inverse|[ sh:inversePath ex:p ]|VALUE ex:p ex:a .\nex:a a ex:T .|0"
  "zeroOrOne|[ sh:zeroOrOnePath ex:p ]|ex:a a ex:T ; ex:p VALUE .|1"
  "zeroOrMore|[ sh:zeroOrMorePath ex:p ]|ex:a a ex:T ; ex:p VALUE .|1"
  "oneOrMore|[ sh:oneOrMorePath ex:p ]|ex:a a ex:T ; ex:p VALUE .|0"
)

# Paths that traverse more than one hop can meet a literal PARTWAY along. The
# literal is placed on a sibling branch so a genuine, still-reachable IRI value
# exists alongside it: the cell then reports whether that value survives.
# `seen` here means the sibling value was reported, not the literal.
intermediates=(
  "sequence|( ex:p ex:q )|ex:a a ex:T ; ex:p \"leaf\" ; ex:p ex:m .\nex:m ex:q ex:v .|0"
  "zeroOrMore|[ sh:zeroOrMorePath ex:p ]|ex:a a ex:T ; ex:p \"leaf\" ; ex:p ex:m .\nex:m ex:p ex:v .|1"
  "oneOrMore|[ sh:oneOrMorePath ex:p ]|ex:a a ex:T ; ex:p \"leaf\" ; ex:p ex:m .\nex:m ex:p ex:v .|0"
)

psql_q() { psql -v ON_ERROR_STOP=1 -tAq -c "$1" 2>&1; }

validate() { # $1=shapes $2=data -> conforms | REFUSED | ERROR
  local out
  out="$(psql_q "$(cat <<SQL
DO \$m\$
DECLARE sg bigint; dg bigint; rep jsonb;
BEGIN
  BEGIN PERFORM pgrdf.drop_graph('urn:pgrdf-pathmatrix:s'); PERFORM pgrdf.drop_graph('urn:pgrdf-pathmatrix:d'); EXCEPTION WHEN OTHERS THEN NULL; END;
  sg := pgrdf.add_graph('urn:pgrdf-pathmatrix:s');
  dg := pgrdf.add_graph('urn:pgrdf-pathmatrix:d');
  PERFORM pgrdf.parse_turtle(\$s\$$1\$s\$, sg);
  PERFORM pgrdf.parse_turtle(\$d\$$2\$d\$, dg);
  rep := pgrdf.validate(dg, sg, 'native');
  PERFORM pgrdf.drop_graph('urn:pgrdf-pathmatrix:s'); PERFORM pgrdf.drop_graph('urn:pgrdf-pathmatrix:d');
  RAISE NOTICE 'R=%', rep->>'conforms';
END \$m\$;
SQL
)")"
  grep -q 'unenforced constraint component' <<<"$out" && { echo REFUSED; return; }
  local v; v="$(sed -n 's/^NOTICE:  R=//p' <<<"$out" | head -1)"
  echo "${v:-ERROR}"
}

rows=""; printf '%-12s %-8s %-10s %-10s %s\n' PATH TERM SEEN CHECKED NOTE
printf -- '---------------------------------------------------------------\n'
for spec in "${paths[@]}"; do
  IFS='|' read -r key pathexpr datatpl bound <<<"$spec"
  for term in iri literal; do
    if [[ "$key" == "inverse" && "$term" == "literal" ]]; then
      printf '%-12s %-8s %-10s %-10s %s\n' "$key" "$term" n/a n/a "a literal cannot be an RDF subject"
      rows+="$(printf '{"path":"%s","term":"%s","seen":null,"checked":null,"verdict":"n/a-by-rdf"}' "$key" "$term"),"
      continue
    fi
    [[ "$term" == iri ]] && val="ex:v" || val='"leaf"'
    data="$DPFX
$(printf '%b' "${datatpl//VALUE/$val}")"

    # seen: cardinality bound exceeded only if the value is in the value set
    s_shapes="$PFX
ex:S a sh:NodeShape ; sh:targetClass ex:T ; sh:property [ sh:path $pathexpr ; sh:maxCount $bound ] ."
    seen_raw="$(validate "$s_shapes" "$data")"
    [[ "$seen_raw" == "false" ]] && seen=yes || seen=NO

    # checked: nodeKind must reject the opposite kind
    [[ "$term" == iri ]] && nk="sh:Literal" || nk="sh:IRI"
    c_shapes="$PFX
ex:S a sh:NodeShape ; sh:targetClass ex:T ; sh:property [ sh:path $pathexpr ; sh:nodeKind $nk ] ."
    chk_raw="$(validate "$c_shapes" "$data")"
    [[ "$chk_raw" == "false" ]] && checked=yes || checked=NO

    if [[ "$seen" == yes && "$checked" == yes ]]; then v=ok; note=""
    elif [[ "$seen" == NO && "$checked" == NO ]]; then v=value-invisible; note="reached value is absent from the value set"
    else v=partial; note="seen=$seen checked=$checked"; fi

    printf '%-12s %-8s %-10s %-10s %s\n' "$key" "$term" "$seen" "$checked" "$note"
    rows+="$(printf '{"path":"%s","term":"%s","seen":"%s","checked":"%s","verdict":"%s"}' \
              "$key" "$term" "$seen" "$checked" "$v"),"
  done
done

# --- literals partway along a multi-hop path -------------------------------
for spec in "${intermediates[@]}"; do
  IFS='|' read -r key pathexpr datatpl bound <<<"$spec"
  data="$DPFX
$(printf '%b' "$datatpl")"
  s_shapes="$PFX
ex:S a sh:NodeShape ; sh:targetClass ex:T ; sh:property [ sh:path $pathexpr ; sh:maxCount $bound ] ."
  r="$(validate "$s_shapes" "$data")"
  [[ "$r" == "false" ]] && seen=yes || seen=NO
  c_shapes="$PFX
ex:S a sh:NodeShape ; sh:targetClass ex:T ; sh:property [ sh:path $pathexpr ; sh:nodeKind sh:Literal ] ."
  r2="$(validate "$c_shapes" "$data")"
  [[ "$r2" == "false" ]] && checked=yes || checked=NO
  if [[ "$seen" == yes && "$checked" == yes ]]; then v=ok; note=""
  else v=branch-lost; note="a literal partway along discards the sibling IRI value"; fi
  printf '%-12s %-8s %-10s %-10s %s\n' "$key" "lit-mid" "$seen" "$checked" "$note"
  rows+="$(printf '{"path":"%s","term":"literal-intermediate","seen":"%s","checked":"%s","verdict":"%s"}' \
            "$key" "$seen" "$checked" "$v"),"
done

(( PRINT_ONLY )) && exit 0
PGRDF_VER="$(psql_q 'SELECT pgrdf.version();' | tail -1)"
python3 - "$OUT" "$PGRDF_VER" "${rows%,}" <<'PY'
import json, sys
out, ver, rows = sys.argv[1], sys.argv[2], sys.argv[3]
cells = json.loads("["+rows+"]")
bad = [c for c in cells if c["verdict"] not in ("ok", "n/a-by-rdf")]
for c in cells:
    c.setdefault("term", "?")
doc = {
  "artifact": "pgrdf-shacl-path-matrix",
  "generated_by": "tests/shacl-capability/path-matrix.sh",
  "hand_edited": False,
  "pgrdf_version": ver,
  "axes": {
    "path": "SHACL 2.3.1 path types",
    "term": "iri | literal (at the path endpoint) | literal-intermediate (partway along a multi-hop path, with a sibling IRI value that should survive)",
  },
  "method": {
    "seen": "sh:maxCount bound exceeded => the value is in the value set",
    "checked": "sh:nodeKind rejects the opposite kind => a value-level constraint fires on it",
    "bound": "0, except zeroOrOne/zeroOrMore which include the focus node (zero-length path) and use 1",
  },
  "cells": cells,
  "defective_cells": [f'{c["path"]}/{c["term"]}' for c in bad],
  "note": (
    "A per-feature verdict cannot express this: the fault is in the interaction "
    "between a path type and the term type of the value it reaches. "
    "SILENTLY-SKIPPED in CAPABILITY.json overstates it -- these paths are "
    "evaluated, and literal values reached through them are not."
  ),
}
json.dump(doc, open(out, "w"), indent=2, sort_keys=True); open(out, "a").write("\n")
print(f"\nwrote {out}: {len(cells)} cells, {len(bad)} defective")
PY
