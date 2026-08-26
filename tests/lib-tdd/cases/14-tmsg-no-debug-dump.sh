#!/usr/bin/env bash
# TDD T-MSG-1 (PL-6) · gate messages are prose, not Rust Debug dumps.
# TARGET  the algebra refusal names the construct (SERVICE), no AST dump
# PREDICTED TODAY  message contains `NamedNode { … }` / `Variable { … }` debris
. "$(dirname "$0")/../lib.sh"
msg="$(sql "SELECT * FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o . SERVICE <http://remote/> { ?s ?p2 ?o2 } }')")"
printf '%s' "$msg" | grep -q '^ERROR' || broken "SERVICE query did not error — re-derive the case"
if printf '%s' "$msg" | grep -qE 'NamedNode \{|Variable \{|TriplePattern \{'; then
  red "algebra refusal is a Rust Debug dump, not prose — names structs, not the construct"
elif printf '%s' "$msg" | grep -qi 'SERVICE'; then
  green "algebra refusal names the construct (SERVICE) without AST debris"
else
  broken "algebra refusal has neither the Debug dump nor the construct name — message changed shape"
fi
