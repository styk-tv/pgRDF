---
title: "SPEC.pgRDF.BENCH — the pgRDF benchmark contract (ingest-at-scale + LUBM semantic pipeline)"
version: 0.6.14
status: live
date: 2026-06-25
applies-to: pgRDF v0.6.14 (the T1–T6 native staged bulk loader + the LUBM semantic pipeline — M4 join-order + M1 auto-ANALYZE + batched materialize)
hardware: at-scale ingest on Azure E128ads_v7 (128 vCPU / 1 TiB) and E64ads_v7 (64 vCPU / 503 GiB); LUBM semantic pipeline on Azure Standard_E32as_v7 (32 vCPU / 256 GiB); see §7
evidence: tests/perf/lubm/RESULTS.m4-join-order.md ; tests/perf/lubm/k8s-daemonset/ ; tests/perf/benchmark-runner.sh ; the `33-pgRDF-wikidata-scale/RESULTS` companion (a private benchmark repo — referenced by name; no private paths/hosts embedded here)
cross-ref: specs/SPEC.pgRDF.LLD.v0.6.14.md (§3 ingest, §4 the staged loader, §6 query translator, §7 materialize/validate)
---

# pgRDF benchmark — methodology + contract (v0.6.14)

The authoritative method for benchmarking pgRDF, so any run (laptop, Azure VM,
AKS) is comparable and self-verifying. It pins **what** is measured, **how**
(to avoid the known confounds), **what "correct" means**, and the
**performance model** that decides hardware and the optimization roadmap.

## 0. Two benchmark classes (read this first)

pgRDF is benchmarked in **two distinct classes** with different performance
models, different hardware sizing, and different correctness gates. Conflating
them is the most common error — one is parallel and scales with the box, the
other is single-threaded and sized to the box.

- **(A) Ingest at scale** — raw bulk load only. The native **staged bulk loader**
  (`pgrdf.load_turtle_staged_run`) over the full Wikidata `truthy` N-Triples dump.
  This phase is **parallel and scales with cores** (a background-worker pool fans
  STAGE COPY and INDEX builds across the box). It is **raw ingest, not reasoning** —
  truthy statements are already asserted, so there is nothing to infer. NEW in
  this spec — see **§A** below.

- **(B) Semantic pipeline** — load → reason → query (+ validate). The canonical
  **LUBM** benchmark. Reasoning (OWL-RL materialization) is **single-threaded and
  sized to hardware**; queries parallelize. This is the existing LUBM content,
  unchanged in method — see **§1–§9** below.

The performance model (§6) and the optimization roadmap (§9) treat these classes
separately: ingest = parallel/scales-with-the-box; reason+validate = single-
threaded/sized-to-fit.

---

# Part A — Ingest at scale (the native staged bulk loader)

## §A.1 What it is

The **native staged bulk loader** is pgRDF's billion-scale ingest path:
`pgrdf.load_turtle_staged_run(path, graph_id, n_workers)` (and its
`CALL pgrdf.load_turtle_staged(...)` procedure wrapper). It loads a single
N-Triples dump by committing one phase at a time over a **background-worker
pool** — the only design that can COMMIT mid-load, run several index builds at
once, and own multiple concurrent COPY streams (a single `#[pg_extern]`
function cannot). See `SPEC.pgRDF.LLD.v0.6.14.md` §4 for the full design.

**Four committed phases, gated A→B→C→D:**

| phase | workers | what it does | scales with |
|---|---|---|---|
| **STAGE** | *N* (auto = cores, capped 32) | newline-snapped byte ranges of the file, each streamed in bounded windows, parsed across all cores (rayon), bulk-loaded via concurrent server-side **COPY** into one UNLOGGED staging heap | **cores** (multi-backend COPY + intra-worker rayon) |
| **DICT** | 1 | resolve all distinct terms into the dictionary (bulk, full-identity keyed), build the transient resolve index | PG intra-query parallel hash-agg |
| **RESOLVE** | 1 | 3× hash-join every staged triple to its dict ids → a standalone CTAS, then `ATTACH PARTITION` | PG intra-query parallel hash-join |
| **INDEX** | 5 | rebuild the 3 hexastore covering indexes + the dict hash index + the `unique_term` UNIQUE, one build per worker, all at once | **cores** (5 concurrent builds) |

**Commit-per-phase = resumability.** Each phase commits in its own worker
transaction; the coordinator records a high-water mark after each success. On any
phase failure it ABORTS and **leaves the staging table in place as the resume
point** — a re-run reconciles against the committed-per-phase artifacts via
idempotent drops / `IF NOT EXISTS`.

## §A.2 Required engine state

- **`shared_preload_libraries = 'pgrdf'` is REQUIRED.** The staged loader's
  job-control shared-memory segment, the worker pool, and the cross-backend
  dictionary cache all live in shmem allocated at postmaster start. Without the
  preload the jobctl segment never exists, the staged dispatch is not ready, and
  ingest falls back to the non-staged path. **A staged-ingest benchmark on a build
  without the preload is not a valid run.**
- **N-Triples input only.** The STAGE phase parses with a line-oriented
  N-Triples parser. A real Turtle file routed here would have every directive /
  prefixed / multi-line statement silently SKIPPED = data loss; the dispatch is
  deliberately conservative (any ambiguity → the full Turtle parser, never the
  staged path). Benchmark the staged loader on **`.nt`** dumps.
- **No `postgresql.conf` tuning beyond the preload.** All staged tunables are
  GUCs (§A.4); the out-of-the-box result needs no other config.

## §A.3 The flagship result — full Wikidata `truthy`

The reference at-scale corpus is the complete Wikidata **`truthy`** dump as
N-Triples. Results land in the `33-pgRDF-wikidata-scale/RESULTS` companion (a
private benchmark repo — referenced by name only).

**Corpus (locked ground truth):**

| metric | value |
|---|---|
| input triples | **8,199,708,346** (0 dropped) |
| distinct dictionary terms | **1,801,847,593** |
| on-disk footprint | **~2.0 TB** (heap 729 GB + indexes 1448 GB) |

**Flagship — Azure E128ads_v7 (128 vCPU / 1 TiB):**

| metric | value |
|---|---|
| end-to-end | **4 h 53 m** |
| throughput | **466 K triples/sec** |
| vs. v0.6.13 all-hash baseline | **37 % faster** (baseline 6 h 41 m / 340.7 K tps) |

Per-phase split (flagship):

| phase | wall | note |
|---|---|---|
| STAGE | **13.8 min** | T3 parallel multi-backend COPY, ~7.3× over single-worker |
| DICT | **1 h 51 m** | |
| RESOLVE | **2 h 00 m** | `index` resolve strategy (the at-scale default) |
| INDEX | **31.9 min** | 5 concurrent index builds |

**Companion — Azure E64ads_v7 (64 vCPU / 503 GiB, 3.4 TB disk):**

| metric | value |
|---|---|
| end-to-end | **~10.3 h** |
| throughput | **~221 K triples/sec** |
| configuration | **out-of-the-box** (stock PostgreSQL + the preload; no custom tuning) |

The E64 run is the **out-of-the-box proof**: the full 8.2 B-triple graph ingests
on a stock single-NUMA box with only `shared_preload_libraries=pgrdf` set.

## §A.4 Correctness gate (ingest)

A staged ingest **PASSES** only when both hold:

- **`quads == triples`** — every input triple lands as exactly one stored quad
  (8,199,708,346 in → 8,199,708,346 quads). Zero loss, zero phantom rows.
- **Exact literal dedup on full identity.** A literal is interned once, keyed on
  the **full `(value, datatype, language)` identity** — NULL-safe (`IS NOT
  DISTINCT FROM`). The proof case: `"Berlin"` is preserved as a **distinct term
  per language tag across 268 distinct languages** (the same lexical value with
  different language tags are different terms, never collapsed). The historical
  failure this guards against was a partial-key collapse that destroyed
  language-distinct literals; the full-key fix is the v0.6.14 contract.

As in the LUBM class: a fast run with wrong counts or collapsed literals is a
**failed run**.

## §A.5 Tunables (all GUCs)

All staged tunables are **GUCs** (`GucContext::Userset`) — no `postgresql.conf`
beyond the preload. See `SPEC.pgRDF.LLD.v0.6.14.md` §4.4–§4.6.

| GUC | values | default | purpose |
|---|---|---|---|
| `pgrdf.staged_resolve_strategy` | `index` \| `hash` \| `auto` | **`index`** | Forces RESOLVE's planner join method. Identical output — performance knob only. **`index` is the at-scale default**: it is the low-spill strategy that avoids the multi-TB hash-join temp spill (and the ENOSPC it caused at 8.2 B rows). Use `hash` only on small inputs with abundant temp space. |
| `pgrdf.staged_temp_tablespaces` | comma-separated tablespace names | empty (inherit server `temp_tablespaces`) | Routes temp spill — dominated by RESOLVE — **off the data disk** onto a dedicated tablespace, the operable answer to multi-TB intermediate spill. Validated against an identifier allowlist before interpolation. |

**Adaptive self-tune (T5) — computed, not configured.** Inside every phase the
loader re-derives `work_mem` / `maintenance_work_mem` and the parallelism levers
(`max_parallel_workers`, `…_per_gather`, `…_maintenance_workers`,
`enable_parallel_hash`, zeroed parallel costs) from **host RAM + core count**
(read from `/proc/meminfo` + `available_parallelism`). This is **OOM hardening,
not a knob**: it sizes RESOLVE's parallel-hash budget to stay under ~50 % of RAM
so a RAM-tight host spills rather than OOM-kills, while a high-RAM host pins at
the 2 GB cap. The decision is logged per phase for operator visibility. The
`index` resolve strategy (above), not self-tune, is what delivers the
out-of-the-box at-scale result; self-tune lowers OOM risk where RAM is tight.

## §A.6 This is ingest, NOT reasoning

The at-scale benchmark measures **raw bulk ingest only**. Wikidata `truthy`
statements are already-asserted facts — there is **nothing to infer**, so no
materialization runs and the single-threaded reasoner (Part B) is never on the
critical path. This is exactly why ingest scales with the box: it is all
parallel STAGE COPY + concurrent index build, with no reasoner wall in the loop.

---

# Part B — Semantic pipeline (the LUBM benchmark)

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
| **Import** | `load_turtle_verbose` (LUBM scope); `load_turtle_staged_run` (at-scale, Part A) | `triples`, `elapsed_ms`, `parse_ms`/`dict_ms`/`insert_ms`; staged: per-phase walls | parallel & scales — §6 |
| **Materialization** | `materialize(g,profile)` | `inferred_triples_written`, `elapsed_ms`, and the materialize phase timers `load_ms`/`reason_ms`/`diff_ms`/`write_ms`/`analyze_ms` | **reasoner single-core** — §6 |
| **Validation** | `validate(data,shapes,'native')` | `conforms`, violation count, wall | SHACL single-core |
| **Query (multi-hop)** | `sparql` ×14 | per-query wall + `count(*)`, **verified** vs the locked reference | parallelizable — §6 |

The materialize phase timers are the attribution tool — they show write-back vs.
reasoner vs. base-load split (lubm-50: reason ~14 %, write ~50 %, load ~22 %),
which is what tells you whether a slowdown is engine or data.

## 4. Required engine state (do not benchmark against a mis-configured build)

- **pgRDF v0.6.14.** Below 0.5.45, multi-hop queries hit the cross-product wall
  (q02 = 649 s); the benchmark is meaningless without the M4 join-order fix.
- **`pgrdf.auto_analyze = on`** (default). Without it, queries on a freshly
  materialized graph mis-plan catastrophically (q02 owl-rl: 180 s timeout → 2 s).
  A run with it off is **not a valid pgRDF benchmark** — it measures a planner
  starved of stats, not the engine. (See `SPEC.pgRDF.LLD.v0.6.14.md` §7 — auto-
  ANALYZE fires after a non-empty materialize.)
- Reads go through `pgrdf.sparql/construct/describe` (which pin
  `join_collapse_limit = 1` / `from_collapse_limit = 1`, the M4 cross-product-
  proof plan — `SPEC.pgRDF.LLD.v0.6.14.md` §6). Raw lowered SQL executed directly
  (without that pin) is invalid.

## 5. Correctness gate — counts, not just timings

A run **PASSES** only if the owl-rl-materialized 14-query counts match the locked
`-seed 0` reference (LUBM-100): q01=4, q02=129,401, q03=6, q04=34, q05=719,
q06=1,048,532, q07=67, q08=7,790, q09=27,247, q10=4, q11=224, q12=15, q13=472,
q14=795,970. (LUBM-50 / others: see `tests/perf/lubm/queries/expected-counts.json`
and the daemonset runner's reference table.) **A fast run with wrong counts is a
failed run.** Correctness has held across every machine/RAM/config tested — it is
config-independent; only wall-time moves.

## 6. Performance model — the finding that drives hardware (§7) and the roadmap (§9)

Measured across a laptop (arm64), the LUBM-class Azure node sizes, and the
at-scale boxes (Part A):

- **Import is now PARALLEL and scales with the box.** The earlier "import is
  single-core" claim is **superseded**. The native staged loader fans STAGE COPY
  across a multi-backend pool and builds the 5 indexes concurrently — STAGE and
  INDEX scale with cores, DICT/RESOLVE use PostgreSQL's intra-query parallel
  hash. Proven at 8.2 B triples (Part A): 466 K tps on 128 cores, ~221 K on 64.
  More cores → faster ingest.
- **Materialization REMAINS single-threaded.** The `reasonable` OWL-RL fixpoint
  (datafrog) holds the closure in single-threaded `Variables` — it does **not**
  speed up with more vCPUs, only with **faster per-core** (newer silicon). This
  is now the **sole single-threaded phase** in the pipeline and the reason
  reasoning must be sized to a carved / right-sized graph (issue #1, upstream).
- **SHACL validation is single-core** (`rudof`) — parallel-over-shapes is an
  upstream/integration target.
- **Multi-hop queries DO parallelize** — PostgreSQL parallel workers over the
  hexastore indexes; more cores help the query phase.
- **RAM is the hard limit for the *tuned* config.** OWL-RL reasoner peak ≈ 28 GiB
  at LUBM-100; with `shared_buffers=8GB` that needs > 32 GiB → **32 GiB OOMs**,
  ≥ 64 GiB fits, 256 GiB is comfortable with headroom for bigger scales. Reasoner
  RAM scales ~linearly with closure size (§8).

**Net.** Ingest = parallel / scales-with-the-box (throw cores at it). Reason +
validate = single-threaded / sized-to-fit (throw fast-per-core + RAM at it). The
two classes want different hardware for different reasons.

## 7. Hardware

**Ingest at scale (Part A).** **E128ads_v7** (128 vCPU / 1 TiB) is the flagship —
maximal cores for STAGE COPY + concurrent index build, RAM headroom for the dict
and the resolve hash. **E64ads_v7** (64 vCPU / 503 GiB, 3.4 TB disk) is the
single-NUMA out-of-the-box companion. Both run a plain VPN-reachable VM with
native Docker, `shared_preload_libraries=pgrdf`, and a temp tablespace routed off
the data disk for the multi-TB RESOLVE spill (§A.5). Deallocate when idle.

**Semantic pipeline (Part B).** **`Standard_E32as_v7`** — 32 vCPU / **256 GiB**,
v7 AMD (Genoa-class). Chosen for **newest per-core** (the materialize bottleneck)
+ RAM headroom that lets the **tuned `shared_buffers=8GB` config run without OOM**
and leaves room for LUBM-250+. Plain VPN-reachable VM, native Docker (no AKS/dind
IO nesting, no pod memory cap). Deallocate when idle.

Config matrix per LUBM run: **default** `shared_buffers` (always safe) AND
**tuned** `shared_buffers=8GB` (only on ≥ 64 GiB; the headline config on
E32as_v7). Report both; default is the "zero-tuning" claim, tuned is the
operable-deployment number.

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
primary optimization target as scale grows. (Ingest no longer limits scale: Part
A demonstrates 8.2 B triples already ingest in hours; the wall is reasoning.)

## 9. Optimization targets (the "multi-core" goal, honestly)

The benchmark stresses import / materialization / validation / multi-hop queries.
As of v0.6.14 the engine-parallelism picture has changed: **ingest is now
multi-core; reasoning is the remaining single-threaded wall.**

- **Import (ingest) — SHIPPED in v0.6.14.** The staged-partition bulk load is
  **done**, not a `_WIP/PLAN.T2` future. The T-line landed: **T3** parallel
  multi-backend STAGE COPY, **T2** the `index`/`hash`/`auto` resolve-strategy
  selector (`index` the low-spill at-scale default), **T1** `staged_temp_tablespaces`
  off-disk temp routing, **T5** adaptive self-tune (work_mem / parallelism scale
  to host RAM + cores). Proven at 8.2 B triples (Part A). No further core ingest
  ask for v0.7.
- **Materialization — the remaining single-threaded wall.** The `reasonable`
  OWL-RL fixpoint (datafrog) is single-threaded upstream and cannot be cored
  internally. The **forward edge is graph carving** — right-sizing the reasoning
  graph to hardware (the **C-series**, gating v0.7) so the single-threaded
  reasoner only ever closes a graph that fits the box. pgRDF owns the write-back
  (already batched) and the `rdfs` profile (its own forward-chain — candidate for
  parallel rule application). True ∝delta is engine-ask #2 (incremental).
- **Query (multi-hop)** — already parallel; the M4 pin keeps the plan sane. Tune
  `max_parallel_workers_per_gather` on the big-node config.
- **Validation** — SHACL (`rudof`) single-core; parallel-over-shapes is an
  upstream/integration target.

Each is its own measured slice. Ingest parallelism is delivered (v0.6.14); the
next frontier is making reasoning fit the box (carving) rather than coring the
upstream reasoner.

## 10. Execution vehicles

1. **Plain Azure VM** (the chosen path, §7) — SSH in, native Docker. For the
   LUBM class, run `tests/perf/benchmark-runner.sh` against a glibc/bookworm
   Postgres-17 image carrying the published pgRDF extension + the Tbox. For the
   at-scale class (Part A), the same image with `shared_preload_libraries=pgrdf`
   and a routed temp tablespace, driving `load_turtle_staged_run` on the `.nt`
   dump. Full RAM, no nesting.
2. **k8s DaemonSet** (`tests/perf/lubm/k8s-daemonset/`) — declarative per-node run
   for AKS (LUBM class); honest caveat: AKS pods cap RAM below the node + dind
   adds IO overhead, so it OOMs the tuned config on 32 GiB nodes. Use ≥ 64 GiB
   nodes or default `shared_buffers`. Not the vehicle for at-scale ingest (which
   needs the full host RAM + a dedicated temp tablespace).

**Container tool requirement (gate):** `benchmark-runner.sh` drives the DB via
`docker exec <sidecar> psql` and `pg_isready`, so the benchmark image **MUST**
carry `psql` + `pg_isready` on PATH. Stock `postgres:17.4-bookworm` does;
slimmed images must be confirmed to include the `postgresql-client` tools before
use (verify with
`docker run --rm --entrypoint sh <img> -c 'command -v psql pg_isready'`). The
extension `.so` is the **glibc-bookworm** build (the published
`pgrdf-<v>-pg17-glibc-{amd64,arm64}.tar.gz` / `ghcr.io/styk-tv/pgrdf-bundle`),
so the image base must be glibc/bookworm — not musl/alpine.

## 11. Cross-references

- Low-level design: **`specs/SPEC.pgRDF.LLD.v0.6.14.md`** — §3 ingest paths,
  **§4 the staged loader** (phases, GUCs, self-tune, resume-safety), §6 the query
  translator (M4 pin), §7 materialize (auto-ANALYZE) + validate.
- At-scale results: the **`33-pgRDF-wikidata-scale/RESULTS`** companion (a private
  benchmark repo — referenced by name; no private paths/hosts in this spec).
- LUBM measured evidence: `tests/perf/lubm/RESULTS.m4-join-order.md` (v0.5.45 →
  v0.6.x full passes, per-query tables).
- Runner: `tests/perf/benchmark-runner.sh` (the canonical LUBM driver).
- DaemonSet: `tests/perf/lubm/k8s-daemonset/` (AKS vehicle, LUBM class).
- Forward optimization: graph carving (the C-series, gating v0.7); the reasoner
  wall is issue #1.
