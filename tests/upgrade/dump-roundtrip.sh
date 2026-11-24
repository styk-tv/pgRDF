#!/usr/bin/env bash
# Dump/restore gate (#152): pg_dump a database holding pgRDF graphs, restore
# it into an empty database, and prove nothing was lost and the restored
# database keeps working.
#
# What must survive: the graph registry with every column (owner, lock
# state, source digest), each graph's triples and digest, the default
# partition's rows, the dictionary, and the allocators (graph ids and
# dictionary ids continue past the dumped values, so nothing is handed out
# twice). The restore itself must finish without a single error.
#
# Usage: dump-roundtrip.sh
#   PGRDF_UPGRADE_PSQL is the psql command for a superuser connection
#   (default: psql -X); pg_dump and pg_restore are taken from PGRDF_PGBIN
#   (default: the directory of that psql) with the same connection options.
#   The gate creates and drops its own databases.
set -euo pipefail

read -r -a PSQL <<< "${PGRDF_UPGRADE_PSQL:-psql -X}"
PGBIN=${PGRDF_PGBIN:-$(dirname "$(command -v "${PSQL[0]}")")}
# Connection options are everything after the binary, minus psql-only flags.
CONN=()
for a in "${PSQL[@]:1}"; do [ "$a" = "-X" ] || CONN+=("$a"); done
SRC=pgrdf_dump_src
DST=pgrdf_dump_dst
WORK=$(mktemp -d)

q() { local db=$1; shift; "${PSQL[@]}" -d "$db" -v ON_ERROR_STOP=1 -At "$@"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok   $*"; }
cleanup() {
  "${PSQL[@]}" -d postgres -q -c "DROP DATABASE IF EXISTS $SRC" -c "DROP DATABASE IF EXISTS $DST" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
cleanup; mkdir -p "$WORK"
trap cleanup EXIT

q postgres -q -c "CREATE DATABASE $SRC" -c "CREATE DATABASE $DST"

# 1. A database with three graphs (one locked), rows in the default
#    partition, and a blank node and typed literals in the dictionary.
q $SRC -q -c "CREATE EXTENSION pgrdf"
q $SRC -q <<'SQL' >/dev/null
SELECT pgrdf.parse_turtle_verbose(
  '<urn:rt:s> <urn:rt:p> "kept" . <urn:rt:s> <urn:rt:q> _:b1 . _:b1 <urn:rt:p> 42 .',
  pgrdf.add_graph('urn:rt:open'));
SELECT pgrdf.parse_turtle('<urn:rt:l> <urn:rt:p> "sealed"@en .', pgrdf.add_graph('urn:rt:locked'));
SELECT pgrdf.lock_graph(pgrdf.graph_id('urn:rt:locked'), 'round trip');
SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:rt:locked'));
SELECT pgrdf.add_graph('urn:rt:empty');
SELECT pgrdf.parse_turtle('<urn:rt:d> <urn:rt:p> <urn:rt:o> .', 0);
SQL

state() {
  local db=$1
  # The default graph's row (graph_id 0) is created by every install and is
  # not dumped, so its custody columns do not travel; its triples do.
  echo "-- registry"
  q "$db" -c "SELECT to_jsonb(g) FROM pgrdf._pgrdf_graphs g WHERE graph_id <> 0 ORDER BY graph_id"
  q "$db" -c "SELECT graph_id, iri FROM pgrdf._pgrdf_graphs WHERE graph_id = 0"
  echo "-- rows per graph"
  q "$db" -c "SELECT graph_id, count(*), count(*) FILTER (WHERE is_inferred) FROM pgrdf._pgrdf_quads GROUP BY 1 ORDER BY 1"
  echo "-- digests"
  q "$db" -c "SELECT iri, pgrdf.graph_digest(graph_id) FROM pgrdf._pgrdf_graphs ORDER BY graph_id"
  echo "-- dictionary"
  q "$db" -c "SELECT id, term_type, lexical_value, datatype_iri_id, language_tag FROM pgrdf._pgrdf_dictionary ORDER BY id"
  echo "-- lock triggers"
  q "$db" -c "SELECT c.relname, t.tgname FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
               WHERE c.relnamespace = 'pgrdf'::regnamespace AND NOT t.tgisinternal ORDER BY 1, 2"
  echo "-- acts"
  if [ "$(q "$db" -c "SELECT to_regclass('pgrdf._pgrdf_graph_acts') IS NOT NULL")" = "t" ]; then
    q "$db" -c "SELECT to_jsonb(a) FROM pgrdf._pgrdf_graph_acts a ORDER BY act_id"
  fi
}
state $SRC > "$WORK/before.txt"
max_graph=$(q $SRC -c "SELECT max(graph_id) FROM pgrdf._pgrdf_graphs")
max_term=$(q $SRC -c "SELECT max(id) FROM pgrdf._pgrdf_dictionary")
ok "source database: $(q $SRC -c "SELECT count(*) FROM pgrdf._pgrdf_graphs") registry rows, $(q $SRC -c "SELECT count(*) FROM pgrdf._pgrdf_quads") triples, $max_term terms"

# 2. Dump, then restore into an empty database. Any error stops the restore.
"$PGBIN/pg_dump" "${CONN[@]}" -d $SRC -Fc -f "$WORK/src.dump"
"$PGBIN/pg_restore" "${CONN[@]}" -d $DST "$WORK/src.dump" 2> "$WORK/restore.err" || true
# Known and harmless: the parent table's own indexes exist as soon as a
# partition is attached, so the dump's CREATE INDEX for the same index on
# the partition reports "already exists". Anything else fails the gate.
known='relation "_pgrdf_quads_g[0-9]+_(predicate_id_object_id_subject_id|object_id_subject_id_predicate_id|subject_id_predicate_id_object_id)_is_inferr_idx" already exists'
unexpected=$(grep 'ERROR:' "$WORK/restore.err" | grep -Ev "$known" || true)
[ -z "$unexpected" ] || fail "pg_restore reported: $unexpected"
ok "pg_dump and pg_restore: no error except $(grep -c 'ERROR:' "$WORK/restore.err" || true) known index notices"

# 3. Everything that was there is there.
state $DST > "$WORK/after.txt"
diff "$WORK/before.txt" "$WORK/after.txt" || fail "the restored database differs from the source"
ok "registry, rows, digests, dictionary and lock triggers are identical"

# 4. The restored database works, and its allocators continue: a new graph
#    and a new term get ids above everything dumped.
locked=$("${PSQL[@]}" -d $DST -At -c "\\set VERBOSITY verbose" \
  -c "SELECT pgrdf.parse_turtle('<urn:rt:x> <urn:rt:p> <urn:rt:y> .', pgrdf.graph_id('urn:rt:locked'))" 2>&1 || true)
echo "$locked" | grep -q "55P03" || fail "the restored lock does not refuse writes: $locked"
g=$(q $DST -c "SELECT pgrdf.add_graph('urn:rt:after')")
[ "$g" -gt "$max_graph" ] || fail "graph id $g was handed out again (highest before the dump: $max_graph)"
q $DST -q -c "SELECT pgrdf.parse_turtle('<urn:rt:new-subject> <urn:rt:p> \"new\" .', $g)" >/dev/null
t=$(q $DST -c "SELECT id FROM pgrdf._pgrdf_dictionary WHERE lexical_value = 'urn:rt:new-subject'")
[ "$t" -gt "$max_term" ] || fail "dictionary id $t was handed out again (highest before the dump: $max_term)"
dangling=$(q $DST -c "SELECT count(*) FROM pgrdf._pgrdf_quads q WHERE NOT EXISTS
  (SELECT 1 FROM pgrdf._pgrdf_dictionary d WHERE d.id = q.subject_id)")
[ "$dangling" = "0" ] || fail "$dangling triples point at missing dictionary terms"
q $DST -q -c "SELECT pgrdf.unlock_graph(pgrdf.graph_id('urn:rt:locked'), 'round trip done')" \
  -c "SELECT pgrdf.drop_graph(pgrdf.graph_id('urn:rt:locked'), true)" >/dev/null
ok "after the restore: the lock refuses 55P03, new graph id $g > $max_graph, new term id $t > $max_term"

echo "dump round trip: passed"
