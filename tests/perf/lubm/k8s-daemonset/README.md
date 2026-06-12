# LUBM-100 benchmark — Kubernetes DaemonSet (Azure `Standard_D8s_v6`)

An **isolated, cluster-runnable** form of the pgRDF LUBM-100 benchmark. It
reproduces the full-pass measurement we run locally — generate → ingest →
query → materialize → query — on a real Azure node, declaratively, with **zero
database tuning** (default `postgresql.conf`). Hand this directory to the
cluster operator to execute.

> This is a **self-contained test fixture**, not part of CI. It is meant to be
> built and applied to a cluster on demand, then torn down.

---

## What it measures

`Standard_D8s_v6` = 8 vCPU / 32 GiB — the same shape as the developer VM that
produced the published numbers, so the result is directly comparable. Each pod
runs, in order:

| phase | what | local reference (v0.6.x, 32 GiB) |
|---|---|---|
| 1. generate | UBA `-seed 0`, 100 universities → 13,879,970 triples | — |
| 2. ingest | Turtle → `_pgrdf_quads` (+ Tbox) | ~3.5 min |
| 3. query ×14 | the 14 LUBM reference queries, plain graph | each ≤ 3 s |
| 4. materialize | OWL 2 RL closure → 22,463,054 quads, auto-ANALYZE | ~5 min (v0.6.1) |
| 5. query ×14 | the 14 queries on the materialized graph, **counts verified** | each ≤ 5 s |

The runner prints a timing + count table and compares the materialized-profile
counts against the locked `-seed 0` reference (`q02=129,401`, `q06=1,048,532`,
`q09=27,247`, `q14=795,970`, …). `count mismatches = 0/14` ⇒ correct.

> x86 vs. the arm64 dev VM: absolute wall-times may differ; correctness
> (the counts) must match exactly.

---

## Architecture (one pod per matching node)

```
DaemonSet  (nodeSelector: node.kubernetes.io/instance-type = Standard_D8s_v6)
  └─ pod
     ├─ initContainer  generate   → writes /data/lubm-100/nt/lubm-100.nt   (shared emptyDir)
     ├─ container      postgres   → stock PG17 + published pgRDF, DEFAULT config,
     │                              reads /data server-side, PGDATA on node disk
     └─ container      runner     → run-benchmark: waits, ingests, queries,
                                    materializes, queries+verifies, idles
```

- **Default Postgres config** — the benchmark's entire claim. Only
  `shared_preload_libraries=pgrdf` is set.
- **PGDATA on node disk** (`emptyDir`), not tmpfs — the OWL-RL reasoner peaks
  ~28 GiB of RAM at this scale, so the 32 GiB must stay free for it.
- Two app containers share the pod network; the runner talks to `127.0.0.1:5432`.

---

## Two images to build + push

The manifest references `REGISTRY_PLACEHOLDER/...`; replace with your registry.

```bash
REG=<your-registry>            # e.g. myacr.azurecr.io  (must be amd64-pullable by the pool)
PGRDF_VERSION=0.6.0            # latest PUBLISHED GitHub release; bump to 0.6.1 once pushed

# 1) LUBM data generator (java + UBA + raptor). Context = the generator dir.
docker build --platform linux/amd64 \
  -t "$REG/pgrdf-lubm-generator:latest" \
  tests/perf/lubm/generator
docker push "$REG/pgrdf-lubm-generator:latest"

# 2) Benchmark image (PG17 + published pgRDF + queries + Tbox + runner).
#    Context MUST be tests/perf/lubm (repo root carries ref-* symlinks).
docker build --platform linux/amd64 \
  -f tests/perf/lubm/k8s-daemonset/Dockerfile.benchmark \
  --build-arg PGRDF_VERSION="$PGRDF_VERSION" \
  -t "$REG/pgrdf-lubm-bench:$PGRDF_VERSION" \
  tests/perf/lubm
docker push "$REG/pgrdf-lubm-bench:$PGRDF_VERSION"
```

The benchmark image pulls the pgRDF `.so`/`.control`/`.sql` from
`github.com/styk-tv/pgRDF/releases/download/v${PGRDF_VERSION}/pgrdf-${PGRDF_VERSION}-pg17-glibc-amd64.tar.gz`
— the extension is the *published release artifact*, not a local build.

---

## Run it

```bash
# point the manifest at your registry + image tag
sed -e "s#REGISTRY_PLACEHOLDER#$REG#g" \
    -e "s#pgrdf-lubm-bench:0.6.0#pgrdf-lubm-bench:$PGRDF_VERSION#g" \
    tests/perf/lubm/k8s-daemonset/daemonset.yaml | kubectl apply -f -

# watch it come up (generate ~5 min, then postgres+runner)
kubectl get pods -l app=pgrdf-lubm100-bench -w

# collect the result table (full pass is ~12–15 min once running)
POD=$(kubectl get pod -l app=pgrdf-lubm100-bench -o name | head -1)
kubectl logs "$POD" -c runner -f
```

The runner ends with `RESULT: count mismatches = N / 14` then idles, so the log
is always retrievable. Re-run by deleting + re-applying.

### Teardown

```bash
kubectl delete -f <(sed "s#REGISTRY_PLACEHOLDER#$REG#g" tests/perf/lubm/k8s-daemonset/daemonset.yaml)
```

---

## Knobs

- **pgRDF version** — `--build-arg PGRDF_VERSION=` on the benchmark image (must
  be a published GitHub release). 0.6.0 today; 0.6.1 once it ships.
- **Node selector** — swap `node.kubernetes.io/instance-type: Standard_D8s_v6`
  for `kubernetes.azure.com/agentpool: <pool>` to target a named pool.
- **Taints** — uncomment the `tolerations` block if the pool is tainted.
- **Scale** — `100` (universities) in the `generate` init-container args; lower
  it (e.g. `10` ≈ 1.3M triples) for a fast smoke run.
- **Query cap** — `QUERY_TIMEOUT` env on the runner (default `600s`).

## Notes / caveats

- **DaemonSet vs. Job** — a DaemonSet is used per your request (one pod per
  matching node, declarative). It is a long-running shape; the runner idles
  after the pass so logs persist. A `Job` (with the same two containers via a
  generator init-container) is the alternative if you want auto-completion
  instead of an idling pod.
- **Dedicate the pool** — the pod requests ~7 vCPU / ~26 GiB; co-scheduling
  other heavy workloads on the node will skew the wall-times.
- **`Dsv6` availability** — the D-series v6 is region-limited; confirm the pool
  exists before applying.
- **No external state** — everything is in `emptyDir`s; nothing persists after
  teardown. Results live only in the pod logs (copy them out).
