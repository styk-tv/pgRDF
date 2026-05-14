#!/usr/bin/env bash
#
# tests/perf/smoke-ontologies.sh — load every file in fixtures/ontologies/
# through pgrdf.load_turtle, report the triple count or parse error per
# ontology. Manual: not gated in CI because upstream ontologies can
# change (the fetched payloads are not committed; sha256s in
# fixtures/ontologies.manifest.json are the only stability hint).
#
# Usage:
#   tests/perf/smoke-ontologies.sh           # all ontologies in fixtures/ontologies/
#   tests/perf/smoke-ontologies.sh core.ttl  # one file by name
#   tests/perf/smoke-ontologies.sh --check   # regression: diff vs smoke-ontologies.expected.tsv
#
# Each ontology is loaded into its own graph (400 + hash(name) % 1000)
# so they don't collide.
#
# --check mode (regression-coverage slice #58, locked 2026-05-14):
#   Runs the smoke, generates an alphabetically-sorted TSV of
#   filename<TAB>triples for every ontology that parses, and
#   diff -u's it against tests/perf/smoke-ontologies.expected.tsv.
#   Exits non-zero on any difference. Ontologies that PARSE ERROR are
#   omitted from both sides (locked-state set is the parsing subset).
#   Not gated in CI yet — the fetched ontology payloads are gitignored
#   so CI can't run it without a fetch step. See TEST.ONTOLOGY-SET.md
#   "Locked-state regression".
#
# These ontologies are work-in-progress and may contain authoring
# errors. A parse failure here is *signal*, not noise — oxttl is
# strict about RFC 3987 IRIs and the Turtle 1.1 grammar, so anything
# it rejects is genuinely off-spec.
#
# Known parse failures from the 2026-05-13 fetch:
#   prov.ttl     — relative IRIs without @base. Fixed by passing
#                  base_iri to the UDF (see base_iri_for() below).
#   workflow.ttl — `<ckp://Kernel.Name:v0.1>` IRI form (colon in
#                  path) is outside RFC 3986/3987. Source fix needed
#                  in the CKP workflow ontology.

set -u

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RUNTIME="${PGRDF_RUNTIME:-podman}"
CONTAINER="${PGRDF_CONTAINER:-pgrdf-postgres}"
USR="${POSTGRES_USER:-pgrdf}"
DB="${POSTGRES_DB:-pgrdf}"

# Map filename -> base IRI for ontologies that need one. Add lines as
# new vocabularies are introduced.
base_iri_for() {
  case "$1" in
    prov.ttl)   echo "http://www.w3.org/ns/prov#" ;;
    prov-o.ttl) echo "http://www.w3.org/ns/prov-o#" ;;
    *)          echo "" ;;
  esac
}

check_mode=0
declare -a files=()
declare -a pos_args=()
for arg in "$@"; do
  case "$arg" in
    --check) check_mode=1 ;;
    *)       pos_args+=( "$arg" ) ;;
  esac
done

# `${pos_args[@]:-}` keeps `set -u` happy when no positional args were
# given (bash 3.2 on macOS treats an empty array as unset).
if [ ${#pos_args[@]} -eq 0 ]; then
  files=( "${REPO_ROOT}"/fixtures/ontologies/*.ttl )
else
  for arg in "${pos_args[@]}"; do
    files+=( "${REPO_ROOT}/fixtures/ontologies/${arg}" )
  done
fi

ok=0
err=0
total_triples=0

# In --check mode we collect filename<TAB>triples rows here and emit
# them sorted at the end so the comparison is deterministic. The
# pretty-printed per-ontology lines still stream to stdout so a human
# watching can see the run.
actual_rows=""

printf '%-32s %12s %s\n' "ONTOLOGY" "TRIPLES" "NOTES"
printf '%-32s %12s %s\n' "--------" "-------" "-----"

for f in "${files[@]}"; do
  [ -f "$f" ] || continue
  name="$(basename "$f")"
  base="$(base_iri_for "$name")"
  # `10#` forces base-10 so a leading '0' in the digit slice doesn't
  # get interpreted as octal (which fails on 8/9).
  g=$(( 400 + 10#$(printf '%s' "$name" | shasum | tr -dc '0-9' | head -c5) % 1000 ))
  base_arg=""
  [ -n "$base" ] && base_arg=", '$base'"
  "${RUNTIME}" exec -i "${CONTAINER}" psql -U "${USR}" -d "${DB}" -X -A -t -q \
    -c "SELECT pgrdf.add_graph(${g})" >/dev/null 2>&1
  out="$("${RUNTIME}" exec -i "${CONTAINER}" psql -U "${USR}" -d "${DB}" -X -A -t -q \
        -c "SELECT pgrdf.load_turtle('/fixtures/ontologies/${name}', ${g}${base_arg})" 2>&1)"
  case "$out" in
    ERROR:*)
      err=$((err + 1))
      printf '  \033[31m%-30s\033[0m %12s %s\n' "$name" "-" "${out#ERROR:  }"
      ;;
    *)
      ok=$((ok + 1))
      total_triples=$((total_triples + out))
      base_note=""
      [ -n "$base" ] && base_note="base=$base"
      printf '  \033[32m%-30s\033[0m %12d %s\n' "$name" "$out" "$base_note"
      actual_rows="${actual_rows}${name}	${out}
"
      ;;
  esac
done

echo
printf 'Summary: %d ok, %d failed, %d triples loaded across them.\n' \
  "${ok}" "${err}" "${total_triples}"

if [ "${check_mode}" -eq 1 ]; then
  expected="${REPO_ROOT}/tests/perf/smoke-ontologies.expected.tsv"
  if [ ! -f "${expected}" ]; then
    echo
    echo "smoke-ontologies.sh --check: missing lock-file ${expected}" >&2
    exit 2
  fi
  actual_file="$(mktemp -t smoke-ontologies-actual.XXXXXX)"
  printf '%s' "${actual_rows}" | sort > "${actual_file}"
  echo
  if diff -u "${expected}" "${actual_file}"; then
    echo "smoke-ontologies.sh --check: OK (${ok} ontologies, ${total_triples} triples)"
    rm -f "${actual_file}"
    exit 0
  else
    echo "smoke-ontologies.sh --check: FAIL — actual differs from ${expected}" >&2
    echo "(actual rows preserved at ${actual_file} for inspection)" >&2
    exit 1
  fi
fi
