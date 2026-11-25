#!/usr/bin/env bash
# Upgrade gate: install the PREVIOUS release's SQL, hand the storage tables
# to another role the way a consumer does, run ALTER EXTENSION pgrdf UPDATE
# as a superuser, then prove a role holding only the documented grants can
# create, fill, drop and re-create graphs, existing graphs are untouched, and
# the upgraded database exposes exactly what a fresh install does.
#
# Usage: upgrade-gate.sh <previous-version> <current-version>
#   The current library and every pgrdf--*.sql of the current release must be
#   installed, plus pgrdf--<previous-version>.sql from the previous release.
#   PGRDF_UPGRADE_PSQL is the psql command for a superuser connection
#   (default: psql -X). The gate creates and drops its own databases and roles.
#
# #153: the 0.6.39 upgrade created the graph-id sequence owned by the role
# that ran the upgrade, and a consumer that had already given the storage
# tables to its own role could no longer create any graph. This gate
# reproduces that order of events.
set -euo pipefail

PREV=${1:?previous version}
CUR=${2:?current version}
read -r -a PSQL <<< "${PGRDF_UPGRADE_PSQL:-psql -X}"
UP=pgrdf_upgrade_gate
FRESH=pgrdf_upgrade_fresh
STORE=pgrdf_ug_store
APP=pgrdf_ug_app

q() { local db=$1; shift; "${PSQL[@]}" -d "$db" -v ON_ERROR_STOP=1 -At "$@"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok   $*"; }

held_pid=""
cleanup() {
  exec 3>&- || true
  [ -n "$held_pid" ] && kill "$held_pid" 2>/dev/null || true
  "${PSQL[@]}" -d postgres -q -c "DROP DATABASE IF EXISTS $UP" -c "DROP DATABASE IF EXISTS $FRESH" \
    -c "DROP ROLE IF EXISTS $APP" -c "DROP ROLE IF EXISTS $STORE" >/dev/null 2>&1 || true
}
cleanup
trap cleanup EXIT

q postgres -q -c "CREATE DATABASE $UP" -c "CREATE DATABASE $FRESH" \
  -c "CREATE ROLE $STORE NOLOGIN" -c "CREATE ROLE $APP NOLOGIN"

# 1. The previous release, with graphs that must survive the upgrade: one
#    with asserted and inferred triples, one empty, and one with no recorded
#    owner, as every graph created before 0.6.41 is.
q $UP -q -c "CREATE EXTENSION pgrdf VERSION '$PREV'"
q $UP -q -c "SELECT pgrdf.parse_turtle('<urn:ug:s> <urn:ug:p> \"kept\" . <urn:ug:s> <urn:ug:q> <urn:ug:o> . <urn:ug:Dog> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <urn:ug:Animal> . <urn:ug:rex> a <urn:ug:Dog> .', pgrdf.add_graph('urn:ug:kept'))" >/dev/null
q $UP -q -c "SELECT pgrdf.materialize(pgrdf.graph_id('urn:ug:kept'), 'rdfs')" >/dev/null
q $UP -q -c "SELECT pgrdf.add_graph('urn:ug:empty')" >/dev/null
q $UP -q -c "SELECT pgrdf.parse_turtle('<urn:ug:s> <urn:ug:p> \"old\" .', pgrdf.add_graph('urn:ug:old'))" >/dev/null
q $UP -q -c "UPDATE pgrdf._pgrdf_graphs SET owner = NULL WHERE iri = 'urn:ug:old'"
kept_before=$(q $UP -c "SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:ug:kept'))")
kept_structure=$(q $UP -c "SELECT pgrdf.structural_digest(pgrdf.graph_id('urn:ug:kept'))")
scanned="SELECT string_agg(graph_id || ':' || a || '+' || i, ' ' ORDER BY graph_id)
           FROM (SELECT g.graph_id,
                        (SELECT count(*) FROM pgrdf._pgrdf_quads q WHERE q.graph_id = g.graph_id AND NOT q.is_inferred) AS a,
                        (SELECT count(*) FROM pgrdf._pgrdf_quads q WHERE q.graph_id = g.graph_id AND q.is_inferred) AS i
                   FROM pgrdf._pgrdf_graphs g) t"
inferred_before=$(q $UP -c "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE is_inferred")
[ "$inferred_before" -gt 0 ] || fail "the fixture derived no inferred triple"
ok "installed $PREV; graphs urn:ug:kept ($inferred_before inferred), urn:ug:empty, urn:ug:old (no recorded owner)"

# 2. A consumer hands every storage table to its own role (ALTER TABLE moves
#    a column-linked sequence with its table).
q $UP -q -c "GRANT USAGE, CREATE ON SCHEMA pgrdf TO $STORE" -c "
DO \$\$DECLARE r record; BEGIN
  FOR r IN SELECT relname FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relkind IN ('r', 'p')
  LOOP EXECUTE format('ALTER TABLE pgrdf.%I OWNER TO $STORE', r.relname); END LOOP;
  FOR r IN SELECT relname FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relkind = 'S'
  LOOP EXECUTE format('ALTER SEQUENCE pgrdf.%I OWNER TO $STORE', r.relname); END LOOP;
END\$\$"
q $UP -q -c "GRANT USAGE ON SCHEMA pgrdf TO $APP" \
  -c "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pgrdf TO $APP" \
  -c "GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pgrdf TO $APP"
ok "storage tables owned by $STORE; $APP holds the documented grants"

# 3. Library already new, SQL still old. Nothing refuses: a write lands and
#    is simply in no stored count yet, and the inventory answers through its
#    old signature with exact counts. One session stays connected across the
#    upgrade: it wrote before, and it must be counted when it writes after.
fifo=$(mktemp -u "${TMPDIR:-/tmp}/pgrdf-ug.XXXXXX"); mkfifo "$fifo"
held_out=$(mktemp "${TMPDIR:-/tmp}/pgrdf-ug-out.XXXXXX")
"${PSQL[@]}" -d $UP -v ON_ERROR_STOP=1 -At < "$fifo" > "$held_out" 2>&1 &
held_pid=$!
exec 3> "$fifo"
echo "SELECT pgrdf.parse_turtle('<urn:ug:w> <urn:ug:p> \"before the upgrade\" .', pgrdf.graph_id('urn:ug:kept'));" >&3
for _ in $(seq 1 50); do
  [ "$(q $UP -c "SELECT count(*) FROM pgrdf._pgrdf_quads q JOIN pgrdf._pgrdf_dictionary d ON d.id = q.object_id WHERE d.lexical_value = 'before the upgrade'")" = "1" ] && break
  sleep 0.2
done
old_inv=$(q $UP -c "SELECT string_agg(graph_id || ':' || asserted || '+' || inferred, ' ' ORDER BY graph_id) FROM pgrdf.graph_inventory()")
[ "$old_inv" = "$(q $UP -c "$scanned")" ] || fail "before the upgrade the inventory said '$old_inv', a scan says '$(q $UP -c "$scanned")'"
ok "before the upgrade: a write landed, and graph_inventory() answers with exact counts ($old_inv)"

# 4. The upgrade, as a superuser. It says how many graphs have no owner.
notices=$("${PSQL[@]}" -d $UP -v ON_ERROR_STOP=1 -q -c "ALTER EXTENSION pgrdf UPDATE" 2>&1)
echo "$notices" | grep -q "1 graph(s) have no recorded owner" \
  || fail "the upgrade did not report the graph with no recorded owner; it said: $notices"
got=$(q $UP -c "SELECT extversion FROM pg_extension WHERE extname = 'pgrdf'")
[ "$got" = "$CUR" ] || fail "extversion after update is $got, expected $CUR"
blocking=$(q $UP -c "SELECT count(*) FROM pgrdf.ownership_drift() WHERE blocking")
[ "$blocking" = "0" ] || fail "ownership_drift() reports $blocking blocking relation(s) after update: $(q $UP -c "SELECT string_agg(relname, ', ') FROM pgrdf.ownership_drift() WHERE blocking")"
count_owners=$(q $UP -c "SELECT string_agg(DISTINCT relowner::regrole::text, ',') FROM pg_class WHERE relnamespace = 'pgrdf'::regnamespace AND relname LIKE '\\_pgrdf\\_graph\\_count%' AND relkind IN ('r', 'S')")
[ "$count_owners" = "$STORE" ] || fail "the count tables are owned by '$count_owners', expected $STORE"
ok "updated to $CUR; the count tables are owned by $STORE; no blocking drift"

# 4b. Every graph was counted by the upgrade: verified, and equal to a scan.
truth="SELECT count(*) FROM pgrdf.graph_inventory() i
        WHERE NOT i.counts_verified
           OR i.asserted <> (SELECT count(*) FROM pgrdf._pgrdf_quads q
                              WHERE q.graph_id = i.graph_id AND NOT q.is_inferred)
           OR i.inferred <> (SELECT count(*) FROM pgrdf._pgrdf_quads q
                              WHERE q.graph_id = i.graph_id AND q.is_inferred)"
wrong=$(q $UP -c "$truth")
[ "$wrong" = "0" ] || fail "$wrong graph(s) are unverified or miscounted after the upgrade: $(q $UP -c "SELECT string_agg(graph_id || ':' || asserted || '+' || inferred || ':' || counts_verified, ' ') FROM pgrdf.graph_inventory()")"
# And the dictionary (#178): the stored number of terms is verified and
# equal to the table, which held terms before the upgrade.
terms="SELECT coalesce(terms::text, 'null') || ' ' || coalesce(terms_verified::text, 'null') || ' of ' || (SELECT count(*) FROM pgrdf._pgrdf_dictionary) FROM pgrdf.graph_totals()"
dict=$(q $UP -c "SELECT count(*) FROM pgrdf._pgrdf_dictionary")
[ "$dict" -gt 0 ] || fail "the fixture left the dictionary empty"
[ "$(q $UP -c "$terms")" = "$dict true of $dict" ] || fail "after the upgrade the term count reads '$(q $UP -c "$terms")', expected '$dict true of $dict'"
markers=$(q $UP -c "SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid WHERE c.relname = '_pgrdf_quads_g' || pgrdf.graph_id('urn:ug:kept') AND t.tgname LIKE 'pgrdf_count_bypass%'")
[ "$markers" = "2" ] || fail "the partition of urn:ug:kept carries $markers count markers, expected 2"
# The session that was connected before the upgrade writes again: its write
# must be counted. A backend that remembered "no count tables" would leave
# the graph verified and one row short.
echo "SELECT pgrdf.parse_turtle('<urn:ug:w> <urn:ug:p> \"after the upgrade\" .', pgrdf.graph_id('urn:ug:kept'));" >&3
exec 3>&-
wait $held_pid || fail "the session held across the upgrade failed: $(cat "$held_out")"
rm -f "$fifo" "$held_out"
wrong=$(q $UP -c "$truth")
[ "$wrong" = "0" ] || fail "a session connected before the upgrade wrote an uncounted row: $(q $UP -c "SELECT string_agg(graph_id || ':' || asserted || '+' || inferred || ':' || counts_verified, ' ') FROM pgrdf.graph_inventory()")"
# That write brought new terms with it, and they are counted too.
now=$(q $UP -c "SELECT count(*) FROM pgrdf._pgrdf_dictionary")
[ "$now" -gt "$dict" ] || fail "the write after the upgrade added no term"
[ "$(q $UP -c "$terms")" = "$now true of $now" ] || fail "a session connected before the upgrade added uncounted terms: '$(q $UP -c "$terms")', expected '$now true of $now'"
ok "every graph is verified and equal to a scan, and so is the term count ($dict at the upgrade, $now now), also after a write from a session that was connected before the upgrade"

# 5. The app role creates by IRI and by explicit id, fills, drops, and a
#    dropped id is never handed out again.
as_app() { q $UP -c "SET ROLE $APP" -c "$1" | tail -n 1; }
g1=$(as_app "SELECT pgrdf.add_graph('urn:ug:after')")
n=$(as_app "SELECT pgrdf.parse_turtle('<urn:ug:a> <urn:ug:p> <urn:ug:b> .', $g1)")
[ "$n" = "1" ] || fail "parse_turtle into the new graph loaded $n triples"
high=$((g1 + 100))
as_app "SELECT pgrdf.add_graph($high::bigint, 'urn:ug:explicit')" >/dev/null
as_app "SELECT pgrdf.drop_graph($high::bigint, true)" >/dev/null
g3=$(as_app "SELECT pgrdf.add_graph('urn:ug:after2')")
[ "$g3" -gt "$high" ] || fail "a dropped id came back: $g3 after dropping $high"
ok "$APP created $g1 by IRI and $high explicitly, dropped $high, next id $g3"

# 5b. The listing follows the role. The app role sees the graphs it
#     created and nothing else; its totals say so; a graph that is not its
#     own refuses 42501 and one that does not exist returns no row.
listed=$(as_app "SELECT string_agg(iri, ' ' ORDER BY iri) FROM pgrdf.graph_inventory()")
[ "$listed" = "urn:ug:after urn:ug:after2" ] || fail "$APP is listed '$listed', expected its own two graphs"
totals=$(as_app "SELECT scope || ' ' || graphs_listed || ' of ' || graphs_total || ' ' || asserted || ' ' || coalesce(terms::text, 'null') || ' ' || coalesce(terms_verified::text, 'null') FROM pgrdf.graph_totals()")
all=$(q $UP -c "SELECT count(*) FROM pgrdf._pgrdf_graphs")
[ "$totals" = "own 2 of $all 1 null null" ] || fail "$APP totals are '$totals', expected 'own 2 of $all 1 null null'"
admin=$(q $UP -c "SELECT scope || ' ' || (graphs_listed = graphs_total) || ' ' || unverified_graphs || ' ' || (terms = pgrdf.count_terms()) || ' ' || terms_verified FROM pgrdf.graph_totals()")
[ "$admin" = "all true 0 true true" ] || fail "the superuser's totals are '$admin', expected 'all true 0 true true'"
# The app role added terms of its own on the way, with no privilege on the
# count tables; recounting the dictionary is not its act.
norecount=$("${PSQL[@]}" -d $UP -At -c "\\set VERBOSITY verbose" -c "SET ROLE $APP" \
  -c "SELECT count(*) FROM pgrdf.recount_terms()" 2>&1 || true)
echo "$norecount" | grep -q "42501" || fail "$APP running recount_terms() should refuse 42501; got: $norecount"
foreign=$("${PSQL[@]}" -d $UP -At -c "\\set VERBOSITY verbose" -c "SET ROLE $APP" \
  -c "SELECT count(*) FROM pgrdf.graph_inventory('urn:ug:kept')" 2>&1 || true)
echo "$foreign" | grep -q "42501" || fail "$APP asking for urn:ug:kept should refuse 42501; got: $foreign"
none=$(as_app "SELECT count(*) FROM pgrdf.graph_inventory('urn:ug:no-such-graph')")
[ "$none" = "0" ] || fail "an unknown graph returned $none row(s)"
ok "$APP is listed its own graphs ($totals); urn:ug:kept refuses 42501; an unknown graph returns no row"

# 5c. The app role's writes are counted without any privilege on the count
#     tables, and a write in plain SQL is seen for what it is.
privs=$(q $UP -c "SELECT string_agg(privilege_type, ',' ORDER BY privilege_type) FROM information_schema.role_table_grants WHERE grantee = 'PUBLIC' AND table_schema = 'pgrdf' AND table_name LIKE '\\_pgrdf\\_graph\\_count%'")
[ "$privs" = "INSERT" ] || fail "PUBLIC holds '$privs' on the count tables, expected INSERT on the delta table only"
forged=$("${PSQL[@]}" -d $UP -At -c "\\set VERBOSITY verbose" -c "SET ROLE $APP" \
  -c "INSERT INTO pgrdf._pgrdf_graph_count_deltas (graph_id, d_asserted, d_inferred) VALUES ($g1, 1000, 0)" 2>&1 || true)
echo "$forged" | grep -q "42501" || fail "a direct INSERT into the delta table should refuse 42501; got: $forged"
as_app "INSERT INTO pgrdf._pgrdf_quads_g$g1 (subject_id, predicate_id, object_id, graph_id) SELECT subject_id, object_id, predicate_id, graph_id FROM pgrdf._pgrdf_quads_g$g1" >/dev/null
state=$(as_app "SELECT asserted || ' ' || counts_verified FROM pgrdf.graph_inventory($g1)")
[ "$state" = "2 false" ] || fail "after a direct write, graph $g1 reads '$state', expected '2 false'"
cured=$(as_app "SELECT asserted || ' ' || was_asserted || ' ' || was_verified FROM pgrdf.recount_graph($g1)")
[ "$cured" = "2 1 false" ] || fail "recount_graph($g1) returned '$cured', expected '2 1 false'"
state=$(as_app "SELECT asserted || ' ' || counts_verified FROM pgrdf.graph_inventory($g1)")
[ "$state" = "2 true" ] || fail "after the recount, graph $g1 reads '$state', expected '2 true'"
wrong=$(q $UP -c "$truth")
[ "$wrong" = "0" ] || fail "$wrong graph(s) are unverified or miscounted at the end of the gate"
ok "PUBLIC holds INSERT on the delta table only; a forged delta refuses 42501; a direct write reads unverified and exact, and recount_graph cures it"

# 6. The graph from before the upgrade kept its triples: the two rows the
#    gate added are the only difference, and its digest is what this
#    library computed for it before the upgrade plus those rows.
kept_rows=$(q $UP -c "SELECT asserted || '+' || inferred FROM pgrdf.graph_inventory('urn:ug:kept')")
[ "$kept_rows" = "6+$inferred_before" ] || fail "urn:ug:kept holds $kept_rows, expected 6+$inferred_before"
q $UP -q -c "SELECT * FROM pgrdf.sparql('DELETE WHERE { GRAPH <urn:ug:kept> { <urn:ug:w> ?p ?o } }')" >/dev/null
kept_after=$(q $UP -c "SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:ug:kept'))")
[ "$kept_before" = "$kept_after" ] || fail "urn:ug:kept: graph_digest is $kept_after, it was $kept_before before the upgrade"
structure_after=$(q $UP -c "SELECT pgrdf.structural_digest(pgrdf.graph_id('urn:ug:kept'))")
[ "$kept_structure" = "$structure_after" ] || fail "urn:ug:kept: structural_digest is $structure_after, it was $kept_structure before the upgrade"
wrong=$(q $UP -c "$truth")
[ "$wrong" = "0" ] || fail "$wrong graph(s) are unverified or miscounted after the SPARQL delete"
ok "urn:ug:kept: graph_digest and structural_digest are what they were before the upgrade"

# 7. The upgraded database exposes exactly what a fresh install does.
q $FRESH -q -c "CREATE EXTENSION pgrdf"
sig="SELECT p.oid::regprocedure::text || ' -> ' || pg_get_function_result(p.oid)
       FROM pg_proc p WHERE p.pronamespace = 'pgrdf'::regnamespace ORDER BY 1"
rels="SELECT relname || ' ' || relkind::text FROM pg_class
       WHERE relnamespace = 'pgrdf'::regnamespace AND relkind <> 'i'
         AND relname !~ '^_pgrdf_quads_g[0-9]+$' ORDER BY 1"
# Function privileges, with role names normalised (the two databases have
# different owners after step 2), and what pg_dump is told to carry.
acl="SELECT p.oid::regprocedure::text || ' ' || CASE WHEN p.proacl IS NULL THEN 'default'
              WHEN EXISTS (SELECT 1 FROM aclexplode(p.proacl) a WHERE a.grantee = 0) THEN 'public'
              ELSE 'not public' END
       FROM pg_proc p WHERE p.pronamespace = 'pgrdf'::regnamespace ORDER BY 1"
dumpcfg="SELECT c.relname || ' ' || quote_literal(e.cond)
           FROM pg_extension x, unnest(x.extconfig, x.extcondition) AS e(rel, cond)
           JOIN pg_class c ON c.oid = e.rel
          WHERE x.extname = 'pgrdf' ORDER BY 1"
# Columns, indexes and triggers of the extension's own relations. Graph
# partitions are left out: the two databases hold different graphs.
shape="SELECT 'column ' || c.relname || '.' || a.attname || ' ' || format_type(a.atttypid, a.atttypmod)
              || CASE WHEN a.attnotnull THEN ' not null' ELSE '' END
         FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid
        WHERE c.relnamespace = 'pgrdf'::regnamespace AND c.relkind IN ('r', 'p')
          AND a.attnum > 0 AND NOT a.attisdropped AND c.relname !~ '^_pgrdf_quads_g[0-9]+\$'
       UNION ALL
       SELECT 'index ' || indexdef FROM pg_indexes
        WHERE schemaname = 'pgrdf' AND tablename !~ '^_pgrdf_quads_g[0-9]+\$'
       UNION ALL
       SELECT 'trigger ' || c.relname || ' ' || pg_get_triggerdef(t.oid)
         FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid
        WHERE c.relnamespace = 'pgrdf'::regnamespace AND NOT t.tgisinternal
          AND c.relname !~ '^_pgrdf_quads_g[0-9]+\$'
        ORDER BY 1"
# Captured first, so a query that fails stops the gate instead of comparing
# two empty outputs.
fresh_sig=$(q $FRESH -c "$sig"); up_sig=$(q $UP -c "$sig")
fresh_rels=$(q $FRESH -c "$rels"); up_rels=$(q $UP -c "$rels")
fresh_acl=$(q $FRESH -c "$acl"); up_acl=$(q $UP -c "$acl")
fresh_cfg=$(q $FRESH -c "$dumpcfg"); up_cfg=$(q $UP -c "$dumpcfg")
fresh_shape=$(q $FRESH -c "$shape"); up_shape=$(q $UP -c "$shape")
[ -n "$fresh_sig" ] && [ -n "$fresh_rels" ] && [ -n "$fresh_acl" ] && [ -n "$fresh_cfg" ] \
  || fail "fresh install lists no functions, relations, privileges or dump registrations"
diff <(echo "$fresh_sig") <(echo "$up_sig") || fail "functions differ between a fresh install and the upgrade"
diff <(echo "$fresh_rels") <(echo "$up_rels") || fail "relations differ between a fresh install and the upgrade"
diff <(echo "$fresh_acl") <(echo "$up_acl") || fail "function privileges differ between a fresh install and the upgrade"
diff <(echo "$fresh_cfg") <(echo "$up_cfg") || fail "pg_dump registrations differ between a fresh install and the upgrade"
[ -n "$fresh_shape" ] || fail "fresh install lists no columns"
diff <(echo "$fresh_shape") <(echo "$up_shape") || fail "columns, indexes or triggers differ between a fresh install and the upgrade"
ok "functions ($(echo "$up_sig" | wc -l | tr -d ' ')), relations ($(echo "$up_rels" | wc -l | tr -d ' ')), columns, indexes and triggers ($(echo "$up_shape" | wc -l | tr -d ' ')), function privileges and pg_dump registrations ($(echo "$up_cfg" | wc -l | tr -d ' ')) match a fresh install"

# 8. The hand-over on the current release, in the fresh database: every
#    storage table goes to another role, the report's cures are run, and
#    the app role then creates and fills a graph. The bypass log's sequence
#    moves with its table; nothing about the counts is left behind.
q $FRESH -q -c "GRANT USAGE, CREATE ON SCHEMA pgrdf TO $STORE" -c "
DO \$\$DECLARE r record; BEGIN
  FOR r IN SELECT relname FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relkind IN ('r', 'p')
  LOOP EXECUTE format('ALTER TABLE pgrdf.%I OWNER TO $STORE', r.relname); END LOOP;
  FOR r IN SELECT cure FROM pgrdf.ownership_drift() WHERE blocking
  LOOP EXECUTE r.cure; END LOOP;
END\$\$"
blocking=$(q $FRESH -c "SELECT count(*) FROM pgrdf.ownership_drift() WHERE blocking")
[ "$blocking" = "0" ] || fail "after the hand-over and its cures, ownership_drift() still reports: $(q $FRESH -c "SELECT string_agg(relname, ', ') FROM pgrdf.ownership_drift() WHERE blocking")"
q $FRESH -q -c "GRANT USAGE ON SCHEMA pgrdf TO $APP" \
  -c "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pgrdf TO $APP" \
  -c "GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pgrdf TO $APP"
fg=$(q $FRESH -c "SET ROLE $APP" -c "SELECT pgrdf.add_graph('urn:ug:handed-over')" | tail -n 1)
q $FRESH -q -c "SET ROLE $APP" -c "SELECT pgrdf.parse_turtle('<urn:ug:a> <urn:ug:p> <urn:ug:b> .', $fg)" >/dev/null
q $FRESH -q -c "INSERT INTO pgrdf._pgrdf_quads_g$fg (subject_id, predicate_id, object_id, graph_id) VALUES (5, 6, 7, $fg)"
state=$(q $FRESH -c "SELECT asserted || ' ' || counts_verified FROM pgrdf.graph_inventory($fg)")
[ "$state" = "2 false" ] || fail "after the hand-over, a direct write reads '$state', expected '2 false'"
q $FRESH -q -c "SET ROLE $APP" -c "SELECT pgrdf.recount_graph($fg)" >/dev/null
state=$(q $FRESH -c "SELECT asserted || ' ' || counts_verified FROM pgrdf.graph_inventory($fg)")
[ "$state" = "2 true" ] || fail "after the hand-over and a recount the row reads '$state', expected '2 true'"
# The dictionary's count went with the tables: the app role's terms are
# counted, a term written in plain SQL is seen, and the storage owner (not
# the app role) recounts.
fdict=$(q $FRESH -c "SELECT count(*) FROM pgrdf._pgrdf_dictionary")
[ "$(q $FRESH -c "$terms")" = "$fdict true of $fdict" ] || fail "after the hand-over the term count reads '$(q $FRESH -c "$terms")', expected '$fdict true of $fdict'"
q $FRESH -q -c "INSERT INTO pgrdf._pgrdf_dictionary (term_type, lexical_value) VALUES (1, 'urn:ug:direct-term')"
fdict=$((fdict + 1))
[ "$(q $FRESH -c "$terms")" = "null false of $fdict" ] || fail "after a direct term the totals read '$(q $FRESH -c "$terms")', expected 'null false of $fdict'"
[ "$(q $FRESH -c "SET ROLE $APP" -c "SELECT pgrdf.count_terms()" | tail -n 1)" = "$fdict" ] || fail "count_terms() as $APP is not $fdict"
cured=$(q $FRESH -c "SET ROLE $STORE" -c "SELECT terms || ' ' || was_terms || ' ' || was_verified FROM pgrdf.recount_terms()" | tail -n 1)
[ "$cured" = "$fdict $((fdict - 1)) false" ] || fail "recount_terms() as $STORE returned '$cured', expected '$fdict $((fdict - 1)) false'"
[ "$(q $FRESH -c "$terms")" = "$fdict true of $fdict" ] || fail "after recount_terms() the totals read '$(q $FRESH -c "$terms")'"
ok "storage handed to $STORE on $CUR with the report's cures: $APP creates, fills and recounts a graph; a direct write is still seen; the term count follows ($fdict terms, recounted by $STORE)"

echo "upgrade gate: $PREV -> $CUR passed"
