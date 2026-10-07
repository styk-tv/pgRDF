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
kept_before=$(q $UP -c "SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:ug:kept'))")
ok "installed $PREV; graph urn:ug:kept digest ${kept_before:0:16}…"

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
ok "before the upgrade: refused 55000 with HINT ALTER SEQUENCE … OWNER TO $STORE"

# 4. The upgrade, as a superuser.
q $UP -q -c "ALTER EXTENSION pgrdf UPDATE"
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

# 6. The graph from before the upgrade is untouched.
kept_after=$(q $UP -c "SELECT pgrdf.graph_digest(pgrdf.graph_id('urn:ug:kept'))")
[ "$kept_before" = "$kept_after" ] || fail "urn:ug:kept changed across the upgrade"
ok "urn:ug:kept digest unchanged"

# 7. The upgraded database exposes exactly what a fresh install does.
q $FRESH -q -c "CREATE EXTENSION pgrdf"
sig="SELECT p.oid::regprocedure::text || ' -> ' || pg_get_function_result(p.oid)
       FROM pg_proc p WHERE p.pronamespace = 'pgrdf'::regnamespace ORDER BY 1"
rels="SELECT relname || ' ' || relkind::text FROM pg_class
       WHERE relnamespace = 'pgrdf'::regnamespace AND relkind <> 'i'
         AND relname !~ '^_pgrdf_quads_g[0-9]+$' ORDER BY 1"
# Captured first, so a query that fails stops the gate instead of comparing
# two empty outputs.
fresh_sig=$(q $FRESH -c "$sig"); up_sig=$(q $UP -c "$sig")
fresh_rels=$(q $FRESH -c "$rels"); up_rels=$(q $UP -c "$rels")
[ -n "$fresh_sig" ] && [ -n "$fresh_rels" ] || fail "fresh install lists no functions or relations"
diff <(echo "$fresh_sig") <(echo "$up_sig") || fail "functions differ between a fresh install and the upgrade"
diff <(echo "$fresh_rels") <(echo "$up_rels") || fail "relations differ between a fresh install and the upgrade"
ok "functions ($(echo "$up_sig" | wc -l | tr -d ' ')) and relations ($(echo "$up_rels" | wc -l | tr -d ' ')) match a fresh install"

echo "upgrade gate: $PREV -> $CUR passed"
