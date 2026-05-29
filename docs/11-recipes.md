# 11 — Operational recipes

Short, copy-pasteable recipes for two stock-Postgres extensions that
play well with pgRDF. Neither is a hard dependency; both are
documented as samples for the cases where they're useful.

## `pg_prewarm` — warming pgRDF tables (TE-3)

`pg_prewarm` is contrib (stock PostgreSQL). On a freshly-started
postgres process, pgRDF's hexastore tables and the dictionary are
cold; the first batch of queries pays the cost of pulling pages off
disk into shared_buffers. For benchmarks and warm-path measurements,
pre-loading the relevant relations gives a much more stable
baseline.

```sql
-- One-time setup (idempotent; ignores if already installed).
CREATE EXTENSION IF NOT EXISTS pg_prewarm;

-- Warm pgRDF's hot tables. The optional second arg picks the strategy:
--   'buffer'   → load into shared_buffers (default)
--   'read'     → just read pages from disk (warm the OS page cache)
--   'prefetch' → posix_fadvise; lighter, async
SELECT pg_prewarm('pgrdf._pgrdf_dictionary');
SELECT pg_prewarm('pgrdf._pgrdf_quads');

-- Warm the indexes pgRDF queries hit most often.
SELECT pg_prewarm('pgrdf._pgrdf_dictionary_pkey');
SELECT pg_prewarm('pgrdf._pgrdf_quads_pkey');
SELECT pg_prewarm('pgrdf._pgrdf_quads_spo_idx');
SELECT pg_prewarm('pgrdf._pgrdf_quads_pos_idx');
SELECT pg_prewarm('pgrdf._pgrdf_quads_osp_idx');
```

### When to use this

- **Benchmark stability.** First-query latency is uninteresting for
  perf comparisons; the warm path is what you want to measure.
  `pg_prewarm` before timing.
- **Cold-restart steady-state.** After a postmaster restart, the
  page cache empties. If your workload is consistently warm in
  production, warm before letting traffic hit.

### When NOT to bother

- Single-shot queries against small graphs (under a few MB total).
  shared_buffers warms during the first query anyway; the extra
  step doesn't change wall-clock measurably.
- If `shared_buffers` is much smaller than your hexastore. You'll
  thrash the cache loading tables you can't keep resident; the
  recipe expects `shared_buffers ≥ working-set`.

### Caveats

- `pg_prewarm` does not preserve across restarts. For autostart
  recipes, see the `pg_prewarm.autoprewarm` setting in PG docs.
- The `_pgrdf_quads_spo_idx` / `_pgrdf_quads_pos_idx` /
  `_pgrdf_quads_osp_idx` index names match pgRDF's hexastore at
  v0.5; if an internal storage change renames them, this recipe
  needs updating. The relation names are stable per LLD v0.5 §2.

## `pg_stat_statements` — aggregating pgRDF's translated SQL (TE-2)

`pg_stat_statements` (also stock contrib) tracks execution
statistics per normalised SQL statement. pgRDF translates SPARQL
into parameterised SQL where dictionary IDs flow through as `$N`
placeholders — the prepared SQL is **statement-id stable** across
calls, so `pg_stat_statements` aggregates correctly.

```sql
CREATE EXTENSION IF NOT EXISTS pg_stat_statements;

-- Add to postgresql.conf and restart:
--   shared_preload_libraries = 'pgrdf,pg_stat_statements'
--   pg_stat_statements.track = all
--   pg_stat_statements.max = 10000

-- Then, after a workload run:
SELECT
    calls,
    round(total_exec_time::numeric, 2) AS total_ms,
    round(mean_exec_time::numeric, 3)  AS mean_ms,
    rows,
    substring(query, 1, 120) AS query_prefix
FROM pg_stat_statements
WHERE query ILIKE '%_pgrdf_quads%'
   OR query ILIKE '%_pgrdf_dictionary%'
ORDER BY total_exec_time DESC
LIMIT 20;
```

### What you'll see

Each distinct SPARQL query lowers to one (or a small number of)
SQL statements. `pg_stat_statements` reports them with the dict-id
parameters normalised away — so two SPARQL queries that differ only
in their literal IRIs aggregate to the same row, with `calls`
incremented appropriately. That's the contract pgRDF's translator
maintains:

- **Constant predicates** (e.g. `{ ?s rdf:type ex:Class }`) translate
  to the same prepared statement regardless of which `ex:Class` is
  named at the SPARQL level; the class IRI resolves to a dict-id
  bound as `$N`.
- **Plan-cache hits** in pgRDF correspond 1:1 to repeat hits on the
  same `pg_stat_statements` row (see `pgrdf.plan_cache_stats()`).
- **Aggregates over UNION** produce a single CTE-shaped SQL whose
  text is stable; UNION branches don't fork the statement id.

### When to use this

- Production observability for SPARQL workload mix.
- Cross-checking which SPARQL forms are hot vs. tail.
- Verifying the plan cache is doing its job — if a `pg_stat_statements`
  row's `calls` count is high but pgRDF's plan-cache miss counter
  for the same SPARQL shape is also high, something has invalidated
  the prepared plan.

### Caveats

- `pg_stat_statements.max` defaults to 5000 in stock postgres. A
  busy pgRDF workload with many distinct SPARQL shapes can fill it;
  bump to 10000 or 50000 as needed.
- Statement IDs change across pgrdf releases when the translator
  changes (rare, but happens with major SPARQL feature additions).
  Don't pin reports to specific statement IDs across upgrades.

## See also

- [`03-query.md`](03-query.md) — SPARQL surface that the prepared
  SQL lowers from.
- [`06-installation.md`](06-installation.md) — `shared_preload_libraries`
  ordering when pgRDF + pg_stat_statements both want it.
- [`09-release.md`](09-release.md) — what changes between releases
  (the place to check before assuming statement IDs survived an
  upgrade).
