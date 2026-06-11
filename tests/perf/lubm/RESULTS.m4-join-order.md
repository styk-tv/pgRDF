# LUBM benchmark — M4 connected join ordering (v0.5.45)

The headline result of the v0.5.45 cut: the multi-pattern BGP join blowup that
made complex SPARQL queries minutes-slow on large graphs is fixed, with **no
operator action** — no manual indexes, no `ANALYZE`, no PG config tuning.

## Result

LUBM Q2 (a 6-pattern, 3-variable triangle join — the worst case), freshly
loaded graph, **default PostgreSQL config, no `ANALYZE`**:

| build | LUBM-100 Q2 |
|---|---|
| v0.5.43 (pre-M4), planner free | **649 s** (10.8 min) |
| v0.5.43 + manual `ANALYZE` | **600 s+** (ANALYZE alone does not fix it) |
| **v0.5.45 (M4)** | **~3 s** (≈ **200× faster**), 129,401 rows |

All 14 LUBM queries on a freshly loaded graph (lubm-50, 6.89M triples, **no
`ANALYZE`**, M4): every query returns in **0–1 s**.

```
q01 0s/4    q02 1s/32923   q03 0s/6     q14 1s/393730   (data-bearing)
q04–q13  0s/count=0   (empty at none-profile — require owl-rl reasoning to
                       return rows; expected LUBM behaviour, not a slow path)
```

## What the fix is

Two parts, both inside the extension, both automatic:

1. **`connected_order` (`executor.rs::build_from_and_where`)** — the mandatory
   BGP is emitted in a *connected, selectivity-ordered* sequence: each pattern
   after the seed shares a variable with the already-placed set, so no `INNER
   JOIN` is ever a cross join. Previously patterns emitted in query order, so
   standalone patterns (e.g. Q2's three `rdf:type` patterns) became a Cartesian
   product (GraduateStudents × Universities × Departments ≈ 10¹¹ rows).
2. **`pin_join_order` (`sparql()`)** — `SET LOCAL join_collapse_limit = 1` +
   `from_collapse_limit = 1` so PostgreSQL honours pgRDF's emitted order instead
   of re-deriving its own (which, on the single-table `_pgrdf_quads` store with
   poor cardinality estimates, picks the cross product). `SET LOCAL` is
   txn-scoped and auto-resets.

Connected emission alone is *not* enough — the planner re-flattens and re-orders
the joins unless pinned. Both parts are required.

### Why no `ANALYZE` is needed

With the order pinned, the planner doesn't need cardinality estimates to choose
it, and each pinned join hits a hexastore index (SPO/POS/OSP) via its equality
predicate — index scans regardless of statistics. Measured: Q2 is the same 1 s
with `reltuples = -1` (never analyzed) as after `ANALYZE`. So the fix works
out-of-the-box on a bulk-loaded, never-analyzed graph.

## Correctness

- **Result-preserving.** 93/93 compose regression tests pass with M4 active —
  `join_collapse_limit` constrains plan *search* only, never the result set, and
  M4 reorders commutative inner joins. Q2's 129,401 count is correct (LUBM-10
  none-profile Q2 = 1,721 in `expected-counts.json`; scales consistently).

## Environment

| | |
|---|---|
| Host | Colima `k8s` VM — 8 vCPU, 32 GiB RAM, aarch64 (Apple Silicon), Docker |
| Postgres | `postgres:17.4-bookworm`, **default config** (+ `work_mem=64MB`) |
| PGDATA | tmpfs (RAM) — isolates the measurement from disk I/O |
| Extension | pgRDF 0.5.45 (M4), glibc-bookworm `.so`, `shared_preload_libraries=pgrdf` |
| Dataset | LUBM via UBA 1.7 generator (`-seed 0`); lubm-50 = 6.89M triples, lubm-100 = 13.88M |
| Profile | none (no reasoning) — Q2's join is asserted-triple-only |

## Scope

v0.5.45 fixes the **none-profile** multi-hop blowup (Q2 the worst case). The
**full LUBM-100 pass across all profiles** — including the owl-rl materialized
profile where Q8/Q9 exercise heavy inferred-type joins — is the **v0.6.0** gate
and is not yet verified here.

---

# Addendum — M1 auto-ANALYZE after materialize (v0.5.46)

The materialized-profile complement. After `materialize('owl-rl')` (which adds
the `subOrganizationOf` transitive closure — lubm-50: 6.89M → 11.15M quads),
Q2 regressed from 1 s back to **180 s+ (timeout)**: materialize wrote millions
of rows but never refreshed statistics, so the planner mis-planned the
inflated joins. `ANALYZE` was the missing piece (verified: extended
`CREATE STATISTICS` adds nothing further — regular ANALYZE + M4's pinned
order suffices).

**Fix:** `pgrdf.materialize` now runs `ANALYZE pgrdf._pgrdf_quads` at its tail
(GUC `pgrdf.auto_analyze`, default on; `auto_analyzed` field in the JSONB).

| owl-rl materialized graph (lubm-50, 11.15M quads, NO manual ANALYZE) | Q2 |
|---|---|
| v0.5.45 (M4 only) | **180 s+ (timeout)** |
| **v0.5.46 (M4 + M1)** | **2 s** (32,923 rows) |

Materialize wall cost of the ANALYZE: 240 s → 250 s (noise-level). All 14 LUBM
queries: 0–2 s on **both** profiles at lubm-50. Validation: 93/93 regression
with M1 active; GUC-off honored. Same environment as above (k8s VM docker,
`postgres:17.4-bookworm` default config, tmpfs PGDATA).
