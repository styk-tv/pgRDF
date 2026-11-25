#!/usr/bin/env bash
# Counts gate (0.6.42): what the stored graph counts and the act feed must
# hold across committed transactions and more than one session, which the
# in-database test suite cannot reach (each of its tests is one transaction).
#
#   A. Many committed writes: the count stays equal to a scan through every
#      fold, and the deltas waiting to be folded stay few.
#   A2. The same for the dictionary's term count, from one session and
#      from four at once.
#   A3. Writes into a graph id that was never registered leave no count
#      row behind.
#   A4. A graph without its count row (the state a restore leaves) keeps
#      few deltas, reads exact, and is verified by one recount.
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
#   E2. The feed on a record of PGRDF_GATE_ACTS acts: the cursor-only
#      call a client starts with, and a poll from the end repeated in one
#      session, stay within the bounds of E.
#   F. Speed under churn, twice: while a reader holds its snapshot (the
#      bounds of E hold), and while an idle transaction holds an xid (the
#      inventory stays within a wider, stated bound, and returns to the
#      bounds of E when that transaction ends).
#
# Usage: counts-gate.sh
#   PGRDF_UPGRADE_PSQL is the psql command for a superuser connection
#   (default: psql -X). PGRDF_GATE_TRIPLES (default 10000000),
#   PGRDF_GATE_ACTS (default 200000) and PGRDF_GATE_CHURN (default 20000)
#   size parts E, E2 and F. The gate creates and drops its own database.
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
held2=""
cleanup() {
  exec 3>&- || true
  exec 4>&- || true
  [ -n "$held" ] && kill "$held" 2>/dev/null || true
  [ -n "$held2" ] && kill "$held2" 2>/dev/null || true
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
# A second session that stays connected, on fd 4.
hold2() {
  mkfifo "$WORK/fifo2"
  # Without the first held session's pipe: a copy of its write end here
  # would keep that session from seeing the end of its input.
  "${PSQL[@]}" -d $DB -v ON_ERROR_STOP=1 -At < "$WORK/fifo2" > "$WORK/held2.out" 2>&1 3>&- &
  held2=$!
  exec 4> "$WORK/fifo2"
  echo "$1" >&4
}
say2() { echo "$1" >&4; }
release2() {
  exec 4>&-
  wait "$held2" || fail "the second held session failed: $(cat "$WORK/held2.out")"
  held2=""
  rm -f "$WORK/fifo2"
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
# The term count as the totals give it, beside the dictionary counted.
terms_state="SELECT coalesce(terms::text, 'null') || ' ' || terms_verified || ' of ' || (SELECT count(*) FROM pgrdf._pgrdf_dictionary) FROM pgrdf.graph_totals()"
terms_or_fail() {
  local d s; d=$(q -c "SELECT count(*) FROM pgrdf._pgrdf_dictionary"); s=$(q -c "$terms_state")
  [ "$s" = "$d true of $d" ] || fail "$1: the term count reads '$s', expected '$d true of $d'"
}
# n autocommit put_term calls, IRIs urn:cg:t:<from> .. <from+n-1>.
new_terms() {
  echo "SELECT format('SELECT pgrdf.put_term(%L, 1::smallint);', 'urn:cg:t:' || i) FROM generate_series($1, $1 + $2 - 1) i \gexec" \
    | q -q > /dev/null
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

# ── A2. Terms (#178) ────────────────────────────────────────────────────
# The dictionary's count is kept the same way, under one id for the whole
# database: committed writers, folding, and four sessions at once.
terms_or_fail "at the start"
t0=$(q -c "SELECT pgrdf.count_terms()")
new_terms 1 3000
terms_or_fail "after 3000 committed new terms"
[ "$(q -c "SELECT pgrdf.count_terms()")" = "$((t0 + 3000))" ] || fail "3000 new terms moved the count from $t0 to $(q -c "SELECT pgrdf.count_terms()")"
tpending=$(q -c "SELECT count(*) FROM pgrdf._pgrdf_graph_count_deltas WHERE graph_id = -1")
[ "$tpending" -lt 1000 ] || fail "$tpending deltas of the dictionary are waiting after 3000 new terms: nothing folds them"
new_terms 1 500
[ "$(q -c "SELECT pgrdf.count_terms()")" = "$((t0 + 3000))" ] || fail "500 terms that were already there moved the count"
pids=""
for s in 1 2 3 4; do
  new_terms $((s * 100000 + 1)) 1000 &
  pids="$pids $!"
done
for p in $pids; do wait "$p" || fail "a concurrent term writer failed"; done
terms_or_fail "after four concurrent term writers"
[ "$(q -c "SELECT pgrdf.count_terms()")" = "$((t0 + 7000))" ] || fail "four writers of 1000 terms left $(q -c "SELECT pgrdf.count_terms()"), expected $((t0 + 7000))"
ok "7000 committed new terms from one and from four sessions: the term count is verified and equal to the dictionary ($((t0 + 7000))), $tpending deltas waited after the first 3000; terms that were already there moved nothing"

# ── A3. A graph id that was never registered (#180) ─────────────────────
# Its rows land in the default partition and are counted by scan. It has
# no count row, so it records no delta: nothing would fold or remove one.
writes 918001 1 500
[ "$(q -c "SELECT pgrdf.count_quads(918001)")" = "500" ] || fail "500 writes into an unregistered id stored $(q -c "SELECT pgrdf.count_quads(918001)") rows"
stray=$(q -c "SELECT (SELECT count(*) FROM pgrdf._pgrdf_graph_count_deltas WHERE graph_id = 918001) + (SELECT count(*) FROM pgrdf._pgrdf_graph_counts WHERE graph_id = 918001)")
[ "$stray" = "0" ] || fail "$stray count rows were recorded for an id that was never registered: nothing folds or removes them"
true_or_fail "after writes into an unregistered id (the default graph shares its partition)"
ok "500 committed writes into an id that was never registered are stored, counted by scan, and leave no count row behind"

# ── A4. A graph without its count row (#180) ────────────────────────────
# Every graph is in this state after a restore, until its recount. Its
# writes must not pile up as deltas nobody folds: the first fold opens the
# row, unverified, and the recount then sets it.
gz=$(q -c "SELECT pgrdf.add_graph('urn:cg:norow')")
q -q -c "DELETE FROM pgrdf._pgrdf_graph_counts WHERE graph_id = $gz"
writes "$gz" 1 3000
zpending=$(q -c "SELECT count(*) FROM pgrdf._pgrdf_graph_count_deltas WHERE graph_id = $gz")
[ "$zpending" -lt 1000 ] || fail "$zpending deltas wait for a graph without a count row after 3000 writes: nothing folds them"
state=$(q -c "SELECT asserted || ' ' || counts_verified FROM pgrdf.graph_inventory($gz)")
[ "$state" = "3000 false" ] || fail "a graph without a count row reads '$state', expected '3000 false'"
q -q -c "SELECT pgrdf.recount_graph($gz)" > /dev/null
state=$(q -c "SELECT asserted || ' ' || counts_verified || ' ' || (SELECT count(*) FROM pgrdf._pgrdf_graph_count_deltas WHERE graph_id = $gz) FROM pgrdf.graph_inventory($gz)")
[ "$state" = "3000 true 0" ] || fail "after its recount the graph reads '$state', expected '3000 true 0'"
true_or_fail "after the recount of a graph that had no count row"
ok "a graph without its count row (the state a restore leaves): 3000 committed writes left $zpending deltas, read exact and unverified, and one recount verified it"

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

# C2. A recount beside a fold. An engine write is open (B); another session
#     has folded and holds the graph's base row (C); a recount starts (A),
#     scans without B's row and waits for C. B commits; C folds B's delta
#     into the base and commits. The recount must not then store its older
#     scan over that base: the row would be in the graph and in no count.
gd=$(q -c "SELECT pgrdf.add_graph('urn:cg:c2')")
writes "$gd" 1 5
q -q -c "SELECT pgrdf._fold_counts($gd)" > /dev/null
hold "BEGIN; SELECT pgrdf.put_quad(901, 1, 1, $gd);"
until_true "SELECT count(*) = 1 FROM pg_stat_activity WHERE state = 'idle in transaction' AND datname = '$DB'"
# A fold from a third session brings the base row's bound up to B's
# transaction, so that C's first fold below finds nothing to move and only
# holds the row: a session that had changed the row would make the recount
# wait before it scans, and the order under test would not arise.
q -q -c "SELECT pgrdf._fold_counts($gd)" > /dev/null
hold2 "BEGIN; SELECT pgrdf._fold_counts($gd);"
until_true "SELECT count(*) = 2 FROM pg_stat_activity WHERE state = 'idle in transaction' AND datname = '$DB'"
# Without the two held sessions' pipes: a copy of their write ends in this
# subshell would keep them from seeing the end of their input.
( exec 3>&- 4>&-; q -q -c "SELECT pgrdf.recount_graph($gd)" > "$WORK/recount.out" 2>&1 ) &
recount_pid=$!
until_true "SELECT count(*) > 0 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND datname = '$DB'"
say "COMMIT;"
release
say2 "SELECT pgrdf._fold_counts($gd); COMMIT;"
release2
wait "$recount_pid" || fail "the recount beside a fold failed: $(cat "$WORK/recount.out")"
state=$(q -c "SELECT asserted || ' ' || counts_verified FROM pgrdf.graph_inventory($gd)")
[ "$state" = "6 true" ] || fail "after a recount beside a fold the row reads '$state'; the graph holds $(q -c "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = $gd") rows"
true_or_fail "after a recount beside a fold"
ok "a recount that waited for a fold which took in a write committed meanwhile counts that write: 6, verified"

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
            "SELECT count(*) FROM pgrdf.graph_totals()" "SELECT pgrdf.count_quads($ga)" \
            "SELECT pgrdf.count_terms()" "$feed"; do
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
t_tot=$(under "graph_totals()" "SELECT count(*) FROM pgrdf.graph_totals()" 5)
t_terms=$(under "count_terms()" "SELECT pgrdf.count_terms()" 1)
terms_or_fail "at speed"
awk -v big="${t_inv%% *}" -v small="${small%% *}" 'BEGIN { exit !(big <= 2 * small + 1) }' \
  || fail "graph_inventory() took ${small%% *} ms before the triples and ${t_inv%% *} ms after: it grows with the data"
ok "at $graphs graphs and $total triples (median, p95 ms): graph_inventory() $t_inv, one graph $t_one, feed $t_feed, graph_totals() $t_tot, count_terms() $t_terms; the inventory took ${small%% *} ms before the triples were there; none of the listing calls takes a transaction id"

# ── E2. The feed on a long record ───────────────────────────────────────
# A client that starts with an empty cache wants the cursor and not the
# history, and then polls from the end, many times in one session. Neither
# call may walk the record: PGRDF_GATE_ACTS acts of history, written here
# as rows (the gate is about reading them), spread over the transaction
# ids used so far.
ACTS=${PGRDF_GATE_ACTS:-200000}
q -q -c "INSERT INTO pgrdf._pgrdf_graph_acts (act_id, graph_id, graph_iri, act, at, xact, session_role, acting_role)
         SELECT nextval('pgrdf._pgrdf_graph_act_id_seq'), $ga, 'urn:cg:a', 'load',
                now() - make_interval(secs => $ACTS - i),
                (3 + (pg_current_xact_id()::text::bigint - 4) * i / $ACTS)::text::xid8, 'gate', 'gate'
           FROM generate_series(1, $ACTS) i" -c "VACUUM ANALYZE pgrdf._pgrdf_graph_acts" > /dev/null
q -q -c "SELECT pgrdf.parse_turtle('<urn:cg:tail> <urn:cg:p> <urn:cg:o> .', $ge)" > /dev/null
acts=$(q -c "SELECT count(*) FROM pgrdf._pgrdf_graph_acts")
mid=$(q -c "SELECT percentile_disc(0.5) WITHIN GROUP (ORDER BY xact::text::bigint) FROM pgrdf._pgrdf_graph_acts")
start_row=$(q -c "SELECT count(*) || ' ' || count(act_id) || ' ' || bool_and(complete_below IS NOT NULL) FROM pgrdf.graph_act_feed(max_acts => 1)")
[ "$start_row" = "1 1 true" ] || fail "the cursor-only call returned '$start_row', expected one row carrying one act and the cursor"
f_start=$(under "graph_act_feed(max_acts => 1) on $acts acts" "SELECT count(*) FROM pgrdf.graph_act_feed(max_acts => 1)" 2)
f_end=$(under "graph_act_feed(cursor at the end) on $acts acts" "$feed" 2)
f_graph=$(under "graph_act_feed(cursor at the end, one graph) on $acts acts" "SELECT count(*) FROM pgrdf.graph_act_feed((SELECT max(xact) FROM pgrdf._pgrdf_graph_acts), $ge)" 2)
f_page=$(under "graph_act_feed(cursor in the middle), a page of 1000, on $acts acts" "SELECT count(*) FROM pgrdf.graph_act_feed('$mid'::xid8)" 25)
ok "on a record of $acts acts (median, p95 ms; 51 calls in one session each): the cursor-only call $f_start, a poll from the end $f_end, the same for one graph $f_graph, a page of 1000 from the middle $f_page"

# ── F. Speed under churn ────────────────────────────────────────────────
# (ii) A reader holds its snapshot: nothing a fold deletes can be pruned.
hold "BEGIN ISOLATION LEVEL REPEATABLE READ; SELECT count(*) FROM pgrdf.graph_inventory();"
until_true "SELECT count(*) > 0 FROM pg_stat_activity WHERE state = 'idle in transaction' AND datname = '$DB'"
writes "$ga" 1000001 "$CHURN"
q -q -c "ANALYZE pgrdf._pgrdf_graph_count_deltas, pgrdf._pgrdf_graph_counts" > /dev/null
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
# With statistics that say every waiting delta belongs to one graph, which
# is what autovacuum's analyze leaves here sooner or later: a plan that
# scans the whole delta table once per graph is 50 times slower than the
# index range, and whether analyze has run must not decide the result.
q -q -c "ANALYZE pgrdf._pgrdf_graph_count_deltas, pgrdf._pgrdf_graph_counts" > /dev/null
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
