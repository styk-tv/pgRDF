---
title: "SPEC.pgRDF.BENCH — the pgRDF LUBM benchmark contract"
version: 0.6.0
status: live
date: 2026-06-12
applies-to: pgRDF v0.6.x (M4 join-order fix + M1 auto-ANALYZE + v0.6.1 batched materialize)
hardware: a dedicated Azure VM — Standard_E32as_v7 (32 vCPU / 256 GiB); see §7
evidence: tests/perf/lubm/RESULTS.m4-join-order.md ; tests/perf/lubm/k8s-daemonset/ ; tests/perf/benchmark-runner.sh
---

# pgRDF LUBM benchmark — methodology + contract (v0.6.0)

The authoritative method for benchmarking pgRDF, so any run (laptop, Azure VM,
AKS) is comparable and self-verifying. It pins **what** is measured, **how**
(to avoid the confounds we already hit), **what "correct" means**, and the
**performance model** that decides hardware and the optimization roadmap.

## 1. Benchmark + scales

**LUBM** — the Lehigh University Benchmark (Guo/Pan/Heflin), the canonical RDF-
store benchmark. Data via the **UBA 1.7** generator, **`-seed 0`** (deterministic
— identical bytes every run; the generator image is `tests/perf/lubm/generator/`).

| scale | universities | base triples | role |
|---|---|---|---|
| LUBM-1 | 1 | ~103 k | unit smoke |
| LUBM-10 | 10 | ~1.3 M | dev gate (CI-adjacent) |
| LUBM-50 | 50 | 6,890,640 | mid-scale tuning |
| **LUBM-100** | 100 | **13,879,970** | **release gate** |
| LUBM-250 | 250 | ~35 M | the next scale target (§8) |

Plus the **Univ-Bench Tbox** (`tests/perf/lubm/fixtures/univ-bench.ttl`, namespace
`file:///opt/lubm/univ-bench.owl#`, matching the UBA `-onto` ABox IRIs).

## 2. The three profiles (and the confound that forced this rule)

LUBM queries are run under three reasoning profiles:

- **none** — query the asserted graph (no reasoning). q04–q13 are 0 by design
  (they need the class/property hierarchy); q01/q02/q03/q14 are data-bearing.
- **rdfs** — `pgrdf.materialize(g,'rdfs')` first (pgRDF's own RDFS forward-chain).
- **owl-rl** — `pgrdf.materialize(g,'owl-rl')` first (the `reasonable` OWL 2 RL
  fixpoint). All 14 queries return data.

**Confound rule (mandatory).** Each profile MUST run on a **fresh database**
(or at minimum: `materialize` once, then query, never stacking profiles on one
DB). A full `none → rdfs → owl-rl` sequence on a single DB leaves the table
bloated with three closures + stale stats and makes the third profile ~100×
slower (the HW spec §6 saw q07/q08 blow up exactly this way). **For any cross-
machine comparison: identical sequence, standalone-per-profile, fresh DB.**

## 3. The four measured phases (the surfaces under test)

| phase | UDF | what to capture | cost class |
|---|---|---|---|
| **Import** | `load_turtle_verbose` | `triples`, `elapsed_ms`, `parse_ms`/`dict_ms`/`insert_ms` | single-core (§6) |
| **Materialization** | `materialize(g,profile)` | `inferred_triples_written`, `elapsed_ms`, and the v0.6.1 phase timers `load_ms`/`reason_ms`/`diff_ms`/`write_ms`/`analyze_ms` | reasoner single-core (§6) |
| **Validation** | `validate(data,shapes,'native')` | `conforms`, violation count, wall | SHACL single-core |
| **Query (multi-hop)** | `sparql` ×14 | per-query wall + `count(*)`, **verified** vs the locked reference | parallelizable (§6) |

The v0.6.1 materialize phase timers are the attribution tool — they show
write-back vs. reasoner vs. base-load split (lubm-50: reason ~14 %, write ~50 %,
load ~22 %), which is what tells you whether a slowdown is engine or data.

## 4. Required engine state (do not benchmark against a mis-configured build)

- **pgRDF ≥ 0.6.1.** Below 0.5.45, multi-hop queries hit the cross-product wall
  (q02 = 649 s); the benchmark is meaningless without the M4 fix.
- **`pgrdf.auto_analyze = on`** (default). Without it, queries on a freshly
  materialized graph mis-plan catastrophically (q02 owl-rl: 180 s timeout → 2 s).
  A run with it off is **not a valid pgRDF benchmark** — it measures a planner
  starved of stats, not the engine.
- Reads go through `pgrdf.sparql/construct/describe` (which pin `join_collapse_
  limit`). Raw lowered SQL executed directly (without that pin) is invalid.

## 5. Correctness gate — counts, not just timings

A run **PASSES** only if the owl-rl-materialized 14-query counts match the locked
`-seed 0` reference (LUBM-100): q01=4, q02=129,401, q03=6, q04=34, q05=719,
q06=1,048,532, q07=67, q08=7,790, q09=27,247, q10=4, q11=224, q12=15, q13=472,
q14=795,970. (LUBM-50 / others: see `tests/perf/lubm/queries/expected-counts.json`
and the daemonset runner's reference table.) **A fast run with wrong counts is a
failed run.** Correctness has held across every machine/RAM/config tested — it is
config-independent; only wall-time moves.

## 6. Performance model — the finding that drives hardware (§7) and the roadmap (§9)

Measured across a laptop (arm64) and two Azure node sizes (8 vCPU / 32 GiB and
8 vCPU / 64 GiB):

- **Import and materialization are single-core / reasoner-bound.** They do **not**
  speed up with more vCPUs — only with **faster per-core** (newer silicon). The
  laptop's 217 s ingest beat the v6 cloud cores. The `reasonable` OWL-RL fixpoint
  (datafrog) is single-threaded; pgRDF's import loop is sequential.
- **Multi-hop queries DO parallelize** — PostgreSQL parallel workers over the
  hexastore indexes; more cores help the query phase.
- **RAM is the hard limit for the *tuned* config.** OWL-RL reasoner peak ≈ 28 GiB
  at LUBM-100; with `shared_buffers=8GB` that needs > 32 GiB → **32 GiB OOMs**,
  ≥ 64 GiB fits, 256 GiB is comfortable and headroom for bigger scales. Reasoner
  RAM scales ~linearly with closure size (§8).

Corollary: throwing cores at LUBM-100 helps only the (already-fast) query phase;
the long phases want **fast per-core + enough RAM**. Multi-core wins on
import/materialize/validate require *engine* parallelism (§9), not bigger nodes.

## 7. Hardware

**`Standard_E32as_v7`** — 32 vCPU / **256 GiB**, v7 AMD (Genoa-class, newest
generation in the subscription). Chosen for **newest per-core** (the import/
materialize bottleneck) + RAM headroom that lets the **tuned `shared_buffers=8GB`
config run without OOM** and leaves room for LUBM-250+. Plain VPN-reachable VM,
native Docker (no AKS/dind IO nesting, no pod memory cap). Deallocate when idle.

Config matrix per run: **default** `shared_buffers` (always safe) AND **tuned**
`shared_buffers=8GB` (only on ≥ 64 GiB; the headline config on E32as_v7). Report
both; default is the "zero-tuning" claim, tuned is the operable-deployment number.

## 8. Scaling toward LUBM-250

| scale | base | materialized (owl-rl, ~1.6×) | reasoner RAM peak (est.) | fits E32as_v7 (256 GiB)? |
|---|---|---|---|---|
| LUBM-100 | 13.9 M | 22.5 M | ~28 GiB | yes (huge headroom) |
| LUBM-250 | ~35 M | ~57 M | **~70 GiB** | yes |
| LUBM-500 | ~69 M | ~113 M | ~140 GiB | yes |
| LUBM-700 | ~97 M | ~158 M | ~196 GiB | tight |

The **reasoner RAM is the scaling limiter** (holds the closure in datafrog
Variables); 256 GiB carries LUBM-250 comfortably and ~LUBM-500 before pressure.
Materialize wall scales super-linearly (reasoner) — the phase to watch and the
primary optimization target as scale grows.

## 9. Optimization targets (the "multi-core" goal, honestly)

The goal of optimizing import / materialization / validation / multi-hop queries
on E32as_v7's 32 cores meets a real constraint: **today only the query phase is
multi-core.** The named pgRDF tracks to extend that (forward, v0.7+):

- **Query (multi-hop)** — already parallel; M4 pin keeps the plan sane. Tune
  `max_parallel_workers_per_gather` on the big-node config.
- **Import** — currently sequential (parse → dict → insert). Targets: the staged-
  partition bulk load (index-build-once, `_WIP/PLAN.T2-staged-partition-load.md`),
  and a parallel dict-resolution pass (dict is ~59 % of import e2e at LUBM-10).
- **Materialization** — reasoner is single-threaded upstream (`reasonable`/
  datafrog). pgRDF owns the write-back (already batched, v0.6.1, 2× faster) and
  the `rdfs` profile (pgRDF's own forward-chain — candidate for parallel rule
  application). True ∝delta is engine-ask #2 (incremental).
- **Validation** — SHACL (`rudof`) single-core; parallel-over-shapes is an
  upstream/integration target.

Each is its own measured slice; none is a config knob. The HW (E32as_v7) gives
fast per-core + RAM now; the engine parallelism unlocks the cores over time.

## 10. Execution vehicles

1. **Plain Azure VM** (the chosen path, §7) — SSH in, native Docker, run
   `tests/perf/benchmark-runner.sh` against a glibc/bookworm Postgres-17 image
   carrying the published pgRDF extension + the Tbox. Full RAM, no nesting.
2. **k8s DaemonSet** (`tests/perf/lubm/k8s-daemonset/`) — declarative per-node run
   for AKS; honest caveat: AKS pods cap RAM below the node + dind adds IO overhead,
   so it OOMs the tuned config on 32 GiB nodes. Use ≥ 64 GiB nodes or default
   `shared_buffers`.

**Container tool requirement (gate):** `benchmark-runner.sh` drives the DB via
`docker exec <sidecar> psql` and `pg_isready`, so the benchmark image **MUST**
carry `psql` + `pg_isready` on PATH. Stock `postgres:17.4-bookworm` does;
slimmed images (~150 MB postgres+pgrdf builds) must be confirmed to include
the `postgresql-client` tools before use (verify with
`docker run --rm --entrypoint sh <img> -c 'command -v psql pg_isready'`). The
extension `.so` is the **glibc-bookworm** build (the published
`pgrdf-<v>-pg17-glibc-{amd64,arm64}.tar.gz` / `ghcr.io/styk-tv/pgrdf-bundle`),
so the image base must be glibc/bookworm — not musl/alpine.

## 11. Cross-references

- Hardware plan: the dedicated-VM hardware spec (maintained in the cluster repo).
- Measured evidence: `tests/perf/lubm/RESULTS.m4-join-order.md` (v0.5.45 → v0.6.1
  full passes, per-query tables).
- Runner: `tests/perf/benchmark-runner.sh` (the canonical driver).
- DaemonSet: `tests/perf/lubm/k8s-daemonset/` (AKS vehicle).
- Forward optimization: `_WIP/PLAN.v0.6-forward.md`, `_WIP/PLAN.T2-staged-partition-load.md`.
