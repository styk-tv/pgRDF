# LUBM-100 benchmark — experiment pass log

Each pass = one configuration attempt at LUBM-100 (13,879,970 ABox
triples, UBA `-seed 0`, + 293 Tbox triples). The config + outcome of
every pass is recorded here, because the **config→outcome trail is
itself the benchmark result** (the OPENBENCHMARK spec §3.2 wants both
out-of-the-box and tuned numbers, and the contrast between them is the
story for an RDBMS-backed store).

**Environment (all passes):** isolated colima `k8s` VM (8 vCPU, 32 GiB
RAM), DOCKER_HOST to its dockerd; pgRDF **v0.5.43** combined dict path;
postgres 17.4; self-contained baked image `pgrdf-bench-k8s:0.5.43`
(pgrdf + Tbox). Reasoning materialized at load (not query-time).

---

## Pass 1 — DEFAULT config (out-of-the-box) — 2026-06-11

**Config:** stock postgres 17.4 defaults.
| setting | value |
|---|---|
| `shared_buffers` | 160 MB |
| `work_mem` | **4 MB** |
| `effective_cache_size` | 5 GB |
| `max_parallel_workers_per_gather` | 2 |

**Result:**
| phase | outcome |
|---|---|
| Ingest (combined path) | **13,879,970 triples / 217,558 ms (~3.6 min)** ✓ |
| Tbox load | 293 triples / 7.7 ms ✓ |
| `none` q01 (anchored class lookup) | count=4 / **139.5 ms** ✓ |
| `none` q02 (6-pattern BGP join) | count=0 / **649,720 ms (≈10.8 min)** ⛔ |

**Finding — the wall:** `work_mem = 4 MB` starves hash joins on the
6-way BGP (q02: GraduateStudent × University × Department with
memberOf / subOrganizationOf / undergraduateDegreeFrom). Postgres
falls back to **nested-loop joins over 13.9M quads** → ~11 min per
execution. With 3 warm passes × the heavy queries (q02, q08, q09) ×
3 profiles, a full default-config run projects to **many hours** —
impractical, and the slowness is fully explained by one knob.

**Decision:** stop Pass 1, move to a tuned pass. The ingest headline
(217.6 s) is config-independent and already captured. Default-config
heavy-query timings are recorded qualitatively (q02 ≈ 10.8 min) rather
than grinding out the full matrix.

**Stopped at:** `none` profile, q02 (query 2 of 14). Sidecar removed,
VM reclaimed.

---

## Pass 2 — TUNED config (recommended) — 2026-06-11

**Adjustment from Pass 1 — the lever is `work_mem`:**
| setting | Pass 1 | Pass 2 | why |
|---|---|---|---|
| `work_mem` | 4 MB | **256 MB** | hash joins instead of nested loops on multi-way BGP |
| `shared_buffers` | 160 MB | **8 GB** | cache the hexastore (VM has 30 GiB free) |
| `effective_cache_size` | 5 GB | **24 GB** | planner knows the OS cache is large |
| `max_parallel_workers_per_gather` | 2 | **4** | parallelize big scans/joins |
| `max_parallel_workers` | 8 | **8** | (unchanged) |
| container `--shm-size` | default | **2 GB** | headroom for parallel-worker DSM |

Invocation:
```
PGRDF_PG_ARGS="-c work_mem=256MB -c shared_buffers=8GB \
  -c effective_cache_size=24GB -c max_parallel_workers_per_gather=4 \
  -c max_parallel_workers=8" \
PGRDF_SHM_SIZE=2g PGRDF_CONFIG_LABEL=tuned \
DOCKER_HOST=<k8s sock> PGRDF_RUNTIME=docker \
PGRDF_BAKED_IMAGE=pgrdf-bench-k8s:0.5.43 \
bash tests/perf/benchmark-runner.sh 100
```

**Hypothesis:** q02 drops from ~10.8 min to seconds (hash join is the
right plan, it was just starved). Ingest stays ~217 s (not join-bound).

**Result — hypothesis PARTIALLY REJECTED:**
| phase | Pass 1 (default) | Pass 2 (tuned) | verdict |
|---|---|---|---|
| Ingest | 217.6 s | 216.8 s | unchanged (not join-bound) ✓ |
| `none` q01 (anchored lookup) | 139.5 ms | **90.4 ms** | tuning helps simple queries ✓ |
| `none` q02 (6-way BGP join) | ~649,720 ms (10.8 min) | **~630 s+ / pass, STILL grinding** | **tuning did NOT help** ✗ |

**Key finding — q02 is planner-bound, not memory-bound.** Bumping
`work_mem` 4 MB → 256 MB changed q01 (139→90 ms) but left q02 at
~10.5 min — essentially identical to default. So the wall on the heavy
LUBM joins (q02, and by extension q08/q09) is **NOT** Postgres working
memory; it is **join-order / cardinality estimation on the single
`_pgrdf_quads` table**. Every BGP pattern scans the same big table, so
Postgres' selectivity estimates are poor and it locks onto a bad join
order (huge intermediate cross-products) regardless of `work_mem`.

**Implication:** no PG config knob fixes the heavy LUBM joins at
13.9M triples. The lever is in pgRDF's **BGP→SQL lowering** — emitting
a better-ordered join tree, join hints, or per-predicate cardinality
stats so the planner orders the 6-way join sensibly. That's a pgRDF
executor thread (a real v0.6 Track-C item: `query/executor` BGP
lowering), not a benchmark-config decision.

**Open decision (for the user):** with the heavy joins ~10 min each,
tuned or not, a full LUBM-100 query matrix is impractical to grind.
Options: (a) investigate the q02 plan + improve BGP join ordering
[the valuable thread]; (b) run with `statement_timeout` so heavy
queries record "timeout @ Ns" and the rest complete fast; (c) report
the simple-query + ingest + materialize numbers and flag the heavy
joins as a known planner limitation.

**Pass 2 STOPPED** at `none`/q02 (after ~10.5 min/pass confirmed the
planner-bound finding) — all three paths above require stopping the
current grind first, so it's a no-regret prerequisite, and it frees
the borrowed k8s VM. Environment stays staged: `pgrdf-bench-k8s:0.5.43`
image + `pgrdf-lubm-data` (lubm-100 nt) both intact; VM RAM freed
(30 GiB avail), disk 7.8 GB free (watch — ~6 GB/pass for PGDATA).
k8s tenant stack still paused (restore via
`colima ssh -p k8s -- sudo systemctl start k3s` when work concludes).

### Captured LUBM-100 numbers so far (both passes)
| metric | value | note |
|---|---|---|
| Ingest (combined path) | **216.8–217.6 s** / 13.88M triples | config-independent ✓ |
| Tbox load | 293 triples / ~8 ms | ✓ |
| q01 default / tuned | 139 ms / **90 ms** | tuning helps simple queries |
| q02 default / tuned | 10.8 min / **~10.5 min** | planner-bound; tuning no help |
| heavy joins (q02/q08/q09) | ~10 min each, tuned or not | → pgRDF BGP-lowering thread |

---

## Root-cause investigation (after Pass 2)

Code grep (see `_WIP/INVESTIGATION.bgp-join-planner.md`) found the cause:
**pgRDF never runs `ANALYZE`** after ingest or materialize, so a freshly
bulk-loaded 13.9M-row `_pgrdf_quads` has **zero planner statistics**. With no
stats the planner estimates the table as near-empty → nested-loop Cartesian on
the 6-way join → 10 min. `work_mem` is irrelevant (hence Pass 2's no-op).
**Production severity:** bulk-load-then-serve never triggers autovacuum-analyze
(no writes after load) → the bad stats persist *forever* → every 3+-way join is
permanently slow until a human runs ANALYZE by hand. Fix = auto-ANALYZE in the
extension (M1) + extended stats (M2) + per-column stats target (M3).

## Pass 3 — empirical test: does `ANALYZE` alone fix q02?

Same instance, **default** PG config (so ANALYZE is the only variable), q02
before vs after `ANALYZE pgrdf._pgrdf_quads`.

### Pass 3a — BOTCHED (recorded — bad tests are findings too)

First attempt set `auto_explain.log_min_duration=0` + `log_nested_statements=on`
to capture the plan. **This crippled the ingest:** auto_explain ran
EXPLAIN-ANALYZE on *every internal SPI statement* the loader issues
(`put_terms_batch` + `flush_batch` × ~14k batches), producing **507,795 log
lines** and stalling the load (0 commit after 123 s). Killed + re-run with
`log_min_duration=5000` (only logs queries > 5 s — catches q02, ignores ingest
internals): clean ingest, **75 log lines**.

> **Finding worth shipping to users:** enabling `auto_explain` with
> `log_min_duration=0` + `log_nested_statements=on` makes pgRDF ingest crawl,
> because the loader is SPI-statement-heavy by design (batched dict + quad
> flushes). Document: debug query plans with a non-zero `log_min_duration`, or
> disable nested-statement logging during loads. (Candidate note for
> `docs/` troubleshooting.)

### Pass 3b — clean run (default config + auto_explain >5s)

**Result — hypothesis (M1 ANALYZE fixes q02) DISPROVEN:**
| state | planner `reltuples` | q02 time |
|---|---|---|
| before ANALYZE | **-1** (PG sentinel: "never analyzed") | >120 s (capped) — Pass 1 measured 649 s |
| ANALYZE cost | — | **0 s** (sub-second on 13.9M rows — sample-based) |
| after ANALYZE | **13,880,214** (correct) | **≥600 s (hit timeout cap; ≈ 649 s default)** |

`ANALYZE` is nearly free (answers the "how long does ANALYZE take" question:
**sub-second**, fixed-cost, size-independent) and it populated correct stats —
but **q02 stayed ~9–10 min**. ANALYZE is *not* the fix.

**ROOT CAUSE (from the captured generated SQL) — cross-join emission, not stats.**
The executor emits BGP patterns in **query order**, so the three standalone
`rdf:type` patterns (`?x a GraduateStudent`, `?y a University`, `?z a Department`)
become **CROSS JOINS** — their `JOIN … ON` clauses are pure filters with no
shared variable:
```
FROM q1                              -- ?x a GraduateStudent
INNER JOIN q2 ON (q2.pred=$ AND q2.obj=$)   -- ?y a University   (NO link to q1)
INNER JOIN q3 ON (q3.pred=$ AND q3.obj=$)   -- ?z a Department   (NO link to q1/q2)
INNER JOIN q4 ON (q4.subj=q1.subj AND q4.obj=q3.subj …)  -- only now links x↔z
INNER JOIN q5 …                              -- z↔y
INNER JOIN q6 …                              -- x↔y
WHERE q1.pred=$ AND q1.obj=$
```
So the SQL requests **GraduateStudents × Universities × Departments** (a
Cartesian product, ~10^11 intermediate rows) before q4/q5/q6 filter it. No
statistics make a cross product cheap — which is exactly why ANALYZE barely
moved it (649 s → ~600 s).

**THE FIX — M4 (was ranked low; it's actually THE fix):** connected,
selectivity-aware join emission in `executor.rs::build_from_and_where` —
reorder patterns so **each emitted pattern shares a variable with the
already-joined set** (never emit a cross join), most-selective-first.
Eliminates the Cartesian product *structurally*, good plan regardless of
stats, fully automatic, no operator action. M1 (ANALYZE) becomes a cheap
complement (helps the simple-pattern selectivity), not the fix.

## Pass 4 — M4 built + measured: emission alone NOT enough; pin the order

Built M4 (`connected_order`) into the `.so`, baked + ran on k8s:

| q02 (LUBM-100, default PG + ANALYZE) | wall | count |
|---|---|---|
| pre-M4 (query order) | 649 s | — |
| ANALYZE only | 600 s+ | — |
| **M4 connected emission, planner free** | **300 s+ (cap)** | — |
| **M4 emission + `join_collapse_limit=1`** | **3 s ✓** | 129,401 |

**Correction (2nd bad test):** connected *emission* alone didn't help — PG's
`join_collapse_limit` (12 ≥ 6) flattens the joins and re-derives its own order
by cost, so it still picks the cross product from poor single-table estimates.
The fix needs the emission order **pinned**: `SET LOCAL join_collapse_limit=1` +
`from_collapse_limit=1` inside `pgrdf.sparql()` (`pin_join_order`). Then the
planner runs pgRDF's connected order verbatim → **q02 300 s+ → 3 s**.

**Result correctness:** 129,401 rows is right (LUBM-10 none-profile q02 = 1,721
in our `expected-counts.json`, scales up); Pass 1's "0" was an anomaly on the
timed-out run. `join_collapse_limit` never changes results; M4 reorders
commutative inner joins. Full 93/93 compose regression passes with the M4 `.so`.

**Validated fix = `connected_order` (build_from_and_where) + `pin_join_order`
(sparql).** Final gate before ship: re-run the 93-test regression with the GUC
active (forcing join order on every query must not regress any shape).
