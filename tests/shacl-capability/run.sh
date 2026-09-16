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
  local out; out="$(psql_q "$(cat <<SQL
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
)" 2>&1 || true)"
  # 0.6.34 is FAIL-CLOSED on unenforced constraint components: it raises
  # rather than returning a verdict it cannot stand behind. That is correct
  # behaviour, not a probe failure, so it is captured as its own result.
  # Older builds returned conforms:true silently; both must be distinguishable.
  if grep -q 'unenforced constraint component' <<<"$out"; then
    echo "REFUSED"; return 0
  fi
  sed -n 's/^NOTICE:  PROBE=//p' <<<"$out"
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
  elif [[ "$bad" == "REFUSED" ]]; then
    # The engine declined to answer and said why. Unsupported, but fail-closed:
    # no caller can mistake this for a clean validation.
    verdict="refused-fail-closed"; not_enforced+=( "$c" )
  elif [[ "$bad" == "true" ]]; then
    # The dangerous one: no violation AND no error. A caller reading
    # conforms:true cannot tell "validated clean" from "never evaluated".
    verdict="SILENTLY-SKIPPED"; not_enforced+=( "$c" )
  else
    verdict="INDETERMINATE";    not_enforced+=( "$c" )
  fi
  printf '  %-22s violating=%-5s control=%-5s  %s\n' "$c" "$bad" "$good" "$verdict"
  # `conforms` is a JSON boolean; REFUSED / empty are not, so they are
  # emitted as a JSON string / null rather than pasted in bare.
  jsonval() { case "$1" in true|false) printf '%s' "$1";; "") printf 'null';; *) printf '"%s"' "$1";; esac; }
  rows+="$(printf '{"component":"%s","violating_conforms":%s,"control_conforms":%s,"verdict":"%s"}' \
            "$c" "$(jsonval "$bad")" "$(jsonval "$good")" "$verdict"),"
done

(( PRINT_ONLY )) && exit 0

SURFACE_TOTAL="$(grep -cvE '^#|^$' "${HERE}/SHACL-CORE-SURFACE.tsv" 2>/dev/null || echo 0)"

python3 - "$OUT" "$PGRDF_VER" "$PG_VER" "${rows%,}" "$SURFACE_TOTAL" <<'PY'
import json, sys
out, pgrdf, pg, rows = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
surface_total = int(sys.argv[5])
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
  # Coverage carries its denominator on purpose. "43 enforced" is a claim;
  # "43 enforced of 47 probed, 47 of 47 known" is a measurement. The harness
  # reported the first form for a year while it was measuring 17 of 46.
  "surface_source": "SHACL-CORE-SURFACE.tsv (W3C SHACL Recommendation)",
  "surface_features_known": surface_total,
  "surface_features_probed": len(probes),
  "surface_complete": len(probes) == surface_total,
  "caveats": [
    "validate does NOT entail: sh:targetClass matches ASSERTED rdf:type only. "
    "A node typed only by a subclass is not targeted by a shape on its parent "
    "unless pgrdf.materialize has run, or the parent type is stamped explicitly.",
    "CONSTRAINT COMPONENTS ARE FAIL-CLOSED as of 0.6.34: an unenforced component "
    "raises, naming the component and the mode that does evaluate it, rather than "
    "returning a verdict. Verdict `refused-fail-closed` records that. The older "
    "warning that conforms:true could not distinguish 'validated clean' from "
    "'never evaluated' (pgRDF#80) NO LONGER HOLDS FOR COMPONENTS.",
    "THE FAIL-CLOSED CHECK COVERS CONSTRAINT COMPONENTS ONLY, NOT PATH TYPES. A "
    "path defect would produce no violation and no error, so a per-feature verdict "
    "here cannot be trusted for paths on its own.",
    "READ PATH-MATRIX.json FOR PATH BEHAVIOUR. A per-feature verdict cannot "
    "express a fault living in the interaction between a path type and the term "
    "type of the value it reaches, and this file reported such a fault wrongly in "
    "BOTH directions before the matrix existed: `SILENTLY-SKIPPED` overstated it "
    "on oneOrMorePath/zeroOrMorePath, and `enforced` understated it on "
    "sequencePath. The underlying defect (literal values lost on recursive and "
    "sequence paths, rudof-project/rudof#818) is FIXED in the pinned build; the "
    "matrix measures 17/17 cells clean. The matrix, not this file, is what says "
    "so.",
    "`enforced_only_in_mode` names components no default-mode probe catches but "
    "another mode evaluates correctly. sh:sparql is the case: silently skipped by "
    "'native' and 'sparql', evaluated by 'pgrdf'. Reading the native verdict alone "
    "reports 'unsupported' about an engine that supports it.",
  ],
}
with open(out, "w") as f:
    json.dump(doc, f, indent=2, sort_keys=True); f.write("\n")
print(f"\nwrote {out}: {len(doc['enforced'])} enforced, {len(doc['not_enforced'])} not enforced, "
      f"{len(probes)}/{surface_total} of the known surface probed")
PY
