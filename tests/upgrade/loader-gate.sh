#!/usr/bin/env bash
# Loader gate: what the parallel in-database test suite cannot reach, because
# it needs committed transactions, background workers, a second session, or
# index DDL on the shared quad table.
#
#   A. The staged loader (background workers): stores a set, keeps blank
#      subjects as blank nodes, registers its graph with an owner, and writes
#      its acts with the caller's declared identity.
#   B. A term containing U+0000 is refused 22P05 by the staged and the
#      streaming routes, and nothing is stored.
#   C. graph_activity() shows a load from a second session while it runs.
#   D. pgrdf.notify_acts delivers NOTIFY pgrdf_acts on commit.
#   E. Two transactions on one connection record their own declared values.
#
# Usage: loader-gate.sh
#   The server must have pgrdf in shared_preload_libraries and be able to read
#   files this script writes under ${TMPDIR:-/tmp}. PGRDF_UPGRADE_PSQL is the
#   psql command for a superuser connection (default: psql -X). The gate
#   creates and drops its own databases.
set -euo pipefail

read -r -a PSQL <<< "${PGRDF_UPGRADE_PSQL:-psql -X}"
A=pgrdf_loader_gate_a
B=pgrdf_loader_gate_b
WORK=$(mktemp -d "${TMPDIR:-/tmp}/pgrdf-loader-gate.XXXXXX")
chmod 755 "$WORK"

q() { local db=$1; shift; "${PSQL[@]}" -d "$db" -v ON_ERROR_STOP=1 -At "$@"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok   $*"; }
cleanup() {
  "${PSQL[@]}" -d postgres -q -c "DROP DATABASE IF EXISTS $A" -c "DROP DATABASE IF EXISTS $B" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT
"${PSQL[@]}" -d postgres -q -c "DROP DATABASE IF EXISTS $A" -c "DROP DATABASE IF EXISTS $B" >/dev/null 2>&1 || true
q postgres -q -c "CREATE DATABASE $A" -c "CREATE DATABASE $B"
q $A -q -c "CREATE EXTENSION pgrdf"
q $B -q -c "CREATE EXTENSION pgrdf"
[ "$(q $A -c "SELECT (pgrdf.stats() ->> 'shmem_ready')")" = "true" ] \
  || fail "pgrdf is not in shared_preload_libraries on this server"

# ── A. the staged loader ────────────────────────────────────────────────
cat > "$WORK/set.nt" <<'NT'
<urn:lg:a> <urn:lg:p> "v" .
<urn:lg:a> <urn:lg:p> "v" .
_:b1 <urn:lg:p> <urn:lg:a> .
<urn:lg:a> <urn:lg:q> _:b1 .
NT
chmod 644 "$WORK/set.nt"
res=$(q $A -c "SET pgrdf.workspace = 'lg-ws'" -c "SET pgrdf.agent = 'lg-agent'" \
  -c "SET pgrdf.thread = 't-1'" -c "SET application_name = 'loader-gate'" \
  -c "SELECT pgrdf.load_turtle_staged_run('$WORK/set.nt', 500, 2)" | tail -n 1)
[ "$(echo "$res" | "${PSQL[@]}" -d $A -At -c "SELECT (\$j\$$res\$j\$::jsonb ->> 'ok')")" = "true" ] \
  || fail "the staged load did not succeed: $res"
load=$(q $A -c "SELECT concat_ws('/', l ->> 'triples_read', l ->> 'triples_stored', l ->> 'duplicates_dropped', l ->> 'graph_was_empty')
                  FROM (SELECT \$j\$$res\$j\$::jsonb -> 'load' AS l) x")
[ "$load" = "4/3/1/true" ] || fail "staged import report is '$load', expected 4/3/1/true (read/stored/dropped/was empty)"
rows=$(q $A -c "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = 500")
[ "$rows" = "3" ] || fail "graph 500 holds $rows rows, expected 3: the repeated line was stored"
[ "$(q $A -c "SELECT count(*) FROM pgrdf.duplicate_report()")" = "0" ] || fail "duplicate_report() lists the staged graph"
ok "staged load stores a set: read 4, stored 3, dropped 1; the graph carries its unique index"

blank=$(q $A -c "SELECT concat_ws('/',
    (SELECT count(*) FROM pgrdf._pgrdf_dictionary WHERE term_type = 2),
    (SELECT count(*) FROM pgrdf._pgrdf_dictionary WHERE term_type = 1 AND lexical_value IN ('b1', '_:b1')),
    (SELECT count(*) FROM pgrdf._pgrdf_quads s JOIN pgrdf._pgrdf_quads o ON o.object_id = s.subject_id
      JOIN pgrdf._pgrdf_dictionary d ON d.id = s.subject_id
      WHERE s.graph_id = 500 AND o.graph_id = 500 AND d.term_type = 2))")
[ "$blank" = "1/0/1" ] || fail "blank subject: '$blank', expected 1/0/1 (one blank term, no IRI term for it, its two uses join)"
ok "a blank-node subject is stored as a blank node and joins its use as an object"

reg=$(q $A -c "SELECT concat_ws('/', iri, owner, created_act IS NOT NULL) FROM pgrdf._pgrdf_graphs WHERE graph_id = 500")
[ "$reg" = "urn:pgrdf:graph:500/postgres/t" ] || fail "registry row of the staged graph: '$reg'"
acts=$(q $A -c "SELECT string_agg(act, ',' ORDER BY act_id) FROM pgrdf.graph_acts(500)")
[ "$acts" = "create,load,dedup" ] || fail "acts of the staged graph: '$acts', expected create,load,dedup"
who=$(q $A -c "SELECT string_agg(DISTINCT concat_ws('/', workspace, agent, thread, application_name, acting_role, session_role), ';') FROM pgrdf.graph_acts(500)")
[ "$who" = "lg-ws/lg-agent/t-1/loader-gate/postgres/postgres" ] || fail "identity on the staged acts: '$who'"
route=$(q $A -c "SELECT detail ->> 'route' FROM pgrdf.graph_acts(500) WHERE act = 'load'")
[ "$route" = "staged" ] || fail "the load act's route is '$route'"
ok "the staged graph is registered with its owner; acts $acts carry the caller's declared identity"

# The same session reads the report back, and so does a load that the
# staged route declines: this database now holds data, so load_turtle of an
# N-Triples file tries the staged loader, is told to fall back, and loads
# through the standard parser. That load is recorded like any other.
last=$(q $A -c "SELECT pgrdf.load_turtle_staged_run('$WORK/set.nt', 501, 1) -> 'fallback'" \
  -c "SELECT pgrdf.load_turtle('$WORK/set.nt', pgrdf.add_graph('urn:lg:fallback'))" \
  -c "SELECT concat_ws('/', l ->> 'triples_read', l ->> 'triples_stored', l ->> 'duplicates_dropped') FROM (SELECT pgrdf.last_call_stats() -> 'load' AS l) x" | tr '\n' ' ')
[ "$last" = "true 4 4/3/1 " ] || fail "fallback load: got '$last', expected 'true 4 4/3/1 ' (declined, 4 read, report 4/3/1)"
fb=$(q $A -c "SELECT concat_ws('/', count(*), max(detail -> 'load' ->> 'triples_stored')) FROM pgrdf.graph_acts(pgrdf.graph_id('urn:lg:fallback')) WHERE act = 'load'")
[ "$fb" = "1/3" ] || fail "fallback load act: '$fb', expected one load act with 3 stored"
ok "a load the staged route declines is recorded: one load act, and last_call_stats() carries its report"

# ── B. U+0000 through the staged and the streaming routes ───────────────
printf '<urn:lg:n> <urn:lg:p> "ok" .\n<urn:lg:n> <urn:lg:p> "a\\u0000b" .\n' > "$WORK/nul.nt"
chmod 644 "$WORK/nul.nt"
res=$(q $B -c "SELECT pgrdf.load_turtle_staged_run('$WORK/nul.nt', 600, 1)")
st=$(q $B -c "SELECT concat_ws('/', j ->> 'ok', j ->> 'sqlstate') FROM (SELECT \$j\$$res\$j\$::jsonb AS j) x")
[ "$st" = "false/22P05" ] || fail "staged load of a U+0000 term: '$st', expected false/22P05 ($res)"
for call in "pgrdf.load_turtle('$WORK/nul.nt', 601)" "pgrdf.load_turtle_streaming('$WORK/nul.nt', 602)"; do
  out=$("${PSQL[@]}" -d $B -At -c "\\set VERBOSITY verbose" -c "SELECT $call" 2>&1 || true)
  echo "$out" | grep -q "22P05" || fail "$call did not refuse 22P05: $out"
done
stored=$(q $B -c "SELECT (SELECT count(*) FROM pgrdf._pgrdf_dictionary) + (SELECT count(*) FROM pgrdf._pgrdf_quads)")
[ "$stored" = "0" ] || fail "a refused load left $stored rows behind"
# The staged route drops indexes before it reads the file. A refusal puts
# them back and removes its staging table: the database is as it was.
left=$(q $B -c "SELECT concat_ws('/',
    (SELECT count(*) FROM pg_indexes WHERE schemaname = 'pgrdf'
        AND indexname IN ('_pgrdf_idx_pos', '_pgrdf_idx_osp', '_pgrdf_dict_val_idx', '_pgrdf_quads_g600_spo_uniq')),
    (SELECT count(*) FROM pg_constraint WHERE conname = 'unique_term'),
    (SELECT count(*) FROM pg_class WHERE relname LIKE '\_pgrdf\_stg\_%'))")
[ "$left" = "4/1/0" ] || fail "after a refused staged load: indexes/constraint/staging tables = $left, expected 4/1/0"
ok "a term containing U+0000 is refused 22P05 by the staged, default and streaming routes; nothing is stored, and the staged route restores the indexes it dropped"

# ── C. graph_activity from a second session ─────────────────────────────
awk 'BEGIN { for (i = 0; i < 80000; i++) printf "<urn:lg:s%d> <urn:lg:p> \"%d\" .\n", i, i }' > "$WORK/big.nt"
chmod 644 "$WORK/big.nt"
q $A -q -c "SELECT pgrdf.add_graph(700)" >/dev/null
# Fresh statistics, so the load below takes seconds and not minutes: the
# batched loader's dictionary lookups plan badly on a never-analyzed table.
q $A -q -c "ANALYZE pgrdf._pgrdf_dictionary"
( q $A -q -c "SET pgrdf.workspace = 'live-ws'" \
    -c "SELECT pgrdf.load_turtle('$WORK/big.nt', 700, 'http://base.example/')" >/dev/null ) &
loader=$!
seen=""
for _ in $(seq 1 400); do
  seen=$(q $A -c "SELECT concat_ws('/', graph_id, act, part, workspace) FROM pgrdf.graph_activity(700) LIMIT 1")
  [ -n "$seen" ] && break
  kill -0 $loader 2>/dev/null || break
done
wait $loader
[ "$seen" = "700/load/backend/live-ws" ] || fail "graph_activity never showed the running load (saw '$seen')"
[ "$(q $A -c "SELECT count(*) FROM pgrdf.graph_activity()")" = "0" ] || fail "graph_activity still lists a backend after the load ended"
ok "graph_activity showed the load from a second session ($seen) and nothing after it ended"

# ── D. NOTIFY on committed acts ─────────────────────────────────────────
out=$("${PSQL[@]}" -d $A -v ON_ERROR_STOP=1 -c "LISTEN pgrdf_acts" -c "SET pgrdf.notify_acts = on" \
  -c "SELECT pgrdf.add_graph('urn:lg:notify')" -c "SELECT 1" 2>&1)
g=$(q $A -c "SELECT pgrdf.graph_id('urn:lg:notify')")
a=$(q $A -c "SELECT act_id FROM pgrdf.graph_acts($g) WHERE act = 'create'")
echo "$out" | grep -q "\"pgrdf_acts\" with payload \"$a:$g:create\"" \
  || fail "no notification for act $a on graph $g: $out"
quiet=$("${PSQL[@]}" -d $A -v ON_ERROR_STOP=1 -c "LISTEN pgrdf_acts" \
  -c "SELECT pgrdf.add_graph('urn:lg:quiet')" -c "SELECT 1" 2>&1)
echo "$quiet" | grep -q "pgrdf_acts" && fail "an act notified with pgrdf.notify_acts off: $quiet"
ok "pgrdf.notify_acts delivers pgrdf_acts '$a:$g:create' on commit, and nothing when off"

# ── E. one connection, two transactions, two identities ─────────────────
q $A -q <<'SQL' >/dev/null
BEGIN; SET LOCAL pgrdf.workspace = 'first';  SET LOCAL pgrdf.agent = 'a1'; SELECT pgrdf.add_graph('urn:lg:pool:1'); COMMIT;
BEGIN; SET LOCAL pgrdf.workspace = 'second'; SELECT pgrdf.add_graph('urn:lg:pool:2'); COMMIT;
SELECT pgrdf.add_graph('urn:lg:pool:3');
SQL
pool=$(q $A -c "SELECT string_agg(concat_ws('/', coalesce(a.workspace, '-'), coalesce(a.agent, '-')), ' ' ORDER BY g.iri)
                  FROM pgrdf._pgrdf_graphs g JOIN pgrdf._pgrdf_graph_acts a ON a.act_id = g.created_act
                 WHERE g.iri LIKE 'urn:lg:pool:%'")
[ "$pool" = "first/a1 second/- -/-" ] || fail "declared values leaked between transactions: '$pool'"
ok "each transaction's act carries its own declared values, never the previous one's"

# F. After every route above (staged with a dedup, a declined staged load,
#    refused loads, loads from two sessions), every graph's stored count is
#    verified and equal to a scan.
for db in $A $B; do
  wrong=$(q $db -c "SELECT count(*) FROM pgrdf.graph_inventory() i
                     WHERE NOT i.counts_verified
                        OR i.asserted <> (SELECT count(*) FROM pgrdf._pgrdf_quads q WHERE q.graph_id = i.graph_id AND NOT q.is_inferred)
                        OR i.inferred <> (SELECT count(*) FROM pgrdf._pgrdf_quads q WHERE q.graph_id = i.graph_id AND q.is_inferred)")
  [ "$wrong" = "0" ] || fail "$db: $wrong graph(s) unverified or miscounted: $(q $db -c "SELECT string_agg(graph_id || ':' || asserted || '+' || inferred || ':' || counts_verified, ' ') FROM pgrdf.graph_inventory()")"
done
ok "every graph's stored count is verified and equal to a scan after the staged, declined, refused and two-session loads"

echo "loader gate: passed"
