#!/usr/bin/env bash
#
# tests/shacl-capability/run.sh — generate the SHACL capability document.
#
# Answers ONE question, per constraint component and target selector:
#   does `pgrdf.validate` actually ENFORCE it, on THIS build?
#
# Method mirrors tests/w3c-shacl/run.sh in spirit — hermetic,
# checked-in `.ttl` fixtures, no fetch at test time. Each probe ships
# as a TRIPLE:
#
#   <component>.shapes.ttl     — the shapes graph
#   <component>.violating.ttl  — data that breaks exactly that component
#   <component>.control.ttl    — data that satisfies it
#
# Shapes and data go into SEPARATE graphs, because that is how the seal
# calls the validator — and because a self-validating graph makes the
# shape node its own typed subject, which silently breaks any probe
# using `sh:targetSubjectsOf`.
#
# A component is ENFORCED only when violating => conforms:false AND
# control => conforms:true. The control is what distinguishes a working
# constraint from an engine that reports false for everything; the
# violating case is what distinguishes it from one that silently skips.
#
# Output is `CAPABILITY.json`, written next to this script. It is
# GENERATED — never hand-edited. Regenerate after any change to the
# shacl crate pin, the validator, or the PG major.
#
# Why this exists: CKP RULE-13 binds the core ontology to "constraints
# this engine enforces", not "constraints SHACL Core defines". Before
# this harness, that allowlist lived in prose, and a shape chosen
# against prose is a shape chosen against a guess.
#
# Usage:  ./run.sh                 (uses PG* env, or the defaults below)
#         PGHOST=… PGPORT=… ./run.sh
#         ./run.sh --print         (human-readable table, no file write)
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIX="${HERE}/fixtures"
OUT="${HERE}/CAPABILITY.json"
PRINT_ONLY=0
[[ "${1:-}" == "--print" ]] && PRINT_ONLY=1

psql_q() { psql -v ON_ERROR_STOP=1 -tAq -c "$1"; }

# Load one probe into scratch shapes + data graphs, validate, return
# the `conforms` value. Both graphs are dropped immediately, so this
# harness is safe against a shared database (bench class B1).
probe() {
  local comp="$1"
  local kind="$2"
  local mode="$3"
  local sg="urn:pgrdf-capability:${comp}:${kind}:${mode}:shapes"
  local dg="urn:pgrdf-capability:${comp}:${kind}:${mode}:data"
  local shapes; shapes="$(cat "${FIX}/${comp}.shapes.ttl")"
  local data;   data="$(cat "${FIX}/${comp}.${kind}.ttl")"
  psql_q "$(cat <<SQL
DO \$probe\$
DECLARE sg bigint; dg bigint; rep jsonb;
BEGIN
  BEGIN PERFORM pgrdf.drop_graph('${sg}'); PERFORM pgrdf.drop_graph('${dg}'); EXCEPTION WHEN OTHERS THEN NULL; END;
  sg := pgrdf.add_graph('${sg}');
  dg := pgrdf.add_graph('${dg}');
  PERFORM pgrdf.parse_turtle(\$s\$${shapes}\$s\$, sg);
  PERFORM pgrdf.parse_turtle(\$d\$${data}\$d\$, dg);
  rep := pgrdf.validate(dg, sg, '${mode}');
  PERFORM pgrdf.drop_graph('${sg}'); PERFORM pgrdf.drop_graph('${dg}');
  RAISE NOTICE 'PROBE=%', coalesce(rep->>'conforms','null');
END \$probe\$;
SQL
)" 2>&1 | sed -n 's/^NOTICE:  PROBE=//p'
}

PG_VER="$(psql_q 'SHOW server_version;' | cut -d. -f1)"
PGRDF_VER="$(psql_q 'SELECT pgrdf.version();')"

components=()
for v in "${FIX}"/*.shapes.ttl; do
  components+=( "$(basename "$v" .shapes.ttl)" )
done

rows=""; enforced=(); not_enforced=()
for c in "${components[@]}"; do
  bad="$(probe "$c" violating native)"
  good="$(probe "$c" control native)"
  # A component absent from `native` may still be evaluated by another
  # mode. `sh:sparql` is exactly that case: skipped silently by native
  # and sparql, evaluated correctly by 'pgrdf'. Reporting only the
  # native verdict would say "not enforced" about an engine that
  # enforces it — the same error in the other direction.
  alt=""
  if [[ "$bad" != "false" ]]; then
    for m in pgrdf sparql; do
      ab="$(probe "$c" violating "$m")"; ag="$(probe "$c" control "$m")"
      if [[ "$ab" == "false" && "$ag" == "true" ]]; then alt="$m"; break; fi
    done
  fi
  if [[ "$bad" == "false" && "$good" == "true" ]]; then
    verdict="enforced";      enforced+=( "$c" )
  elif [[ -n "$alt" ]]; then
    verdict="enforced-in-mode:$alt"; enforced+=( "$c" )
  elif [[ "$bad" == "true" ]]; then
    verdict="SILENTLY-SKIPPED"; not_enforced+=( "$c" )
  else
    verdict="INDETERMINATE";    not_enforced+=( "$c" )
  fi
  printf '  %-22s violating=%-5s control=%-5s  %s\n' "$c" "$bad" "$good" "$verdict"
  rows+="$(printf '{"component":"%s","violating_conforms":%s,"control_conforms":%s,"verdict":"%s"}' \
            "$c" "${bad:-null}" "${good:-null}" "$verdict"),"
done

(( PRINT_ONLY )) && exit 0

python3 - "$OUT" "$PGRDF_VER" "$PG_VER" "${rows%,}" <<'PY'
import json, sys
out, pgrdf, pg, rows = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
probes = json.loads("["+rows+"]")
doc = {
  "artifact": "pgrdf-shacl-capability",
  "generated_by": "tests/shacl-capability/run.sh",
  "hand_edited": False,
  "pgrdf_version": pgrdf,
  "postgres_major": int(pg),
  "enforced": sorted(p["component"] for p in probes if p["verdict"] == "enforced"),
  "enforced_only_in_mode": {p["component"]: p["verdict"].split(":", 1)[1]
                            for p in probes if p["verdict"].startswith("enforced-in-mode:")},
  "not_enforced": sorted(p["component"] for p in probes
                         if not p["verdict"].startswith("enforced")),
  "probes": probes,
  "caveats": [
    "validate does NOT entail: sh:targetClass matches ASSERTED rdf:type only. "
    "A node typed only by a subclass is not targeted by a shape on its parent "
    "unless pgrdf.materialize has run, or the parent type is stamped explicitly.",
    "A constraint component absent from `enforced` contributes no violation and "
    "no error. conforms:true therefore does not distinguish 'validated clean' "
    "from 'never evaluated'. See pgRDF#80.",
    "`enforced_only_in_mode` names components no default-mode probe catches but "
    "another mode evaluates correctly. sh:sparql is the case: silently skipped by "
    "'native' and 'sparql', evaluated by 'pgrdf'. Reading the native verdict alone "
    "reports 'unsupported' about an engine that supports it.",
  ],
}
with open(out, "w") as f:
    json.dump(doc, f, indent=2, sort_keys=True); f.write("\n")
print(f"\nwrote {out}: {len(doc['enforced'])} enforced, {len(doc['not_enforced'])} not enforced")
PY
