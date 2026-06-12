#!/usr/bin/env bash
#
# LUBM-100 full-pass benchmark runner (runs in the `runner` container, talks to
# the in-pod `postgres` container over 127.0.0.1). Phases:
#   1. wait for postgres + CREATE EXTENSION
#   2. ingest LUBM-100 (13.88M triples) + the Tbox
#   3. 14 reference queries on the plain graph (none-profile)
#   4. materialize OWL 2 RL (-> ~22.46M quads, auto-ANALYZE)
#   5. 14 reference queries on the materialized graph, counts verified vs. the
#      locked reference
# Prints a timing + count table, then idles so `kubectl logs` collects it.
#
# DEFAULT PostgreSQL config — no tuning. That is the benchmark's whole claim.
set -uo pipefail

export PGHOST=127.0.0.1 PGPORT=5432 \
       PGUSER="${POSTGRES_USER:-pgrdf}" \
       PGPASSWORD="${POSTGRES_PASSWORD:-pgrdf}" \
       PGDATABASE="${POSTGRES_DB:-pgrdf}"
GID=92000
NT="${LUBM_NT:-/data/lubm-100/nt/lubm-100.nt}"
TBOX=/fixtures/univ-bench.ttl
QDIR=/queries
QCAP="${QUERY_TIMEOUT:-600s}"

P(){ psql -X -A -t -q -v ON_ERROR_STOP=0 "$@"; }

echo "================ pgRDF LUBM-100 benchmark ================"
echo "node: $(uname -m) | $(nproc) vCPU | $(free -g 2>/dev/null | awk '/Mem:/{print $2}')GiB RAM | $(date -u +%FT%TZ)"

echo "== [1/5] wait for postgres =="
for i in $(seq 1 150); do pg_isready -q && break; sleep 2; done
pg_isready || { echo "FATAL: postgres not ready after 5m"; sleep infinity; }
[ -f "$NT" ] || { echo "FATAL: dataset $NT missing (generator initContainer failed?)"; sleep infinity; }
P -c "CREATE EXTENSION IF NOT EXISTS pgrdf;" >/dev/null
echo -n "  pgrdf.version() = "; P -c "SELECT pgrdf.version();"
P -c "SELECT pgrdf.add_graph($GID);" >/dev/null

# Locked owl-rl-materialized reference counts (UBA -seed 0, LUBM-100).
declare -A REF=( [01]=4 [02]=129401 [03]=6 [04]=34 [05]=719 [06]=1048532 \
  [07]=67 [08]=7790 [09]=27247 [10]=4 [11]=224 [12]=15 [13]=472 [14]=795970 )

runq() { # $1 = phase label, $2 = verify(yes|no)
  echo "== 14 queries [$1] =="
  local fails=0
  for q in $(seq -w 1 14); do
    f="$QDIR/q${q}.rq"; [ -f "$f" ] || continue
    local ql; ql=$(grep -v '^[[:space:]]*#' "$f" | tr '\n' ' ' | sed "s/'/''/g")
    local t0 t1 cnt; t0=$(date +%s)
    cnt=$(P -c "SET statement_timeout='$QCAP'; SELECT count(*) FROM pgrdf.sparql('$ql');" 2>&1 | head -1)
    t1=$(date +%s)
    if [ "$2" = "yes" ]; then
      local exp="${REF[$q]}" v
      if [ "$cnt" = "$exp" ]; then v="OK"; else v="MISMATCH(exp $exp)"; fails=$((fails+1)); fi
      printf "  q%s  %5ss  count=%-9s %s\n" "$q" "$((t1-t0))" "$cnt" "$v"
    else
      printf "  q%s  %5ss  count=%s\n" "$q" "$((t1-t0))" "$cnt"
    fi
  done
  return $fails
}

echo "== [2/5] ingest LUBM-100 + Tbox =="
t0=$(date +%s)
P -c "SELECT pgrdf.load_turtle_verbose('$NT', $GID)->>'triples';" | xargs echo "  base triples ="
t1=$(date +%s); echo "  INGEST WALL = $((t1-t0))s"
P -c "SELECT pgrdf.load_turtle('$TBOX', $GID);" >/dev/null

echo "== [3/5] none-profile queries =="
runq "none-profile" no || true

echo "== [4/5] materialize owl-rl =="
t0=$(date +%s)
P -c "SELECT pgrdf.materialize($GID,'owl-rl')->>'inferred_triples_written';" | xargs echo "  inferred ="
t1=$(date +%s); echo "  MATERIALIZE WALL = $((t1-t0))s"
P -c "SELECT 'total quads = '||count(*) FROM pgrdf._pgrdf_quads;"

echo "== [5/5] owl-rl-materialized queries (counts verified) =="
runq "owl-rl materialized" yes; fails=$?

echo "========================================================="
echo "RESULT: count mismatches = ${fails:-?} / 14  ($([ "${fails:-1}" -eq 0 ] && echo PASS || echo CHECK))"
echo "  (reference counts = our LUBM-100 UBA -seed 0 baseline; pgRDF v0.6.x"
echo "   on a 32 GiB node hits ingest ~3.5min, materialize ~5min, queries <=5s)"
echo "========================================================="
echo "Pod idling — collect with: kubectl logs <pod> -c runner"
sleep infinity
