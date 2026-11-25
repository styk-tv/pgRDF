#!/usr/bin/env bash
# Counts gate (0.6.42): what the stored graph counts and the act feed must
# hold across committed transactions and more than one session, which the
# in-database test suite cannot reach (each of its tests is one transaction).
#
#   A. Many committed writes: the count stays equal to a scan through every
#      fold, and the deltas waiting to be folded stay few.
#   B. Several sessions writing one graph at once: no writer waits on a
#      count, nothing deadlocks, and the count is exact.
#   C. A write in plain SQL that commits after a recount scanned the graph
#      leaves the count unverified: a recount never hides a row it did not see.
#   D. The act feed delivers an act whose transaction commits late, pages
#      without losing or repeating the cursor, and never shows its caller
#      acts that can still roll back.
#   E. Speed: the inventory, one graph's row and the feed, at 50 graphs
#      and PGRDF_GATE_TRIPLES triples, and that the inventory does not grow
#      with the number of triples.
#   F. Speed under churn, twice: while a reader holds its snapshot (the
#      bounds of E hold), and while an idle transaction holds an xid (the
#      inventory stays within a wider, stated bound, and returns to the
#      bounds of E when that transaction ends).
#
# Usage: counts-gate.sh
#   PGRDF_UPGRADE_PSQL is the psql command for a superuser connection
#   (default: psql -X). PGRDF_GATE_TRIPLES (default 10000000) and
#   PGRDF_GATE_CHURN (default 20000) size parts E and F. The gate creates
#   and drops its own database.
set -euo pipefail

read -r -a PSQL <<< "${PGRDF_UPGRADE_PSQL:-psql -X}"
DB=pgrdf_counts_gate
TRIPLES=${PGRDF_GATE_TRIPLES:-10000000}
CHURN=${PGRDF_GATE_CHURN:-20000}
WORK=$(mktemp -d "${TMPDIR:-/tmp}/pgrdf-counts-gate.XXXXXX")

q() { "${PSQL[@]}" -d $DB -v ON_ERROR_STOP=1 -At "$@"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok   $*"; }
held=""
cleanup() {
  exec 3>&- 2>/dev/null || true
  [ -n "$held" ] && kill "$held" 2>/dev/null || true
  "${PSQL[@]}" -d postgres -q -c "DROP DATABASE IF EXISTS $DB" >/dev/null 2>&1 || true
  rm -rf "$WORK"
}
trap cleanup EXIT
"${PSQL[@]}" -d postgres -q -c "DROP DATABASE IF EXISTS $DB" >/dev/null 2>&1 || true
"${PSQL[@]}" -d postgres -q -v ON_ERROR_STOP=1 -c "CREATE DATABASE $DB"
q -q -c "CREATE EXTENSION pgrdf"

# A session that stays connected: hold "<sql to run first>" opens it on fd 3,
# say "<sql>" sends it more, release closes it and waits for it to end.
hold() {
  mkfifo "$WORK/fifo"
  "${PSQL[@]}" -d $DB -v ON_ERROR_STOP=1 -At < "$WORK/fifo" > "$WORK/held.out" 2>&1 &
  held=$!
  exec 3> "$WORK/fifo"
  echo "$1" >&3
}
say() { echo "$1" >&3; }
release() {
  exec 3>&-
  wait "$held" || fail "the held session failed: $(cat "$WORK/held.out")"
  held=""
  rm -f "$WORK/fifo"
}
# Wait until a statement run in another session answers 't'.
until_true() {
  for _ in $(seq 1 100); do
    [ "$(q -c "$1")" = "t" ] && return 0
    sleep 0.1
  done
  fail "timed out waiting for: $1"
}

# Graphs whose stored count is unverified or differs from a scan.
truth="SELECT count(*) FROM pgrdf.graph_inventory() i
        WHERE NOT i.counts_verified
           OR i.asserted <> (SELECT count(*) FROM pgrdf._pgrdf_quads q
                              WHERE q.graph_id = i.graph_id AND NOT q.is_inferred)
           OR i.inferred <> (SELECT count(*) FROM pgrdf._pgrdf_quads q
                              WHERE q.graph_id = i.graph_id AND q.is_inferred)"
true_or_fail() {
  local wrong; wrong=$(q -c "$truth")
  [ "$wrong" = "0" ] || fail "$1: $wrong graph(s) unverified or miscounted: $(q -c "SELECT string_agg(graph_id || ':' || asserted || '+' || inferred || ':' || counts_verified, ' ') FROM pgrdf.graph_inventory()")"
}
# n autocommit single-row writes into graph g, subjects from..from+n-1.
writes() {
  echo "SELECT format('SELECT pgrdf.put_quad(%s, 1, 1, $1);', i) FROM generate_series($2, $2 + $3 - 1) i \gexec" \
    | q -q > /dev/null
}

# ── A. Many committed writes ────────────────────────────────────────────
ga=$(q -c "SELECT pgrdf.add_graph('urn:cg:a')")
writes "$ga" 1 3000
true_or_fail "after 3000 committed writes"
pending=$(q -c "SELECT count(*) FROM pgrdf._pgrdf_graph_count_deltas WHERE graph_id = $ga")
[ "$pending" -lt 1000 ] || fail "$pending deltas are waiting after 3000 writes: nothing folds"
state=$(q -c "SELECT asserted || ' ' || change_seq FROM pgrdf.graph_inventory($ga)")
[ "$state" = "3000 3000" ] || fail "after 3000 writes the row reads '$state', expected '3000 3000'"
writes "$ga" 1 500
state=$(q -c "SELECT asserted || ' ' || change_seq FROM pgrdf.graph_inventory($ga)")
[ "$state" = "3000 3000" ] || fail "500 writes that stored nothing moved the row to '$state'"
ok "3000 committed writes: count and change_seq are 3000, $pending deltas wait to be folded; 500 writes that stored nothing moved neither"

# ── B. Four sessions, one graph ─────────────────────────────────────────
gb=$(q -c "SELECT pgrdf.add_graph('urn:cg:b')")
pids=""
for s in 0 1 2 3; do
  writes "$gb" $((s * 100000 + 1)) 1000 &
  pids="$pids $!"
done
for p in $pids; do wait "$p" || fail "a concurrent writer failed"; done
true_or_fail "after four concurrent writers"
[ "$(q -c "SELECT asserted FROM pgrdf.graph_inventory($gb)")" = "4000" ] || fail "four writers of 1000 rows left $(q -c "SELECT asserted FROM pgrdf.graph_inventory($gb)")"
ok "four sessions wrote 1000 rows each into one graph: no failure, count 4000 and equal to a scan"

# ── C. A recount never hides a row it did not see ───────────────────────
gc=$(q -c "SELECT pgrdf.add_graph('urn:cg:c')")
writes "$gc" 1 10
hold "BEGIN; INSERT INTO pgrdf._pgrdf_quads_g$gc (subject_id, predicate_id, object_id, graph_id) VALUES (900, 1, 1, $gc);"
until_true "SELECT count(*) > 0 FROM pg_locks l JOIN pg_class c ON c.oid = l.relation WHERE c.relname = '_pgrdf_quads_g$gc' AND l.mode = 'RowExclusiveLock'"
recount=$(q -c "SELECT asserted || ' ' || was_verified FROM pgrdf.recount_graph($gc)")
[ "$recount" = "10 true" ] || fail "a recount beside an open direct write returned '$recount', expected '10 true'"
say "COMMIT;"
release
state=$(q -c "SELECT asserted || ' ' || counts_verified FROM pgrdf.graph_inventory($gc)")
[ "$state" = "11 false" ] || fail "after the direct write committed the row reads '$state', expected '11 false'"
q -q -c "SELECT pgrdf.recount_graph($gc)" > /dev/null
state=$(q -c "SELECT asserted || ' ' || counts_verified FROM pgrdf.graph_inventory($gc)")
[ "$state" = "11 true" ] || fail "after the second recount the row reads '$state', expected '11 true'"
true_or_fail "after the recount race"
ok "a direct write that committed after a recount scanned the graph leaves the count unverified and exact (11); the next recount verifies it"

# ── D. The act feed ─────────────────────────────────────────────────────
# A transaction that creates a graph holds the quad table until it ends, so
# the late act here is a load into a graph that already exists.
gl=$(q -c "SELECT pgrdf.add_graph('urn:cg:late')")
ge=$(q -c "SELECT pgrdf.add_graph('urn:cg:early')")
loads="coalesce(string_agg(DISTINCT graph_iri, ',' ORDER BY graph_iri) FILTER (WHERE act = 'load' AND graph_iri IN ('urn:cg:late', 'urn:cg:early')), '')"
hold "BEGIN; SELECT pgrdf.parse_turtle('<urn:cg:l> <urn:cg:p> <urn:cg:o> .', $gl);"
until_true "SELECT count(*) > 0 FROM pg_stat_activity WHERE state = 'idle in transaction' AND datname = '$DB'"
q -q -c "SELECT pgrdf.parse_turtle('<urn:cg:e> <urn:cg:p> <urn:cg:o> .', $ge)" > /dev/null
first=$(q -c "SELECT max(complete_below::text) || '|' || $loads FROM pgrdf.graph_act_feed()")
cursor=${first%%|*}
[ "${first#*|}" = "urn:cg:early" ] || fail "the first read should show the early load only; it showed '${first#*|}'"
by_time=$(q -c "SELECT clock_timestamp()")
say "COMMIT;"
release
second=$(q -c "SELECT $loads FROM pgrdf.graph_act_feed('$cursor'::xid8)")
[ "$second" = "urn:cg:early,urn:cg:late" ] || fail "reading from the cursor $cursor should deliver the late load; it delivered '$second'"
missed=$(q -c "SELECT count(*) FROM pgrdf.graph_acts(NULL, '$by_time'::timestamptz) WHERE act = 'load' AND graph_iri = 'urn:cg:late'")
[ "$missed" = "0" ] || fail "graph_acts(since) was expected to miss the late act; this gate's premise no longer holds"
# Paging: one act a page, to the end. Every act once, one cursor throughout.
all=$(q -c "SELECT count(*) FROM pgrdf._pgrdf_graph_acts")
page=""; seen=0; cursors=""
for _ in $(seq 1 $((all + 5))); do
  if [ -z "$page" ]; then arg="NULL"; else arg="'$page'"; fi
  row=$(q -c "SELECT max(complete_below::text) || '|' || coalesce(max(next_page), '') || '|' || count(act_id) FROM pgrdf.graph_act_feed(NULL, NULL, $arg, 1)")
  cursors="$cursors ${row%%|*}"
  seen=$((seen + ${row##*|}))
  page=$(echo "$row" | cut -d'|' -f2)
  [ -n "$page" ] || break
done
[ "$seen" = "$all" ] || fail "paging one act at a time delivered $seen of $all acts"
[ "$(echo $cursors | tr ' ' '\n' | sort -u | wc -l | tr -d ' ')" = "1" ] || fail "the cursor changed between the pages of one read: $cursors"
own=$(q -q -c "BEGIN" -c "SELECT pgrdf.parse_turtle('<urn:cg:m> <urn:cg:p> <urn:cg:o> .', $ge)" -c "SELECT count(*) FROM pgrdf.graph_act_feed() WHERE xact = pg_current_xact_id()" -c "ROLLBACK" | sed -n 2p)
[ "$own" = "0" ] || fail "the feed showed its caller $own act(s) of its own open transaction"
ok "the feed delivered an act that committed late (graph_acts since a time misses it), paged $all acts one at a time under one cursor, and showed no act of its caller's open transaction"

# ── E. Speed ────────────────────────────────────────────────────────────
q -q -c "CREATE FUNCTION public.gate_ms(stmt text, runs int DEFAULT 51) RETURNS text LANGUAGE plpgsql AS \$\$
DECLARE t0 timestamptz; ms numeric[] := '{}'; BEGIN
  FOR i IN 1..runs LOOP
    t0 := clock_timestamp(); EXECUTE stmt;
    ms := ms || (extract(epoch FROM clock_timestamp() - t0) * 1000)::numeric;
  END LOOP;
  RETURN (SELECT round(percentile_cont(0.5) WITHIN GROUP (ORDER BY x)::numeric, 3) || ' ' ||
                 round(percentile_cont(0.95) WITHIN GROUP (ORDER BY x)::numeric, 3) FROM unnest(ms) x);
END \$\$"
for i in $(seq 1 46); do q -q -c "SELECT pgrdf.parse_turtle('<urn:cg:s$i> <urn:cg:p> <urn:cg:o> .', pgrdf.add_graph('urn:cg:small:$i'))" > /dev/null; done
graphs=$(q -c "SELECT count(*) FROM pgrdf._pgrdf_graphs")
[ "$graphs" -ge 50 ] || fail "the speed fixture has $graphs graphs, expected at least 50"
inventory="SELECT count(*) FROM pgrdf.graph_inventory()"
one_row="SELECT count(*) FROM pgrdf.graph_inventory($ga)"
feed="SELECT count(*) FROM pgrdf.graph_act_feed((SELECT max(xact) FROM pgrdf._pgrdf_graph_acts))"
q -q -c "SELECT pgrdf.recount_graph()" > /dev/null
# A session that lists graphs takes no transaction id: it must not become a
# transaction every fold and every feed cursor has to wait out.
for stmt in "$inventory" "$one_row" "SELECT count(*) FROM pgrdf.graph_inventory('urn:cg:a')" \
            "SELECT count(*) FROM pgrdf.graph_totals()" "SELECT pgrdf.count_quads($ga)" "$feed"; do
  xid=$(q -q -c "BEGIN" -c "$stmt" -c "SELECT coalesce(pg_current_xact_id_if_assigned()::text, 'none')" -c "ROLLBACK" | tail -1)
  [ "$xid" = "none" ] || fail "a read took transaction id $xid: $stmt"
done
small=$(q -c "SELECT public.gate_ms('$inventory')")
# The triples go in with plain SQL, five graphs at a time: the gate is about
# reading counts, not loading. The recount that follows is the one scan.
per=$((TRIPLES / 5))
for i in 1 2 3 4 5; do
  g=$(q -c "SELECT pgrdf.add_graph('urn:cg:big:$i')")
  q -q -c "INSERT INTO pgrdf._pgrdf_quads_g$g (subject_id, predicate_id, object_id, graph_id) SELECT i, 1, i, $g FROM generate_series(1, $per) i"
done
q -q -c "SELECT pgrdf.recount_graph()" -c "VACUUM ANALYZE" > /dev/null
true_or_fail "after loading $TRIPLES triples"
total=$(q -c "SELECT asserted FROM pgrdf.graph_totals()")
[ "$total" -ge "$TRIPLES" ] || fail "graph_totals() counts $total triples, expected at least $TRIPLES"
# under <label> <statement> <median bound ms>: prints "median p95".
under() {
  local got; got=$(q -c "SELECT public.gate_ms(\$q\$$2\$q\$)")
  awk -v m="${got%% *}" -v b="$3" 'BEGIN { exit !(m <= b) }' \
    || fail "$1: median ${got%% *} ms (p95 ${got##* } ms) is over $3 ms"
  echo "$got"
}
t_inv=$(under "graph_inventory() at $graphs graphs and $total triples" "$inventory" 5)
t_one=$(under "graph_inventory(graph_id)" "$one_row" 1)
t_feed=$(under "graph_act_feed(cursor)" "$feed" 2)
awk -v big="${t_inv%% *}" -v small="${small%% *}" 'BEGIN { exit !(big <= 2 * small + 1) }' \
  || fail "graph_inventory() took ${small%% *} ms before the triples and ${t_inv%% *} ms after: it grows with the data"
ok "at $graphs graphs and $total triples (median, p95 ms): graph_inventory() $t_inv, one graph $t_one, feed $t_feed; the inventory took ${small%% *} ms before the triples were there; none of the listing calls takes a transaction id"

# ── F. Speed under churn ────────────────────────────────────────────────
# (ii) A reader holds its snapshot: nothing a fold deletes can be pruned.
hold "BEGIN ISOLATION LEVEL REPEATABLE READ; SELECT count(*) FROM pgrdf.graph_inventory();"
until_true "SELECT count(*) > 0 FROM pg_stat_activity WHERE state = 'idle in transaction' AND datname = '$DB'"
writes "$ga" 1000001 "$CHURN"
c_inv=$(under "graph_inventory() after $CHURN writes under a held snapshot" "$inventory" 5)
c_one=$(under "graph_inventory(graph_id) after $CHURN writes under a held snapshot" "$one_row" 1)
c_feed=$(under "graph_act_feed(cursor) after $CHURN writes under a held snapshot" "$feed" 2)
say "COMMIT;"
release
true_or_fail "after the churn under a held snapshot"
ok "after $CHURN committed writes while a reader held its snapshot (median, p95 ms): graph_inventory() $c_inv, one graph $c_one, feed $c_feed"

# (i) An idle transaction holds an xid: no fold can pass it, so the deltas
#     written meanwhile stay in every reader's range until it ends.
hold "BEGIN; SELECT pg_current_xact_id();"
until_true "SELECT count(*) > 0 FROM pg_stat_activity WHERE state = 'idle in transaction' AND datname = '$DB'"
writes "$ga" 2000001 "$CHURN"
waiting=$(q -c "SELECT count(*) FROM pgrdf._pgrdf_graph_count_deltas WHERE graph_id = $ga")
x_inv=$(under "graph_inventory() with $waiting deltas held back by an open transaction" "$inventory" 25)
x_one=$(under "graph_inventory(graph_id) with $waiting deltas held back by an open transaction" "$one_row" 25)
true_or_fail "while an open transaction holds the deltas back"
say "COMMIT;"
release
folded=$(q -c "SELECT pgrdf._fold_counts($ga)")
[ "$folded" -gt 0 ] || fail "nothing folded after the open transaction ended ($waiting deltas were waiting)"
q -q -c "VACUUM pgrdf._pgrdf_graph_count_deltas, pgrdf._pgrdf_graph_counts" > /dev/null
r_inv=$(under "graph_inventory() after the open transaction ended" "$inventory" 5)
r_one=$(under "graph_inventory(graph_id) after the open transaction ended" "$one_row" 1)
true_or_fail "at the end of the gate"
ok "with $waiting deltas held back by an idle transaction that holds an xid (median, p95 ms): graph_inventory() $x_inv, one graph $x_one, exact throughout; after it ended: $r_inv and $r_one"

echo "counts gate: passed"
