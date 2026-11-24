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

cleanup() {
  "${PSQL[@]}" -d postgres -q -c "DROP DATABASE IF EXISTS $UP" -c "DROP DATABASE IF EXISTS $FRESH" \
    -c "DROP ROLE IF EXISTS $APP" -c "DROP ROLE IF EXISTS $STORE" >/dev/null 2>&1 || true
}
cleanup
trap cleanup EXIT

q postgres -q -c "CREATE DATABASE $UP" -c "CREATE DATABASE $FRESH" \
  -c "CREATE ROLE $STORE NOLOGIN" -c "CREATE ROLE $APP NOLOGIN"

# 1. The previous release, with a graph that must survive the upgrade.
q $UP -q -c "CREATE EXTENSION pgrdf VERSION '$PREV'"
q $UP -q -c "SELECT pgrdf.parse_turtle('<urn:ug:s> <urn:ug:p> \"kept\" . <urn:ug:s> <urn:ug:q> <urn:ug:o> .', pgrdf.add_graph('urn:ug:kept'))" >/dev/null
# The digest of that graph as the previous release computes it, recorded
# from released 0.6.40. The library under test cannot compute a digest on
# the old SQL (it sorts in SQL by a function the upgrade adds), so "the
# upgrade leaves the digest unchanged" is checked against this constant.
kept_before=0fdeb2e944b3e46c1f335b1381a5e862b9e287782aa61d0178038ccbbcdaa28f
kept_structure=300306054a8ce699835c2a6b957f8fd080d68f02e0a2301a429dfde1c56d5815
# A graph in the state the previous release left when a source stated one
# triple twice: the row is stored twice. (Written with plain SQL: the
# library under test is already the new one and would not store it.) The
# upgrade must leave the graph exactly as it is.
q $UP -q -c "SELECT pgrdf.parse_turtle('<urn:ug:r> <urn:ug:p> \"twice\" . <urn:ug:r> <urn:ug:q> <urn:ug:o> .', pgrdf.add_graph('urn:ug:repeats'))" >/dev/null
# The previous release's library built no per-graph unique index; the one
# under test does, even on the old SQL. Remove them so both graphs are in
# the state that release left, then store the repeated row.
q $UP -q -c "
DO \$\$DECLARE r record; BEGIN
  FOR r IN SELECT indexname FROM pg_indexes WHERE schemaname = 'pgrdf' AND indexname ~ '_spo_uniq\$'
  LOOP EXECUTE format('DROP INDEX pgrdf.%I', r.indexname); END LOOP;
END\$\$"
q $UP -q -c "INSERT INTO pgrdf._pgrdf_quads SELECT q.* FROM pgrdf._pgrdf_quads q JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id WHERE q.graph_id = pgrdf.graph_id('urn:ug:repeats') AND o.lexical_value = 'twice'"
repeats_before=$(q $UP -c "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = pgrdf.graph_id('urn:ug:repeats')")
[ "$repeats_before" = "3" ] || fail "expected 3 rows for 2 triples in urn:ug:repeats, got $repeats_before"
ok "installed $PREV; graph urn:ug:kept; urn:ug:repeats holds $repeats_before rows for 2 triples"

# 2. A consumer hands every storage table to its own role (ALTER TABLE moves
#    a column-linked sequence with its table). The graph-id sequence is left
#    behind, as it was for a consumer that did this before the release that
#    added it.
q $UP -q -c "GRANT USAGE, CREATE ON SCHEMA pgrdf TO $STORE" -c "
DO \$\$DECLARE r record; BEGIN
  FOR r IN SELECT relname FROM pg_class
            WHERE relnamespace = 'pgrdf'::regnamespace AND relkind IN ('r', 'p')
  LOOP EXECUTE format('ALTER TABLE pgrdf.%I OWNER TO $STORE', r.relname); END LOOP;
END\$\$"
q $UP -q -c "GRANT USAGE ON SCHEMA pgrdf TO $APP" \
  -c "GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pgrdf TO $APP" \
  -c "GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pgrdf TO $APP"
ok "storage tables owned by $STORE; $APP holds the documented grants"

# 3. Library already new, SQL still old: graph creation must refuse with
#    the cure, never a bare permission error.
before=$("${PSQL[@]}" -d $UP -At -c "\\set VERBOSITY verbose" -c "SET ROLE $APP" \
  -c "SELECT pgrdf.add_graph('urn:ug:before')" 2>&1 || true)
echo "$before" | grep -q "55000" || fail "before the upgrade, expected 55000; got: $before"
echo "$before" | grep -q "HINT:  ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO $STORE" \
  || fail "before the upgrade, expected the cure in HINT; got: $before"
# The same state, for the reads that need the new SQL: a refusal naming the
# upgrade, never "function does not exist".
digest=$("${PSQL[@]}" -d $UP -At -c "\\set VERBOSITY verbose" \
  -c "SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:ug:kept'))" 2>&1 || true)
echo "$digest" | grep -q "55000" && echo "$digest" | grep -q "HINT:  ALTER EXTENSION pgrdf UPDATE" \
  || fail "before the upgrade, graph_digest should refuse 55000 with the upgrade as HINT; got: $digest"
ok "before the upgrade: add_graph refused 55000 with HINT ALTER SEQUENCE … OWNER TO $STORE; graph_digest refused 55000 with HINT ALTER EXTENSION pgrdf UPDATE"

# 4. The upgrade, as a superuser. It names the graph it could not constrain.
notices=$("${PSQL[@]}" -d $UP -v ON_ERROR_STOP=1 -q -c "ALTER EXTENSION pgrdf UPDATE" 2>&1)
rg=$(q $UP -c "SELECT pgrdf.graph_id('urn:ug:repeats')")
echo "$notices" | grep -q "_pgrdf_quads_g${rg} holds repeated triples" \
  || fail "the upgrade did not name the graph holding repeats; it said: $notices"
got=$(q $UP -c "SELECT extversion FROM pg_extension WHERE extname = 'pgrdf'")
[ "$got" = "$CUR" ] || fail "extversion after update is $got, expected $CUR"
seq_owner=$(q $UP -c "SELECT relowner::regrole FROM pg_class WHERE oid = 'pgrdf._pgrdf_graph_id_seq'::regclass")
[ "$seq_owner" = "$STORE" ] || fail "graph-id sequence owned by $seq_owner after update, expected $STORE"
blocking=$(q $UP -c "SELECT count(*) FROM pgrdf.ownership_drift() WHERE blocking")
[ "$blocking" = "0" ] || fail "ownership_drift() reports $blocking blocking relation(s) after update: $(q $UP -c "SELECT string_agg(relname, ', ') FROM pgrdf.ownership_drift() WHERE blocking")"
ok "updated to $CUR; graph-id sequence owned by $STORE; no blocking drift"

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

# 5b. Ownership and the act record. The graph from before the upgrade has
#     no recorded owner (never a guess); a graph created after it belongs to
#     the role that created it, not to the storage owner; the act record is
#     owned by the storage owner and carries the app role's acts.
kept_owner=$(q $UP -c "SELECT coalesce(owner, '<not recorded>') FROM pgrdf._pgrdf_graphs WHERE iri = 'urn:ug:kept'")
[ "$kept_owner" = "<not recorded>" ] || fail "the pre-upgrade graph got an owner: $kept_owner"
new_owner=$(q $UP -c "SELECT owner FROM pgrdf._pgrdf_graphs WHERE graph_id = $g1")
[ "$new_owner" = "$APP" ] || fail "graph $g1 is owned by '$new_owner', expected $APP"
acts_owner=$(q $UP -c "SELECT relowner::regrole FROM pg_class WHERE oid = 'pgrdf._pgrdf_graph_acts'::regclass")
[ "$acts_owner" = "$STORE" ] || fail "the act record is owned by $acts_owner, expected $STORE"
acts=$(q $UP -c "SELECT string_agg(act, ',' ORDER BY act_id) FROM pgrdf.graph_acts($g1) WHERE acting_role = '$APP'")
[ "$acts" = "create,load" ] || fail "acts of graph $g1 by $APP: '$acts', expected create,load"
ok "urn:ug:kept has no recorded owner; graph $g1 is owned by $APP with acts $acts; the act record is owned by $STORE"

# 5c. Set semantics. The upgrade rewrote nothing: the graph holding repeats
#     has the same rows, is reported, gains no more from a reload, and
#     dedup_graph cures it. The clean graph got its unique index.
repeats_after=$(q $UP -c "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = $rg")
[ "$repeats_after" = "$repeats_before" ] || fail "the upgrade changed urn:ug:repeats: $repeats_before -> $repeats_after rows"
report=$(q $UP -c "SELECT concat_ws('/', rows, distinct_rows, surplus, set_enforced) FROM pgrdf.duplicate_report() WHERE graph_id = $rg")
[ "$report" = "3/2/1/f" ] || fail "duplicate_report() on urn:ug:repeats: '$report', expected 3/2/1/f"
others=$(q $UP -c "SELECT count(*) FROM pgrdf.duplicate_report() WHERE graph_id IS DISTINCT FROM $rg")
[ "$others" = "0" ] || fail "duplicate_report() lists $others other row(s): $(q $UP -c "SELECT string_agg(coalesce(iri, 'dictionary'), ', ') FROM pgrdf.duplicate_report() WHERE graph_id IS DISTINCT FROM $rg")"
as_app "SELECT pgrdf.parse_turtle('<urn:ug:r> <urn:ug:p> \"twice\" .', $rg)" >/dev/null
[ "$(q $UP -c "SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = $rg")" = "3" ] || fail "a graph holding repeats gained another on reload"
removed=$(as_app "SELECT pgrdf.dedup_graph($rg)")
[ "$removed" = "1" ] || fail "dedup_graph removed $removed rows, expected 1"
[ "$(q $UP -c "SELECT count(*) FROM pgrdf.duplicate_report()")" = "0" ] || fail "duplicate_report() is not empty after the cure"
dup=$("${PSQL[@]}" -d $UP -At -c "\\set VERBOSITY verbose" \
  -c "INSERT INTO pgrdf._pgrdf_quads SELECT * FROM pgrdf._pgrdf_quads WHERE graph_id = pgrdf.graph_id('urn:ug:kept') LIMIT 1" 2>&1 || true)
echo "$dup" | grep -q "23505" || fail "the upgraded clean graph accepts a repeated row: $dup"
ok "urn:ug:repeats untouched by the upgrade ($report), no further repeat on reload, cured by dedup_graph (removed $removed); urn:ug:kept refuses a repeat 23505"

# 6. The graph from before the upgrade is untouched.
kept_after=$(q $UP -c "SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:ug:kept'))")
[ "$kept_before" = "$kept_after" ] || fail "urn:ug:kept: graph_digest is $kept_after, the previous release computed $kept_before"
structure_after=$(q $UP -c "SELECT pgrdf.structural_digest(pgrdf.graph_id('urn:ug:kept'))")
[ "$kept_structure" = "$structure_after" ] || fail "urn:ug:kept: structural_digest is $structure_after, the previous release computed $kept_structure"
ok "urn:ug:kept: graph_digest and structural_digest equal what the previous release computed"

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

echo "upgrade gate: $PREV -> $CUR passed"
