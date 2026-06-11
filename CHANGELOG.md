# Changelog

All notable changes to pgRDF are tracked here. Format follows
[Keep a Changelog](https://keepachangelog.com/). Versioning is SemVer
once we cut v1.0; pre-1.0 minor bumps may include breaking changes.

## [Unreleased]

### Fixed (M4 — BGP join-order blowup on large graphs; v0.5.45) ★ headline

Complex multi-pattern SPARQL queries (3+ way BGP joins) ran **minutes-slow on
large graphs** and were effectively unusable at scale. On LUBM-100 (13.9M
triples) Q2 — a 6-pattern, 3-variable triangle join — took **649 s**. Root
cause: the executor emitted BGP patterns in **query order**, so standalone
patterns (Q2's three `rdf:type` patterns) became **cross joins**
(GraduateStudents × Universities × Departments ≈ 10¹¹ intermediate rows), and
PostgreSQL's `join_collapse_limit` re-derived its own (still cross-product)
order from the single-table store's poor cardinality estimates. **`ANALYZE`
does not fix it** (no statistics make a Cartesian product cheap — verified:
Q2 stayed 600 s+ after ANALYZE).

Two-part fix, both inside the extension, both fully automatic — **no manual
indexes, no `ANALYZE`, no PG config**:

- **`executor.rs::build_from_and_where` → `connected_order`** — reorders the
  mandatory BGP into a *connected, selectivity-ordered* sequence: each pattern
  after the seed shares ≥1 variable with the already-placed set, so no emitted
  `INNER JOIN` is ever a cross join. A genuinely disconnected BGP component
  keeps its (semantically required) cross join.
- **`executor.rs::sparql` → `pin_join_order`** — `SET LOCAL
  join_collapse_limit = 1` + `from_collapse_limit = 1` so PostgreSQL honours
  pgRDF's emitted order verbatim instead of re-flattening it. `SET LOCAL` is
  txn-scoped (auto-resets after a bare autocommit `SELECT pgrdf.sparql(...)`).
  Connected emission alone is **not** sufficient — both parts are required.

**Result (LUBM-100 Q2, default PG, no `ANALYZE`): 649 s → ~3 s (≈ 200×)**,
result identical (129,401 rows). All 14 LUBM queries on a freshly loaded
lubm-50 graph (6.89M triples, **`reltuples = -1`, never analyzed**) return in
**0–1 s** — the fix works out-of-the-box because each pinned join hits a
hexastore index via its equality predicate (index scans regardless of stats).

**Result-preserving:** `join_collapse_limit` constrains plan *search* only,
never the result set; M4 reorders commutative inner joins. 93/93 compose
regression tests pass with M4 active. See
`tests/perf/lubm/RESULTS.m4-join-order.md` for the full measurement +
environment (Colima k8s VM 8 vCPU/32 GiB, `postgres:17.4-bookworm`, tmpfs
PGDATA). The owl-rl materialized-profile heavy joins (Q8/Q9) and the **full**
LUBM-100 pass remain the **v0.6.0** gate.

### Measured (LUBM-10 under the final combined ingest path — first post-TA-7 baseline)

v0.5.43 re-runs the LUBM-10 benchmark against the now-final combined
dict path (`pgrdf.ingest_dict_path = 'combined'`, the v0.5.37+
default) and commits the result as a persistent comparison anchor.
This is the measurement gate before the LUBM-100 first run.

- **Headline (combined v0.5.43 vs pre-TA-7 baseline v0.5.36)**: ingest e2e **24,505 ms → 16,539 ms (−32.5 %)**; ingest dict phase **17,331 ms → 9,746 ms (−43.8 %)** — the combined path's batched + hot-cache dict resolution paying off at 10× LUBM-1 scale. Insert phase flat (5,507 → 5,351 ms; the insert path was deliberately left unchanged per the TA-9 decision). Materialize is reasoner-bound (the `reasonable` crate's forward-chaining) and unchanged within run noise: RDFS ~39 s / 287,422 inferred, OWL-RL ~63 s / 815,968 inferred.
- **Correctness: byte-identical.** Same `dict_db_calls` (315,060), same `dict_cache_hits` (4,057,182), same `triples_inferred` per profile (287,422 RDFS / 815,968 OWL-RL), and **zero Q1-Q14 count drift** — every cell still matches the locked `tests/perf/lubm/queries/expected-counts.json`. The combined path changed *how fast* ingest resolves the dictionary, not *what* it produces.
- **`tests/perf/lubm/baseline.lubm-10.combined.json`** + **`baseline.lubm-10.combined.md`** (new) — committed reference snapshot in the richer `benchmark-runner.sh` shape (full ingest Phase-0 breakdown + RDFS/OWL-RL materialize + Q1-Q14 per profile), with a `comparison_vs_pre_ta7_baseline` block and ±30 % timing tolerance. Volatile fields (timestamp, host, git sha/branch) are stripped so the file is a stable run-to-run anchor. The older `baseline.lubm-10.json` (the `run-lubm.sh` contract consumed by `compare-to-baseline.py`) is left in its own schema, untouched.

### Fixed (parse-timer accounting bug in `ingest_turtle_combined`, surfaced by the LUBM-10 measurement)

- **`src/storage/loader.rs`** — `ingest_turtle_combined` drove the parser with `for triple_result in iter`, which polls `iter.next()` (the actual parse work) at the top of each iteration BEFORE the in-body `t_parse` timer starts. The timer therefore measured only the trivial `Result::expect()` unwrap, attributing ~0 ns to parse and leaking the real parse time into the unaccounted per-iteration gap. The LUBM-10 run exposed it: the combined path reported `parse_ms = 29` against the baseline path's honest `1549`. Fixed by driving the parser with an explicit `loop { let t = now(); let next = iter.next(); parse_ns += t.elapsed(); … }` (mirroring the `ingest_turtle_with_stats` baseline shape) so the timer wraps the real `next()` call. Verified honest: LUBM-1 now reports `parse_ms = 107` (was ~2), and the three phase timers sum to within ~0.6 % of `elapsed_ms`. Behavior-preserving — 289/289 pgrx + 93/93 regression green. The quad combined path (`ingest_quads_combined`) was unaffected: it carries no phase timers and honestly reports `parse_ms = 0` on the quad surface (documented in `quad_stats_to_jsonb`).

### Verified locally

- 289/289 pgrx tests pass (loop restructure is behavior-preserving).
- 93/93 regression tests pass.
- LUBM-10 e2e 16,539 ms; zero Q1-Q14 drift.

### Changed (six sources of truth, mechanical bump 0.5.42 → 0.5.43)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output, **`README.md`** Status badge + Status row + Install row + Quickstart example. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.42.sql` → `sql/pgrdf--0.5.1--0.5.43.sql` (no-op; SQL surface unchanged).

### Added (TA-4 — dict-path parity matrix completed across formats × routes)

v0.5.42 is a **test-only** release closing the dict-path correctness
invariant before the Track A ship-it. The SQL regression gates 130
(Turtle) + 132 (N-Quads/TriG) already locked dict-path parity against
the compose Postgres; v0.5.42 adds the same invariant at the **pgrx
unit level** (CI's `test (17)` job, which runs against the freshly
built `.so` — a distinct execution path from the pg_regress-style
runner) and completes the TriG × 4-paths coverage that 132 previously
only spot-checked in `combined` mode.

- **`src/storage/loader.rs`** — three new `#[pg_test]` matrix tests + a shared `assert_path_matrix_parity` helper. `dict_path_matrix_turtle` / `dict_path_matrix_nquads` / `dict_path_matrix_trig` each ingest the same blank-node-free fixture (URI / typed-literal / lang-literal / plain-string / IRI-object term shapes) under all four `pgrdf.ingest_dict_path` values (`baseline` / `batched` / `shmem_warm` / `combined`) into four distinct graphs, then assert via a single aggregate query that all four graphs hold the exact same decoded-lexical triple set (`count(DISTINCT (s,p,o,o_type,o_has_dt,o_lang))` over the four graphs equals the per-graph count, all four graphs present, each holding exactly the expected quad count). The fixture deliberately carries no blank nodes so every triple compares by decoded lexical value with no parser-assigned-label caveat.
- **`tests/regression/sql/132-quad-dict-paths-parity.sql`** — assertion `E` upgraded from a single `combined`-only TriG smoke to a full TriG × 4-paths parity check (same fixture ingested under each `ingest_dict_path` value into per-path-distinct default graphs; decoded-lexical triple sets asserted identical across all four). The former TriG named-graph routing check is retained as assertion `F`. Gate grows from 5 → 6 assertions.

### Verified locally

- 289/289 pgrx tests pass (was 286 — the 3 new matrix tests included).
- 93/93 regression tests pass (132 now exercises 6 assertions; total test count unchanged).

### Changed (six sources of truth, mechanical bump 0.5.41 → 0.5.42)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output, **`README.md`** Status badge + Status row + Install row + Quickstart example. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.41.sql` → `sql/pgrdf--0.5.1--0.5.42.sql` (no-op; SQL surface + runtime both unchanged — TA-4 is test-only).

### Added (TA-5 — verbose-ingest JSONB reports the dispatched `path`)

v0.5.41 adds a `path` field to the verbose-ingest JSONB output of
the dispatched ingest UDFs (`parse_turtle_verbose` /
`load_turtle_verbose` / `parse_trig` / `parse_nquads`). It reports
which `pgrdf.ingest_dict_path` route the dispatcher actually
selected for the call — `baseline` / `batched` / `shmem_warm` /
`combined`. Before this, a caller could SET the GUC but had no way
to confirm the route a given ingest took except by inferring it
from timing; the benchmark harness + operators now read it
directly off the result.

- **`src/query/guc.rs`** — `IngestDictPath::as_str()` returns the canonical lowercase route name (`baseline` / `batched` / `shmem_warm` / `combined`) matching the GUC enum values.
- **`src/storage/loader.rs`** — `LoaderStats` gains a `path: &'static str` field. `ingest_dispatch` (Turtle) and `ingest_quads_dispatch` (TriG/N-Quads) set `stats.path = path.as_str()` after the chosen ingest function returns, so the recorded route reflects the dispatcher's actual decision (notably: `baseline` and `shmem_warm` share the same physical ingest function but record distinct path strings). Both `stats_to_jsonb` and `quad_stats_to_jsonb` emit the `path` field. The `parse_turtle_dict_batched` spike UDF continues to override `path` with its own `dict_batched` discriminator after serialization (unchanged — it's a separate explicit surface, not a GUC-dispatched route).
- **`tests/regression/sql/133-verbose-path-field.sql`** + **`tests/regression/expected/133-verbose-path-field.out`** — TA-5 correctness gate. Sets each of the four `ingest_dict_path` values in turn and asserts the verbose JSONB echoes it back for the Turtle path (`parse_turtle_verbose`), the N-Quads path (`parse_nquads`), and the TriG path (`parse_trig`); also locks the unrecognised-value → `combined` fallback. 8 boolean assertions all `t`.

### Verified locally

- 286/286 pgrx tests pass (existing verbose-JSONB assertions unaffected — they field-probe, not exact-match).
- 93/93 regression tests pass (was 92 — the new 133 path-field gate included).

### Changed (six sources of truth, mechanical bump 0.5.40 → 0.5.41)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output, **`README.md`** Status badge + Status row + Install row + Quickstart example. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.40.sql` → `sql/pgrdf--0.5.1--0.5.41.sql` (no-op; SQL surface unchanged — TA-5 is a runtime-only change in the `.so`'s JSONB serialization).

### Added (TA-6 — TriG + N-Quads ingest routes through the combined dict path)

v0.5.40 extends TA-7's `pgrdf.ingest_dict_path` dispatch from
`parse_turtle` / `load_turtle` to the quad-stream UDFs
`pgrdf.parse_trig` + `pgrdf.parse_nquads`. Before this, the quad
path always used the legacy per-term `put_term_full` SPI path
(it was written before the TA-D3/TA-D2/TA-7 dict-path work
landed); now it honours the same four-way GUC switch
(`baseline` / `batched` / `shmem_warm` / `combined`, default
`combined`) the Turtle path uses.

- **`src/storage/loader.rs`** — three new internal pieces:
  - **`ingest_quads_combined`** — combined dict path for the quad stream. Mirrors `ingest_turtle_combined`'s single-pass design (shmem hot-cache check first → defer queue for misses → bulk-resolve via `put_terms_batch` at `dict_batch_size` or before draining the pending buffer) but routes each resolved quad into its destination graph's `GraphBatches` partition instead of a single flat batch. The dictionary is global, so the defer queue is shared across graphs; only the quad routing is per-graph. Strict-mode semantics preserved: `resolve_graph_id` resolves (and under `strict` may reject) the destination graph BEFORE a quad's terms are queued, and a rejection panics — aborting the surrounding statement and rolling back every flushed dict row + quad batch, so no partial ingest survives.
  - **`drain_pending_quads_into_batches`** — quad analogue of `drain_pending_into_batch`: walks drained pending quads, looks up s/p/o ids from the resolved cache, routes each into its destination graph's batch partition via `GraphBatches::push`.
  - **`ingest_quads_dispatch`** — reads `pgrdf.ingest_dict_path` + applies the `shmem_prewarm_on_init` latch, then routes to `ingest_quads_with_stats` (baseline / shmem_warm) or `ingest_quads_combined` (batched / combined). `parse_trig` + `parse_nquads` now call this instead of `ingest_quads_with_stats` directly. There is no separate 2-pass quad spike (the quad path post-dates TA-D3), so `batched` maps to the same single-pass combined mechanism — both produce byte-identical dict + quad rows.
- **`src/storage/loader.rs`** — extracted **`subject_key`** + **`object_key`** `DictKey` builders shared by the Turtle and quad combined paths so both produce byte-identical dict rows (lang-tagged literals → `datatype_id = None`; every other literal including plain `xsd:string` → explicit datatype IRI dict id). `ingest_turtle_combined` refactored to use them (no behavior change — the inline logic was identical).
- **`tests/regression/sql/132-quad-dict-paths-parity.sql`** + **`tests/regression/expected/132-quad-dict-paths-parity.out`** — TA-6 correctness gate. Ingests the same N-Quads blob (3-position lines + one per-run-distinct named-graph line) under each of the four `ingest_dict_path` values into per-path-distinct graphs, then asserts: quad-count parity across all four runs; decoded-lexical quad equivalence (s,p,o,o_type,o_has_dt,o_lang) between `combined` and each of `baseline` / `batched` / `shmem_warm`; and that `parse_trig` also routes through the combined path (2 named-graph quads + 1 default-graph quad land correctly). 5 boolean assertions all `t`.

### Verified locally

- 286/286 pgrx tests pass (existing `parse_trig` / `parse_nquads` coverage unchanged + green through the new dispatch).
- 92/92 regression tests pass (was 91 — the new 132 quad-parity gate included).

### Changed (six sources of truth, mechanical bump 0.5.39 → 0.5.40)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output, **`README.md`** Status badge + Status row + Install row + Quickstart example. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.39.sql` → `sql/pgrdf--0.5.1--0.5.40.sql` (no-op; SQL surface unchanged — TA-6 is a runtime-only change in the `.so`).

### Parked (TA-NEW-W — schema migration to `UNIQUE NULLS NOT DISTINCT` deferred to v0.7+)

v0.5.39 attempted TA-NEW-W per the published release plan: migrate
`_pgrdf_dictionary.unique_term` to `UNIQUE NULLS NOT DISTINCT`
(PG 15+) so `INSERT ... ON CONFLICT DO NOTHING` could dedup
NULL-containing dict rows directly, replacing the
`WHERE NOT EXISTS` anti-join introduced in v0.5.37 and recovering
the small (~3%) e2e perf cost the workaround imposed. The
migration was code-correct (schema + upgrade SQL + put_terms_batch
revert all idempotent, structurally clean) but exposed a PG
concurrency hazard that blocks shipping the migration as-is.
**The migration is parked for v0.7+; v0.5.39 ships the analysis
as the durable record + bumps the version stack for cadence
hygiene.** No behavior change from v0.5.38.

- **What failed**: PG 17's NULLS NOT DISTINCT semantics make the constraint index treat NULL-bearing rows as duplicate-prone. INSERT's constraint-check acquires a per-index-leaf ShareLock on the inserting transaction's xid whenever it sees an in-progress conflicting row — even when the OTHER transaction inserts a DIFFERENT row that happens to share a leaf page. Under pgrx parallel test execution (~8 worker threads against a shared postmaster running START-TX → INSERT-dict-rows → INSERT-quad-rows → ROLLBACK cycles), the cross-TX ShareLock graph cycles within seconds: `ERROR: deadlock detected DETAIL: Process N waits for ShareLock on transaction T; blocked by process M. CONTEXT: while inserting index tuple (X,Y) in relation "unique_term"`. Reproduced 10–12 deadlocks per `cargo pgrx test` run, varying across `query::executor::tests::pg_*` per run. The deadlocks abort UNRELATED tests (the one the deadlock victim happened to be running), turning a stable 286/286 suite into 280/286 with no stable failure set.

- **Why this isn't a TA-NEW-W design bug**: the same migration would surface the same hazard against ANY parallel ingest workload — not just pgrx tests. Concurrent backends doing INSERT-many-then-rollback against a `UNIQUE NULLS NOT DISTINCT` index will hit this whenever their NULL-bearing inserts share index leaf pages. Shipping the migration requires ONE of: (a) a retry loop in `put_terms_batch` that catches `ERRCODE_T_R_DEADLOCK_DETECTED` (PG SQLSTATE `40P01`) and retries with bounded backoff — the cleanest path, matches standard PG application practice; (b) a sorted-input INSERT path so all parallel transactions touch index pages in globally-consistent order — eliminates the lock-graph cycle but adds per-call ordering cost; (c) an advisory-lock around dict writes — serialises ingest globally, defeats parallel-scale; (d) pgrx test infrastructure that serialises ingest tests — outside pgRDF's scope. Filed as **TA-NEW-W.v2** for v0.7+, gated on the retry-loop approach.

- **What v0.5.39 reverts**: `sql/schema_v0_2_0.sql` UNIQUE constraint back to default (NULLS DISTINCT) semantics; `sql/pgrdf--0.5.1--0.5.39.sql` body back to the no-op upgrade shape (filename increments per the bridge convention); `src/storage/dict.rs::put_terms_batch` keeps the `WHERE NOT EXISTS` anti-join from v0.5.37. The TA-NEW-W attempt + outcome live in `_WIP/DECISION.TA-NEW-W.nulls-not-distinct-deferred.md` (gitignored — the local working record; this CHANGELOG entry carries the publishable summary).

### Changed (six sources of truth, mechanical bump 0.5.38 → 0.5.39)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output, **`README.md`** Status badge + Status row + Install row + Quickstart example. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.38.sql` → `sql/pgrdf--0.5.1--0.5.39.sql` (no-op; schema unchanged). The release-plan table the user approved at end of v0.5.38 put `v0.5.39 = TA-NEW-W`; shipping the decision keeps the cadence honest and unblocks v0.5.40 (TA-6 — TriG + N-Quads route through combined path).

### Verified locally

- 286/286 pgrx tests pass.
- 91/91 regression tests pass.
- LUBM-1 ingest e2e: ~1,250 ms (unchanged from v0.5.38 — no behavior change shipped).

### Fixed (v0.5.37 regression suite turn-red: three bugs surfaced after the combined path landed as default)

The v0.5.37 commit shipped TA-7 + got the release chain green
(release.yml + oci-publish.yml + update-latest-md.yml — LATEST.md
points at 0.5.37), but the ci.yml `regression` job turned red:
75 pass / 16 fail. pgrx test suite stayed all-green throughout
because the failing scenarios only surface in the long-lived
compose postmaster used by the pg_regress-style runner. v0.5.38
ships the three root causes back-to-back. All 16 tests recover.

- **`src/storage/stats.rs`** `shmem_cache_prewarm_impl` — switched the per-row publish from `shmem_cache::insert_committed` (direct shmem slot write) to `shmem_cache::stage_for_commit` (the same transactional callback path the loader uses). **Why this matters**: when prewarm runs inside a transaction that has UNCOMMITTED `_pgrdf_dictionary` rows — exactly what happens when TA-7's auto-prewarm latch fires via `pgrdf.ingest_dict_path = 'shmem_warm'` after an earlier `parse_turtle` call in the same outer transaction wrote dict rows — the `SELECT * FROM _pgrdf_dictionary` sees those uncommitted rows via MVCC and the original `insert_committed` form pushed them to shmem IMMEDIATELY. On ROLLBACK, the dict rows vanish but shmem stays populated with `(fingerprint, id)` entries pointing to dict rows that no longer exist. A future backend's `shmem_cache::lookup` then returns those stale ids, the per-call cache binds them, the quad gets written with a dict_id whose row was rolled back, and downstream `SELECT FROM _pgrdf_dictionary WHERE lexical_value = 'Alice'` returns 0 rows because the dict row never made it past commit. Symptom: every regression test downstream of `130-ingest-dict-paths-parity` (which triggered the leak via its `shmem_warm` pass) returned 0 rows for the affected literals — 20-load-turtle, 21-typed-literals, 22-lang-tags, 31-44 sparql tests, every SPARQL-FILTER-LITERAL test in the suite. The `stage_for_commit` form ties the publish to the same commit/abort callback set the loader uses elsewhere, so the prewarm becomes transactional: on COMMIT entries land in shmem, on ABORT they're discarded along with the dict rows they reference.

- **`src/storage/loader.rs`** `ingest_turtle_combined` panic prefixes — changed `ingest_turtle_combined: invalid base IRI` → `load_turtle: invalid base IRI` and `ingest_turtle_combined: turtle parse error` → `load_turtle: turtle parse error` to match the baseline `ingest_turtle_with_stats` error contract that downstream tooling (CloudNativePG operators, client libraries, CI scripts) routes on. `tests/regression/sql/81-error-paths.sql` documents the contract: "the prefix says `load_turtle:` regardless of whether the caller entered through `pgrdf.load_turtle()` or `pgrdf.parse_turtle()`". The TA-7 combined path was leaking its internal function name through the panic message and breaking the lock-step contract.

- **`src/storage/loader.rs`** `ingest_turtle_dict_batched` literal datatype handling — applied the same baseline-equivalent rule (`if lang.is_some() { None } else { Some(<datatype IRI dict id>) }`) that the combined path now uses, including for plain `xsd:string` literals. The TA-D3 spike's original literal handling left `datatype_id = None` for plain `xsd:string` literals (an explicit short-circuit on the `xsd:string` IRI to skip dict resolution), which produces a dict row shape that doesn't round-trip via the SPARQL executor's term equality. With the combined path becoming default in v0.5.37 and using the corrected rule, the BATCHED path (still selectable via `SET pgrdf.ingest_dict_path = 'batched'`) became the odd one out — the `130-ingest-dict-paths-parity` regression test's `c_combined_subset_of_batched` assertion turned red because combined wrote literals with `datatype_iri_id = Some(<xsd:string id>)` and batched wrote `datatype_iri_id = NULL`. Fixed both Phase 1.5 (datatype IRI collection now includes `xsd:string`) and Phase 2 (literal key construction uses the unified rule) of `ingest_turtle_dict_batched`. Net effect: all four dict-paths (baseline, batched, shmem_warm, combined) write byte-identical dict rows for the same Turtle input.

### Changed (six sources of truth, mechanical bump 0.5.37 → 0.5.38)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output, **`README.md`** Status badge + Status row + Install row + Quickstart example. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.37.sql` → `sql/pgrdf--0.5.1--0.5.38.sql` (no-op; schema unchanged).

### Verified locally

- 286/286 pgrx tests pass.
- **91/91 regression tests pass** (was 75/91 on v0.5.37).
- LUBM-1 ingest e2e: **1,252 ms** (vs baseline 1,517 ms = −17%, vs v0.5.37's 1,217 ms = +3% — the stage_for_commit switch adds a few µs per resolved term for the deferred publish; net win is unchanged in shape).

### Fixed (put_terms_batch NULL-aware dedup — TA-D3 spike's hidden parity bug)

- **`src/storage/dict.rs`** `put_terms_batch` — replaced the `INSERT ... ON CONFLICT (term_type, lexical_value, datatype_iri_id, language_tag) DO NOTHING` form with `INSERT ... WHERE NOT EXISTS (SELECT 1 FROM _pgrdf_dictionary d WHERE d.term_type = t.tt AND d.lexical_value = t.lv AND d.datatype_iri_id IS NOT DISTINCT FROM t.di AND d.language_tag IS NOT DISTINCT FROM t.lt)`. **Why this matters**: PostgreSQL's `UNIQUE` constraint defaults to `NULLS DISTINCT` semantics — two rows `(URI, 'ex:p', NULL, NULL)` and `(URI, 'ex:p', NULL, NULL)` are NOT considered duplicates because each NULL is distinct from every other NULL. The original ON CONFLICT form therefore silently INSERTed duplicate URI / blank-node rows whose `datatype_iri_id` and `language_tag` were both NULL whenever a SECOND batched ingest call in the same backend tried to reuse a predicate or subject from a PRIOR batched call. The subsequent JOIN-back (which uses `IS NOT DISTINCT FROM`) then matched MULTIPLE dict rows for one input position, the result `Vec<i64>` raced between them, and the caller got an unstable id. Downstream SPARQL queries against the affected predicates returned 0 rows because the quads had been written with one id while the dictionary's "canonical" lookup found a different one. The TA-D3 spike's own parity test (`128-parse-turtle-dict-batched-parity.sql`) didn't catch this: that test compares baseline vs spike in adjacent graphs but the duplicate rows only manifest when a second batched call reuses a term from a prior batched call. **Caught by**: TA-7's `tests/regression/sql/130-ingest-dict-paths-parity.sql` + the pgrx test suite (16 multi-graph SPARQL tests turned red the moment the combined path became default). The `WHERE NOT EXISTS` form uses the `IS NOT DISTINCT FROM` semantics in the anti-join so NULLs match. Single-backend correctness restored. Cross-backend concurrent inserts of the same NULL-containing term could still race (each backend passes NOT EXISTS, each INSERTs); the race-safe fix is a schema migration to `UNIQUE NULLS NOT DISTINCT` (PG 15+) — surfaced as **TA-NEW-W** for v0.5.38+ consideration.

### Added (TA-7 production landing — combined dict path becomes default `parse_turtle`)

- **`src/query/guc.rs`** — three new pgRDF custom GUCs registered in `_PG_init`:
  - **`pgrdf.ingest_dict_path`** (string, default `combined`) — enum-style switch routing `parse_turtle` / `load_turtle` (and verbose variants) through one of four dict-resolution paths: `baseline` (legacy single-term `put_term_full` SPI, what v0.5.36 and earlier defaulted to, kept for parity tests), `batched` (TA-D3 spike's 2-pass path), `shmem_warm` (baseline path AFTER a forced one-shot `pgrdf.shmem_cache_prewarm` so the cross-backend cache is hot), `combined` (TA-7 production single-pass: shmem hot-cache check + defer queue + bulk-resolve at `dict_batch_size` or quad-flush boundary). Unrecognised values silently fall back to `combined`.
  - **`pgrdf.dict_batch_size`** (int, default 500, range 0..10_000) — terms per `put_terms_batch` flush in the `batched` / `combined` paths. `0` is reserved as an alias for the legacy single-term path (equivalent to selecting `ingest_dict_path = 'baseline'`); the path resolver consults `dict_batch_size` first and routes to baseline on `0` regardless of the enum.
  - **`pgrdf.shmem_prewarm_on_init`** (bool, default off) — when on, the first `parse_turtle` / `load_turtle` call in a backend triggers a one-shot `shmem_cache_prewarm(100_000)` before its main work. A per-backend `PREWARM_DONE` thread-local cell makes the prewarm idempotent across the backend's lifetime. Default off because the prewarm cost (one full SPI scan of `_pgrdf_dictionary`) rarely amortises in short-lived workloads.
- **`src/storage/loader.rs`** — three new internal helpers:
  - **`ingest_turtle_combined`** — TA-7 production single-pass ingest. Streams the parser; for every s/p/o term tries cache (per-call HashMap) → shmem hot-cache (`shmem_cache::lookup`) → defer queue (a `Vec<DictKey>` + `HashSet<DictKey>` for in-batch dedup). When the defer queue hits `dict_batch_size` OR the pending-triple buffer is about to flush its quads, calls `put_terms_batch` for bulk resolution + `shmem_cache::stage_for_commit` for every freshly resolved (key, id) pair so subsequent ingests in the same backend see hot-cache hits. Datatype IRIs (rare, low cardinality — `xsd:*`, RDF/RDFS/OWL URIs) are resolved synchronously via `resolve_datatype_iri_sync` (cache → shmem → `put_term_full`) because the literal's key depends on the datatype's dict_id; the synchronous path is bounded by the unique datatype-IRI count, not the literal count, so it stays fast after the first few SPI roundtrips warm RDF/RDFS/XSD into the cache.
  - **`ingest_dispatch`** — reads `pgrdf.ingest_dict_path`, applies the `shmem_prewarm_on_init` latch, dispatches the actual reader to one of `ingest_turtle_with_stats` (baseline / shmem_warm), `ingest_turtle_dict_batched` (batched, the TA-D3 spike), or `ingest_turtle_combined` (combined).
  - **`maybe_prewarm_once`** — backend-scoped lazy prewarm gated by `PREWARM_DONE` thread-local cell.
- **`src/storage/stats.rs`** — `shmem_cache_prewarm` UDF now wraps `shmem_cache_prewarm_impl` (newly exposed as `pub(crate)`) so `maybe_prewarm_once` can invoke the prewarm body without round-tripping through the UDF dispatcher.
- **`src/storage/loader.rs::ingest_turtle_combined::resolve_datatype_iri_sync`** — fixes a subtle datatype-handling divergence from baseline `intern_term`: lang-tagged literals carry `datatype_id = None` (rdf:langString is implicit), every OTHER literal carries an explicit datatype IRI dict id — **including plain `xsd:string`**. The TA-D3 spike's literal handling left `datatype_id = None` for plain `xsd:string` literals, which produced a dict row shape that doesn't round-trip via the SPARQL executor's term equality (the executor keys plain literals on the explicit `xsd:string` dict id, so `?n = "Alice"` against a row with `datatype_iri_id = NULL` returns 0 matches). The combined path mirrors baseline's `if lang.is_some() { None } else { Some(intern(datatype_iri)) }` rule.
- **`tests/regression/sql/130-ingest-dict-paths-parity.sql`** + **`tests/regression/expected/130-ingest-dict-paths-parity.out`** — TA-7 correctness gate. Ingests the same Turtle blob into four separate graphs under each `pgrdf.ingest_dict_path` value (baseline, batched, shmem_warm, combined) in turn, then asserts: triple-count parity across all four; lexical-triple equivalence between combined and each of (baseline, batched, shmem_warm); silent fallback to combined when the GUC value is unrecognised. 5 boolean assertions all evaluating to `t`. Blank-node subjects / objects are excluded from lexical parity per the precedent in `128-parse-turtle-dict-batched-parity.sql` (bnode labels are parser-assigned and need not match byte-for-byte across two parser invocations).
- **`src/storage/loader.rs::try_resolve_or_defer`** semantic note: in the combined path `dict_cache_hits` counts the union of per-call HashMap hits AND defer_set hits ("term already queued earlier in this call"). This keeps the per-term invariant `dict_cache_hits + shmem_cache_hits + dict_db_calls = 3 × triples` consistent with the baseline / batched / shmem_warm paths — only the SPI shape differs across paths; the counter remains a stable term-reference taxonomy.

### Verified locally (LUBM-1)

- **Cold cache, default combined path**: ingest e2e = **1,217 ms** (vs baseline 1,517 ms = **−20%** improvement), dict_ms = 774 ms (vs 1,113 ms = −30%), insert_ms = 331 ms, dict_cache_hits = 315,918, shmem_cache_hits = 0 (cold), dict_db_calls = 26,473 (per-term counter), quad_batches = 104. The TA-D2 prewarm layer is no-op on a fresh DB because `_pgrdf_dictionary` is empty; the e2e win comes purely from the TA-D3-style batched resolve. The decision doc's `−65%` estimate assumed a populated dict (warm cache); reaching it requires either `shmem_prewarm_on_init = on` against a populated dict OR a second consecutive ingest in the same backend.
- **All 286 pgrx tests pass** after the put_terms_batch dedup fix (was 0 → 16 → 9 → 0 failures as the bug was isolated and fixed).
- **LUBM-100 unlock**: at 1,217 ms / 103k triples for LUBM-1, projected LUBM-100 (13M triples) cold-cache ingest ≈ 155 s with default settings (a ~−20% e2e win over the 195 s baseline projection). The TA-NEW-W schema migration follow-up — `UNIQUE NULLS NOT DISTINCT` on `_pgrdf_dictionary` — should recover the lost perf from the WHERE NOT EXISTS rewrite while keeping correctness, returning the cold-cache LUBM-1 number closer to the 1,074 ms measured before the dedup fix (~−29% e2e).

### Changed (six sources of truth, mechanical bump 0.5.36 → 0.5.37)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output, **`README.md`** Status badge + Status row + Install row + Quickstart example. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.36.sql` → `sql/pgrdf--0.5.1--0.5.37.sql` (no-op; schema unchanged).

### Added (LUBM Q1-Q14 fixtures + Tbox load → benchmark reasoning fires correctly)

- **`tests/perf/lubm/queries/q01.rq` .. `q14.rq`** — full canonical LUBM Q1-Q14 SPARQL query suite, one file per query, ~10 lines each including a header comment that names the query and notes its reasoning-profile sensitivity (e.g. Q01 anchored class membership = `none`-sufficient; Q06 = `subClassOf Student` chain = needs `rdfs`; Q13 = `owl:inverseOf alumni` = needs `owl-rl`). Every query carries `PREFIX ub: <file:///opt/lubm/univ-bench.owl#>` matching the rewritten Tbox namespace (see below). UBA generator `-seed 0` remains the deterministic data anchor; UBA per-university seeds are independent of the global seed so LUBM-1 vs LUBM-10 are not subsets of each other (this is upstream UBA behavior — LUBM-N is *not* incremental).
- **`tests/perf/lubm/fixtures/univ-bench.owl`** + **`tests/perf/lubm/fixtures/univ-bench.ttl`** — canonical LUBM Tbox checked in (~15 KB). The `.owl` is the upstream RDF/XML form pulled from the `pgrdf-lubm-generator:latest` image at `/opt/lubm/univ-bench.owl`. The `.ttl` is the same ontology re-rendered to Turtle via `rapper` with the namespace rewritten from `http://swat.cse.lehigh.edu/onto/univ-bench.owl#` → `file:///opt/lubm/univ-bench.owl#` so the Tbox URIs match the IRIs UBA writes into the generated ABox. Without this rewrite the reasoner cannot link student/faculty class IRIs in the ABox to the subClassOf chain in the Tbox and reasoning closures stay empty (the `q06 = 0` symptom we hit pre-fix). `tests/perf/lubm/.gitignore` exception added (`!fixtures/univ-bench.owl`, `!fixtures/univ-bench.ttl`) so these two small files clear the global `*.owl`/`*.ttl` block in that directory.
- **`tests/perf/benchmark-runner.sh`** — Tbox-load step added between the LUBM ABox ingest and the per-profile materialize loop: a single `pgrdf.load_turtle_verbose('/fixtures/univ-bench.ttl', GID)` against the bench sidecar. The `/fixtures` mount is the host `tests/perf/lubm/fixtures` directory bound read-only into the container. Each per-profile pass now executes 14 Q1-Q14 SPARQL queries (3 warm passes per query, median timing reported); the per-run JSON output gains `tbox_triples` + nested `profiles.<profile>.queries.q01..q14.{count, elapsed_ms_median}`. `extract_num` wrapped with `|| true` so `set -euo pipefail` does not propagate grep no-match exit codes (surfaced when an optional key was missing). Key probe order corrected from `triples_inferred` → `inferred_triples_written` (the actual key emitted by `src/inference/reasonable.rs`).
- **`tests/perf/lubm/queries/expected-counts.json`** + **`tests/perf/lubm/queries/update-expected.py`** — captured-on-first-run drift manifest. The JSON is keyed by `(lubm-size, profile, query)` covering LUBM-1 + LUBM-10 × `none`/`rdfs`/`owl-rl` × `q01..q14` = 84 cells total. Initial values are `null`; the runner calls `update-expected.py` after each benchmark to fill `null` cells with the observed count (first-run capture) and print drift warnings to stderr for any cell whose observed count diverges from a populated value (drift never fails the run — the user can lock manifest values manually once a baseline is trusted). LUBM-1 + LUBM-10 baselines now captured (42 entries each populated). Highlights: LUBM-1 `rdfs/q06 = 5916` (all Students via subClassOf chain), `owl-rl/q06 = 7790` (adds Visiting/Research Faculty etc), `owl-rl/q13 = 1` (`alumniOf` via owl:inverseOf — rdfs sees 0); LUBM-10 `rdfs/q06 = 75547`, `owl-rl/q06 = 99566`. Scale-anchored queries (Q01/Q03/Q05/Q07/Q10/Q11/Q12, all bound to University0/Department0) stay invariant between LUBM-1 and LUBM-10 as expected.
- **LUBM-10 baseline numbers (first complete reasoning-fires-correctly run)**: 1,316,700 ABox triples ingested in 24.5 s, Tbox load 293 triples / 25 ms, `rdfs` materialize 37.5 s producing 287,422 inferred triples, `owl-rl` materialize 69.1 s producing 815,968 inferred triples. Per-query medians are now usable as a regression baseline; the run-over-run delta surfaces in `target/perf-history/index.html`. Path forward to LUBM-100 unlock: with LUBM-10 baselines locked + reasoning known-to-fire, the next perf work targets `parse_turtle`/`shmem_cache_prewarm`/`materialize` deltas against these numbers rather than the pre-Tbox 0-inferred baseline.

### Changed (six sources of truth, mechanical bump 0.5.35 → 0.5.36)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output, **`README.md`** Status badge + Status row + Install row + Quickstart example. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.35.sql` → `sql/pgrdf--0.5.1--0.5.36.sql` (no-op; schema unchanged). v0.5.35 (commit 127808e, the benchmark-harness landing) never got an annotated tag — the v0.5.35 work + the v0.5.36 LUBM Q1-Q14 + Tbox load work batch into a single tagged v0.5.36 release per the user's "decrease release frequency in favour of batching several local confirmations into larger releases" directive.

### Added (benchmark harness with persistent run-history + HTML report)

- **`tests/perf/benchmark-runner.sh`** + **`tests/perf/render-history.py`** + `Justfile` recipes `just benchmark [N]` + `just benchmark-report` — new self-contained benchmark harness orchestrating an LUBM-N ingest + RDFS materialization + OWL-RL materialization + Q14 query against an isolated `pgrdf-bench-pg-<pid>` sidecar. Per-run output appended as a single JSON line to `target/perf-history/runs.jsonl` (gitignored — `target/` is already in `.gitignore`); `render-history.py` (pure Python stdlib + Chart.js loaded from jsdelivr CDN) emits a self-contained `target/perf-history/index.html` with the latest-run summary card, run-over-run line charts per metric (ingest elapsed/parse/dict/insert, materialize RDFS/OWL-RL, Q14), and a sortable recent-runs table. Captures: triple count, ingest timings (overall + Phase-0 phase breakdown + cache stats), materialize timings + `inferred_triples_written` per profile, Q14 median-of-3-warm with row-count assertion. **Verified locally on LUBM-1**: 103,104 triples / 1,579 ms ingest, RDFS 384 ms / 0 inferred (LUBM ABox only, no Tbox loaded), OWL-RL 1,727 ms / 17,193 inferred, Q14 22 ms / 1,874 rows. The OWL-RL closure adding 17k triples is exactly the "before LUBM-100 we should be measuring inferenced materializations" target the user flagged.
- **`.github/workflows/perf-nightly.yml`** — extended with three new steps. (1) `Run benchmark harness (LUBM-10 + RDFS + OWL-RL + Q14)` calls the new runner. (2) `Upload benchmark history + HTML report` uploads `target/perf-history/` as a 90-day artifact under the name `perf-history-<run_id>` (reviewer downloads it + opens `index.html` for the chart view). (3) `Job summary` step writes the latest-run numbers into the workflow's `GITHUB_STEP_SUMMARY` so reviewers don't need to download the artifact to see what shipped — `pgRDF version`, `LUBM size`, ingest/materialize/Q14 timings, all rendered as a markdown bullet list directly in the workflow summary panel. **Note on CI history accumulation**: the JSONL only contains the single run from each CI invocation (fresh workspace per run); the local dev path (`just benchmark`) accumulates locally. A follow-up could (a) commit the JSONL to a tracked branch, or (b) use a cache action for cross-run accumulation — both out of MVP scope.

### Decided (Track A spike chain close-out — TA-D1 + TA-9)

- **`_WIP/DECISION.TRACK-A.dict-path-and-insert-path.md`** — written decision closing both TA-D1 (dict-path) and TA-9 (insert-path) of the Track A spike chain. **TA-D1 verdict: ship TA-D3 + TA-D2 combined into the default `parse_turtle` path** as part of TA-7's production-landing work. Production variant uses one-pass design (avoiding TA-D3's 2-pass overhead): hot-cache check via shmem, defer queue for misses, flush queue at `dict_batch_size` (GUC, default 500) OR at next quad-batch flush. Combined estimate at LUBM-1: ~-65% e2e. **TA-9 verdict: REJECT both TA-11 (heap_multi_insert) AND TA-10 (COPY BINARY).** Neither addresses the dominant insert_ms cost component (hexastore index maintenance = 51% of LUBM-1's 312 ms insert_ms; both candidates target the bulk-insert mechanic which is only 13.4%). TA-7 GUC catalog **drops `pgrdf.ingest_path = 'unnest'\|'copy'\|'heap'`** — keep the current prepared-unnest insert path as the only path. Surfaces **TA-NEW-Z** as the real Track A insert-path lever for v0.7+ consideration: bulk-ingest mode that conditionally drops the 3 hexastore indexes, runs the batched-dict + bulk-INSERT pipeline, then `CREATE INDEX` rebuilds them in parallel — classic PG bulk-load pattern, estimated 40-50% insert_ms savings (~10% e2e at LUBM-1). Counter trajectory: 35/92 → 37/92 once this release advertises; TA-D1 + TA-9 close. Track A then has 7 remaining tasks (TA-8 → TA-1 ship-it), all dependent on TA-7 production-landing work.

### Added (Track A spike — TA-10 COPY BINARY prelim → measured-not-worth-implementing)

- **`src/storage/loader_ta11.rs`** gains 3 new spike UDFs: `pgrdf.spike_ta10_logged_flat`, `pgrdf.spike_ta10_logged_partitioned`, `pgrdf.spike_ta10_logged_indexed`. Each runs the same prepared `INSERT ... unnest($1,$2,$3)` SQL against a target table with one cost component added (WAL, partition routing, or 3 hexastore indexes mirroring `_pgrdf_quads`' SPO/POS/OSP shape). The 4-way decomposition (combined with TA-11's UNLOGGED-flat baseline) isolates which component dominates LUBM-1's 3.0 µs/triple insert_ms.
- **`tests/perf/lubm/spike-ta10.lubm-1.json`** + **`tests/perf/lubm/spike-ta10.lubm-1.md`** — 4-way cost decomposition. **Result: hexastore index maintenance is 51% of LUBM-1 insert_ms (159 ms of 312 ms). WAL is 3.8%, partition routing 0.3%, bulk-insert mechanic 13.4%, real-data + partitioned×indexed combination ~31%.** COPY BINARY routes through PG's INSERT machinery and triggers the same per-row index maintenance — it does NOT address the dominant cost. Theoretical ceiling at LUBM-1 scale: 5-10% improvement, for ~200 lines of unsafe Rust against `pgrx::pg_sys` CopyFrom + binary tuple encoding. **Recommended verdict: NOT WORTH IMPLEMENTING (same class as TA-11).** Surfaces a possible future TA-NEW-Z item (drop-indexes / bulk-insert / rebuild-indexes pattern — classic PG bulk-load technique) that addresses the actual 51% lever — out of current Track A scope, suggested for v0.7+ consideration. With TA-D3 (-17% e2e) and TA-D2 (-54% e2e) already shipped via the dict path, TA-9 decision (next) writes up the insert-path findings + closes the Track A spike chain.

### Added (Track A spike — TA-11 heap_multi_insert prelim → measured-not-worth-implementing)

- **`src/storage/loader_ta11.rs`** + `src/storage/mod.rs` registration — new spike entry `pgrdf.spike_ta11_batch_sweep(triple_count INT DEFAULT 100000, batch_size INT DEFAULT 1000) → JSONB` that runs the production prepared `INSERT ... unnest($1,$2,$3)` SQL against an UNLOGGED flat target table. UNLOGGED + flat (un-partitioned) strips WAL writing and partition routing from the measurement so the bulk-insert mechanic alone is visible. Returns timing breakdown.
- **`tests/perf/lubm/spike-ta11.lubm-1.json`** + **`tests/perf/lubm/spike-ta11.lubm-1.md`** — batch-size sweep at 100,000 synthetic triples. **Result: `BATCH_SIZE=1000` is already the sweet spot at 0.40 µs/triple in the bulk-insert mechanic. The total LUBM-1 insert_ms is 3.0 µs/triple — a 7.5× gap that is entirely WAL writing + partition routing (the two components UNLOGGED + flat target eliminated). `heap_multi_insert` would address neither dominant cost. Theoretical ceiling: ~4% total improvement at LUBM-1 scale, for ~300 lines of unsafe Rust against pgrx::pg_sys + partition Oid lookup. Recommended verdict: NOT WORTH IMPLEMENTING. Track A perf wins land via TA-D3 (-17%) + TA-D2 (-54%); insert path is already near-optimal modulo deployment-level GUC tuning (`synchronous_commit`, `wal_level`) which are not extension code.** This is a clean negative result and a meaningful spike — the spec text says "branch + recorded numbers; decision input for TA-9" and the recorded numbers are the deliverable.

### Changed (README staleness refresh)

- **`README.md`** — Status badge bumped from the stale `v0.5.1` literal to `v0.5.31` and renamed to "Track A spikes landing". New `LATEST.md` badge linking to `./LATEST.md` so consumers always have a one-click route to the current advertised version + digests. Status row rewritten to acknowledge the v0.5.10..v0.5.31 cycle's real shipped surface (PGXN packaging, OCI distribution + SLSA Build Provenance v1, the 5-gate `PROVENANCE.md` Rule 7 contract, Phase-0 ingest instrumentation, additive perf-spike UDFs `parse_turtle_dict_batched` -17% e2e + `shmem_cache_prewarm` -54% e2e behind explicit opt-in surfaces — default `parse_turtle` path unchanged). Install row OCI pin updated from `:v0.5.1` to `:0.5.31` plus an `gh attestation verify` line. Quickstart `SELECT pgrdf.version()` example refreshed `0.5.1 → 0.5.31` with a parenthetical pointing at `LATEST.md` so the literal can drift without misleading future readers. **Why this matters**: OCI-GERMINATION's stale manifest line (the v0.5.1 reference they had) lived alongside a stale README v0.5.1 reference — same root-cause class as the original v0.5.1-stuck-internal-label bug, just at a different layer. The README is now the third public surface (after `LATEST.md` and `RELEASE_NOTES.md`) brought into version-coherence with the release pipeline.

### Fixed (release-notes leak — actions/checkout annotated-tag workaround)

- **`.github/workflows/release.yml`** — v0.5.29 release.yml run 26705375440 hit the new pre-publish gate exactly as designed ("Rendered Release body does NOT contain '0.5.29'"). Root cause: `actions/checkout@v4 fetch-tags: true` fetches tag REFS but NOT annotated tag OBJECTS; `git for-each-ref refs/tags/v0.5.29 --format='%(contents)'` falls through to the COMMIT's auto-message ("Merge branch 'main' of github.com:styk-tv/pgRDF" — the merge commit at the tag's ref). Locally the annotated tag object IS present so the gate looked fine in dev. **Fix**: new step "Refetch annotated tag objects (actions/checkout #1467)" runs `git fetch --tags --force origin` between checkout and the body-render step. Reads back the full tag object so `for-each-ref %(contents)` returns the annotation. v0.5.29 stays orphaned per `[[only-forward-never-revert]]` — its tag exists on GitHub but no release page was created (the gate aborted softprops/action-gh-release before it ran) and no OCI artifact was published. Exactly the "wrong-labeled tag exists as orphan GHCR digest but never gets advertised" behavior Rule 7 specifies.

### Fixed (release-notes leak — second OCI-GERMINATION-class issue)

- **`.github/workflows/release.yml`** + **`RELEASE_NOTES.md`** — GitHub Release bodies for v0.5.10..v0.5.28 (19 releases, both advertised and orphaned tags) ALL showed stale "pgRDF v0.5.1 — PGXN, artifact parity, and MIT cleanup" text on github.com because `release.yml` pointed `body_path` at a checked-in `RELEASE_NOTES.md` that hadn't been touched since v0.5.1 (2026-05-23). OCI-GERMINATION flagged it on 2026-05-31. **Fix in two parts**: (1) `release.yml` no longer reads `RELEASE_NOTES.md` — instead it renders the body from `git for-each-ref refs/tags/<tag> --format='%(contents)'` (the annotated tag message that the maintainer writes at `git tag -a -F /tmp/tag-vX.Y.Z.txt`), with a CHANGELOG.md `[Unreleased]` fallback if the tag has no annotation. (2) A new **pre-publish gate** asserts the rendered body actually contains the tag's version string — fails the publish otherwise — so the v0.5.1-stuck class of leak cannot recur under any future template/file-pointer change. `RELEASE_NOTES.md` rewritten as a pointer page (LATEST.md's footer still links to it; the link now resolves to "see GitHub Releases / CHANGELOG.md" guidance instead of the stale 2026-05-23 body). **Backfilled** the advertised v0.5.25..v0.5.28 release pages on github.com via `gh release edit --notes-file <tag-annotation>` — each now shows its actual per-release annotation body instead of the inherited v0.5.1 text. Orphaned tags (v0.5.10..v0.5.24) were NOT backfilled — they never advertised on LATEST.md and stay historically-orphaned per `[[only-forward-never-revert]]`. v0.5.29 chain itself validates the new pipeline end-to-end (the release body for this tag flows through `git for-each-ref` + the gate).

### Added (Track A spike — TA-D2 shmem cache pre-warm)

- **`src/storage/stats.rs`** `pgrdf.shmem_cache_prewarm(limit BIGINT DEFAULT 100000) → BIGINT` — walks `_pgrdf_dictionary` ordered by `id` (oldest first; core RDF/RDFS/OWL predicates absorbed first) and calls existing `shmem_cache::insert_committed` for each row. Returns count of rows pre-warmed. Use cases: boot a fresh backend connecting to a database with an existing populated dictionary; post-`DROP/CREATE EXTENSION` re-establishment; explicit warm-up before a measured ingest run.
- **`tests/perf/lubm/spike-ta-d2.lubm-1.json`** + **`tests/perf/lubm/spike-ta-d2.lubm-1.md`** — 3-way side-by-side measurement: cold ingest → `shmem_reset` + `shmem_cache_prewarm(100000)` → warm ingest into a different graph. **Result: -54.0% e2e ingest time, -72.7% dict_ms, 16,295 of 26,473 would-have-been-SPI calls absorbed by the cache** (61.5% absorption rate; the remaining 10,178 hit DB because SLOTS=16,384 + PROBE_DEPTH=8 evicts the rest under collision pressure — a SLOTS bump is a future tunable). Triples count identical (correctness preserved). Larger win than TA-D3's -17% e2e.
- **`tests/regression/sql/129-shmem-cache-prewarm.sql`** + matching `expected/129-*.out` — correctness gate: 3 boolean assertions covering prewarm on empty dict (returns ≥ 0), prewarm count equals dict row count, and prewarm respects the `limit` arg. All evaluate to `t`.

### Added (Track A spike — TA-D3 batched dict resolution)

- **`src/storage/dict.rs`** `put_terms_batch(terms) → Vec<i64>` — new internal entry point that resolves N dict terms in **2 SPI calls** (bulk INSERT ON CONFLICT DO NOTHING + bulk JOIN-back lookup with `WITH ORDINALITY` for input-order preservation) instead of the per-term `put_term_full` (which is 1-2 SPI calls each). Skips shmem cache integration for the spike — TA-D2 covers warming separately.
- **`src/storage/loader.rs`** `ingest_turtle_dict_batched(reader, graph_id, base_iri, dict_batch_size)` + UDFs `pgrdf.parse_turtle_dict_batched(content, graph_id, base_iri, dict_batch_size)` + `pgrdf.load_turtle_dict_batched(path, graph_id, base_iri, dict_batch_size)` — 2-pass ingest: collect all triples + unique terms, datatype-IRI pre-resolve, bulk-resolve remaining terms in `dict_batch_size`-sized chunks (default 500), then walk triples to build s/p/o arrays via `flush_batch` (same prepared INSERT plan as baseline). Output JSONB adds `"path": "dict_batched"` + `"dict_batch_size": N` discriminators alongside the standard Phase-0 fields.
- **`tests/perf/lubm/spike-ta-d3.lubm-1.json`** + **`tests/perf/lubm/spike-ta-d3.lubm-1.md`** — side-by-side measurement against the v0.5.26 Phase-0 baseline. **Result: -17.4% total ingest time, -30.2% dict_ms, 250× reduction in dict_db_calls (26,473 → 106)**. Triples count identical (correctness preserved). Spike validates the theory but smaller than Phase-0's 60% prediction — the 2-pass overhead in Phase-1 (parse + collect, +16%) and Phase-3 (re-walk + lookup, +21%) eats some of the dict-phase savings. Production landing (TA-7 winner) should use a one-pass design with incremental dict flushes.
- **`tests/regression/sql/128-parse-turtle-dict-batched-parity.sql`** + matching `expected/128-*.out` — correctness gate: ingests the same Turtle into two separate graphs via `parse_turtle` (baseline) and `parse_turtle_dict_batched` (spike). 4 boolean assertions: triple count parity, baseline-subset-of-spike, spike-subset-of-baseline (excluding blank-node subjects/objects since bnode labels are parser-assigned and need not byte-match across two parser invocations), and `path = "dict_batched"` discriminator. All evaluate to `t`.

### Changed (six sources of truth, mechanical bump 0.5.26 → 0.5.27)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.26.sql` → `sql/pgrdf--0.5.1--0.5.27.sql` (no-op; schema unchanged).

### Changed (stabilization window)

- **`.github/workflows/{ci,release,oci-publish}.yml`** — multi-PG matrix temporarily cut to **pg17 only**. The four-PG matrix (`pg: ["14", "15", "16", "17"]`) is preserved in YAML comments verbatim with restoration instructions; do NOT delete. Motivation: v0.5.20 release.yml's `build (15, amd64)` job failed mid-run (chain run 26635709923) — part of a string of intermittent pg15-amd64 flakes that's polluting the green-row signal without changing what consumers can install. Cutting to pg17 stabilizes the release pipeline, restores the auto-fire chain, and lets feature work continue while the multi-PG-matrix flakes get diagnosed separately. Restoration target: once a clean run on `pg: ["14", "15", "16", "17"]` lands locally on an off-main branch, re-enable in a separate commit titled `chore(ci): restore multi-PG matrix`. `LATEST.md` "Older PG majors" row will be misleading during this window (still claims pg14/15/16 ship alongside); a follow-up tightens the renderer or the spec text to match reality.

### Added (Phase-0 — informs TA-11 / TA-10 / TA-9 scope decision)

- **`src/storage/loader.rs`** + **`tests/perf/lubm/run-lubm.sh`** + **`tests/perf/lubm/baseline.lubm-1.json`** + **`tests/perf/lubm/baseline.lubm-1.md`** + **`_WIP/SPIKE.TRACK-A.phase0-findings.md`** — pre-spike instrumentation pass for Track A. Adds three nanosecond accumulators (`parse_ns` / `dict_ns` / `insert_ns`) to `ingest_turtle_with_stats` wrapping the existing rio `next()` / `intern_term` / `flush_batch` calls; converted to ms at the end and surfaced in `parse_turtle_verbose` / `load_turtle_verbose` JSONB as `parse_ms` / `dict_ms` / `insert_ms`. Quad-path stays zero for now (LUBM-1 is Turtle). The runner extracts the three fields from `load_turtle_verbose` output and embeds them in `target/perf-report.json` per fixture. **Finding** (LUBM-1 / pg17 / Apple-Silicon / Colima): **dict_ms = 1,114 ms (73% of total ingest)**, insert_ms = 292 ms (19%), parse_ms = 103 ms (7%). Inserts — the TA-11 / TA-10 spike target — are NOT the bottleneck. The 73% dict_ms lever is 5-10× higher ROI than the spikes as currently scoped. The `_WIP/SPIKE.TRACK-A.phase0-findings.md` working note proposes re-scoping TA-11 / TA-10 / TA-9 and adding three TA-NEW-* tasks (batch dict resolution / shmem pre-warm / re-measure-after); ledger NOT modified — user decision required before re-scope. Instrumentation overhead is ~3% on the wall-clock (well inside the ±50% tolerance; no gate breakage). Baseline re-captured with the new fields against the instrumented loader.

### Changed (six sources of truth, mechanical bump 0.5.25 → 0.5.26)

- **`Cargo.toml`**, **`pgrdf.control`**, **`compose/compose.yml`** SQL mount, **`tests/regression/expected/00-smoke.out`**, **`META.json`** (both fields), **`docs/06-installation.md`** + **`compose/README.md`** example output. Per PROVENANCE.md Rule 7 — all four CI gates (0 / 1 / 2 / 3) check these in concert. Upgrade bridge renamed `sql/pgrdf--0.5.1--0.5.25.sql` → `sql/pgrdf--0.5.1--0.5.26.sql` (no-op upgrade; schema byte-identical between 0.5.1 and 0.5.26).

### Fixed

- **`Cargo.toml`** + **`sql/pgrdf--0.5.1--0.5.25.sql`** + **`.github/workflows/release.yml`** + **`.github/workflows/oci-publish.yml`** + **`PROVENANCE.md`** — the internal-version-stuck-at-0.5.1 bug, plus the three CI gates that prevent it from recurring. **What was wrong**: from v0.5.2 through v0.5.23 the published OCI artifact's `pgrdf.control` carried `default_version = '0.5.1'` and the INSTALL layout named the install SQL `pgrdf--0.5.1.sql`. The `.so` itself was current per-release code (with all TE-5 / TA-12 / TF-* / TG-* changes baked in), so `CREATE EXTENSION pgrdf;` (no version pin) worked and installed as 0.5.1. But `CREATE EXTENSION pgrdf VERSION '0.5.<X != 1>'` failed with `extension "pgrdf" has no installation script nor update path for version "0.5.X"` — the exact failure OCI-GERMINATION surfaced on 2026-05-30. Root cause: `Cargo.toml`'s `version` field was never bumped past `0.5.1` after the initial declaration. **What this release does**: bumps `Cargo.toml` to `0.5.25`, ships a no-op `sql/pgrdf--0.5.1--0.5.25.sql` upgrade bridge so existing installs can `ALTER EXTENSION pgrdf UPDATE TO '0.5.25'` cleanly, AND adds three CI enforcement layers (PROVENANCE.md Rule 7): (1) pre-build assertion in `release.yml` that `Cargo.toml`'s version equals the tag (without leading `v`) — aborts build before `cargo pgrx package` runs if drift; (2) post-build assertion in `release.yml` that the tarball contains `pgrdf--<TAG>.sql` and that `pgrdf.control`'s `default_version` equals the tag — aborts upload if drift; (3) post-publish consumer-style smoke verify in `oci-publish.yml` (between Attest aggregate index and Trigger update-latest-md) that pulls the just-published artifact via ORAS, boots a clean `postgres:17.4-bookworm`, runs `CREATE EXTENSION pgrdf VERSION '<TAG>'`, asserts `pg_extension.extversion == TAG` AND `pgrdf.version() == TAG` — fail-fast means `update-latest-md.yml` never fires and `LATEST.md` stays at the prior version. Includes a NOTIFIES draft in `_WIP/NOTIFIES.oci-germination.0.5.23.internal-version-stuck-at-0-5-1.md` for the user to pass downstream once the v0.5.25 chain settles green and the live artifact has been re-verified end-to-end. v0.5.1..v0.5.23 are NOT retroactively re-cut (per `[[only-forward-never-revert]]`) — they exist as historically-orphaned GHCR tags.

### Added

- **`tests/perf/lubm/compare-to-baseline.py`** + **`.github/workflows/perf-nightly.yml`** — perf-nightly cross-environment fix. The 2026-05-30 first-real-cron run (with the `just`-install fix in v0.5.22) revealed the second layer: the localhost baseline (~21.6s ingest) and the GH-Actions runner (~51.5s ingest) differ by 139% — way past the ±50% timing tolerance the baseline ships. Two changes: (1) `compare-to-baseline.py` gains a `--no-timing-gate` flag — correctness exact-match fields (`conforms`, `violations`, `dict_lookups`, `plan_cache_hits`) stay STRICT (Q14 count divergence still fails the job — that's a real bug), but elapsed_ms drifts become `TIMING:` info lines visible in stderr without gating the script's exit code. In-tolerance numbers now also print as TIMING info, giving the workflow log a poor-man's trend history. (2) `perf-nightly.yml` runs both LUBM-1 + LUBM-10 (LUBM-1 is the smaller-tier baseline added in TA-12; LUBM-10 is the existing dev-gate), passes `--no-timing-gate` to both, and uploads both snapshots as a single `perf-snapshots-<run_id>` artifact (90-day retention, always-on — not just on failure). Localhost-side `just test-lubm-1` / `just test-lubm-10` stay full-gate; tightening or loosening the localhost tolerance is a separate decision.
- **`tests/perf/lubm/baseline.lubm-1.json`** + **`tests/perf/lubm/baseline.lubm-1.md`** + **`Justfile`** `test-lubm-1` recipe — Track A task TA-12. LUBM-1 baseline: smaller, faster-iterating tier under LUBM-10. 103,104 triples ingested in 1,489 ms (~69k triples/sec); Q14 returns 1,874 GraduateStudents in median 1.8 ms (warm). Measured on Apple-Silicon laptop / Colima docker / pgrdf v0.5.1 / pg17. Companion `.md` documents reproducibility, fixture semantics, and what this baseline is NOT (release-gate, dev-gate, cross-engine). Self-verified by running `just test-lubm-1` immediately after baseline capture: compare-to-baseline produced PASS with 2.4% / -2% drift, well within ±50% tolerance. This baseline is the "before" line for the Track A ingest-path optimization spikes (TA-11 `heap_multi_insert`, TA-10 `CopyBinary`); the decision in TA-9 will reference this number set. Closes Track A task TA-12.
- **`src/**/*.rs`** + **`tests/regression/sql/127-search-path-discipline.sql`** — Track E task TE-5. Every `#[pg_extern]` function in production code now carries the sibling `#[search_path(pgrdf, pg_temp)]` attribute (36 functions across 12 files: `inference/reasonable.rs`, `lib.rs`, `query/{executor,parser,plan_cache}.rs`, `storage/{construct_ingest,dict,graphs,hexastore,loader,stats}.rs`, `validation/shacl.rs`). pgrx 0.16's `#[search_path(...)]` attribute emits `SET search_path = pgrdf, pg_temp` in the `CREATE FUNCTION` DDL; PostgreSQL applies that SET before each call, overriding whatever the session had. The new regression file exercises three threat scenarios: T-1 caller session search_path = pg_catalog only (no pgrdf — UDF must still resolve its own catalog); T-2 caller in a custom user schema prepended; T-3 adversarial schema-shadow attack where the caller creates a relation named `_pgrdf_dictionary` in their own schema and prepends it to search_path. All three: pgrdf functions resolve correctly via the function-level SET. 5 assertions all evaluate to `t`; hand-derived expected.out, never ACCEPT=1 baselined. Closes Track E task TE-5.
- **`ROADMAP.md`** — Track G column flipped to ✅ (TG-1 ship-it). Heading gains the ✅ marker; new "**Status:** closed v0.5.16" block enumerates the five G.* items + their landing versions (G.1 v0.5.2, G.2 v0.5.2, G.3 v0.5.3, G.4 v0.5.14, G.5 v0.5.8) plus the two infrastructure spillovers that rode along (SLSA chain v0.5.10, update-latest-md.yml v0.5.13). Track G is the first column to fully close in the v0.6 cycle.
- **`LICENSE`** + **`Cargo.toml`** + **`README.md`** + **`RELEASE_NOTES.md`** + **`META.json`** + **`docs/09-release.md`** — email-surface hygiene. The maintainer email `peter@conceptkernel.org` (updated from the prior `peter@styk.tv`) now lives in `LICENSE` only — that's the canonical source. Every other release-surface that previously duplicated the email (`Cargo.toml authors`, README copyright line, RELEASE_NOTES copyright line, `META.json` maintainer field, `docs/09-release.md` prose) is stripped to "Peter Styk" with cross-reference to `LICENSE`. Avoids the email getting "littered all over every single release" (scraping / aggregator surface). Pairs with a workstation-side `git config user.email peter@conceptkernel.org` so commit metadata matches.
- **`docs/11-recipes.md`** — Track E hygiene cluster (TE-3 + TE-2). New operational-recipes page covering two stock-Postgres extensions that play well with pgRDF: `pg_prewarm` (TE-3 — warming `_pgrdf_dictionary` + `_pgrdf_quads` + their indexes for benchmark stability) and `pg_stat_statements` (TE-2 — pgRDF's prepared SQL is statement-id stable so the aggregation works across SPARQL queries that differ only in literal IRIs). Both are documented as samples, not hard dependencies. Notes plan-cache cross-reference, statement-id stability guarantees, and the realistic caveats (`shared_buffers` sizing, statement IDs changing across translator updates).
- **`pgrdf.control`** — TE-7. Comment block at the top of the control file explicitly documents that pgRDF has NO external extension dependencies in the v0.6 cycle, so `requires =` is intentionally absent (distinct from `requires = ''`, which some parsers warn on as an empty-list declaration). Sister projects that depend on pgRDF declare it explicitly in their own `requires =` line; the comment cross-references pgCK's `requires = 'pgrdf, pgcrypto'` as a worked example.
- **`compose/compose.yml`** + **`compose/parity-check.sh`** — TG-3 (artifact-parity v2). One-shot init container `pgrdf-parity` (busybox:1.36) runs `parity-check.sh` before postgres starts; `postgres` `depends_on` it with `condition: service_completed_successfully`. The script hashes the bind-mounted `.so` / `.control` / `pgrdf--<ver>.sql`, asserts non-empty + readable, parses `.control`'s `default_version`, and verifies the matching `pgrdf--<default_version>.sql` exists. Non-zero exit on any drift mode (.so empty=2, .control absent/malformed=3, SQL missing=4, name mismatch=5); postgres never starts. Catches the realistic drift case at compose-up: a release cut bumps `pgrdf.control`'s `default_version` but `compose.yml`'s bind-mount line still references the previous SQL file → fail fast at startup rather than later at `CREATE EXTENSION` with a `pgrdf--<old>.sql not found` confusion. `compose/README.md` documents the new boot sequence. Closes Track G task TG-3 (artifact-parity v2); only TG-1 (Track G ship-it) remains.
- **`.github/workflows/update-latest-md.yml`** + **`tools/render-latest-md.py`** — TG-3.update-latest-md scaffold. Workflow triggers on `workflow_run: oci-publish completed`; resolves the head version from the GHCR API (highest semver `X.Y.Z-pg17-amd64` tag); runs `gh attestation verify` against the aggregate index + pg17 amd64 + pg17 arm64 digests; on full-pass, renders the entire `LATEST.md` via the Python script and commits via `github-actions[bot]` with `[skip ci]`. Refuses to advance if any digest fails to verify — that's how PROVENANCE.md Rule 3 stops being aspirational. Adopted from the pgCK sibling-repo pattern (`/Users/neoxr/git_conceptkernel/pgCK/{tools/render-latest-md.py, .github/workflows/update-latest-md.yml}`); pgRDF's variant is simpler because pgRDF ships only one OCI surface (no web layer) — no per-side preservation logic needed. First real end-to-end exercise lands with the next tagged release after this commit: the chain `release.yml` → `oci-publish.yml` → `update-latest-md.yml` is unproven until that tag fires it.
- **`PROVENANCE.md`** + matrix-refactored **`oci-publish.yml`** with SLSA Build Provenance v1 attestations — adopts the pgCK sibling-repo provenance pattern. `oci-publish.yml` reshaped from a single bash-loop job into a matrix (8 leaf jobs at `[pg14/15/16/17] × [amd64/arm64]` + 1 dependent index job); every leaf push and the aggregate index get attested via `actions/attest-build-provenance@v1`, pushed as an OCI referrer. Consumers verify with `gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:<tag> --repo styk-tv/pgRDF`. PROVENANCE.md declares the six hard rules (Actions-only build/push, attestation gate on LATEST.md, automated-only LATEST.md writes, no tag without prior in LATEST.md, release-often-small-groups, report counts every turn) with a one-time bootstrap exception for v0.5.0–v0.5.9 (the pre-attestation cycle). v0.5.10 is the first release whose digests verify under `gh attestation verify`. The `update-latest-md.yml` automation that closes Rule 3 strictly is tracked as a follow-up Track G item (TG-3.update-latest-md); until it lands, LATEST.md is hand-maintained per the discipline laid out in the doc.
- **`tests/perf/lubm/`** — TF-12 test bed intake closed. Scaffold for the v0.6 LUBM-N localhost benchmark. Generator source TRACKED (`generator/Dockerfile` wrapping the Lehigh SWAT UBA generator in `eclipse-temurin:17-jre-jammy`, `generator/generate.sh` entrypoint, `generator/README.md`). Generated data DISCARDABLE — lives in the `pgrdf-lubm-data` docker named volume, NOT on the host filesystem; `.gitignore` keeps any accidental host-side copies out of git. `schema/baseline.schema.json` (JSON Schema 2020-12) locks the per-fixture `{conforms, violations, elapsed_ms, plan_cache_hits, dict_lookups}` shape + `comparison_tolerance.elapsed_ms_pct` block. New Justfile recipes `lubm-build` / `lubm-gen N` / `lubm-clean` — docker-only via Colima; never runs Java on the host workstation per the [[lubm-localhost-only]] discipline. Sibling micro-fixtures (`tests/perf/lubm-shape/`, `tests/perf/lubm-shacl-sparql/`) stay handcrafted for every-CI runs; this new directory is the UBA-generated LUBM-10 baseline that TF-11 / TF-10 / TF-9 build on. Closes Track F task TF-12.
- **`tests/regression/sql/123-dictionary-lexical-contract.sql`** + matching `tests/regression/expected/123-*.out` — Track F task TF-5. Narrower dictionary-surface contract complementing TF-4's full-pipeline test (file 124). 18 hand-derived assertions covering: 4 IRI shapes (http://, urn:, custom-scheme+frag `ckp://Task#001`, percent-encoded), plain literal, zero-padded integer `"0030"^^xsd:integer` (CRITICAL — pgRDF must NOT silently strip leading zeros), boolean, dateTime with TZ verbatim, lang-tag single subtag (`@en`) + region subtag case-preserved (`@fr-CA`), embedded escape (quote, tab), Unicode UTF-8 literal, blank-node type + followthrough. Per CX-002 EVAL: asserts EXACT lexical bytes round-trip through `parse_turtle → dict → pgrdf.sparql`. Also marks TF-4 (`124-end-to-end-lexical-rehydration.sql`) explicitly in the spec ledger as v0.5.3-shipped (it was filed as a shared single-source-of-truth with TG-4, which DID get marked at the time but TF-4's own row was empty).
- **`tests/perf/lubm/compare-to-baseline.py`** + **`Justfile`** `test-lubm-10` recipe — Track F task TF-7. Fast-failing LUBM-10 dev-gate. The new comparison script reads two `perf-report.json` files (actual + baseline) and emits per-fixture regression reports: correctness fields (`conforms`, `violations`, `dict_lookups`, `plan_cache_hits`) are exact-match where present in both; timing (`elapsed_ms`) is tolerance-compared per fixture via `comparison_tolerance.elapsed_ms_pct` (defaults to ±50% — CI runner noise floor). Exit 0 on full-pass, 1 on regression, 2 on invocation error. Missing fields in actual surface as warnings (forward-compat for runner additions); missing fixtures in actual surface as hard regressions (preventing silent "we stopped measuring this" drift). The `just test-lubm-10` recipe wires this into the dev loop: runs the TF-10 runner with `OUTFILE=target/perf-report.json`, then `compare-to-baseline.py` against `tests/perf/lubm/baseline.lubm-10.json`. Closes Track F task TF-7.
- **`.github/workflows/perf-nightly.yml`** — Track F task TF-8. Nightly LUBM-10 performance regression gate. Schedule: `0 3 * * *` (03:00 UTC daily) + `workflow_dispatch` for on-demand. Single job `lubm-10`: builds the extension artifacts via `just build-ext`, builds the UBA generator via `just lubm-build`, generates LUBM-10 into the `pgrdf-lubm-data` docker named volume via `just lubm-gen 10`, then runs `just test-lubm-10`. Fails the workflow if `compare-to-baseline.py` reports any regression; on failure, uploads `target/perf-report.json` as an artifact for triage. `timeout-minutes: 25` hard ceiling. `permissions: contents: read` — nothing writes back; baseline updates require a human commit after investigation, never an auto-bless from the workflow. Closes Track F task TF-8.
- **`tests/perf/lubm/data/lubm-10/`** + widened `tests/perf/lubm/.gitignore` — Track F task TF-11. In-repo layout placeholder for the LUBM-10 dataset; the actual ~195-MB N-Triples / 106-MB Turtle / 101-MB OWL corpus lives in the `pgrdf-lubm-data` docker named volume, NOT on the host filesystem. `.gitignore` widened from `data/*` to `data/**` with an explicit `!data/lubm-*/.gitkeep` allow-list so accidental host-side copies of the dataset (or `.owl` / `.nt` / `.ttl` files in general) never leak. The placeholder `.gitkeep` carries the reproduction-loop pointer + the docker-volume inspection one-liner. TF-11 also forward-fixed the generator: Dockerfile now vendors `http://swat.cse.lehigh.edu/onto/univ-bench.owl` (UBA 1.7's `-onto` argument needs the ontology and the upstream zip doesn't bundle it), `generate.sh` works around UBA's hardcoded `<cwd>\<file>` Windows-style backslash output prefix (file separator hardcoded in UBA's Java source) by post-renaming files into `raw/`. End-to-end: `just lubm-build` → ~30s, image ~295 MB; `just lubm-gen 10` → ~10s, 1,316,700 triples at `-seed 0`. Closes Track F task TF-11.
- **`tests/perf/lubm/run-lubm.sh`** — Track F task TF-10. TF-10 LUBM runner; boots an isolated `pgrdf-perf-pg-<pid>` postgres+pgrdf sidecar (docker-only via Colima, per [[docker-only-pgrdf-prefix]]) with `compose/extensions/` bind-mounted at the canonical Postgres paths (mirrors `compose/compose.yml`) and the `pgrdf-lubm-data` volume read-only at `/lubm-data`. Ingests via `pgrdf.load_turtle_verbose('/lubm-data/lubm-<N>/nt/lubm-<N>.nt', <gid>)` and runs LUBM Q14 ("count GraduateStudents") with one warm + three measured passes; reports median `elapsed_ms`. Emits `target/perf-report.json` validated against `tests/perf/lubm/schema/baseline.schema.json` when `JSON_SCHEMA_VALIDATE=1` is set (CI sets it; local dev defaults off so the runner has no Python dependency). Trap on EXIT/INT/TERM tears the sidecar down. Pre-flight checks: docker required, `pgrdf-lubm-data` volume present, extension files present, `pgrdf--<default_version>.sql` matches `.control` (mirrors TG-3 parity contract). Safe to invoke alongside parallel agents — all names `pgrdf-perf-*`-prefixed; reuses no other container/volume namespace. Closes Track F task TF-10.
- **`tests/perf/lubm/baseline.lubm-10.json`** + **`baseline.lubm-10.md`** — Track F task TF-9. First checked-in TF-10 runner output (LUBM-10 dev-gate baseline) + companion reproduction guide. Captured via `OUTFILE=tests/perf/lubm/baseline.lubm-10.json JSON_SCHEMA_VALIDATE=1 bash tests/perf/lubm/run-lubm.sh 10` — schema-validated against `tests/perf/lubm/schema/baseline.schema.json` at capture time. Headline numbers (on the maintainer's Colima ARM64 box): `lubm-10-ingest-nt.elapsed_ms ≈ 21,600` (1,316,700 triples → ~60K triples/s, dict_lookups ≈ 4,372,242), `lubm-10-q14-graduate-students.elapsed_ms ≈ 8` (median of 3 warm passes, result count = 24,019 GraduateStudent instances — UBA at `-seed 0` is deterministic). Both fixtures carry the conservative ±50% `elapsed_ms_pct` tolerance (CI runner noise floor); correctness fields (Q14 result count, triple count) are exact-match per the TF-12 intake rule. Note: the OG TF-9 row referenced `tests/perf/lubm.expected.json` (older naming) — superseded by the unified TF-11 layout under `tests/perf/lubm/`. Closes Track F task TF-9.
- **`tests/perf/lubm-shacl-sparql/`** — Track H task TH-4 dev-gate. Handcrafted ~10-university LUBM-shape ABox (`data.ttl`, ~50 triples, no Java UBA generator dependency), one SHACL-SPARQL constraint (`shapes.ttl` — "Course taught by at most one Professor", pure BGP + FILTER `!=`), shell harness (`run.sh`) that loads both into pgRDF and runs `pgrdf.validate` under both `'sparql'` (rudof) and `'pgrdf'` (pgRDF-native) modes for a side-by-side comparison, hand-derived `expected.json`. 2 intentional teaching collisions (u0:CS101 and u3:CS101) → 4 violations under `'pgrdf'` (correct), 0 under `'sparql'` (per E-014 — same shape topology that surfaces the rudof gap on the W3C node-sparql-001 fixture). New pgrx test `lubm_shacl_sparql_dev_gate` locks both verdicts per PG major; CI `regression` step `Run LUBM-shape SHACL-SPARQL dev-gate (TH-4)` runs the compose harness on PG17. Real LUBM-10/100/1000 with the Java generator + cross-engine timing comparison vs Jena TDB / Apache AGE land as TH-3 + the OPENBENCHMARK trajectory.
- **`guide/06-validation-recipes.md`** — Track H task TH-5: new user-facing guide page covering when to use `'native'` vs `'sparql'` vs `'pgrdf'` modes of `pgrdf.validate`. Includes the SSN-uniqueness SHACL-SPARQL worked example, decision matrix, pgRDF-native pipeline description, performance notes, and current Track A limitations (FILTER NOT EXISTS, `(expr AS ?var)`, `$PATH`). Cross-references ERRATA.v0.6 E-014 (rudof `SparqlEngine` upstream gap on common shape topologies; pgRDF-native is the recommended SHACL-SPARQL engine).
- **`tests/w3c-shacl/fixtures/sparql/`** — Track H task TH-7 MVP:
  W3C SHACL-SPARQL manifest fixture vendoring. First fixture
  `node-sparql-001` (sh:sparql at node level, 3×sh:targetNode,
  BGP + FILTER predicate-equality). Vendored as `<name>.w3c.ttl`
  (unmodified W3C provenance from
  `data-shapes-test-suite/tests/sparql/node/sparql-001.ttl` on the
  `w3c/data-shapes` gh-pages branch) + `<name>.ttl` (the
  `<>`-stripped data+shapes split oxttl can parse) +
  `<name>.expected.json` (`{"conforms":false}` hand-derived from
  the W3C `mf:result` block: 3 violation results, one per labelled
  Invalid* target). The harness `tests/w3c-shacl/run.sh` `--sparql`
  flag now walks this directory instead of running Core fixtures
  through sparql mode — a real W3C SHACL-SPARQL conformance gate
  (`mf:result`-matching) replaces the pre-TH-7 weak "conforms is
  Boolean" assertion (which was redundant with the default Core
  run). Additional W3C SHACL-SPARQL fixtures (`node-sparql-002`,
  `property/sparql-001`, `pre-binding-001` …) land incrementally
  as Track A SPARQL feature work (FILTER NOT EXISTS, advanced
  builtins) and TH-9 enhancements (`$PATH` pre-binding) ship.
- **`src/validation/pgrdf_sparql.rs`** — Track H Architecture-1 (pgRDF-native
  SHACL-SPARQL execution) module. TH-12 scaffold + TH-11 schema walker +
  TH-9 focus-node iteration / `$this` **VALUES-pre-binding** / SPI dispatch to
  `pgrdf.sparql` / result-row → `sh:ValidationResult` mapping all land.
  **Substitution mechanism note**: SHACL Part 2 §5.2 says `$this` is a
  pre-bound *variable*, not a textual macro. The first TH-9 cut tried
  naive text-replacement (`$this` → `<iri>`) — the SPARQL 1.1 grammar
  rejects IRIs in SELECT projections, so `SELECT <iri>` raised
  `parse error: expected DISTINCT` at the `pgrdf.sparql` boundary.
  Corrected within the same micro-release: `$this` → `?_pgrdf_this`
  plus a `VALUES ?_pgrdf_this { <iri> }` inline-data block injected
  at the head of the WHERE clause. End-to-end pgrx integration test
  `validate_pgrdf_mode_real_violation` proves the fix.
  Public entry `run_pgrdf_sparql(data_g, shapes_g) → JSONB`. Target
  resolution covers five well-formed `Target` variants (Node, Class,
  ImplicitClass, SubjectsOf, ObjectsOf) via direct SPI scans of
  `_pgrdf_quads`+`_pgrdf_dictionary` (no `InMemoryGraph` rehydrate —
  the whole performance point). Constraint dispatch routes the
  rewritten SPARQL through `pgrdf.sparql` (the dictionary-indexed
  hexastore path), so plan-cache reuse and indexes kick in across the
  focus-node iteration. Module ships with 6 plain Rust unit tests
  (empty-schema walk + 5 `$this` substitution edge cases including
  lowercase WHERE and missing WHERE).
  W3C SHACL-SPARQL manifest sub-run + LUBM benchmark land in TH-7 /
  TH-6 / TH-4 / TH-3.
- **`src/validation/shacl.rs`** — `serialise_graph_to_ntriples`
  visibility flipped from private to `pub(crate)` so the Track H
  pgRDF-native handler can rehydrate the shapes graph through the
  same SPI scan without duplication. **TH-8**: dispatcher arm for
  `mode => 'pgrdf'` short-circuits to
  `pgrdf_sparql::run_pgrdf_sparql(...)` before the rudof
  serialise-and-rehydrate path runs (the whole point of the
  pgRDF-native mode is to avoid `InMemoryGraph` materialisation of
  the data graph). `elapsed_ms` layered post-hoc for benchmark-row
  parity with `'native'` / `'sparql'` modes. Unknown-mode error
  message updated to list `'pgrdf'` alongside `'native'` /
  `'sparql'`. Two new pgrx integration tests
  (`validate_pgrdf_mode_real_violation`: end-to-end ex:alice as
  foaf:Person without ex:age ⇒ sh:Violation with
  `sourceConstraintComponent = sh:SPARQLConstraintComponent`;
  `validate_pgrdf_mode_empty_when_no_sparql_constraint`: no
  sh:sparql block ⇒ vacuous conform).
  `validate_unknown_mode_errors` expected string updated for the
  new mode list. No behaviour change for the existing `'native'` /
  `'sparql'` paths.
- New regression `tests/regression/sql/124-end-to-end-lexical-rehydration.sql`
  per CX-002 EVAL recommendation. Locks the dictionary rehydrate path
  against term-lexical drift across the full pipeline: parse_turtle →
  CONSTRUCT → materialize(RDFS) → CONSTRUCT → validate(SHACL Native)
  → put_construct_rows → dict de-dup. Asserts EXACT lexical values
  (IRIs + literals + datatypes + language tags) on every term shape
  pgRDF supports (custom-scheme IRIs like `ckp://Task#001` included).
  Track G task TG-4 / Track F task TF-4 (shared single source-of-truth).

### Changed

- **`Cargo.toml` / `Cargo.lock`** — bumped `shacl` from `0.3` (0.3.1)
  to `0.3.2` (workspace also rolls forward: sparql_service, rudof_rdf,
  rudof_iri, prefixmap, mie all 0.3.1 → 0.3.2). shacl 0.3.2 closes
  ERRATA.v0.5 E-012 by shipping `IRComponent::Sparql` + sh:sparql
  parser + functional `SparqlEngine` target-resolution methods that
  0.3.1 stubbed with `unimplemented!()`. Track H task TH-15. No
  behaviour change in this commit alone (the short-circuit guard
  still intercepts; deleted in the next commit).
- **`src/validation/shacl.rs`** — deleted the 16-line E-012
  short-circuit guard. `pgrdf.validate(d, s, 'sparql')` now
  dispatches into rudof's working `SparqlEngine` instead of
  returning a deterministic-unavailable structured report.
  Surface signature unchanged. Track H tasks TH-14 (guard delete)
  + TH-13 (pgrx test rewrite) shipped together.
  **Note (v0.5.4 corrected):** rudof's `SparqlValidator` trait is
  implemented for a SUBSET of Core constraints (Class, NodeKind,
  Pattern, MinLength, MaxLength, value-range bounds) but NOT yet
  for `MinCount` / `MaxCount`. So a shape relying on cardinality
  constraints may report `conforms:true` under `'sparql'` mode
  even when `'native'` reports `conforms:false`. This asymmetry
  is a rudof-side cardinality-constraint follow-up, not a pgRDF
  regression; pgRDF asserts only the contract it owns ("the
  guard is gone; dispatch reaches the upstream engine"). Tracked
  via Track H W3C SHACL-SPARQL manifest fixtures (TH-7).
- **`tests/regression/sql/122-shacl-modes.sql`** + matching
  expected/ — §D regression rewritten to lock the pgRDF-side
  contract only: `mode` echoes `sparql`, the `error` field is
  absent, `conforms` is a real Boolean (not JSON null). The
  pre-0.3.2 short-circuit-shape assertions (conforms:null, error
  field present) are gone. Track H task TH-13 (corrected
  v0.5.4).
- **`src/query/plan_cache.rs`** — added `shmem_cache::is_ready()`
  guards to `insert()`, `record_hit()`, `record_miss()`. Brings the
  plan-cache module to parity with the dict-cache module's defensive
  discipline. Correct (preloaded) deployments are unaffected;
  lazy-loaded backends now degrade to a no-op stats path instead of
  panicking with `PgAtomic was not initialized` on the first
  plan-cache miss. Track G task TG-7; regression locked by TG-6
  (plain Rust unit tests outside the pgrx postmaster fixture).
- **`README.md`** — new "Required `postgresql.conf` changes" section
  documenting the `shared_preload_libraries='pgrdf'` requirement.
  Track G task TG-5; carries the documentation commitment from
  `NOTIFIES.pgRDF.0.5.1.shared-preload-required-RESPONSE.md`.
- Public top-level **`ROADMAP.md`** — v0.6 forward look (8 tracks,
  scale gates at LUBM-10 / LUBM-100 / LUBM-1000, SHACL-SPARQL
  dual-path Track H added in r2). Stakeholder-facing "what and why";
  the engineering "how and when" stays in `docs/10-roadmap.md`.

### Fixed

- **`src/storage/graphs.rs`** — fixed a lock-order inversion in the
  pgrx test `pg_drop_graph_idempotent_absent`. The test seeded a
  graph row by INSERTing directly into `_pgrdf_graphs` (bypassing
  `add_graph` to avoid a different partition-DDL flake) and then
  called `drop_graph`, which acquires the partition-DDL advisory
  gate; the production path acquires the gate first, then the
  table-level lock — classic A→B vs B→A deadlock with the test
  taking the inverse order. Fix calls
  `acquire_partition_ddl_gate()` once at the top of the test before
  the seed INSERT; the gate is re-entrant within a transaction so
  drop_graph's own acquire later is a no-op count bump. Audited
  three sibling INSERT-direct tests (`graph_id_after_iri_add`,
  `graph_iri_direct_insert_lookup`, `graph_iri_roundtrip`); none
  subsequently take the advisory gate, so they don't form the
  inverse-order trap and were left unchanged. Track G flake
  follow-up.

### Errata

- **E-012** — closed upstream by `shacl 0.3.2` (2026-05-26) +
  pgRDF-side guard deletion (this release). LLD v0.5 §5.3 #1 status
  flip (from "adjusted per E-012" to "fully met") and the formal
  `specs/ERRATA.v0.5.md` close-out land as TH-2 in a follow-up
  commit (it's the first v0.6-era delta to ERRATA, so it opens
  `specs/ERRATA.v0.6.md` per TG-2 as a side-effect).
- **E-014** — `shacl 0.3.2` SparqlEngine returns the wrong conforms
  verdict on the W3C `tests/sparql/node/sparql-001.ttl` fixture
  (returns `conforms=true / 0 violations` even though the IR carries
  the BasicSparql constraint and the W3C `mf:result` asserts
  `conforms=false / 3 violations`). **pgRDF-native handler (TH-9 +
  TH-8) returns the correct W3C verdict** — promoted to the
  authoritative SHACL-SPARQL gate (`tests/w3c-shacl/run.sh --pgrdf`).
  The rudof `--sparql` sub-run is downgraded to a pgRDF-side
  contract assertion (conforms is Boolean, dispatch reached) —
  honest reflection of the upstream gap. New file
  `specs/ERRATA.v0.6.md` opens with E-014 as the first v0.6-era
  delta (closes Track G task TG-2 incidentally). pgrx test
  `validate_w3c_node_sparql_001_cross_mode` locks the cross-mode
  behaviour per PG major.

## [0.5.1] — 2026-05-23

**Maintenance release on top of the v0.5.0 engine surface.** No RDF /
SPARQL / SHACL / OWL engine delta; this cut packages PGXN source
distribution support, aligns the release/legal surface with MIT, and
refreshes the install + conformance docs.

### Added

- Compose install-artifact parity verification. New
  `tests/regression/scripts/verify-installed-artifacts.sh` proves a
  fresh build matches `compose/extensions/`, the running container is
  mounting this repo's exact extension files, the container bytes match
  the host bytes, and the SQL-visible version surface matches
  `pgrdf.control`. Wired into the compose PG17 regression job in CI,
  exposed as `just test-artifact-parity`, and folded into
  `just smoke-cold`. The compose-runtime default container name is now
  standardised as `pgrdf-pgrdf-postgres` across the harness scripts and
  docs.
- PGXN release mechanics. Added root `META.json`, `Makefile`,
  `INSTALL.md`, and `README.pgxn.md`; added `just pgxn-dist`; and wired
  the tagged GitHub release to attach `pgrdf-<version>.zip` alongside
  the existing binary tarballs.

### Changed

- License surface aligned to MIT across `LICENSE`, `Cargo.toml`,
  `README.md`, release notes, and release packaging. Release tarballs
  now ship `LICENSE` only; `NOTICE` is removed.
- Public install docs refreshed to current `0.5.1` examples and the
  PGXN path (`README.md`, `guide/01-install.md`,
  `docs/06-installation.md`, `compose/README.md`, `INSTALL.md`,
  `README.pgxn.md`).
- W3C SHACL docs synced to the genuine **25 / 25** Core full-pass and
  the `--sparql` known-state contract (`docs/05-validation.md`,
  `docs/08-testing.md`, `tests/w3c-shacl/README.md`,
  `tests/w3c-shacl/run.sh`).

### Fixed

- Removed a tracked local absolute-path reference from
  `specs/ERRATA.v0.4.md`.

## [0.5.0] — 2026-05-16

**The complete RDF / SPARQL / SHACL / OWL surface.** v0.5.0 is the
final cut of the v0.5 cycle (supersedes the v0.5.0-rc1
prerelease). Every v0.5-gate track §3–§8 is shipped: the
reasoning-profile selector, IRI lifecycle overloads, TriG/N-Quads
ingest, the aggregates-over-UNION residuals, the SHACL `mode`
argument, and the W3C SHACL Core manifest gate (genuine 25/25
full-pass, no exclusion — ERRATA.v0.5 **E-013** resolved). One
documented honest limitation carries: ERRATA.v0.5 **E-012**
(`shacl 0.3.1` SHACL-SPARQL constraint execution is an upstream
stub — the `mode => 'sparql'` surface ships honest +
forward-compatible, NOT a pgRDF defect, consistent with the
E-011/RDF-1.2 posture). crates.io publish stays gated on
gtfierro/reasonable#50 (E-011); v0.5.0 ships the 8 platform
tarballs + SHA256SUMS via release.yml and the OCI bundle via the
new oci-publish workflow.

### Added

- SHACL `mode` argument + W3C SHACL Core manifest gate — Phase G group G3 (slices 13-12). `pgrdf.validate(data, shapes, mode TEXT DEFAULT 'native')` adds the `mode` argument (`'native'` | `'sparql'`) alongside the byte-identical 2-arg form; JSONB gains a `mode` field; unknown modes error (`validate: unknown mode`, no silent fallback). Validation against a materialised data graph reports violations against entailed triples (regression-locked). `'sparql'` mode returns a deterministic structured "unavailable" report (ERRATA.v0.5 E-012 — the upstream `shacl 0.3.1` SHACL-SPARQL constraint component + `SparqlEngine` are unimplemented; pgRDF never invokes the broken engine, forward-compatible). New `just test-shacl-manifest` harness runs the vendored W3C SHACL Core suite (genuine 25/25 full-pass on `sh:conforms`, no exclusion — E-013 resolved) + the `--sparql` E-012 known-state assertion, wired into CI on every PG major. Closes LLD v0.5 §5 + §6 — all v0.5-gate tracks (§3-§8) complete.
- TriG / N-Quads ingest + aggregates-over-UNION residual refinements — Phase G group G2 (slices 17-14). `pgrdf.parse_trig(content, default_graph_id, strict)` and `pgrdf.parse_nquads(...)` honour inline/4th-position graph IRIs (auto-allocate via v0.4 §3.2, or reject under `strict`), reusing the v0.3 batched-insert path. Closes the six LLD v0.5 §8 aggregate-over-UNION residuals (GRAPH-scope group key, computed-BIND join key, BIND in CONSTRUCT/DESCRIBE template, nested UNION-of-UNION, cross-branch HAVING, GROUP_CONCAT DISTINCT+SEPARATOR) — the F2 stable panics are lifted. Closes LLD v0.5 §4 + §8.
- Reasoning-profile selector + IRI lifecycle overloads — Phase G group G1 (slices 21-18). `pgrdf.materialize(graph_id, profile TEXT DEFAULT 'owl-rl')` adds `'rdfs'` (RDFS rule subset) alongside `'owl-rl'`; JSONB gains a `profile` field; unknown profiles error (`materialize: unknown profile`, no silent fallback). The bare `pgrdf.materialize(g)` form is unchanged. IRI-keyed overloads `pgrdf.{drop,clear,copy,move}_graph(iri TEXT, …)` resolve via `_pgrdf_graphs` and dispatch to the v0.4 §5 partition-DDL path (error `<fn>: unknown iri` on an unbound IRI, distinct from the BIGINT no-op). Closes LLD v0.5 §3 (last ONTOSYS P1 gap) + §7.
- `specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md` (Phase H) — the next forward-look sibling beyond v0.5. Carries the v1.0 forward look (incremental delta-driven materialisation, RDF 1.2 triple terms gated on gtfierro/reasonable#50 / E-011, federated `SERVICE`, Postgres custom-scan hooks) **plus** the documented post-v0.5.0 deferrals: the **`executor.rs` core-BGP module carve** (explicitly deferred post-v0.5.0 — a large behaviour-neutral refactor, too risky to gate the v0.5.0 cut on), `heap_multi_insert`/`COPY BINARY` ingest phase B (LLD v0.4 §12, perf, non-gating), and a real SHACL-SPARQL engine (when upstream rudof ships `IRComponent::Sparql` / E-012 re-check trigger fires).
- `.github/workflows/oci-publish.yml` (Phase H) — publishes the release tarballs as OCI artifacts to `ghcr.io/styk-tv/pgrdf-bundle` (pgRDF's roadmapped target per `ERRATA.v0.2.md` / `docs/10-roadmap.md` / `INSTALL.v0.2.md`). Triggers on `release: [published]` (and `workflow_dispatch`); downloads the existing release tarballs (no rebuild), pushes per-PG×arch artifacts via ORAS, and builds aggregate `:VER` / `:vTAG` index manifests. Anonymous pull requires the GHCR package be made public (a one-time maintainer step — the Actions `GITHUB_TOKEN` lacks `admin:packages`).

### Changed

- Spec promotion (Phase H) — `specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md` promoted to authoritative `specs/SPEC.pgRDF.LLD.v0.5.md` via `git mv` (history preserved). The promoted file's §0 status flips from "draft / forward-looking" to "authoritative — shipped in pgRDF v0.5.0"; future-tense framing for every v0.5-gate track (§3–§8) converted to present-tense shipped reality. §5.3 #1 (SHACL-SPARQL constraint execution) is documented in v0.5 as an upstream-gated limitation (ERRATA.v0.5 E-012 — consistent with the E-011/RDF-1.2 posture). Every `v0.5-FUTURE` cross-reference (≈14 docs/spec files) rewired: now-shipped §§ point at `SPEC.pgRDF.LLD.v0.5.md`, forward/v1.0 items point at the new `SPEC.pgRDF.LLD.v0.6-FUTURE.md`. Mirrors the `761b82f` v0.4→v0.4 promotion precedent.

## [0.4.6] — 2026-05-16

### Added

- SPARQL multi-triple OPTIONAL + VALUES inline tables — Phase F group F1 (slices 34-31). OPTIONAL now accepts an N-triple BGP right side (LATERAL-style derived table inside the LEFT JOIN); nested OPTIONAL, OPTIONAL-internal FILTER, and optional-var outer FILTER all compose. VALUES materialises inline rows (incl. UNDEF, typed/lang literals) joined on shared variables. Both compose with GRAPH scoping + property paths and are inherited by pgrdf.construct and SPARQL UPDATE WHERE. `pgrdf.sparql_parse` no longer flags these in `unsupported_algebra` (LLD §11 acceptance).
- SPARQL downstream BIND + aggregates over UNION — Phase F group F2 (slices 30-27). BIND-introduced variables are now usable in later FILTER, BGP joins, and chained BIND (v0.3 projection-only limitation lifted). Aggregates (COUNT/SUM/AVG/type-aware MIN-MAX/GROUP_CONCAT/SAMPLE, with GROUP BY/HAVING/DISTINCT) now run over UNION'd patterns via a derived-table refactor. Both compose with GRAPH scoping, F1 OPTIONAL/VALUES, and property paths, and are inherited by pgrdf.construct + SPARQL UPDATE WHERE. `pgrdf.sparql_parse` no longer flags these in `unsupported_algebra` (LLD §11 acceptance). Residual aggregate-over-UNION refinements tracked in LLD v0.5 §8.
- SPARQL DESCRIBE via `pgrdf.describe(q TEXT) → SETOF JSONB` — Phase F group F3 (slices 26-24). Sibling UDF to pgrdf.construct (same {subject,predicate,object} structured-term shape). Supports `DESCRIBE <iri>`, `DESCRIBE ?v WHERE {...}`, mixed constant+variable terms, and `DESCRIBE *`; the description is the closure of each resource (every triple with the resource as subject) transitively expanded one hop through blank-node objects per W3C §16.4 (cycle-safe, dedup'd). Composes with GRAPH scoping. `pgrdf.sparql_parse` reports `form:"DESCRIBE"`; no longer flagged in `unsupported_algebra` (LLD §11 acceptance).
- SPARQL type-aware ORDER BY + Phase F close-out — group F4 (slices 23-22). ORDER BY now sorts across the SPARQL 1.1 §15.1 value space (unbound < bnode < IRI < literal; numerics numerically, xsd:dateTime chronologically, strings by codepoint), DESC + multi-key + expression sort keys; stable/total (never raises on incomparable). Phase F W3C-shape fixtures 42-47. Compose extension-mount made version-agnostic (removes the stale pgrdf--0.4.1.sql hardcoded pin). §11 SPARQL backlog complete; residual aggregate-over-UNION refinements tracked in LLD v0.5 §8.

## [0.4.5] — 2026-05-16

**Marquee: full SPARQL 1.1 property paths.** Closes the LLD v0.4 §7
property-path column across the four-group Phase E countdown
(49 → 35). `^` inverse, `+` one-or-more, `*` zero-or-more, `?`
zero-or-one, and `|` alternation all execute, composing with
named-graph scoping, multi-pattern BGP joins, OPTIONAL/UNION/MINUS,
and `pgrdf.construct` for free (the shared WHERE walker recognises
`GraphPattern::Path` at the single chokepoint every query form
routes through — path support is inherited, not special-cased).
Recursive operators lower to a `WITH RECURSIVE` CTE as a derived
FROM relation with Postgres's `CYCLE src, dst` clause for
cycle-safe termination and a `pgrdf.path_max_depth` GUC (default
64, range 1-1024) depth guard that truncates rather than errors
(`pgrdf.stats().path_depth_truncations` accounts a genuine acyclic
cap-hit). `*`/`?` carry the precise W3C SPARQL 1.1 §9.3
zero-length-path semantics (a bound endpoint's self-pair holds
unconditionally; an unbound endpoint's node-set is the active
scope's subject∪object). The §7.1 `|` stretch shipped **in full**:
the predicate match was generalised from a single `predicate_id =
$P` to a predicate **set** (`predicate_id IN (…)` — a 1-element
set is byte-identical, so `+`/`*`/`?` are unchanged), a cheap
uniform one-line change at each builder rather than a translator
balloon, so the recursion compositions `(a|b)+` / `(a|b)*` /
`(a|b)?` and the inverse `^(a|b)` / `(^a|^b)` all ship too. The
materialised-closure no-CTE fallback (§7.2 v0.4 heuristic / §7.3
acceptance) elides the recursive CTE for a `+`/`*` over a single
well-known transitive predicate (`rdfs:subClassOf` /
`rdfs:subPropertyOf` / `owl:sameAs`) once `pgrdf.materialize` has
entailed the closure — the executed plan carries no `CTE Scan` and
the result set is byte-identical. The §7.1-permitted gated
remainder (an alternation arm that is itself a sequence/recursive
path; a recursive op whose inner box is a sequence) stays
preview-panicking by spec allowance; negated property sets remain
out of v0.4 scope. The deferred-all-phase Phase E W3C-shape
consolidation landed (6 fixtures `36-path-inverse` …
`41-path-materialised`, 35 → 41).

Phase E slice attribution (countdown 49 → 35):

  * **E1 (49 → 46)** — property-path AST detection + translator
    dispatch; `^` inverse fully supported (`?s ^p ?o` ≡ `?o p ?s`;
    nested `^(^p)` folds by parity; bare-predicate degenerate
    `Path` lowers to a triple). New GUC `pgrdf.path_max_depth`;
    new `pgrdf.stats()` field `path_depth_truncations` (scaffold —
    enforcement lands E2). Recursive/alternation operators
    preview-panic with stable rollout-schedule prefixes.
  * **E2 (45 → 42)** — `+` one-or-more: the LLD v0.4 §7.2
    `WITH RECURSIVE walk(src, dst, depth)` CTE as a derived FROM
    relation, cycle-safe via Postgres's `CYCLE` clause (a bare
    `UNION` can't dedup a cycle once the tuple carries the
    depth-guard column), depth guard enforced (truncate, never
    error). All property-path SQL generation carved into
    `src/query/path.rs`.
  * **E3 (41 → 38)** — `*` zero-or-more (the cycle-safe `+` walk
    `UNION` the W3C §9.3 zero-length node-set) and `?` zero-or-one
    (non-recursive: the single direct edge `UNION` the same
    node-set). Full W3C SPARQL 1.1 §9.3 `ZeroLengthPath` rules;
    inverse composition (`^(p*)` / `(^p)*` / `^(p?)` / `(^p)?`).
  * **E4 (37 → 35)** — `|` alternation (top-level `a|b`, n-ary
    `a|b|c`, `(a|b)+` / `(a|b)*` / `(a|b)?`, `^(a|b)` / `(^a|^b)`)
    via the predicate-set generalisation; materialised-closure
    no-CTE fallback + the `pgrdf.sparql_sql(q) → TEXT` debug hook
    (the §7.3 EXPLAIN-scrape acceptance); Phase E W3C-shape
    consolidation; the v0.4.5 release cut.

Test bar:

  pgrx integration  230  (was 222 at v0.4.4 / Phase E3)
  pg_regress         73  (path coverage 108–111)
  w3c-sparql         41  (was 35 — +6 property-path fixtures)
  LUBM-shape          3  (unchanged)
  Total: 347 green, plus the pg_dump round-trip gate.

Version touches:
  * Cargo.toml      0.4.4 → 0.4.5 (Cargo.lock pgrdf entry too)
  * pgrdf.control   default_version = '0.4.5'
  * compose/compose.yml  adds pgrdf--0.4.5.sql bind mount
  * tests/regression/expected/00-smoke.out  0.4.4 → 0.4.5

E-011 carried: `publish-crate.yml` stays disabled until upstream
`gtfierro/reasonable#50` merges. The tag push fires `release.yml`
only (8 platform tarballs PG14-17 × amd64/arm64 + SHA256SUMS); no
crates.io publish this cut.

### Added

- SPARQL property-path foundation — Phase E group E1 (slices 49-46). Property-path AST detection + translator dispatch; `^` inverse operator fully supported (`?s ^p ?o` ≡ `?o p ?s`, composes with GRAPH scoping / BGP joins / pgrdf.construct). New GUC `pgrdf.path_max_depth` (default 64, range 1-1024). New `pgrdf.stats()` field `path_depth_truncations` (depth-guard enforcement lands with the recursive operators in group E2). Recursive operators `*`/`+`/`?` and alternation `|` preview-panic with stable rollout-schedule prefixes.
- SPARQL property path `+` (one-or-more) — Phase E group E2 (slices 45-42). Transitive non-reflexive closure via WITH RECURSIVE; cycle-safe via Postgres `CYCLE src, dst` clause (a bare UNION can't dedup a cycle once the working tuple carries the depth-guard column). Depth-guard now enforced: traversal is capped at `pgrdf.path_max_depth` (truncate, not error) and `pgrdf.stats().path_depth_truncations` increments on a genuine acyclic cap-hit (a fully-resolved cyclic query reports no truncation). `^p+` inverse-composition, GRAPH scoping, BGP joins, and `pgrdf.construct` inheritance all supported. Property-path translation carved into `src/query/path.rs` (behaviour-preserving; executor.rs shrinks).
- SPARQL property paths `*` (zero-or-more) and `?` (zero-or-one) — Phase E group E3 (slices 41-38). Full W3C SPARQL 1.1 §9.3 zero-length-path semantics: reflexive pairs follow endpoint-binding rules (bound endpoint → unconditional self-pair; unbound → graph node-set, scoped to the active GRAPH). `*` reuses E2's cycle-safe recursive CTE + depth-guard for its transitive part; `?` is non-recursive (direct ∪ identity). Inverse composition (`^(p*)` etc.), GRAPH scoping, BGP joins, and pgrdf.construct inheritance all supported.
- SPARQL property-path alternation `|` + materialised-closure no-CTE fallback — Phase E group E4 (slices 37-35), closing the §7 property-path surface. The §7.1 alternation stretch shipped in full: the predicate match generalised from a single `predicate_id = $P` to a predicate set (`predicate_id IN (…)` — a 1-element set is identical, so plain `+`/`*`/`?` are byte-unchanged), a cheap uniform change at each builder. Top-level `a|b`, n-ary `a|b|c`, the recursion compositions `(a|b)+`/`(a|b)*`/`(a|b)?`, and the inverse `^(a|b)`/`(^a|^b)` all execute. The materialised-closure no-CTE fallback: a `+`/`*` over a single well-known transitive predicate (`rdfs:subClassOf`/`rdfs:subPropertyOf`/`owl:sameAs`) with `is_inferred` rows present emits a direct match instead of the recursive CTE (no `CTE Scan` in the executed plan — semantics-preserving, per-query detection). New `pgrdf.sparql_sql(q) → TEXT` debug hook returns the translated SQL (dict ids inlined) for the §7.3 EXPLAIN-scrape acceptance. The §7.1-permitted gated remainder (an alternation arm that is itself a sequence/recursive path; a recursive op whose inner box is a sequence) stays preview-panicking by spec allowance; negated property sets remain out of v0.4 scope. Phase E W3C-shape consolidation: 6 new property-path fixtures (`36-path-inverse` … `41-path-materialised`).

## [0.4.4] — 2026-05-15

**Marquee: SPARQL 1.1 CONSTRUCT surface complete.** Closes the LLD
v0.4 §6 CONSTRUCT column by landing the full query form end-to-end
on the SQL engine. `pgrdf.construct(q TEXT) → SETOF JSONB` is a
sibling UDF to `pgrdf.sparql` (callers signal intent at the SQL
boundary): it evaluates the WHERE pattern through the existing
SELECT-side translator (`parse_select` → `build_bgp_sql` →
`execute`), then instantiates the template once per solution and
emits one JSONB row per template triple. Constant, variable, and
blank-node template positions are all supported; blank-node labels
mint fresh per solution and join to the same fresh label across
positions (single-triple) and across all N triples within a
multi-triple template (same solution). The `CONSTRUCT WHERE {
pattern }` shorthand (W3C SPARQL 1.1 §16.2.4) and GRAPH-scoped
WHERE (`GRAPH <iri>` literal + `GRAPH ?g` variable, §13.3) compose
with every template surface. Round-trip is closed:
`pgrdf.put_construct_row` / `put_construct_rows` re-ingest any
construct rowset back into the hexastore, preserving typed
literals, language tags, and within-batch blank-node joining
idempotently (LLD v0.4 §6.3). `pgrdf.sparql_parse(q)` mirrors the
executor's CONSTRUCT classification (`form: "CONSTRUCT"`, `template`
+ `where_shape` blocks, `shorthand` flag, `unsupported_algebra`)
so callers can preview translatability without executing.

Phase D slice attribution (countdown 59 → 50):

  * 59 — `pgrdf.construct(q)` foundation, constant-only templates
    per W3C 1.1 §16.2; structured term shape `{type, value,
    datatype?, language?}` per LLD v0.4 §6.1.
  * 58 — template variable substitution (subject / predicate /
    object positions; typed + language-tagged literals carry full
    structured shape).
  * 57 — blank-node templates, fresh-per-solution labels with
    within-solution label sameness (single-triple scope).
  * 56 — multi-triple templates (N triples emit N rows per
    solution; blank-node labels shared across the N triples within
    one solution; empty templates reject).
  * 55 — GRAPH-scoped WHERE (`GRAPH <iri>` + `GRAPH ?g`; default-
    graph quads excluded per §13.3 — also corrected a latent
    slice-79 / slice-87 SELECT-side bleed).
  * 54 — `CONSTRUCT WHERE { pattern }` shorthand (§16.2.4; pure-
    BGP, blank-node-free).
  * 53 — round-trip ingest (`pgrdf.put_construct_row` /
    `put_construct_rows`), closing §6.3.
  * 52 — `pgrdf.sparql_parse` CONSTRUCT shape enrichment.
  * 51 — W3C-shape CONSTRUCT conformance fixtures 30-35 + docs /
    spec / guide coherence sweep.
  * 50 — version bump + RELEASE_NOTES + tag (this cut).

CI-perf hardening (landed alongside Phase D): the partition-DDL
window in SPARQL UPDATE / lifecycle paths now takes a statement-
outermost transaction advisory lock, so the default parallel
pgrx-test scheduler no longer flakes on concurrent partition DDL —
parallel test threads are restored (no `--test-threads=1`).

Test bar:

  pgrx integration  194  (was 166 at v0.4.3)
  pg_regress         69  (was  61 at v0.4.3)
  w3c-sparql         35  (was  29 at v0.4.3)
  LUBM-shape          3  (unchanged)

  Total: 301 green.

### Added

- `pgrdf.sparql_parse` CONSTRUCT support — Phase D slice 52. Returns `form: "CONSTRUCT"` with `template` shape (triple count, has_variables, has_blank_nodes, has_constants_only, variables) and `where_shape` (kind, triple_count, named_graphs_used, variables). Detects shorthand (`CONSTRUCT WHERE { ... }`) form via `shorthand` flag. Flags `Distinct`/`OrderBy`/`Group`/`Aggregate` wrappings as `unsupported_algebra` (will panic at execute time per LLD §6.2).
- `pgrdf.put_construct_row(row JSONB, graph_id BIGINT DEFAULT 0) → BIGINT` and `pgrdf.put_construct_rows(rows JSONB[], graph_id BIGINT DEFAULT 0) → BIGINT` ingest UDFs — Phase D slice 53. Round-trip pairing for `pgrdf.construct`: any rowset emitted by construct can be re-ingested to reproduce the original graph state (modulo dict id reshuffles per LLD v0.4 §6.3). Typed literals, language tags, plain strings (with the explicit `xsd:string` datatype the construct emitter writes), and blank-node within-solution joining are all preserved. The plural form is the recommended surface — a per-call `HashMap<String, i64>` of blank-node labels keeps repeated bnode references within one batch collapsed onto a single stored blank node, so the within-solution sameness from slice 56 / 57 survives the round-trip. Re-ingestion is idempotent (set semantics via `WHERE NOT EXISTS`, mirroring `executor::insert_quad`). A NULL array (from `array_agg` over an empty construct rowset) is a no-op. Negative `graph_id` and literals in subject/predicate position both panic with the stable `pgrdf.put_construct_row:` prefix.
- `pgrdf.construct` WHERE-shorthand form — Phase D slice 54. `CONSTRUCT WHERE { pattern }` is equivalent to `CONSTRUCT { pattern } WHERE { pattern }` per W3C SPARQL 1.1 §16.2.4. The pattern must be a pure BGP (no OPTIONAL/UNION/MINUS/FILTER/GRAPH/BIND/VALUES) and must contain no blank nodes; both restrictions panic with explicit W3C-citing messages. Compatible with all prior surfaces (multi-triple BGPs emit one row per template triple per solution).
- `pgrdf.construct` GRAPH-scoped WHERE — Phase D slice 55. `WHERE { GRAPH <iri> { ... } }` and `WHERE { GRAPH ?g { ... } }` compose with all prior template surfaces (constant, variable, blank-node, multi-triple). Variable-GRAPH binds `?g` to the source graph IRI per solution; default-graph quads are excluded per W3C SPARQL 1.1 §13.3. Empty named graphs and missing graphs yield zero solutions.
- `pgrdf.construct` multi-triple template support — Phase D slice 56.
  N-triple templates emit N rows per solution; blank-node labels are
  shared across all N triples within the same solution (and fresh per
  solution). Empty templates `{ }` panic with `empty template`.
  Variable substitution + constant resolution + within-solution bnode
  label-joining all compose cleanly across template positions.
- `pgrdf.construct(q TEXT) → SETOF JSONB` UDF — Phase D slice 59
  foundation. Constant-only templates per W3C 1.1 §16.2 supported;
  variables / blank nodes panic with `slice 59 supports constant-only
  templates` until slices 58 / 57 widen. DISTINCT / ORDER BY /
  GROUP BY / aggregates on CONSTRUCT explicitly rejected (out of scope
  per spec). Term encoding is the structured shape
  `{"type": "iri"|"literal"|"bnode", "value": …, "datatype"?: …,
  "language"?: …}` documented in LLD v0.4 §6.1 — sibling UDF rather
  than overloading `pgrdf.sparql` so callers signal intent at the
  SQL boundary. WHERE pattern reuses the full SELECT-side translator
  (`parse_select` → `build_bgp_sql` → `execute`).
- `pgrdf.construct` template variable substitution — Phase D slice 58.
  Variables in subject, predicate, and object positions are resolved
  per solution. Unbound template variables panic with
  `unbound template variable ?X`. Blank nodes still rejected (lands in
  slice 57). Typed and language-tagged literals carry full structured
  shape `{type, value, datatype, [language]}` — language-tagged
  literals carry both the `language` field AND the implicit
  `rdf:langString` datatype IRI per RDF 1.1 §3.3. Blank-node bindings
  the WHERE pattern surfaces shape as `{"type":"bnode","value":"_:b…"}`
  via the dictionary-driven term resolver. Variable predicates are
  legal (RDF admits variable predicates in templates).
- `pgrdf.construct` blank-node template support — Phase D slice 57.
  `_:label` in template positions mints fresh per-solution labels per
  W3C SPARQL 1.1 §16.2; same template label across positions in the
  same solution joins to the same fresh label (single-triple scope;
  multi-triple joining lands in slice 56). Predicate-position blank
  nodes are illegal RDF and rejected at parse time by spargebra
  (surfacing as `pgrdf.construct: parse error: …`). Variable-bound
  blank nodes from WHERE pass through with original labels unchanged.
  Multi-triple templates panic with `pgrdf.construct: slice 57
  supports single-triple templates; multi-triple lands in slice 56`.
  Per-call fresh labels carry the solution index as a prefix
  (`b{solution}_{n}`) so callers can rely on the `value` column alone
  to distinguish per-solution bnodes within one `pgrdf.construct`
  call.

### Fixed

- `GRAPH ?g { ... }` no longer matches default-graph quads in either `pgrdf.sparql` or `pgrdf.construct` (Phase D slice 55). The `_pgrdf_graphs` JOIN now carries an `AND g{S}.graph_id <> 0` predicate so variable GRAPH ranges over named graphs only, per W3C SPARQL 1.1 §13.3. Prior slice-79 / slice-87 behaviour silently bound `?g` to `urn:pgrdf:graph:0` when default-graph quads existed alongside named-graph quads; the slice-87 regression's case-4 expected-count was updated from 5 to 3 to reflect the corrected behaviour. Mandatory, OPTIONAL, MINUS, and UNION-branch joins to `_pgrdf_graphs` all carry the predicate uniformly.

## [0.4.3] — 2026-05-15

**Marquee: SPARQL UPDATE surface complete.** Closes the LLD v0.4 §4
UPDATE-form column by landing every documented variant end-to-end
on the SQL engine: `INSERT DATA`, `DELETE DATA`, `INSERT { template }
WHERE { pattern }`, `DELETE { template } WHERE { pattern }` (and the
shorthand `DELETE WHERE`), and the atomic combined `DELETE … INSERT
… WHERE … modify`. Every form is graph-scope-aware — `GRAPH <iri>`
in the template / WHERE, the `WITH <iri>` shortcut, and cross-graph
copy patterns all work. The lifecycle algebra (`DROP / CLEAR /
CREATE GRAPH` with `DEFAULT / NAMED / ALL` targets and the `SILENT`
modifier) routes through the existing §5 graph-management UDFs so
the SPARQL surface and the SQL UDF surface stay as two consumers of
the same partition-level primitives. `pgrdf.sparql_parse(q)`
mirrors the executor's runtime classification on every UPDATE op,
so callers can preview shape, target graphs, and routing inputs
without running the query.

Phase C slice attribution (countdown 84 → 60):

  * 84 — foundation + `INSERT DATA` (`pgrdf.sparql` dispatch via
    parse_query → parse_update fallback, `_update` summary row).
  * 83 — `DELETE DATA`.
  * 82 — `INSERT { template } WHERE { pattern }`.
  * 81 — `DELETE { template } WHERE { pattern }` (and `DELETE WHERE`
    shorthand).
  * 80 — `DELETE { … } INSERT { … } WHERE { … }` (atomic modify).
  * 79 — graph-scoped variants (`WITH <iri>` + `GRAPH <iri>` in
    template / WHERE; cross-graph copy).
  * 78 — lifecycle algebra (`DROP / CLEAR / CREATE GRAPH` +
    `DEFAULT / NAMED / ALL` targets + `SILENT`).
  * 77-75 — W3C-shape conformance fixtures (`tests/w3c-sparql/27-29`)
    + harness `elapsed_ms` normalisation.
  * 74 — `pgrdf.sparql_parse` UPDATE detail enrichment (`kind`
    label, `with_graph` IRI, `template_graphs` array, lifecycle
    `target` labels).
  * 71-65 — docs + spec + README + CHANGELOG sync.
  * 64-60 — version bump + RELEASE_NOTES + tag.

Test bar:

  pgrx integration   166  (was 159 at v0.4.2: +7 sparql_parse cases)
  pg_regress          61  (was 54 at v0.4.2: +7 UPDATE-form regressions)
  w3c-sparql          29  (was 26 at v0.4.2: +3 UPDATE-form fixtures)
  LUBM-shape           3  (unchanged)

### Phase C slice 74 — `sparql_parse` UPDATE detail enrichment

`pgrdf.sparql_parse(q)` now surfaces enough of the executor's
routing inputs that callers can preview an UPDATE's effect without
running it. Per-op shape changes:

- `InsertData` — unchanged (triples + graphs).
- `DeleteData` — `graphs` array added (matches `InsertData`).
- `DeleteInsert` — `kind` label added, narrowing to one of
  `INSERT_WHERE` / `DELETE_WHERE` / `DELETE_INSERT_WHERE` (mirrors
  the executor's runtime `_update.form` so callers can route on
  the same key). `template_graphs` collects every template-side
  graph IRI (default-graph quads surface as `"DEFAULT"`, variable
  graphs as `"?var"`). `with_graph` carries the `WITH <iri>` IRI
  when the operation has a single-default `using:` field.
- `Clear` / `Drop` — `target` label (`DEFAULT` / `NAMED <iri>` /
  `NAMED_ALL` / `ALL`) + `silent` flag.
- `Create` — `target` (`NAMED <iri>`) + `silent`.

Helpers added: `push_graph_name`, `push_graph_name_pattern`,
`with_iri_from_using` (parser-side mirror of the executor's
namesake, tolerates multi-IRI / USING NAMED by returning `None`
instead of panicking so `sparql_parse` stays infallible on every
parsed AST), `graph_target_label`.

Seven new `#[pg_test]` cases lock the new fields (kind-narrowing
for the three DeleteInsert kinds, `with_graph` surfacing,
`template_graphs` surfacing, `DeleteData.graphs` surfacing, and
lifecycle `target` labels across CLEAR DEFAULT / DROP GRAPH <iri> /
CREATE SILENT GRAPH / DROP ALL).

### Phase C slices 77-75 — W3C-shape conformance fixtures for SPARQL UPDATE

Three new fixtures under `tests/w3c-sparql/` lock the UPDATE
surface through the conformance harness, complementing the
`tests/regression/sql/93-…-99-…` SQL-side regression set:

- `27-update-insert-data` — §3.1.1 INSERT DATA, default + named
  graph in a single query.
- `28-update-delete-where` — §3.1.3 DELETE WHERE pattern-driven.
- `29-update-with-graph-scope` — §3.1.3 ¶3 `WITH <g>` scopes
  WHERE + template.

Each fixture uses `setup.sql` (no `data.ttl`) because UPDATE forms
LAND state via the query itself — the default `data.ttl` seed path
would pre-stage rows the query is trying to verify it lands,
blurring the assertion.

Harness extension: `tests/w3c-sparql/run.sh` now normalises
`elapsed_ms: <N>` inside `_update` rows to `elapsed_ms: 0` before
diffing so bag-equivalence stays stable across runs. The
substitution is narrow (matches only the JSON key/value pair via
a sed regex); SELECT / ASK rows are untouched. Existing 26 tests
(01-26) remain identical.

Test count: w3c-sparql 26 → 29 (+3 UPDATE forms).

### Phase C slice 78 — SPARQL UPDATE lifecycle algebra (`DROP / CLEAR / CREATE GRAPH`)

Closes the LLD v0.4 §4.4 lattice between the SPARQL UPDATE lifecycle
forms and the §5 SQL UDF surface. The three `GraphTarget`-bearing
`spargebra::GraphUpdateOperation` variants (`Drop`, `Clear`,
`Create`) now route through `pgrdf.drop_graph(id, true)`,
`pgrdf.clear_graph(id)`, and `pgrdf.add_graph(iri TEXT)` (§5 slices
99 / 98 / 118 respectively). The three "lands in slice 69/70/71"
panics in `src/query/executor.rs::execute_update` are replaced by
real dispatchers. Toward v0.4.3.

**Routing through SQL, not Rust direct.** The dispatcher uses
`Spi::get_one_with_args("SELECT pgrdf.clear_graph($1)", …)` (and
siblings) rather than calling the `#[pg_extern]` functions in
`src/storage/graphs.rs` directly. This keeps the SPARQL front-end
and the SQL UDF front-end as two consumers of the same partition-
level primitives — every existence check, partition-DDL window
(`DETACH PARTITION` / `DROP TABLE` / `TRUNCATE ONLY`), inferred-row
cascade guard, and `_pgrdf_graphs` binding update happens once in
the UDFs, not twice.

**`GraphTarget` enum coverage.** spargebra-0.4.6 models the SPARQL
`GraphRef` / `GraphRefAll` grammar as a four-variant enum:

```rust
pub enum GraphTarget {
    NamedNode(NamedNode),  // GRAPH <iri>
    DefaultGraph,          // DEFAULT
    NamedGraphs,           // NAMED  — every IRI-bound named graph
    AllGraphs,             // ALL    — including the default partition
}
```

The dispatcher branches on all four:

- `NamedNode(iri)` → lookup `_pgrdf_graphs` for the bigint id;
  panic with `DROP GRAPH <iri>: graph not bound` (or `CLEAR GRAPH
  <iri>: graph not bound`) when absent, unless `SILENT` was
  specified (no-op).
- `DefaultGraph` → direct `DELETE FROM _pgrdf_quads WHERE graph_id
  = 0` for BOTH `CLEAR DEFAULT` AND `DROP DEFAULT`. `pgrdf.clear_graph(0)`
  only handles the explicit `_pgrdf_quads_g0` partition (created
  when `add_graph(0)` runs); routine default-graph inserts land in
  `_pgrdf_quads_default` (the LIST partition catch-all), which
  the §5 UDF misses entirely. The partition-wide DELETE handles
  both via Postgres partition routing. W3C SPARQL 1.1 Update
  §3.1.3 paragraph 7 makes `DROP DEFAULT` an "empty, not destroy";
  this also avoids the slice-99 `pgrdf.drop_graph(0)` panic guard
  (the default catch-all partition is non-droppable).
- `AllGraphs` → enumerate every `graph_id` in `_pgrdf_graphs`
  (including 0) and dispatch per-id; the post-state under `CLEAR
  ALL` is every partition empty with bindings preserved.
- `NamedGraphs` → enumerate every `graph_id <> 0` (default
  excluded per W3C §3.1.3); the post-state under `DROP NAMED` is
  every named partition removed AND its binding gone, with the
  default untouched.

**`CREATE` semantics.** `pgrdf.add_graph(iri TEXT)` (slice 118) is
idempotent on the IRI — re-calls return the existing id without
allocating a second partition — but the W3C SPARQL 1.1 Update
`CREATE GRAPH <iri>` MUST error when the IRI is already bound
unless `SILENT` was specified. The dispatcher pre-checks via
`lookup_graph_id` and panics with `CREATE GRAPH <iri>: graph
already exists` when bound + not silent; SILENT collapses the
"already bound" path to a no-op (the existing binding survives,
the summary still records the touched graph_id for operator
audit). CREATE never touches row counts (`triples_inserted = 0`).

**ADD / MOVE / COPY desugar at parse time.** Per spargebra-0.4.6
parser.rs §Add / §Move / §Copy, the SPARQL surface keywords ADD,
MOVE, COPY are NOT separate `GraphUpdateOperation` variants —
they desugar at parse time into compositions of `Drop +
DeleteInsert` (for COPY) / `Drop + DeleteInsert + Drop` (for MOVE)
/ a plain `DeleteInsert` (for ADD). Those compositions ride the
existing per-form dispatcher arms (`DeleteInsert` + `Drop`)
already wired by slices 80 / 78. No new code path needed.

**`update_op_name` discriminator.** The `form` field in the
`_update` summary continues to use `"CLEAR"` / `"CREATE"` /
`"DROP"` for the single-op shapes (unchanged from the
discriminator table installed at slice 84). Multi-op Updates
that mix lifecycle ops with INSERT/DELETE forms collapse to
`"MIXED"` via the existing `form != op_name` rule — callers
inspect the per-op detail via `pgrdf.sparql_parse(q)`.

**Regression coverage.** `tests/regression/sql/99-update-lifecycle-algebra.sql`
locks eight invariants — DROP GRAPH counter + binding removal,
CLEAR GRAPH counter + binding preservation, CREATE GRAPH happy
path + SILENT idempotency, DROP GRAPH not-bound panic without
SILENT, DROP SILENT GRAPH not-bound no-op, CLEAR DEFAULT
counter + post-state row count, CLEAR ALL summed counter +
binding preservation. Hand-authored expected output via the
shared `_check_error` helper from slice 81-error-paths.
Three `#[pg_test]`s in `src/query/executor.rs`:
`sparql_update_drop_graph_named_happy_path`,
`sparql_update_clear_graph_named_preserves_binding`,
`sparql_update_create_graph_idempotent_silent`.

### Phase C slice 79 — SPARQL UPDATE graph-scoped variants (`WITH <iri>` + `GRAPH <iri>` in template / WHERE)

Closes the graph-aware loop for pattern-driven UPDATEs. The three
DeleteInsert dispatch arms (pure INSERT WHERE, pure DELETE WHERE,
combined DELETE+INSERT WHERE) now honour `WITH <iri>` and
`GRAPH <iri> { … }` end-to-end. Toward v0.4.3.

**Strategy.** Spargebra-0.4.6's parser desugars `WITH <iri>` at
parse time (parser.rs §Modify) into:

1. **Per-quad `graph_name` injection** on every template
   QuadPattern / GroundQuadPattern whose `graph_name` is
   `GraphNamePattern::DefaultGraph` — rewritten to
   `GraphNamePattern::NamedNode(<iri>)` on both DELETE and INSERT
   halves. The per-row instantiators `instantiate_template_quad`
   (slice 82) and `instantiate_ground_template_quad` (slice 81)
   already routed `NamedNode` into the right partition via
   `resolve_or_allocate_graph` (insert path) / `lookup_graph_id`
   (delete path) — that half was a free regression test.
2. **A `using` sentinel** on the DeleteInsert operation:
   `Some(QueryDataset { default: vec![<iri>], named: None })`.

(1) is preserved verbatim. (2) is new: the slice-79 dispatcher
(in `src/query/executor.rs::execute_update`'s three DeleteInsert
arms) calls a small `with_iri_from_using(using, form_label)`
helper that returns `Some(iri)` for the WITH-injected
single-default-graph shape, panics with the stable
`'USING / USING NAMED' not yet supported` prefix on proper USING
forms (multi-default or USING NAMED), and `None` for
`using.is_none()`. When `Some(iri)`, `scope_pattern_to_graph(
pattern, iri)` wraps the WHERE pattern in `GraphPattern::Graph
{ name: NamedNodePattern::NamedNode(iri), inner: Box::new(
pattern) }` before the call into `execute_*_where`. The
slice-112 walker then scopes every emergent BGP triple to
`<iri>` — nested explicit `GRAPH <other> { … }` still overrides
per W3C §13.3, OPTIONAL/UNION/MINUS inside inherit the scope.

**`GRAPH <iri> { … }` in the WHERE pattern** was already
supported (slice 112). **`GRAPH <iri> { … }` in the template
halves** was already wired through the per-quad `graph_name`
branches in slices 80/81/82. Slice 79 makes both first-class
end-to-end by removing the unconditional `using.is_some()`
panic that previously short-circuited the dispatcher arms and
locks the behaviour with hand-authored regressions + pgrx
tests.

**Cross-graph shapes now first-class.**

- `INSERT DATA { GRAPH <g> { … } }` / `DELETE DATA { GRAPH <g>
  { … } }` — already supported since slices 83/84; locked again
  in 98 as a regression.
- `INSERT { GRAPH <g2> { ?x ex:tag "t" } } WHERE { GRAPH <g1>
  { ?x ex:p ?o } }` — cross-graph copy. WHERE scopes to `<g1>`
  via slice 112; template's per-quad `graph_name = NamedNode<g2>`
  routes inserts into `<g2>`'s partition.
- `DELETE { GRAPH <g> { ?s ?p ?o } } WHERE { GRAPH <g> { ?s ?p
  ?o } }` — scoped wipe; default-graph rows untouched.
- `WITH <g> INSERT { ?x ex:tag "t" } WHERE { ?x ex:p ?o }` —
  spargebra injects `graph_name = <g>` into the INSERT template
  AND emits the USING sentinel; the dispatcher wraps the WHERE
  in `GRAPH <g> { … }`. End result: WHERE evaluates against
  `<g>` only, INSERTs land in `<g>` only. The proof in the
  regression is the counter: without the WHERE wrap, a bare-BGP
  `?x ex:p ?o` would match 4 globally (2 in g1 + 1 in g2 + 1 in
  default — per slice 114's preserved "bare BGP scans all
  partitions" semantics); WITH shrinks that to exactly 2.
- `WITH <g> DELETE { ?x ex:status "draft" } INSERT { ?x ex:status
  "approved" } WHERE { ?x ex:status "draft" }` — atomic modify
  scoped to `<g>`; the default-graph's "draft" row stays put.

**Limitations.**

- Proper `USING <iri>` / `USING NAMED <iri>` clauses (distinct
  from the WITH-injected sentinel — recognised by `default.len()
  != 1` OR `named.is_some_and(|v| !v.is_empty())`) panic with
  the stable `'USING / USING NAMED' not yet supported` prefix.
  These have richer semantics (RDF-merge across multiple default
  graphs, named-graph routing) that are out of scope for v0.4.
- `WITH` combined with explicit `USING` would ambiguate "which
  IRI wins for the WHERE default graph" — same panic.
- Variable graph in template (`INSERT { GRAPH ?g { … } }`)
  remains gated to slice 76.

**Test coverage.**

- `tests/regression/sql/98-update-graph-scoped.sql` locks six
  invariants (described above). Hand-authored expected output;
  never ACCEPT=1 baselined.
- Three `#[pg_test]`s in `src/query/executor.rs`:
  `sparql_update_with_insert_where_scopes_both_halves`,
  `sparql_update_cross_graph_insert_where`,
  `sparql_update_with_delete_insert_where_scopes_modify`. All
  bypass the parallel-`add_graph` deadlock flake by routing
  graph allocation through `INSERT DATA { GRAPH <g> { … } }`
  calls (single-step quad-and-graph allocation in one
  transaction) and inspecting the named partitions directly via
  `pgrdf.graph_id(<iri>)`.

**Test bar after slice 79.** 156 pgrx integration + 60 pg_regress
+ 26 W3C-shape + 3 LUBM-shape = **245 automated tests** across
all four layers (up from 241 at slice 80: +3 pgrx, +1
pg_regress).

### Phase C slice 80 — SPARQL UPDATE DELETE+INSERT WHERE (combined modify)

The atomic "modify" form. The DeleteInsert dispatcher arm
`(true, true)` now routes through `execute_delete_insert_where`
rather than panicking with the slice-77 "lands" prefix. Toward
v0.4.3.

**Strategy.** Both halves resolve against the SAME WHERE solutions
snapshot. We share the slice 81/82 WHERE-walk machinery
(`parse_select(pattern)` + `build_from_and_where`), evaluate the
pattern exactly once, and project the UNION of template-referenced
variables from BOTH halves as BIGINT dict ids — DELETE-side vars
first (`collect_ground_template_vars`), INSERT-side vars second
(`collect_template_vars`), de-duplicated by first appearance.
Stable ordering means adding an INSERT-only variable doesn't
reshuffle the DELETE-side columns. Rust iterates the binding rows
via SPI and per row instantiates: (a) the DELETE template through
`instantiate_ground_template_quad` (lookup-only — missing terms
skip the row), then (b) the INSERT template through
`instantiate_template_quad` (interning path — fresh dict rows
allocate on demand).

**W3C §3.1.3 ordering.** The DELETE half is applied before the
INSERT half. This matters for status-flip patterns
(`DELETE { ?x ex:status "draft" } INSERT { ?x ex:status
"approved" } WHERE { ?x ex:status "draft" }`) where the
templates overlap on subject/predicate: DELETE-first removes the
old row, then INSERT adds the new one cleanly. The opposite order
would either duplicate-then-delete (and lose the new row) or
trip the `WHERE NOT EXISTS` guard. Atomicity is naturally
provided by Postgres's transaction model — the whole UDF call is
one transaction, so DELETE and INSERT either both land or neither
does. No two-phase commit dance, no snapshot capture, no save
point.

**Counter semantics.** Inherits the per-half siblings:
`triples_deleted` counts ACTUAL rows removed (RETURNING-driven,
slice 81/83 idiom), so a re-issue against the now-flipped state
reports 0; `triples_inserted` counts template-instance attempts
(slice 82 convention — the `WHERE NOT EXISTS` guard silently
dedupes but the attempt count surfaces for audit-trail callers).

**Summary discriminator.** The `_update` summary's `form` field
reports `"DELETE_INSERT_WHERE"` — `update_op_name`'s DeleteInsert
match already routed combined templates to this label per slice
82, so no shape change required.

**Limitations locked for slice 80** (inherit slices 81/82):

- WHERE pattern may NOT carry aggregates / GROUP BY / UNION —
  panics with a stable `DELETE/INSERT WHERE template feature
  '<X>' not yet supported` prefix.
- Template variables MUST be bound by the WHERE BGP (on EITHER
  half) — an unbound template variable panics with the same
  stable prefix.
- Variable GRAPH in either template (`DELETE { GRAPH ?g { … } }`
  or `INSERT { GRAPH ?g { … } }`) panics with the slice-76
  prefix. A literal graph IRI is admissible.
- `USING / USING NAMED` not yet supported (gated in the
  dispatcher arm with a stable
  `DELETE/INSERT WHERE template feature 'USING / USING NAMED'`
  prefix).

**Test coverage.**

- `tests/regression/sql/97-update-delete-insert-where.sql` locks
  five invariants — status-flip counters (2 deletes + 2 inserts),
  idempotent termination (re-issue against the flipped state
  matches 0 rows ⇒ 0/0 counters), multi-template (1 DELETE quad
  + 2 INSERT quads × 2 solutions = 2 deletes + 4 inserts),
  zero-match no-op (unrelated WHERE ⇒ 0/0), post-state
  round-trip (SELECT confirms table state matches the counter
  trail). Hand-authored expected output; never ACCEPT=1
  baselined.
- Three `#[pg_test]`s in `src/query/executor.rs`
  (`sparql_update_delete_insert_where_happy_path`,
  `sparql_update_delete_insert_where_idempotent_termination`,
  `sparql_update_delete_insert_where_multi_template`).
- The `update-delete-insert-where-lands-82-77` `_check_error`
  assertions in regressions 93 / 94 / 95 were replaced with
  smoke assertions that the dispatcher now returns a well-formed
  `form = "DELETE_INSERT_WHERE"` row. The
  `sparql_update_delete_insert_combined_still_panics` pgrx test
  was removed (it now succeeds instead of panicking).

**Test bar after slice 80.** 153 pgrx integration + 59 pg_regress
+ 26 W3C-shape + 3 LUBM-shape = 241 automated tests across all
four layers (up from 238 at slice 81: +2 pgrx — 3 new slice-80
cases minus 1 dropped slice-77 panic assertion — and
+1 pg_regress).

### Phase C slice 81 — SPARQL UPDATE DELETE WHERE (pattern-driven)

Sibling of slice 82's INSERT WHERE. The DeleteInsert dispatcher
arm `(true, false)` now routes through `execute_delete_where`
rather than panicking with the slice-78 "lands" prefix — the
slice number was renumbered from 78 to 81 to keep countdown
spacing consistent with the Track 2 sequence (84 → 83 → 82 →
81 …). Toward v0.4.3.

**Strategy.** Same recipe as slice 82: the WHERE pattern goes
through the v0.3 `parse_select` walker — sharing the
BGP/FILTER/OPTIONAL/MINUS algebra with SELECT — and a custom
projection emits each template-referenced variable's **dict
id** (BIGINT, not lexical text) one row per solution. Rust
iterates the binding rows via SPI's prepared-statement path
and materialises each `GroundQuadPattern` in the template per
row. The DELETE template's type (`Vec<GroundQuadPattern>`
rather than `Vec<QuadPattern>` for INSERT) bakes the W3C
SPARQL 1.1 §4.1.2 rule "blank nodes are not allowed in the
DELETE clause" directly into the spargebra AST — the
helper-pair `collect_ground_template_vars` /
`instantiate_ground_template_quad` mirrors slice 82's
INSERT-side helpers but matches `GroundTermPattern` (which
has no blank-node arm).

**Lookup-only dict path.** Per W3C §4.1.2 a DELETE is "remove
if exists" — never "error if missing". Each instantiated
template quad routes through the existing `lookup_iri_id` /
`lookup_literal_id` helpers (no interning, mirroring slice
83's DELETE DATA posture). If any of (subject, predicate,
object, graph) is absent from `_pgrdf_dictionary`, the per-row
delete is a spec-correct no-op rather than an error.

**Per-row DELETE counter semantics.** The per-row template-
quad DELETE uses the same `WITH d AS (DELETE … RETURNING 1)
SELECT count(*)::bigint FROM d` idiom slice 83 installed for
DELETE DATA, so `triples_deleted` counts ACTUAL rows removed
(not template instantiations attempted). This is an important
distinction from INSERT WHERE's per-attempt counter — the
WHERE NOT EXISTS guard in `insert_quad` silently dedupes, and
slice 82 trades the "rows actually added" counter for a
per-template-instance audit trail. For DELETE the
spec-correct counter is "rows that left the table", and
that's what slice 81 returns. Concretely: issuing the same
broad DELETE WHERE twice removes N rows on the first call
and 0 on the second.

**Summary discriminator.** The `_update` summary's `form`
field reports `"DELETE_WHERE"` (distinct from slice 83's
`"DELETE_DATA"`). `update_op_name` was already split by
slice 82 — pure-INSERT → `INSERT_WHERE`, pure-DELETE →
`DELETE_WHERE`, combined modify form → `DELETE_INSERT_WHERE`
— so this slice required no shape change there.

**Limitations locked for slice 81** (mirroring slice 82's
limitation set):

- WHERE pattern may NOT carry aggregates / GROUP BY / UNION
  — those produce variable scopes outside the §4.1
  DELETE WHERE intent. Panics with a stable
  `DELETE WHERE template feature '<X>' not yet supported`
  prefix.
- Template variables MUST be bound by the WHERE BGP — an
  unbound template variable panics with the same stable
  prefix. Same fail-fast posture as slice 82, awaiting the
  same spec-conformant silent-skip enhancement when
  CONSTRUCT ships (Track 4).
- Variable GRAPH in template (`DELETE { GRAPH ?g { … } }`)
  panics with the slice-76 prefix. A literal graph IRI
  (`DELETE { GRAPH <iri> { … } }`) is admissible.
- `USING / USING NAMED` not yet supported.

**Test coverage.**

- `tests/regression/sql/96-update-delete-where.sql` locks
  five invariants — filtered-DELETE counter (FILTER narrows
  to one row of four seeded), broad-DELETE counter (three
  remaining persons fall to a single un-filtered DELETE
  WHERE), zero-match no-op (`?x foaf:name ?n` against a
  table with no foaf assertions reports `deleted = 0`),
  post-state round-trip (SELECT confirms table state
  matches the counter trail), set-semantics on re-issue
  (second broad-DELETE returns 0). Hand-authored expected
  output; never ACCEPT=1 baselined.
- Three `#[pg_test]`s in `src/query/executor.rs`
  (`sparql_update_delete_where_happy_path`,
  `sparql_update_delete_where_broad_and_idempotent`,
  `sparql_update_delete_where_zero_match_noop`).
- The slice-82 regression `95-update-insert-where.sql`'s
  "pure DELETE WHERE lands in slice 78" `_check_error`
  assertion was replaced with a smoke-level assertion that
  the dispatcher no longer routes DELETE WHERE through a
  panic (a never-bound-predicate WHERE returns a
  well-formed `form = "DELETE_WHERE"`, `triples_deleted = 0`
  row).

**En passant fix.** Slice 82's two negative-path pgrx tests
(`sparql_update_insert_where_unbound_template_var_panics`,
`sparql_update_delete_insert_combined_still_panics`) were
silently failing on main against the post-1d99406 panic
message text — pgrx-tests does an EXACT string match on the
`error =` attribute (not a substring match), and the
attributes had been trimmed to the prefix while the panic
message carries an explanatory parenthetical suffix. Slice
81 aligns the `error =` strings to the full panic message
so both tests now pass alongside the new slice-81 cases.

**Test bar after slice 81.** 151 pgrx integration + 58
pg_regress + 26 W3C-shape + 3 LUBM-shape = 238 automated
tests across all four layers (up from 231 at slice 82: +5
pgrx — 3 new slice-81 cases + 2 pre-existing slice-82
negative-path tests fixed en passant — and +1 pg_regress).

### Phase C slice 82 — SPARQL UPDATE INSERT WHERE (pattern-driven)

Builds on slice 84's UPDATE foundation to land
`INSERT { template } WHERE { pattern }` end-to-end (LLD v0.4 §4.1
row "INSERT { template } WHERE { pattern }"). Toward v0.4.3.

**Strategy.** The WHERE pattern goes through the v0.3
`parse_select` walker — sharing the BGP/FILTER/OPTIONAL/MINUS
algebra with SELECT — and a custom projection emits each
template-referenced variable's **dict id** (BIGINT, not lexical
text) one row per solution. Rust then iterates the binding rows
via SPI's prepared-statement path and materialises each
`QuadPattern` in the template per row, routing through the
shared `insert_quad` helper with the same `WHERE NOT EXISTS`
set-semantic guard slice 84 installed for INSERT DATA. Returning
dict ids (rather than lexical strings) keeps internment
lossless — the binding's `term_type` / `datatype_iri_id` /
`language_tag` stay attached to the existing dictionary row, so
typed literals and language-tagged literals round-trip through
INSERT WHERE without re-internment overhead.

**Surface.**

- `INSERT { ?x ex:tag "person" } WHERE { ?x rdf:type ex:Person }`
  — each `rdf:type ex:Person` solution row instantiates one
  template quad; 2 rows ⇒ 2 inserted triples.
- Multi-triple template — N solutions × M template quads = N×M
  inserted triples; both the summary counter and the table count
  agree.
- Zero-match no-op — a WHERE that returns no solutions reports
  `triples_inserted = 0` and the quads table is unchanged.
- Set-semantics on re-issue — same INSERT WHERE issued twice
  still reports the attempted-insert counter per template
  instance, but the table stays at the same row count via the
  existing `WHERE NOT EXISTS` guard.

**Summary discriminator.** The `_update` summary's `form` field
reports `"INSERT_WHERE"` (distinct from slice 84's
`"INSERT_DATA"`) so callers can route on which UPDATE variant
ran. `update_op_name` widens the `DeleteInsert` arm to split by
template-half presence: pure-INSERT → `INSERT_WHERE`, pure-DELETE
→ `DELETE_WHERE` (the wiring shipped subsequently in slice 81),
combined modify form → `DELETE_INSERT_WHERE` (slice 77).

**Limitations locked for slice 82.**

- WHERE pattern may NOT carry aggregates / GROUP BY / UNION —
  those would produce variable scopes outside the §4.1 INSERT
  WHERE intent. Panics with a stable
  `INSERT WHERE template feature '<X>' not yet supported`
  prefix.
- Template variables MUST be bound by the WHERE BGP — an unbound
  template variable (`?z` in `INSERT { ?x ex:tag ?z } WHERE
  { ?x ?p ?o }`) panics with the same stable prefix. This is
  fail-fast rather than the W3C §4.2 "Template Group" spec's
  silent-skip; the spec-conformant skip lands later as an
  enhancement when CONSTRUCT does (Track 4).
- Variable GRAPH in template (`INSERT { GRAPH ?g { … } }`)
  panics with the slice-76 prefix (graph-scoped INSERT WHERE
  lands in slice 76). A LITERAL graph IRI
  (`INSERT { GRAPH <iri> { … } }`) is admissible and routes
  through the existing `resolve_or_allocate_graph` helper.
- Blank-node terms in the template panic — fresh blank-label
  semantics per W3C §4.1.3 needs its own slice.

**Per-form dispatch panic refinement.** The slice-84 dispatcher
collapsed every `DeleteInsert` variant onto one panic with
prefix `UPDATE form 'DELETE/INSERT WHERE' lands in slices 82-77`.
Slice 82 splits this:

- Pure INSERT WHERE: implemented (this slice).
- Pure DELETE WHERE: panics with
  `UPDATE form 'DELETE WHERE' (without INSERT) lands in slice 78`.
- Combined DELETE+INSERT WHERE (modify): panics with
  `UPDATE form 'DELETE/INSERT WHERE' lands in slice 77 (combined
  modify form)` — the contiguous substring
  `UPDATE form 'DELETE/INSERT WHERE' lands` is preserved so
  slice 84's regression `tests/regression/sql/93-update-insert-
  data.sql` substring-match expectation
  (`update-delete-insert-where-lands-82-77`) still holds.

**Test coverage.**

- `tests/regression/sql/95-update-insert-where.sql` locks five
  happy-path invariants (form discriminator, multi-row template
  instantiation, zero-match no-op, multi-triple template, set-
  semantics on re-issue) plus four negative-path "lands in slice
  NN" / "not yet supported" prefix locks (unbound template var,
  variable GRAPH in template, combined modify form deferred to
  77, pure DELETE WHERE deferred to 78). Hand-authored expected
  output; never ACCEPT=1 baselined.
- Five `#[pg_test]`s in `src/query/executor.rs`
  (`sparql_update_insert_where_happy_path`,
  `sparql_update_insert_where_zero_match_noop`,
  `sparql_update_insert_where_multi_triple_template`,
  `sparql_update_insert_where_unbound_template_var_panics`,
  `sparql_update_delete_insert_combined_still_panics`).

Test bar after slice 82: 146 pgrx integration + 56 pg_regress +
26 W3C-shape + 3 LUBM-shape = 231 automated tests across all
four layers (up from 225 at slice 84: +5 pgrx + 1 pg_regress).

LLD v0.4 §4.1 row table updated — `INSERT { template } WHERE
{ pattern }` marked `✅ slice 82`; `INSERT { GRAPH <iri> { … } }`
WHERE-driven variant marked `✅ slice 82` (literal IRI form),
variable form deferred to slice 76. `docs/03-query.md` Surface
today gains an INSERT WHERE row; `docs/10-roadmap.md` Track 2
picks up the slice 82 ✅ entry.

### Phase C slice 83 — SPARQL UPDATE DELETE DATA

Symmetric companion to slice 84's INSERT DATA: `DELETE DATA { … }`
removes ground quads (no variables, no WHERE clause) from
`_pgrdf_quads`. spargebra emits
`GraphUpdateOperation::DeleteData { data: Vec<GroundQuad> }`; each
`GroundQuad` carries a `NamedNode` subject + `NamedNode` predicate
+ `GroundTerm` object (no blank nodes — enforced by spargebra at
parse time) + `GraphName` scope.

The executor dispatches each ground quad through a **lookup-only**
dictionary path (`lookup_iri_id` for subject/predicate, new
`lookup_ground_term_id` helper for object) — never interning.
Rationale: DELETE DATA is set-semantic per LLD v0.4 §4. If any
term of the quad is missing from `_pgrdf_dictionary`, the quad
cannot possibly be in `_pgrdf_quads`, so the operation is a
spec-correct no-op rather than an "allocate and then fail to
delete" round-trip. Same for an unbound named-graph IRI: the
partition can't exist, the operation produces zero rows.

- **Default graph** — `DELETE DATA { <s> <p> <o> }` removes the
  quad from `_pgrdf_quads_g0` (when present) and reports
  `triples_deleted = 1`, `graphs_touched: ["DEFAULT"]`.
- **Named graph** — `DELETE DATA { GRAPH <iri> { … } }` scopes the
  removal to that partition only; a same-shape quad in the
  default graph is NOT touched. `graphs_touched` carries the IRI.
- **No-op on missing terms** — DELETE DATA referencing IRIs never
  interned (or a quad whose individual terms exist but never
  appeared together) returns `triples_deleted = 0` without
  erroring.
- **Idempotency on repeat** — deleting the same quad twice
  reports `triples_deleted = 1` the first time and
  `triples_deleted = 0` the second time.
- **Typed-literal payload** — `lookup_ground_term_id` composes
  with the existing `lookup_literal_id` so DELETE DATA can target
  a literal-bearing triple (datatype IRI lookup + value lookup
  match the insert side).

**Multi-op form discriminator.** The summary's `form` field now
collapses to `"MIXED"` when an Update carries operations of more
than one variant kind (e.g. a future
`DELETE DATA { … } ; INSERT DATA { … }` composition). For
single-variant Updates the slice 84 behaviour is preserved
(`"INSERT_DATA"`, `"DELETE_DATA"`, etc.). Forward-looking
compatibility — no caller-visible shape change for slice 84
queries.

**Dispatcher cleanup.** The slice-84 panic for `DELETE DATA`
(`UPDATE form 'DELETE DATA' lands in slice 83`) is removed; the
matching regression assertion in
`tests/regression/sql/93-update-insert-data.sql` is dropped from
the negative-path table and its line in the expected output is
removed. The remaining unimplemented variants
(`DELETE/INSERT WHERE`, `CLEAR/CREATE/DROP GRAPH`, `LOAD`) still
panic with their stable "lands in slice NN" prefixes; the
`sparql_update_form_dispatch_panics_for_unimplemented` pgrx test
retargets to `DELETE/INSERT WHERE`.

**Test coverage.**
- `tests/regression/sql/94-update-delete-data.sql` locks six
  invariants (default-graph removal, missing-term no-op,
  named-graph scope, SELECT round-trip, idempotency on repeat,
  typed-literal payload) plus one negative-path "lands in slice
  NN" prefix sample. Hand-authored expected output; never
  ACCEPT=1 baselined.
- Three `#[pg_test]`s in `src/query/executor.rs`
  (`sparql_update_delete_data_removes_existing`,
  `sparql_update_delete_data_missing_term_is_noop`,
  `sparql_update_delete_data_named_graph`).

Test bar after slice 83: +3 pgrx integration + 1 pg_regress.

LLD v0.4 §4.1 row table updated — `DELETE DATA` marked
`✅ slice 83`. `docs/03-query.md` "Surface today" gains a
DELETE-DATA row beside the existing INSERT-DATA row;
`docs/10-roadmap.md` Track 2 picks up the slice 83 ✅ entry.

### Phase C slice 84 — SPARQL UPDATE foundation + INSERT DATA

Opens Phase C (LLD v0.4 §4 — SPARQL UPDATE) toward v0.4.3.
`pgrdf.sparql(q)` now detects UPDATE queries at the entry point via
a **try-parse-then-fallback** strategy: `parse_query` first (the
v0.3 SELECT/ASK path, unchanged), then
`SparqlParser::new().parse_update(q)` on query-side failure. If
both fail, the stable `sparql: parse error:` prefix from slice #63
is preserved (the query-side error message is surfaced because
that's the locked downstream-tooling contract). The dispatch routes
to `execute_update(&spargebra::Update)`, which walks
`update.operations` and either materialises the operation (INSERT
DATA today) or panics with a stable `sparql: UPDATE form '<name>'
lands in slice <NN>` prefix for the variants that follow-up slices
will land.

`INSERT DATA { … }` lands end-to-end:

- **Default graph** — `INSERT DATA { <s> <p> <o> }` lands the triple
  in `_pgrdf_quads_g0` and reports `graphs_touched: ["DEFAULT"]` in
  the summary row.
- **Named graph** — `INSERT DATA { GRAPH <iri> { … } }` auto-
  allocates a fresh `graph_id` via `pgrdf.add_graph(iri TEXT)`
  (slice 118), creates the partition, and lands the triple there.
  `graphs_touched` carries the IRI, not the synthetic seed
  `urn:pgrdf:graph:<N>`.
- **Multi-triple** — a single statement with N triples reports
  `triples_inserted = N` and all N rows are observable in the
  table.
- **Typed-literal payload** — datatype IRIs are interned first
  (matching the existing `loader.rs::object_to_id` convention) so
  the literal row can reference them by id; round-trip via SELECT
  returns the original lexical form.
- **Idempotent on repeat** — `_pgrdf_quads` has no `UNIQUE`
  constraint (the hexastore indexes are covering, not unique, by
  design — the bulk Turtle loader appends without dedup checks for
  perf). To honour LLD v0.4 §4's "INSERT DATA is set-semantics"
  contract, the INSERT routes through a `WHERE NOT EXISTS` guard
  against the SPO covering index. Cost: one index probe per
  inserted triple. The `_update` summary still reports
  `triples_inserted = 1` on the second call (attempted inserts, not
  net row delta — the explicit semantic is locked by regression).

**Return shape.** UPDATE forms now return a single summary row of
shape `{"_update": {form, triples_inserted, triples_deleted,
graphs_touched, elapsed_ms}}`, paralleling the v0.3 `_ask` JSONB
sentinel for ASK queries. Callers discriminate on the leading
JSONB key. The `form` field for the slice-84-shipped variant is
`"INSERT_DATA"`; per-form follow-up slices (`"DELETE_DATA"`,
`"DELETE_INSERT_WHERE"`, `"CLEAR"`, `"CREATE"`, `"DROP"`) will
populate the discriminator as they ship.

**Per-form dispatch panics.** Forms the executor doesn't translate
yet panic with stable prefixes so callers can preview the rollout
schedule: `DELETE DATA` lands in slice 83, `DELETE/INSERT WHERE`
in slices 82-77, `CLEAR GRAPH` in slice 71, `CREATE GRAPH` in 70,
`DROP GRAPH` in 69. `LOAD <url>` is out of scope for v0.4 (LLD
v0.4 §14). spargebra 0.4.6 does not expose separate `ADD` /
`MOVE` / `COPY` variants — those SPARQL surface keywords desugar
to combinations of `Clear` + `DeleteInsert` at parse time.

**`pgrdf.sparql_parse` integration.** The introspection UDF mirrors
the detection strategy and reports `form: "UPDATE"` for any UPDATE
query, with a per-op summary array (`InsertData` carries `triples`
+ `graphs` counts; the other ops surface only their variant name
for slice 84). Unimplemented ops are NOT flagged in
`unsupported_algebra` — that array stays reserved for genuinely-
out-of-scope shapes (`LOAD <url>`, etc.). The locked syntax-error
prefix `sparql_parse:` is preserved.

**Test coverage.**
- `tests/regression/sql/93-update-insert-data.sql` locks six
  invariants (default-graph, named-graph IRI auto-allocate, multi-
  triple, idempotent-on-repeat, typed-literal round-trip,
  sparql_parse integration) plus six negative-path "lands in slice
  NN" prefix locks via the `_check_error` plpgsql helper shared
  with `81-error-paths.sql` / `88-drop-graph.sql`. Hand-authored
  expected output; never ACCEPT=1 baselined.
- Five `#[pg_test]`s in `src/query/executor.rs`
  (`sparql_update_insert_data_default_graph`,
  `sparql_update_insert_data_named_graph`,
  `sparql_update_returns_update_summary_shape`,
  `sparql_update_insert_data_idempotent_on_repeat`,
  `sparql_update_form_dispatch_panics_for_unimplemented`).
- Three `#[pg_test]`s in `src/query/parser.rs`
  (`sparql_parse_update_insert_data`,
  `sparql_parse_update_insert_data_named_graph`,
  `sparql_parse_update_delete_data_visible`).

Test bar after slice 84: 141 pgrx integration + 55 pg_regress + 26
W3C-shape + 3 LUBM-shape = 225 automated tests across all four
layers (up from 216 at v0.4.2: +8 pgrx + 1 pg_regress).

LLD v0.4 §4.1 row table updated — `INSERT DATA` marked
`✅ slice 84`; per-form follow-up slices flagged with the
appropriate landing-slice number. §4.2 dispatcher described.
`docs/03-query.md` "Surface today" gains an UPDATE foundation row;
`docs/10-roadmap.md` Track 2 picks up the slice 84 ✅ entry and
re-titles to "Phase C countdown 84 → 67 toward v0.4.3".

## [0.4.2] — 2026-05-15

Phase B closes with five countdown slices (99 → 95) shipping LLD v0.4
§5 (graph-level lifecycle UDFs) end-to-end, plus a release preflight
countdown (95 → 85) that cuts v0.4.2. The marquee surface lands four
partition-level primitives — `pgrdf.drop_graph`, `clear_graph`,
`copy_graph`, `move_graph` — and an end-to-end integration regression
wiring the four together against a load → mutate → verify flow. Test
bar at cut: 216 automated (133 pgrx + 54 pg_regress + 26 W3C-shape + 3
LUBM) plus the `pg_dump` round-trip gate.

### Phase B slice 99 — pgrdf.drop_graph lifecycle UDF

Opens Phase B (lifecycle UDFs §5) toward v0.4.2.
`pgrdf.drop_graph(id BIGINT, cascade BOOLEAN DEFAULT TRUE) →
BIGINT` removes the LIST partition `_pgrdf_quads_g<id>` from the
parent `_pgrdf_quads` via `ALTER TABLE ... DETACH PARTITION` +
`DROP TABLE`, deletes the matching `_pgrdf_graphs` row, and
returns the pre-drop triple count. `cascade => FALSE` errors with
the stable `drop_graph: inferred rows present` prefix if any
`is_inferred = TRUE` row exists; `cascade => TRUE` (the default)
drops both base and inferred content. Default partition
(`graph_id = 0`) rejected with `drop_graph: cannot drop default
partition`; negative ids rejected with
`drop_graph: graph_id must be >= 0`. Idempotent: dropping an
absent graph returns 0 (no error) and also prunes any stranded
`_pgrdf_graphs` row so the IRI mapping converges with reality on
a crash-recovery code path. Post-drop, `pgrdf.graph_iri(id)` and
`pgrdf.graph_id(iri)` both return NULL — closes the
`_pgrdf_graphs` invalidation clause from LLD v0.4 §5.2.

Implementation lands in `src/storage/graphs.rs` (the same module
slice 120 introduced for graph-related UDFs). The partition-DDL
metadata window takes an `ACCESS EXCLUSIVE` lock on
`_pgrdf_quads` per Postgres's partition-management semantics —
the user-facing tradeoff documented for the "long-running
maintenance" workflow.

Regression: `tests/regression/sql/88-drop-graph.sql` locks six
invariants (idempotent absent, happy path with triple count,
cascade-FALSE-inferred guard, cascade-TRUE-inferred override,
default-partition guard, negative-id guard) via the `_check_error`
plpgsql helper shared with `81-error-paths.sql`. Expected output
hand-authored; never ACCEPT=1 baselined. Pgrx integration tests
cover the absent + happy + cascade-FALSE + default-partition +
negative-id paths under the `pg_test` harness, bypassing
`add_graph` via manual partition + `_pgrdf_graphs` INSERT to
avoid the documented pgrx-parallelism flake on partition DDL.

LLD v0.4 §5.1 row marked `✅ slice 99`; §2 status row updated to
reflect the `drop_graph` ✅ partial-completion of the
lifecycle-UDFs track. `docs/02-storage.md` gains §2.4 covering
the new `drop_graph` surface; `docs/10-roadmap.md` Track 3 picks
up the slice 99 ✅ entry alongside slice 98.

### Phase B slice 98 — `pgrdf.clear_graph` lifecycle UDF

First landing of the LLD v0.4 §5 graph-level lifecycle UDF
surface. `pgrdf.clear_graph(id BIGINT) → BIGINT` issues
`TRUNCATE ONLY pgrdf._pgrdf_quads_g<id>` against the per-graph
LIST partition and returns the rows-removed count (== the row
count captured immediately before the TRUNCATE). Both base and
inferred rows are wiped; the function is not
`is_inferred`-discriminating per LLD §5.2.

Contract details:

- **Partition shell + IRI binding survive.** Unlike sibling
  slice 99's `drop_graph(id)` (which DETACHes the partition,
  DROPs it, and removes the `_pgrdf_graphs` row), `clear_graph`
  leaves both intact. Subsequent inserts with the same
  `graph_id` route into the same partition without falling
  back to `_pgrdf_quads_default`, and `pgrdf.graph_iri(id)`
  keeps resolving to the bound IRI.
- **Idempotent on absent / empty graphs.** Calling against a
  `graph_id` with no LIST partition returns 0 without erroring;
  re-calling against an already-empty partition returns 0
  again. Callers can `clear_graph` blindly during cleanup
  workflows without first probing partition existence.
- **`graph_id = 0` is permitted.** Unlike `drop_graph(0)` —
  which would destroy the catch-all bucket every unrouted
  `INSERT` depends on, hence its outright rejection in slice 99
  — `clear_graph(0)` just empties the explicit `_pgrdf_quads_g0`
  partition (if `add_graph(0)` was ever called) or returns 0
  (idempotent miss path).
- **Negative id panics** with the stable
  `clear_graph: graph_id must be >= 0, got <N>` prefix —
  matches the error-shape contract `add_graph(id BIGINT)`
  (slice 119) already established.

`TRUNCATE ONLY` (not bare `TRUNCATE`) is deliberate defence-in-
depth: `ONLY` blocks cascade to any descendant partitions. The
per-graph partitions have no children today, but `ONLY` future-
proofs against a sub-partitioning slice silently widening the
scope.

Regression coverage: `tests/regression/sql/89-clear-graph.sql`
locks all six contract invariants end-to-end. Three
`#[pg_test]`s in `src/storage/graphs.rs` exercise the happy path,
idempotent-absent, and clear-twice paths.

### Phase B slice 97 — pgrdf.copy_graph lifecycle UDF

Continues the LLD v0.4 §5 graph-level lifecycle UDF surface.
`pgrdf.copy_graph(src BIGINT, dst BIGINT) → BIGINT` copies every
row from `pgrdf._pgrdf_quads_g<src>` into `pgrdf._pgrdf_quads_g<dst>`
via a single `INSERT INTO … SELECT` against the per-graph LIST
partitions, returning the count copied (== source row count at
INSERT time). `copy_graph` is the only lifecycle UDF that touches
every row — the siblings (`drop_graph`, `move_graph`,
`clear_graph`'s `TRUNCATE`) are all partition-DDL-bounded — so its
cost scales linearly with the source row count.

Contract details:

- **`is_inferred` carries forward.** Both `is_inferred = FALSE`
  and `is_inferred = TRUE` rows are copied verbatim; the
  function is not `is_inferred`-discriminating per LLD §5.2's
  "`copy_graph` copies both — `is_inferred = TRUE` rows carry
  forward as `is_inferred = TRUE` in the destination" clause.
  Materialised entailments in the source survive into the
  destination as inferred, so callers don't have to re-run
  `pgrdf.materialize_owl_rl(dst)` to recover them.
- **Destination auto-create.** If `_pgrdf_quads_g<dst>` does not
  exist, the function calls `pgrdf.add_graph(dst::bigint)` to
  create it. That call also binds a synthetic
  `urn:pgrdf:graph:{dst}` IRI in `_pgrdf_graphs` per slice 119,
  so `pgrdf.graph_iri(dst)` resolves post-copy even if the
  caller hadn't pre-registered the destination. A pre-existing
  IRI binding on `dst` is preserved unchanged.
- **Source absence is idempotent.** Copying from a `graph_id`
  whose partition does not exist returns 0 without erroring; the
  destination partition is NOT auto-created on this short-circuit
  path. Matches the LLD §5.2 idempotency invariant.
- **Re-call duplicates.** Calling `copy_graph(src, dst)` twice
  against the same pair appends another copy of `src`'s rows
  into `dst` — the function does NOT clear `dst` first. Callers
  needing strict re-call idempotency invoke
  `pgrdf.clear_graph(dst)` between calls. This is the W3C SPARQL
  1.1 Update §3.2.6 `ADD` vs `COPY` distinction pushed into the
  caller's responsibility.
- **`src == dst` rejected** with the stable `copy_graph: src and
  dst must differ` prefix — the self-copy degenerate case has no
  defined semantics on a partitioned table (`INSERT … SELECT`
  from a table into itself interleaves scan + insert
  unpredictably) and is surfaced rather than silently
  double-written.
- **Negative ids rejected** with the stable `copy_graph:
  graph_id must be >= 0, got src=<S>, dst=<D>` prefix — matches
  the error-shape contract `add_graph(id BIGINT)` (slice 119)
  and the other lifecycle UDFs already established.

Implementation lands in `src/storage/graphs.rs` alongside the
slice 99 / slice 98 siblings. The single-statement `INSERT INTO
… SELECT` runs in the calling statement's transaction; standard
MVCC snapshot semantics govern the visibility of concurrent
INSERTs on `src` (no partition-DDL lock is involved on this row-
touching path).

Regression: `tests/regression/sql/90-copy-graph.sql` locks seven
invariants (absent-src no-op + no dst auto-create, load + copy
returns count + dst auto-created + `graph_iri` resolves,
`is_inferred` preserved, src untouched, re-call duplicates +
clear-then-copy round-trip, `src == dst` rejected, negative ids
rejected). Expected output hand-authored; never ACCEPT=1
baselined. Three `#[pg_test]`s in `src/storage/graphs.rs` cover
the happy path, absent-src short-circuit, and `src == dst`
rejection paths under the `pg_test` harness, bypassing
`add_graph(src)` via manual partition + direct `_pgrdf_quads`
INSERT to avoid the documented pgrx-parallelism flake on
partition DDL (and deliberately leaving the dst-auto-create path
exercised by the function itself — that's the interesting code
under test on the destination side).

LLD v0.4 §5.1 row marked `✅ slice 97`; §2 status row updated to
reflect the `copy_graph` partial-completion of the
lifecycle-UDFs track. `docs/02-storage.md` §2.2 gains a
`#### copy_graph` subsection alongside the slice-98 `clear_graph`
entry; `docs/10-roadmap.md` Track 3 picks up the slice 97 ✅
entry alongside slices 98 + 99.

### Phase B slice 96 — pgrdf.move_graph lifecycle UDF

Continues the LLD v0.4 §5 graph-level lifecycle UDF track.
`pgrdf.move_graph(src BIGINT, dst BIGINT) → BIGINT` migrates every
quad in graph `src` to graph `dst`, removes the `src` partition,
and returns the count of triples moved (== the `src` row count
at copy time).

**Implementation strategy — compose over siblings.** The v0.4.2
implementation is `pgrdf.copy_graph(src, dst)` (slice 97, parallel
batch) followed by `pgrdf.drop_graph(src, cascade => TRUE)`
(slice 99). Both halves run in the calling statement's
transaction, so a rollback unwinds both. Semantically equivalent
to the LLD §5.2 "DETACH partition + rebind `FOR VALUES IN(<dst>)`
+ ATTACH" path, but tractable without the partition-constraint
dance that a true metadata-only swap would require (every row's
`graph_id` column would need updating to satisfy the post-rebind
LIST constraint check — itself a row scan). The §5.2
"metadata-only" claim is therefore aspirational; downgraded to a
v0.5 perf optimisation in this slice's spec sweep. Both the
spec table at §5.1 and the §5.3 acceptance criteria reflect this
correction.

Contract details:

- **Idempotent on absent src.** When `src`'s partition does not
  exist, `move_graph` returns 0 without erroring — short-circuit
  on the `pg_class` existence check, no compose invocation. Same
  shape contract as `drop_graph` / `clear_graph`.
- **`src == dst` rejected** with stable
  `move_graph: src and dst must differ (both = <N>)` prefix.
  A self-move would copy-then-drop the destination — destructive.
- **`dst` non-empty rejected** with stable
  `move_graph: dst graph_id <N> already has data (<M> rows);
  clear or drop it first` prefix. An empty pre-existing dst
  partition is fine (the copy step inserts into it); the dst
  guard runs the pg_class existence check + row count *before*
  invoking the compose.
- **Negative id rejected** with stable
  `move_graph: graph_id must be >= 0` prefix — matches the
  sibling lifecycle UDFs.
- **`_pgrdf_graphs` invalidation** inherits the compose: drop
  step removes the `src` row; copy step allocates the `dst` row
  with the synthetic IRI `urn:pgrdf:graph:{dst}` if `dst` was
  unbound. A pre-existing IRI binding on `dst` is preserved
  (slice 97 must not clobber it).

**Runtime dependency on slice 97.** `pgrdf.copy_graph` is
referenced by SQL string (not Rust symbol). The build succeeds
standalone — pgrx generates the `#[pg_extern]` SQL declaration
from the Rust signature, and the inner `SELECT pgrdf.copy_graph
(...)` resolves at runtime. Calls to `move_graph`'s happy path
therefore FAIL at runtime until slice 97 (Phase B `copy_graph`)
lands in the parent merge. The standalone shape tests
(self-move, negative-id, dst-has-data, absent-src) are
independent and run green in this slice's worktree.

Regression coverage:
[`tests/regression/sql/91-move-graph.sql`](tests/regression/sql/91-move-graph.sql)
locks five invariants (happy path with row count, idempotent
absent, src==dst rejection, dst-has-data rejection, negative-id
rejection) via the `_check_error` plpgsql helper. Expected output
hand-authored; never ACCEPT=1 baselined. Five `#[pg_test]`s in
`src/storage/graphs.rs` exercise the same paths under the pgrx
test harness; the happy-path test is documented as
slice-97-dependent.

LLD v0.4 §5.1 `move_graph` row marked ✅ slice 96; §2 status row
updated; §5.2 partition-DDL note rewritten to acknowledge the
compose strategy; §5.3 "constant-time move" acceptance criterion
deferred to v0.5. `docs/02-storage.md` gains the `move_graph`
surface section in §2.4; `docs/10-roadmap.md` Track 3 picks up
the slice 96 ✅ entry alongside slices 98 + 99.

### Phase B slice 95 — lifecycle UDF end-to-end integration

Wires the four §5 lifecycle UDFs together against a realistic
load → mutate → verify flow.
[`tests/regression/sql/92-lifecycle-end-to-end.sql`](tests/regression/sql/92-lifecycle-end-to-end.sql)
locks five interaction-level invariants the per-UDF files cannot:

- **Load → copy → drop round-trip.** `parse_turtle` into g1,
  `copy_graph(g1, g2)`, `drop_graph(g1)` — the dst graph still
  answers the original BGP through `pgrdf.sparql`. Catches a
  regression where the loader's side-effects (dict_cache,
  hexastore, `_pgrdf_graphs`) get corrupted by a lifecycle UDF.
- **`move_graph` is a faithful compose of copy + drop.** After
  `move_graph(g1, g2)`, g1 answers like a freshly-dropped graph
  (zero rows, `_pgrdf_graphs` row gone) and g2 answers like a
  freshly-copied graph (rows present, synthetic IRI bound).
- **`clear_graph` isolation under a shared dict.** Loading the
  same vocabulary into g1 and g2 (so the dict cache is shared),
  then clearing g1, must NOT touch g2 — at the row level OR at the
  `_pgrdf_graphs` row level. Pins the per-partition routing
  isolation even when the loader has fed both through the same
  dict / hexastore.
- **SPARQL `GRAPH <iri>` projection survives the lifecycle.** A
  `GRAPH <urn:pgrdf:graph:N>` query against the synthetic IRI of
  a moved-into destination returns the loaded triples — the
  IRI rebinding + partition routing both hold through the move
  step.
- **Drop-then-rebind loop.** Drop a graph, re-add via the
  IRI-keyed surface bound to a fresh IRI; `pgrdf.graph_id(new_iri)`
  resolves. Catches a regression where stale `_pgrdf_graphs` state
  would block re-allocation after a drop.

All expected values hand-computed against the loader semantics
(`parse_turtle` returns the triple count) and the §5 UDF
contracts; never ACCEPT=1 baselined. Regression bar: 53 → 54.

### Phase B slices 89-88 — docs sync for §5 lifecycle UDFs

Documentation catches up with the v0.4.2 lifecycle surface ahead
of cut:

- `docs/02-storage.md` §2.4 status updated to **Phase B shipped —
  v0.4.2**; the in-flight stub headings for slices 96/97 are
  removed (all four UDFs ship via 99/98/97/96/95). A new
  end-to-end lifecycle composition subsection points at
  `tests/regression/sql/92-lifecycle-end-to-end.sql` (slice 95).
- `guide/02-loading-rdf.md` — the legacy "DROP TABLE
  pgrdf._pgrdf_quads_g<N>" recipe is replaced with
  `SELECT pgrdf.drop_graph(<N>)`. A new "Graph lifecycle
  (v0.4.2)" section enumerates the four UDFs with their stable
  error-prefix contracts for downstream tooling.
- `README` — test pill bumps from 118 pgrx + 49 regression to
  133 pgrx + 54 regression (runtime count; the static
  `#[pg_test]` attribute count is 127 — pgrx-tests 0.16 generates
  6 additional harness wrappers); aggregate test bar 196 → 216.
  The Status pill calls out Phase B (v0.4.2) shipped and drops
  "lifecycle UDFs" from the deferred set.


### Release ops — `publish-crate.yml` disabled until E-011 retires

`.github/workflows/publish-crate.yml` renamed to
`.github/workflows/publish-crate.yml.disabled` so subsequent
release-publication events stop attempting `cargo publish`. The
v0.4.1 run already empirically confirmed `[patch.crates-io]` does
not travel with `cargo publish` (the dep resolves against stock
`reasonable 0.4.x`, which lacks the `rdf-12` passthrough feature).
The workflow file is preserved verbatim — re-enable by reverting
the rename when [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
merges and the `[patch.crates-io]` block drops. Until then, pgRDF
distribution remains via prebuilt tarballs (`release.yml`) only;
the existing crates.io `pgrdf 0.3.0` name-claim entry persists
unchanged. Tracked at [`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md)
E-011 step 6.

### Phase A countdown closed at slice 100 — v0.4.1 shipped

Phase A (§3 named-graph) closed end-to-end. v0.4.1 cut on tag
`v0.4.1` (commit `e917be7`) ships 8 prebuilt tarballs (PG 14-17 ×
{amd64, arm64}) + aggregate `SHA256SUMS` via release.yml run
[25911623612](https://github.com/styk-tv/pgRDF/actions/runs/25911623612),
green end-to-end. GitHub Release:
[v0.4.1](https://github.com/styk-tv/pgRDF/releases/tag/v0.4.1).
Tarball smoke verified: aggregate SHA256 OK, internal SHA256SUMS OK,
layout includes `lib/pgrdf.so`, `share/extension/{pgrdf.control,
pgrdf--0.4.1.sql}`, `LICENSE`, `NOTICE`.

crates.io first-publish for v0.4.1 deferred — `publish-crate.yml`
workflow_dispatch run
[25912526300](https://github.com/styk-tv/pgRDF/actions/runs/25912526300)
failed at `cargo publish` with `failed to select a version for
reasonable` (the `[patch.crates-io]` block does not travel with
`cargo publish`, so the `rdf-12` feature is unavailable on the
crates.io-published `reasonable 0.4.x`). Publish unblocks once
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
merges and the patch retires; see ERRATA.v0.4 E-011. The crate is
registered on crates.io with v0.3.0 (pre-work seed); v0.4.1 binaries
remain available via the GitHub Release tarballs.

The GitHub Release `release: published` event did not auto-trigger
`publish-crate.yml` — `softprops/action-gh-release@v2` writing
releases under `GITHUB_TOKEN` does not recursively fire workflow
events by default. Manual workflow_dispatch with `dry_run=false` is
the current entry point.

Phase B (lifecycle UDFs §5) opens next at slice 99.

## [0.4.1] — 2026-05-15

Phase A closes with thirteen countdown slices (120 → 108) shipping
LLD v0.4 §3 (named-graph SPARQL scoping) end-to-end, then the
release pre-flight countdown (107 → 100) cuts v0.4.1. First pgRDF
release on crates.io. The combined surface lands the
`pgrdf._pgrdf_graphs(graph_id, iri)` mapping table, three
`pgrdf.add_graph` overloads, two symmetric lookup UDFs
(`graph_id` / `graph_iri`), SPARQL `GRAPH <iri> { … }` literal-form
and `GRAPH ?g { … }` variable-form translation, GRAPH composition
with OPTIONAL / UNION / MINUS via a per-triple `GraphScope` plan,
and `pg_dump` round-trip discipline (LLD v0.4 §3.1 acceptance
criterion). Test bar at cut: 195 automated (117 pgrx + 49
pg_regress + 26 W3C-shape + 3 LUBM) plus the `pg_dump` round-trip
gate.

### Phase A slice 120 — `_pgrdf_graphs` schema lands (LLD v0.4 §3.1)

New `pgrdf._pgrdf_graphs(graph_id BIGINT PRIMARY KEY, iri TEXT NOT
NULL UNIQUE)` table establishes the IRI ↔ graph_id mapping that
SPARQL `GRAPH { … }` (slices 111-110), the IRI-keyed UDF overloads
(slices 118-115), and §4/§5/§6/§7 graph-scoped surfaces all depend
on. The seed row `(0, 'urn:pgrdf:graph:0')` covers the existing
default-partition catch-all bucket.

Schema-only this slice — no UDF surface change, no behaviour change
to existing `pgrdf.add_graph(id BIGINT)`. Regression coverage:
`tests/regression/sql/72-graphs-table-shape.sql` + one `#[pg_test]`
in `src/storage/graphs.rs`. Test bar: 95 pgrx + 41 pg_regress + 23
W3C + 3 LUBM = 162 green.

### Phase A slice 119 — `add_graph(id BIGINT)` populates `_pgrdf_graphs`

The existing integer-keyed `pgrdf.add_graph(id BIGINT)` UDF now
inserts `(id, 'urn:pgrdf:graph:' || id::text)` into `_pgrdf_graphs`
on each successful partition creation. Idempotent via
`ON CONFLICT (graph_id) DO NOTHING`. No signature change; downstream
callers gain a queryable IRI mapping for every graph they create
through the integer surface.

Regression: `tests/regression/sql/73-add-graph-populates-iri.sql` +
pgrx test `src/storage/graphs.rs::add_graph_populates_synthetic_iri`.
Test bar: 96 pgrx + 42 pg_regress + 23 W3C + 3 LUBM = 164 green.

### Phase A slice 118 — `pgrdf.add_graph(iri TEXT)` overload (LLD v0.4 §3.2)

New `#[pg_extern]` overload `pgrdf.add_graph(iri TEXT) → BIGINT`.
Idempotent on the IRI: if the IRI is already bound in `_pgrdf_graphs`,
returns the existing `graph_id` without creating a new partition.
Otherwise auto-allocates the next `graph_id` (smallest unused
positive integer) and creates both the partition and the IRI
binding atomically.

Uses `LOCK TABLE _pgrdf_graphs IN SHARE ROW EXCLUSIVE MODE` to
serialise concurrent allocate-and-insert sequences. The IRI is
pre-INSERTed before re-entering through the integer overload, so
slice 119's synthetic-IRI insert no-ops on `ON CONFLICT (graph_id)
DO NOTHING` and the user-supplied IRI persists verbatim. Pgrx
surfaces both Rust functions under the SQL name `add_graph` via
`#[pg_extern(name = "add_graph")]`; Postgres dispatches on the
argument types (BIGINT vs TEXT). Empty / whitespace-only IRIs
panic with the stable `add_graph: iri must be non-empty` prefix.
RFC-3987 syntax validation deferred to a later slice (no oxiri
dependency in v0.4.1).

Regression: `tests/regression/sql/74-add-graph-iri.sql` + pgrx
tests `add_graph_iri_idempotent` and `add_graph_iri_empty_rejected`.
Test bar: 98 pgrx + 43 pg_regress + 23 W3C + 3 LUBM = 167 green.

### Phase A slice 117 — `pgrdf.add_graph(id BIGINT, iri TEXT)` explicit binding

Third `pgrdf.add_graph` overload landing. Caller specifies both id
and iri; idempotent on matching pairs; errors on conflicting
bindings (id bound to different iri, or iri bound to different id);
upgrades a synthetic `urn:pgrdf:graph:{id}` binding to a
user-specified iri when the synthetic was auto-allocated by the
integer-keyed overload.

Error message contracts locked:
  add_graph: graph_id <N> is bound to a different IRI (<existing>)
  add_graph: iri <iri> is bound to a different graph_id (<existing>)

Regression: `tests/regression/sql/75-add-graph-id-iri.sql` +
pgrx tests covering fresh pair, synthetic upgrade, id conflict,
iri conflict, negative id, empty iri. Test bar: 102 pgrx + 44
pg_regress + 23 W3C + 3 LUBM = 172 green.

### Phase A slice 116 — `pgrdf.graph_id(iri)` lookup

Read-only `pgrdf.graph_id(iri TEXT) → BIGINT` returns the integer
`graph_id` bound to the given IRI in `_pgrdf_graphs`, or NULL if
unbound. Marked STRICT so NULL input → NULL output without an SPI
round trip.

Regression: `tests/regression/sql/76-graph-id-lookup.sql` covers
seed, post-IRI-add, post-(id,iri)-add, post-integer-add (synthetic
binding), miss, empty-input, and NULL-input cases. Plus four pgrx
tests in `src/storage/graphs.rs` covering seed lookup, post-add
lookup, miss, and NULL input. Test bar: 106 pgrx + 45 pg_regress +
23 W3C + 3 LUBM = 177 green.

### Phase A slice 115 — `pgrdf.graph_iri(id)` symmetric lookup

Read-only `pgrdf.graph_iri(id BIGINT) → TEXT` returns the IRI
bound to `graph_id`, or NULL if the id is unbound. STRICT for NULL
input. Symmetric to slice 116's `pgrdf.graph_id(iri)`.

With slice 115 done, the §3.2 UDF surface is fully landed
(add_graph × 3 overloads + graph_id + graph_iri). SPARQL
`GRAPH { … }` translation lands next in slices 114-110.

Regression: `tests/regression/sql/77-graph-iri-lookup.sql` covers
seed, post-add IRI lookup, integer-add synthetic, miss, NULL
input, and the round-trip via slice 116's `graph_id()`.

### Phase A slice 114 — SPARQL `GRAPH <iri> { … }` translation (LLD v0.4 §3.3)

The SPARQL executor now handles literal-IRI `GRAPH { … }` blocks.
At translate time, the IRI resolves to a `graph_id` via
`_pgrdf_graphs.iri`; unresolved IRIs bind to `-1` (zero-rows
sentinel, spec-correct "no solutions"). Every triple alias inside
the GRAPH block carries an additional `q.graph_id = <resolved>`
WHERE constraint.

`pgrdf.sparql_parse` no longer flags `GRAPH { … }` with a literal
IRI under `unsupported_algebra`. Variable form `GRAPH ?g { … }`
stays 🚧 until slice 113.

Regression: `tests/regression/sql/78-sparql-graph-literal-iri.sql`
verifies the per-graph scoping (g1 vs g2), pre-existing
no-graph-scope path preservation, unresolved-IRI zero-rows
semantics, and the `unsupported_algebra` flip. Plus one pgrx test
exercising the same surface.

### Phase A slice 113 — SPARQL `GRAPH ?g { … }` variable form translation

The SPARQL executor now handles variable-form `GRAPH ?g { … }`
blocks. At translate time, the graph variable name is recorded on
`ParsedSelect.graph_var` (or `UnionBranch.graph_var`) and threaded
through `build_from_and_where`, which appends an
`INNER JOIN pgrdf._pgrdf_graphs g0 ON g0.graph_id = q1.graph_id`
(exactly one such JOIN per inner BGP) and adds
`qN.graph_id = q1.graph_id` for every additional mandatory /
OPTIONAL / MINUS alias inside the GRAPH block — so a multi-triple
inner BGP cannot stitch triples from different graphs together.

The projection layer emits `g0.iri` whenever the projected variable
matches `graph_var`, so the JSONB row carries the IRI string rather
than the integer graph_id. INNER JOIN matches W3C SPARQL 1.1 §13.3:
only graphs present in the IRI mapping bind ?g.

`pgrdf.sparql_parse` no longer flags `GRAPH ?g { … }` under
`unsupported_algebra` — the parser walks `inner` like the literal-IRI
form. Composition with OPTIONAL / UNION / MINUS that spans DIFFERENT
GRAPH scopes is slice 112.

Regression: `tests/regression/sql/79-sparql-graph-variable.sql`
verifies per-row IRI projection, COUNT + GROUP BY ?g, the
multi-triple shared-graph constraint (no cross-graph stitches), and
the `unsupported_algebra` flip. Plus one pgrx test
(`sparql_graph_variable_projects_iri`) exercising the same surface.
`tests/regression/sql/80-unsupported-shapes.sql` retires the gap-4
entry (variable-form GRAPH is no longer a gap).

Slices 111 + 113 ship as the first **parallel batch** in the
countdown — two worktree-isolated agents authored independent
slices that converge on main via cherry-pick. See ERRATA.v0.4 for
the multi-agent pattern. Tests 25 + 26 (slice 111) verify slice
113's translation end-to-end.

### Phase A slice 112 — SPARQL GRAPH composition with OPTIONAL/UNION/MINUS

The SPARQL executor's GRAPH constraint moves from PER-QUERY to
PER-TRIPLE. The previous implementation (slices 114 + 113) carried a
single `graph_id_constraint: Option<i64>` plus a single
`graph_var: Option<String>` on `ParsedSelect` and `UnionBranch`,
applying one graph scope to the whole single-branch BGP (and its
OPTIONAL / MINUS bundle). That worked for simple BGPs but couldn't
express GRAPH inside an OPTIONAL with a different scope, GRAPH
inside individual UNION branches, or GRAPH inside MINUS.

Slice 112 introduces:

- A new `GraphScope` enum with two arms:
  - `Literal(i64)` — `graph_id` resolved at translate time via
    `_pgrdf_graphs.iri`.
  - `Variable { name, scope_id }` — the SPARQL `?g` variable name
    plus a globally-unique scope id (minted on entry to each GRAPH
    block; counter lives on `ParsedSelect.graph_scope_counter`).
- A `ScopedTriple { triple, scope: Option<GraphScope> }` wrapper
  carried by `ParsedSelect.bgp` and `UnionBranch.bgp` instead of
  bare `TriplePattern`. Each triple records the GRAPH scope that
  was active during its walk.
- A `MinusBlock { triples, scope }` struct (replaces
  `Vec<Vec<TriplePattern>>`) and a `scope` field on `OptionalBlock`
  — both pick up their scope from the walk's `current_scope`, OR
  override it when a GRAPH block wraps the OPTIONAL's / MINUS's
  right arm directly.
- A new `walk_select_scoped` / `walk_branch` recursion that threads
  `current_scope: Option<&GraphScope>` down through every algebra
  node; a `GraphPattern::Graph` mints a fresh scope and walks
  `inner` with it bound.

The SQL builder grows a `ScopePlan` that scans the mandatory BGP +
OPTIONALs to figure out which Variable scopes need JOINs to
`_pgrdf_graphs`. Mandatory scopes get an INNER JOIN anchored on
their first BGP alias; OPTIONAL-born scopes get a LEFT JOIN
anchored on the OPTIONAL's alias, so an unmatched OPTIONAL leaves
`?g` NULL without dropping the outer row (W3C SPARQL 1.1 §13.3
LEFT-JOIN semantics for OPTIONAL preserved). When two GRAPH blocks
bind the same `?g` variable, the second scope gets a
`g{later}.graph_id = g{anchor}.graph_id` consistency predicate.
MINUS scopes stay internal to the `NOT EXISTS` subquery — the
subquery emits its own `_pgrdf_graphs g{S}` row and anchors all
inner aliases on it.

Coverage:

- `tests/regression/sql/87-sparql-graph-composition.sql` —
  pg_regress-shape file with the five composition shapes (GRAPH
  inside OPTIONAL, GRAPH inside UNION branches, GRAPH inside MINUS,
  OPTIONAL inside GRAPH ?g, MINUS inside GRAPH literal).
- Four pgrx `#[pg_test]`s in `src/query/executor.rs`:
  `sparql_graph_composition_with_optional`,
  `sparql_graph_composition_with_union`,
  `sparql_graph_composition_with_minus`,
  `sparql_optional_inside_graph_variable`. Each uses direct INSERT
  into `_pgrdf_graphs` + manual partition creation to bypass the
  `add_graph` parallelism flake (same scaffolding as slices 114 +
  113).

Backwards compatibility is preserved: every slice 114 / 113
regression case (`78-sparql-graph-literal-iri.sql`,
`79-sparql-graph-variable.sql`, the W3C-shape fixtures from slice
111) generates the same SQL shape as before — a single GRAPH around
a BGP collapses into "every triple in the BGP carries the same
scope", producing the same constraint set the previous
`graph_id_constraint` / `graph_var` path emitted.

### Phase A slice 111 — W3C-shape conformance for SPARQL GRAPH

Three new W3C-shape fixtures under `tests/w3c-sparql/` covering the
§13.3 `GRAPH { … }` surface:

- `24-graph-named-iri/` — literal-IRI form `GRAPH <iri> { ?s ex:name ?name }`.
  Two named graphs populated via `setup.sql`; query scopes to `g1`
  only; expected single-row result `{"name": "Alice in g1"}`.
- `25-graph-var-projection/` — variable form `GRAPH ?g { ?s ex:name ?name }`.
  Same two-graph fixture; query projects `?g` as the IRI plus
  `?name`; expected two rows.
- `26-graph-var-groupby/` — variable form composed with `COUNT(*)` +
  `GROUP BY ?g` + `ORDER BY ?g` (LLD v0.4 §3.4 acceptance criterion
  2). Two graphs with 3 + 2 triples; expected
  `{"g": "…/g1", "n": "3"}` / `{"g": "…/g2", "n": "2"}`.

`tests/w3c-sparql/run.sh` gains optional per-test `setup.sql`
support. The runner now:

1. Accepts a test directory if it has EITHER `data.ttl` OR
   `setup.sql` (or both); `query.rq` is still always required.
2. Runs `setup.sql` (when present) after `CREATE EXTENSION pgrdf`
   and before any `data.ttl` parse.
3. Skips the default `add_graph(${gid}) + parse_turtle(data.ttl,
   ${gid})` step entirely when `data.ttl` is missing or empty
   (`-s` check).

The extension is backward-compatible: tests 01–23 retain a
non-empty `data.ttl` and no `setup.sql`, and their SQL stream is
byte-identical pre/post the extension. The leading-scaffolding-row
drop in the caller (`grep -E '^\{|^\['`) continues to strip
function return values from any setup.sql calls.

W3C-shape harness count: 23 → 26. All three fixtures pass against
the merged state of parallel batch 1 (slices 111 + 113).

### Phase A slice 110 — pg_dump round-trip for `_pgrdf_graphs`

LLD v0.4 §3.1 carries an explicit acceptance criterion: "`pg_dump`
of a pgRDF database carrying the mapping round-trips the mapping
verbatim". Slice 110 wires the end-to-end regression that proves it.

New shell-orchestrated test
`tests/regression/scripts/pg-dump-roundtrip.sh` (cannot live as a
plain `.sql` fixture because `pg_dump` is an external binary, not a
`psql` builtin) drives a three-step sequence against the compose
Postgres:

1. Drop + recreate the extension; seed two known IRI bindings via
   `pgrdf.add_graph(101::bigint, 'http://example.org/rt-1')` and
   `pgrdf.add_graph(102::bigint, 'http://example.org/rt-2')`.
2. Run `pg_dump` to a tmpfile inside the container; grep the dump
   for both IRI strings as a fast canary on whether row data was
   serialised at all.
3. Drop the extension (wiping the rows), restore from the dump,
   then verify the two rows survived (count check on
   `pgrdf._pgrdf_graphs WHERE iri IN (…)` plus a symmetric
   `pgrdf.graph_iri(101::bigint)` lookup).

Two new Justfile recipes: `just test-pg-dump-roundtrip` is the
direct entry point; `just test-conformance` now lists it as the
fourth prerequisite alongside `test-regression`, `test-w3c`, and
`test-lubm` so the broader compose-based sweep (and `smoke-cold`)
catches it on every cold boot.

`sql/schema_v0_4_0_graphs.sql` gains a
`SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graphs', '');`
registration so the table's row data is included by `pg_dump` rather
than treated as extension-managed DDL. Without this call, the seed
+ user-bound IRI rows would be silently dropped on restore.

### Phase A slice 109 — docs sync for §3 named-graph surface

Coherence pass across the engineering doc set after Phase A
countdown slices 120 → 110 cumulatively shipped LLD v0.4 §3 (the
named-graph track) on `main`. No code, no tests, no schema changes
— this slice synchronises the engineering documentation surface to
the now-shipped reality:

- `specs/SPEC.pgRDF.LLD.v0.4.md`: §0 status note now records the §3
  named-graph track as COMPLETE within the v0.4 cycle (with the
  slice-level breakdown); §2 capability matrix flips the
  "IRI ↔ graph_id mapping table + UDFs" row from 🚧 to ✅; §3
  intro paragraph re-tagged from 🚧 to ✅ shipped, citing the full
  Phase A countdown.
- `docs/02-storage.md`: end-to-end coherence pass on the
  `_pgrdf_graphs` subsection — removed slice-by-slice repetition
  (scalar-subquery wrapper rationale, `#[pg_extern(strict)]`
  discipline, lock idiom), folded slices 116/115 into a single
  symmetric-lookup paragraph, called out the
  `pg_extension_config_dump('_pgrdf_graphs', '')` registration as
  part of the schema migration with the
  `tests/regression/scripts/pg-dump-roundtrip.sh` end-to-end lock,
  and added a worked example showing all five UDFs composing
  through the synthetic-IRI seed, IRI-keyed allocation, the
  explicit-binding upgrade path, NULL-on-miss lookups, and the
  pg_dump round-trip discipline.
- `docs/03-query.md`: "Surface today" section header flipped from
  "v0.4 §3.3 GRAPH landing" to "v0.4 §3.3 GRAPH shipped"; new
  "Named-graph GRAPH-scope translation" subsection captures the
  per-pattern `Option<GraphScope>` algorithm from slice 112 (the
  matrix row alone abbreviates the algorithm) — `Literal` vs
  `Variable` scope arms, mandatory INNER vs OPTIONAL-born LEFT
  JOIN to `_pgrdf_graphs`, two-GRAPH-blocks-same-`?g` consistency
  predicate, MINUS scope inheritance, IRI vs integer projection
  semantics, and bare-BGP "match in any graph" fallback.
- `README.md`: status row extended with "named-graph SPARQL
  scoping (`GRAPH <iri> { … }` literal + `GRAPH ?g { … }` variable
  + composition with OPTIONAL/UNION/MINUS, LLD v0.4 §3 shipped via
  Phase A countdown slices 120 → 110)"; "named-graph" removed from
  the deferred-list; SPARQL feature pill extended with GRAPH; tests
  pill bumped to 118 pgrx + 49 regression + 26 W3C + 3 LUBM = 196
  (verified by counting `#[pg_test]` attributes in `src/` + entries
  in `tests/regression/sql/` + non-README dirs in
  `tests/w3c-sparql/`); new GRAPH SPARQL example block under the
  existing SPARQL examples; new "covers" bullet enumerating the
  shipped §3 surface with all the test artefacts.
- `docs/10-roadmap.md`: Track 1 heading flipped from
  "🚧 (Phase A in flight)" to "✅ (Phase A countdown slices 120 →
  110 shipped)" with the continuation note for slices 109 → 100
  toward a v0.4.1 tag; new "Phase A §3 named-graph shipped"
  test-bar row recording the **196-test total** cumulative state
  (118 + 49 + 26 + 3) with the per-slice landings, the new
  pg_regress file range (`72-79` + `87`), the W3C-shape additions
  (`24` / `25` / `26`), and the `§3 named-graph ✅` phase-status
  marker against the v0.4 LLD §5 track set.

Co-landed with slice 108 in parallel batch 3; slice 108 owns
`guide/` updates while this slice handles the LLD / docs/ /
README surface. No `src/`, `tests/`, `Cargo.toml`, `LICENSE`, or
`NOTICE` edits.

### Phase A slice 108 — user-guide updates for §3 named-graph surface

The user-facing guide under `guide/` picks up the now-shipped §3
surface (slices 120-110): three `pgrdf.add_graph` overloads, the
`graph_id` / `graph_iri` lookups, and SPARQL `GRAPH <iri> { … }` /
`GRAPH ?g { … }` scoping with full composition into OPTIONAL,
UNION, and MINUS. `guide/02-loading-rdf.md` gains a "Named graphs
by IRI" subsection covering the three overloads, the lookup
functions, and the synthetic-IRI binding behaviour of the legacy
integer-keyed call. `guide/03-querying.md` gains a "Named graphs"
section after MINUS, walking through the literal-IRI form, the
variable form with IRI projection, GRAPH-in-OPTIONAL for side-graph
enrichment, GRAPH-in-UNION for cross-graph row collection, and
GRAPH-against-MINUS for side-graph exclusion, plus an end-to-end
worked example combining loading and querying. The surface tables
in `guide/00-intro.md` and the index entries in `guide/README.md`
flip the GRAPH row from ⏳ to ✅ and call out the new entry points;
the previous "cross-graph queries land in v0.4" caveat in
`guide/03-querying.md` is rewritten to point at the now-shipped
forms. No changes outside `guide/` and this CHANGELOG entry.

### Inherits — v0.4.0 lineage

v0.4.0 tagged and released 2026-05-15
([release page](https://github.com/styk-tv/pgRDF/releases/tag/v0.4.0));
v0.4.1 builds on that base, carrying every v0.4.0 surface forward
(real SHACL Core, OWL 2 RL inference, SPARQL SELECT / ASK / OPTIONAL
/ UNION / MINUS / aggregates / BIND / FILTER richness, dictionary-
encoded LIST-partitioned hexastore storage). The `[patch.crates-io]`
block in `Cargo.toml` stays in place through v0.4.x — see ERRATA.v0.4
E-011. The patch retires once gtfierro/reasonable#50 merges.

## [0.4.0] — 2026-05-15

The first pgRDF release with the full four-engine mission shipping
in earnest. **SHACL Core validation is real** —
`pgrdf.validate(data, shapes)` returns a W3C
`sh:ValidationReport`-shape JSONB via `shacl 0.3.1`, replacing the
v0.3.0 stub. Unblocked via a `[patch.crates-io]` override pointing
at the `styk-tv/reasonable@rdf12-passthrough` fork (upstream
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
awaiting maintainer review/merge; v0.4.1 will drop the patch once
the upstream merges). All four engines (storage, SPARQL, OWL 2 RL
inference, SHACL Core validation) are now real implementations.
Test bar: 94 pgrx + 40 pg_regress + 23 W3C-shape + 3 LUBM-shape =
160 automated tests green, plus the 24-ontology / 17,134-triple
manual smoke. PG 14-17 × {amd64, arm64} = 8 prebuilt tarballs.
Apache 2.0.

The v0.4 LLD's named-graph + SPARQL UPDATE + lifecycle UDFs +
CONSTRUCT + property paths + heap_multi_insert phase B + W3C
manifest runner tracks stay 🚧 — they land in subsequent v0.4.x
points or in a refreshed v0.5.0 cut.

### Upstream — `reasonable` PR filed (E-011 step 4)

Filed the upstream patch as
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
on 2026-05-15. The PR description carries the downstream verification
data (pgRDF 160 tests green, real SHACL `pgrdf.validate` running
against `shacl 0.3.1` + the patched `reasonable` via
`[patch.crates-io]`). `specs/ERRATA.v0.4.md` E-011 status flipped
from "verified locally" to "verified locally + upstream PR open
(awaiting maintainer review/merge)". Once the upstream merges, the
`[patch.crates-io]` block in `Cargo.toml` drops and the dep pins
to whatever release ships the patch.

### Spec — v0.4 LLD promoted from FUTURE, v0.5-FUTURE opened

SHACL real impl shipped on `main` in commit `ac40bc2`; v0.4 LLD is
no longer forward-looking. `specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`
renamed to `specs/SPEC.pgRDF.LLD.v0.4.md` via `git mv` (history
preserved). The renamed file's §0 status flips from "draft /
forward-looking / target: pgRDF v0.4 cut" to
"in-progress authoritative contract for the v0.4 cycle". §9 SHACL
restructured from "v0.5 — gated on E-009" to "✅ shipped in v0.4
cycle" — cites commit `ac40bc2`, ERRATA.v0.4 E-011, and regression
`71-shacl-real.sql`. Capability matrix (§2) marks SHACL ✅; all
other v0.4 tracks (named-graph, UPDATE, lifecycle, CONSTRUCT,
paths, SPARQL backlog) stay 🚧.

New `specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md` opened as the next
forward-look sibling. Carries the v0.5-targeted content split out
of the prior v0.4-FUTURE: §3 reasoning profile selector (was v0.4
§8), §4 TriG/N-Quads ingest (was v0.4 §10), §5 SHACL-SPARQL
constraint mode + materialised-graph coverage (was v0.4 §9.5), §6
W3C SHACL manifest runner (was v0.4 §9.5 / §13), §7 IRI overloads
for lifecycle UDFs (was v0.4 §5.1 forward note), §8
aggregates-over-UNION refinements (was v0.4 §11 forward note), §9
v1.0 forward look (was v0.4 §15).

Cross-link updates: v0.3 LLD §0 supersession block now points at
`v0.4.md` (not `-FUTURE`); ERRATA.v0.4 E-011 next-steps row repoints
to LLD.v0.4 §9; `docs/04-inference.md` reasoning-selector pointer
moves to v0.5-FUTURE §3; `docs/05-validation.md` SHACL spec pointer
moves to v0.4 §9; `docs/06-installation.md` two pointers move to
v0.4; `docs/09-release.md` "Deferred to v0.4" pointer moves to v0.4
§2 with promotion note; `docs/10-roadmap.md` ~25 pointers rewritten
(most to v0.4, the v0.5-targeted ones to v0.5-FUTURE);
`RELEASE_NOTES.md` deferral pointer moves to v0.4 §2;
`src/query/{parser,executor}.rs` doc-comment pointers move to v0.4.

No code changes; no test count changes (still 160 = 94 pgrx + 40
pg_regress + 23 W3C + 3 LUBM).

### Phase 5 — Real SHACL validation lands (E-009 / E-011 resolved upstream-pending)

`pgrdf.validate(data_graph, shapes_graph) → JSONB` now executes
real SHACL Core validation via `shacl 0.3.x`. The stub body is
replaced with rehydrate-shapes / build-validator / run-validation
/ shape-W3C-report-as-JSONB. Coverage: new
`71-shacl-real.sql` exercising sh:NodeShape + sh:property +
sh:datatype violations, plus three `#[pg_test]` integration tests
(conforming, violations, unknown graphs). Existing
`70-validate-stub.sql` repurposed to lock the real-impl basic
shape (vacuously-conforming + unknown-graph degenerate cases);
filename retained for diff-friendly history.

Unblocked via `[patch.crates-io]` to the styk-tv/reasonable fork
branch `rdf12-passthrough`, which adds the `TermRef::Triple(_)`
arm needed for coexistence with shacl 0.3.x. Drop the patch once
gtfierro/reasonable merges the upstream PR (held in fork
PR-DRAFT.md).

Surface: `Cargo.toml` gains `shacl = "0.3"`, `rudof_rdf = "0.3"`,
flips `reasonable` to `{ version = "0.4", features = ["rdf-12"] }`,
and adds a `[patch.crates-io]` block pointing `reasonable` at the
fork. `src/validation/shacl.rs` rewritten from stub to real
impl. Specs: LLD v0.4-FUTURE §9 moves SHACL from v0.5 → v0.4 (real
impl section); §2 capability matrix flips Real SHACL output to ✅;
scope list expanded from five tracks to six; ERRATA.v0.4 E-011
gains a "Verified locally in pgRDF" section with the post-slice
test bar.

Test bar after: **94 pgrx + 40 pg_regress + 23 W3C + 3 LUBM = 160
tests** green (was 158).

### Spec — ERRATA.v0.4 file created (v0.4 cycle tracking)

New [`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) carries v0.4-era
spec deltas. E-011 first entry tracks the upstream `reasonable` patch
for RDF 1.2 coexistence (unblocks the remaining
`rdf-12 / TermRef::Triple` half of E-009). Branch
[`styk-tv/reasonable@rdf12-passthrough`](https://github.com/styk-tv/reasonable/tree/rdf12-passthrough)
is pushed; PR draft is held in the fork for review before filing
upstream. v0.2-era entries (E-006, E-007, E-008, E-009, E-010) are
carried forward by cross-link rather than duplicated.

## [0.3.0] — 2026-05-14

The first official pgRDF release. Ships the v0.3 engine surface
feature-complete state: storage (dictionary-encoded terms,
LIST-partitioned quads, hexastore indexes), SPARQL SELECT/ASK
surface, OWL 2 RL inference, SHACL validation stub, storage
performance (shmem dict cache + prepared-plan cache + prepared
bulk-insert), 158 automated tests + 24-ontology smoke, License
attribution + MSRV declared, and the full release pipeline
exercised end-to-end. PG 14-17 across {amd64, arm64} = 8
prebuilt tarballs.

### Release pre-flight — final CI sweep (slices #10-#5)

Last verification gate before tagging v0.3.0. Ran every test/lint
layer locally against the post-version-bump tree (HEAD was `ac514fe`
at start of sweep) to confirm the codebase is release-ready.

Per-layer results:

- **Slice #10 — `cargo fmt --check`**: drift in
  `src/inference/reasonable.rs` (40 lines reflowed; long `.expect()`
  chains broken across multiple lines by rustfmt). Applied
  `cargo fmt --all` and committed as a style-only commit.
- **Slice #9 — `cargo clippy -D warnings`** (in builder container,
  rustc 1.91): one error surfaced — unused import
  `use pgrx::prelude::*;` at `src/storage/shmem_cache.rs:349`. The
  `use super::*;` on the line above already brings `pgrx::prelude::*`
  into scope through the parent module, so the line was genuinely
  redundant. Removed it. Clippy now exits 0.
- **Slice #8 — `just test`** (pgrx integration in Linux builder):
  `test result: ok. 93 passed; 0 failed; 0 ignored`. Matches the
  93-count in README and `docs/08-testing.md`; no doc drift.
- **Slice #7 — `just test-regression`** (pg_regress harness against
  compose Postgres): `39 pass, 0 fail, 0 new baselines`.
- **Slice #6 — `just test-w3c`** (W3C-shape SPARQL harness):
  `23 pass, 0 fail, 0 new baselines`.
- **Slice #5 — `just test-lubm`** (LUBM-shape harness):
  `3 pass, 0 fail, 0 new baselines`.

Final aggregate: **93 (pgrx) + 39 (pg_regress) + 23 (W3C) + 3 (LUBM)
= 158 tests, all green**. Matches the 158 figure cited across
README.md, `docs/08-testing.md`, `docs/10-roadmap.md`,
`docs/09-release.md`, and `RELEASE_NOTES.md` — no test-count doc
updates needed.

Code changes from this sweep:

- `src/inference/reasonable.rs` — rustfmt-only reflow (no semantic
  change), landed in its own commit.
- `src/storage/shmem_cache.rs` — removed redundant
  `use pgrx::prelude::*;` from `mod tests`.

### Release pre-flight — version bump to 0.3.0 (slices #18-#11)

Mechanical version bump landing slices #18 through #11 of the 66→1
release countdown. Touches every surface that pins the package
version. The `pgrdf.version()` UDF (which returns
`env!("CARGO_PKG_VERSION")`) and the extension's `extversion` now
both report `0.3.0`.

Files touched:

- `Cargo.toml` — `version = "0.2.0"` → `version = "0.3.0"` (slice #18).
- `pgrdf.control` — `default_version = '0.2.0'` → `'0.3.0'` (slice #17).
- `compose/compose.yml` — bind-mount path
  `pgrdf--0.2.0.sql` → `pgrdf--0.3.0.sql` (slice #16).
- `compose/README.md` — bind-mount table + `pgrdf.version()` worked
  example output → `"0.3.0"` (slice #16).
- `README.md` — `pgrdf.version()` example output → `0.3.0`
  (slices #15/#14; status pill at `v0.3` was already correct).
- `guide/01-install.md` — `pgrdf.version()` example outputs (Path A
  compose flow, Verify section) → `0.3.0`; Path C manual-install
  worked-example tarball URL → `v0.3.0/pgrdf-0.3.0-...tar.gz`
  (slice #13).
- `docs/02-storage.md` — "No PostgreSQL custom scan hooks at v0.3.0"
  current-version reference (slice #13).
- `docs/06-installation.md` — `pgrdf.version()` example output →
  `'0.3.0'` (slice #13).
- `docs/09-release.md` — preamble reframed to reflect that Cargo.toml
  now reads `version = "0.3.0"` (bump landed), instead of "still
  reads `version = "0.2.0"`; bump-to-0.3.0 happens as part of the
  cut" (slice #13).
- `tests/regression/expected/00-smoke.out` — pgrdf.version() and
  extversion lines `0.2.0` → `0.3.0` (slice #13).
- `Cargo.lock` — `pgrdf 0.2.0 → 0.3.0` via `cargo update -p pgrdf`
  (slice #11).

Build artifact verification (slice #12):

- `just build-ext` produces
  `compose/extensions/share/extension/pgrdf--0.3.0.sql` and
  `pgrdf.control` reads `default_version = '0.3.0'`. The cached
  `pgrdf--0.2.0.sql` left over in the build output was removed (v0.x
  doesn't support `ALTER EXTENSION pgrdf UPDATE` per the slice #21
  upgrade policy, so the legacy migration file isn't needed).
- `just test-regression` reports `39 pass, 0 fail, 0 new baselines`.

Historical references to `0.2.0` are intentionally preserved:

- All `CHANGELOG.md` `[Unreleased]` entries from slices #24-#27 that
  document past pre-flight verifications (manual repack test,
  cargo pgrx package dry-run, etc.) — they accurately describe the
  state at the time those slices ran.
- `sql/schema_v0_2_0.sql` — historical bootstrap-schema filename
  (still referenced by `extension_sql_file!` in `src/lib.rs`); v0.3
  doesn't change the schema layout.
- `specs/SPEC.pgRDF.LLD.v0.2.md` — the v0.2 LLD contract.

CHANGELOG `[Unreleased]` → `[0.3.0]` block conversion stays deferred
to slices #4-#1 (the cut itself).

### Release notes — v0.4 deferral list audit (slice #19)

Bi-directional audit of the "Deferred to v0.4" lists in
`RELEASE_NOTES.md` and `docs/09-release.md` against
`specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md` §2 (canonical v0.4 scope). The
spec lists five major tracks (§3 named-graph + IRI mapping; §4 SPARQL
UPDATE; §5 graph-level lifecycle UDFs; §6 CONSTRUCT; §7 property
paths) plus the carried SPARQL backlog (§11: multi-triple OPTIONAL,
VALUES, BIND-downstream, aggregates over UNION, DESCRIBE) and the
ingest-performance carry (§12: `heap_multi_insert` 2× target).

Drift findings, both directions:

- **`docs/09-release.md` was missing the entire §11 SPARQL backlog** —
  multi-triple OPTIONAL, VALUES, BIND-downstream, aggregates over
  UNION, and DESCRIBE were absent from its "Deferred to v0.4" list
  even though they are listed in `RELEASE_NOTES.md` and named
  explicitly in the spec as in-scope for v0.4. Fixed by adding a
  dedicated bullet covering all five, with the LLD §11 cross-link.
- **`RELEASE_NOTES.md` mentioned `GRAPH { … }` without the IRI
  mapping** — the IRI ↔ `graph_id` mapping table is the hard
  prerequisite for the SPARQL surface (LLD §3.1), and `docs/09-release.md`
  already names it. Fixed by adding "with IRI ↔ `graph_id` mapping"
  to the named-graph entry plus the property-path operator set, and a
  `§2` anchor on the LLD cross-link for parity with `docs/09-release.md`.
- **No v0.5/v1.0 items mislabeled as v0.4** — the LLD's §8
  (reasoning profile selector), §9 (real SHACL output, gated on
  E-009), §10 (TriG / N-Quads ingest), and §15 (incremental
  materialisation, RDF 1.2 triple terms) are correctly absent from
  both consumer-facing files. A short pointer paragraph added to
  `docs/09-release.md` so readers know where the v0.5/v1.0 forward
  look lives (LLD §8-§10 and §15) rather than guessing those items
  were forgotten.
- **Items in consumer-facing lists not in LLD §2**: `SHA256SUMS.asc`
  GPG signature and pgrx 0.18 / PG 18 migration are both legitimate
  v0.4 work items but are release-engineering and toolchain concerns
  outside the LLD scope (tracked under INSTALL OQ4 / roadmap Phase 6
  step 3, and ERRATA E-006 respectively). Annotated as such in
  `docs/09-release.md`; left in place in both files because they're
  user-visible v0.4 deliverables consumers should see in release
  notes.
- **Cross-link anchors**: both files link to the LLD doc top (no
  in-document anchor); the file resolves on disk. Added an explicit
  `§2` reference in both prose pointers so a future reader can find
  the canonical scope section without scrolling.

Outcome: drift closed in both directions. `RELEASE_NOTES.md`'s
"Deferred to v0.4" line now reads parallel to the LLD §2 capability
matrix; `docs/09-release.md` no longer drops the SPARQL backlog. No
spec edits; the LLD remains source of truth. This audit slice is
documentation-only — no code, no tests, no other docs touched.

### Release notes — known issues block consolidated (slice #20)

Cross-file audit of the Known Issues surface across `RELEASE_NOTES.md`,
`docs/09-release.md`, and `specs/ERRATA.v0.2.md` to make sure
consumer-facing release docs cite the v0.3.0-era errata consistently
with the authoritative ERRATA table. Triggered by the E-007
"workflow.ttl" mis-cite caught in slice #22 (corrected in `52c13bf`);
this pass verifies nothing else drifted across the remaining v0.3.0
errata entries (E-006, E-007, E-008, E-009, E-010) and that the
pre-v0.3 entries (E-001, E-002, E-003, E-004, E-005) are correctly
omitted from the v0.3.0 release surface as resolved-by-design or
out-of-scope-for-consumers.

Findings, per E-NNN:

- **E-001** (`shacl-rust` → `shacl_validation` supersession), **E-002**
  (OWL 2 RL only, EL/QL out-of-scope), **E-003** (PG 18 GUC path,
  effectively rolled into E-006), **E-004** (init-script-on-PG18+ no
  longer needed; compose has no init script), **E-005** (repo URL
  `styk-tv/pgRDF` placeholder fix) — **all pre-v0.3 spec corrections
  folded into the v0.3 LLD body or otherwise resolved-by-design.**
  Correctly omitted from v0.3.0 release notes in both files; no
  consumer-facing impact on the v0.3.0 tarball.
- **E-006** (pgrx 0.18 / PG 18 deferred) — cited in both files,
  consistent: `RELEASE_NOTES.md` "pgrx 0.18 / Postgres 18 deferred to
  v0.4."; `docs/09-release.md` "pgrx held at 0.16.1; PG 18 deferred
  to v0.4." Both match ERRATA's "Hold pgrx 0.16.1 for v0.3. Support
  matrix: PG 14–17."
- **E-007** (`extension_control_path` GUC forward path blocked by
  E-006) — cited in both files, consistent: both call out INSTALL §7,
  the E-006 blocker, and the per-file bind-mount workaround. The
  earlier "workflow.ttl" mis-cite from slice #22 is gone (fixed in
  `52c13bf`).
- **E-008** (Linux builder container instead of native macOS) —
  **correctly omitted from both consumer-facing files.** This is a
  contributor / build-environment fact, not a tarball-consumer fact;
  end-users of the release artifacts never encounter the dev-only
  macOS → Linux builder routing. Listed in ERRATA for the dev path.
- **E-009** (SHACL real integration blocked upstream) — cited in
  both files, consistent: `RELEASE_NOTES.md` "SHACL real integration
  blocked by upstream dep conflict."; `docs/09-release.md`
  "`pgrdf.validate` ships as a stub; real SHACL execution blocked by
  upstream `shacl_validation` / `reasonable` feature unification."
  Both match ERRATA's `iri_s` migration + `rdf-12` feature-unification
  story.
- **E-010** (cargo audit informational advisories) — cited in both
  files, consistent: `RELEASE_NOTES.md` "cargo audit advisories — all
  informational, no security impact."; `docs/09-release.md` "4
  informational `cargo audit` advisories accepted for v0.3 (all in
  subtrees of pgrx 0.16.1 / `reasonable 0.4.1` and clear automatically
  when E-006 / E-009 resolve)." Both match ERRATA's "Accept the 4
  informational warnings for v0.3."

Cross-link verification: `specs/ERRATA.v0.2.md` resolves on disk from
both consumer-facing files (`RELEASE_NOTES.md` root-relative;
`docs/09-release.md` via `../specs/ERRATA.v0.2.md`).

**Outcome: zero drift. No edits to `RELEASE_NOTES.md` or
`docs/09-release.md` required.** The two files cite exactly the same
set (E-006, E-007, E-009, E-010), describe each at appropriate
granularity for their audience (marketing-style summary vs engineering
release note), and classify all four as known-issues with no
security-blocking impact. ERRATA remains the source of truth; both
consumer-facing docs are aligned to it. This audit slice is
documentation-only — no code, no spec, no other doc touched.

### Release notes — upgrade policy documented (slice #21)

The v0.x upgrade discipline written down as consumer-facing contract.
Headline: pgRDF v0.x reserves the right to break schema and UDF
signatures between minor releases, `ALTER EXTENSION pgrdf UPDATE` is
not supported until v1.0, and the supported upgrade path is dump-via-SQL
(decode `pgrdf._pgrdf_quads` against `pgrdf._pgrdf_dictionary` per
graph, serialise to Turtle externally), `DROP EXTENSION pgrdf CASCADE`,
install the new version, then `CREATE EXTENSION` + re-load. v1.0 is
flagged as the boundary where proper `ALTER EXTENSION pgrdf UPDATE`
migrations land alongside a frozen on-disk schema; no date committed.

The detailed policy lives in `docs/06-installation.md` as a new
`## Upgrade between v0.x versions` section (procedure with SQL dump
template, "why no in-place upgrade?" rationale calling out fluid
pre-1.0 schema / non-stable dict id space / `is_inferred` flux, and
cluster-managed-installation guidance pointing CloudNativePG /
StackGres / Apache AGE operators at planned maintenance windows +
volume snapshots + staging verification). Two short cross-link
summaries land alongside: `docs/09-release.md` v0.3.0 section gets a
new `### Upgrade policy` subsection above `### Known issues`, and
`RELEASE_NOTES.md` gets a new `## Upgrading` section above
`## License`. Both summaries point back at the canonical
`docs/06-installation.md` anchor. SQL examples schema-qualify the
internal tables (`pgrdf._pgrdf_quads` / `pgrdf._pgrdf_dictionary`)
matching the actual extension schema. No fabricated dates for v0.4 /
v1.0.

### Release notes — RELEASE_NOTES.md drafted + release.yml body_path wired (slice #22)

The GitHub Release body for v0.3.0 — the marketing-style summary that
consumers see in the GH UI when they land on the release page. Lives at
the repo root as `RELEASE_NOTES.md` (Option A per the slice brief: simple,
conventional, rewritten each release). Wired into the workflow via
`body_path: RELEASE_NOTES.md` on the `softprops/action-gh-release@v2`
step alongside the existing `generate_release_notes: true` (GH appends
the auto-generated PR-title commit list under the curated body).

Content is consumer-facing, ~370 words: a one-line elevator pitch,
the feature surface (storage / Turtle / SPARQL SELECT-ASK / OWL 2 RL /
SHACL stub / performance), the consolidated test bar (158 automated +
24 manual smoke matching `docs/09-release.md`), a drop-in install
recipe with the exact `curl` / `sha256sum -c` / `cp` flow, the docker
compose pointer, the {pg14..pg17}×{amd64, arm64} support matrix,
known issues (E-006 / E-007 / E-009 / E-010), the v0.4 deferral list,
and Apache 2.0 attribution. Every relative path (`specs/SPEC.pgRDF.INSTALL.v0.2.md`,
`guide/01-install.md`, `specs/ERRATA.v0.2.md`,
`specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`, `CHANGELOG.md`) resolves on disk.
Numbers cross-checked against the engineering release note in
`docs/09-release.md` (slice #23); no new claims.

### Release notes — docs/09-release v0.3.0 section drafted (slice #23)

Replaces the "No release cut yet" preamble in `docs/09-release.md`
with a `## v0.3.0 — 2026-05-14 (planned)` section: engine surface
recap (storage / SPARQL / Phase 3 perf / Phase 4 inference / Phase 5
stub / Phase 6 CI), the consolidated test bar (93 pgrx + 39 pg_regress
+ 23 W3C-shape + 3 LUBM-shape + 24 ontology smoke / 17 134 triples),
performance characteristics (sub-µs dict cache hit, prepared-plan
reuse, phase A bulk ingest with the 2× wall-clock target carried to
v0.4 phase B), the {pg14..pg17}×{amd64, arm64} matrix with PG 18
deferred per E-006, license + attribution (Apache 2.0, Copyright 2026
Peter Styk, LICENSE + NOTICE inside every tarball per §4(d)), MSRV
1.91, tarball layout per INSTALL §3, known issues (E-006 / E-007 /
E-009 / E-010), and the v0.4 deferral list. Content sourced from
CHANGELOG `[Unreleased]` and LLD v0.3 §2–§7; no new claims, no
fabricated numbers.

The CHANGELOG `[Unreleased]` block is **not yet cut** to a
`[0.3.0] — YYYY-MM-DD` block — that move lands in a later slice
(group 4-1, the actual tag commit). This slice only drafts the
engineering-side narrative inside `docs/09-release.md`; the
GitHub Release body itself is a separate slice (#22).

### Release pre-flight — smoke-install verification (slice #24)

End-to-end install rehearsal of the slice #25 tarball
(`pgrdf-0.2.0-pg17-glibc-arm64.tar.gz`, 872,067 B) against a clean
`postgres:17.4-bookworm` container. This exercises the real consumer
install path the GH Release will ask users to perform: extract tarball,
drop artifacts into PG library paths, `CREATE EXTENSION pgrdf`, parse
Turtle, run SPARQL. Goal: confirm the v0.3.0 release artifact is
ship-ready.

**Procedure (Option A from the slice brief — fresh container, no
compose, bind-mount FROM staged tarball contents):**

```bash
STAGING=$PWD/.smoke-install-test                    # podman-visible
rm -rf "${STAGING}" && mkdir -p "${STAGING}"
tar -xzf /tmp/pgrdf-repack-test/pgrdf-0.2.0-pg17-glibc-arm64.tar.gz \
        -C "${STAGING}"
podman run -d --name pgrdf-smoke \
  -e POSTGRES_USER=pgrdf -e POSTGRES_PASSWORD=pgrdf -e POSTGRES_DB=pgrdf \
  -v "${STAGING}/pgrdf-0.2.0-pg17-glibc-arm64/lib/pgrdf.so:/usr/lib/postgresql/17/lib/pgrdf.so:ro" \
  -v "${STAGING}/pgrdf-0.2.0-pg17-glibc-arm64/share/extension/pgrdf.control:/usr/share/postgresql/17/extension/pgrdf.control:ro" \
  -v "${STAGING}/pgrdf-0.2.0-pg17-glibc-arm64/share/extension/pgrdf--0.2.0.sql:/usr/share/postgresql/17/extension/pgrdf--0.2.0.sql:ro" \
  -p 5433:5432 docker.io/library/postgres:17.4-bookworm \
  -c shared_preload_libraries=pgrdf
```

Container `pgrdf-smoke` runs on port 5433 to avoid colliding with the
regular compose container on 5432. Postgres `pg_isready` returned ok
after 2s. Bind-mounts are read-only — exercises the real distribution
shape (immutable artifact, mutated only at boot via Postgres config
args).

**Initial environment note (caught by the smoke):** the slice brief's
`podman run` snippet omitted `-c shared_preload_libraries=pgrdf`. On
first run, `CREATE EXTENSION` + `pgrdf.version()` succeeded but the
first stateful call (`parse_turtle`) returned `ERROR: PgAtomic was
not initialized` — the canonical signature of pgRDF not being loaded
via `shared_preload_libraries`. This is documented in
[`SPEC.pgRDF.INSTALL.v0.2 §6 + §7`](specs/SPEC.pgRDF.INSTALL.v0.2.md),
[`guide/01-install.md §3`](guide/01-install.md), and
[`docs/06-installation.md §1.2`](docs/06-installation.md), so the
diagnostic chain held: error → check `SHOW shared_preload_libraries`
(empty) → re-launch with `-c shared_preload_libraries=pgrdf` → SHOW
returns `pgrdf` → everything works. **The tarball + install docs are
both correct; only the smoke brief's `podman run` command was
incomplete.**

**Smoke test results (after relaunching with the preload arg):**

| Step | Command | Output | Verdict |
|---|---|---|---|
| 1 | `CREATE EXTENSION pgrdf;` | `CREATE EXTENSION` | OK |
| 2 | `SELECT pgrdf.version();` | `0.2.0` (1 row) | OK |
| 3 | `SELECT pgrdf.add_graph(1);` | `t` (1 row) | OK |
| 4 | `SELECT pgrdf.parse_turtle('@prefix ex: <http://example.org/> . ex:a ex:b ex:c .', 1);` | `1` (1 triple inserted) | OK |
| 5 | `SELECT * FROM pgrdf.sparql('SELECT ?o WHERE { ?s ?p ?o }');` | `{"o": "http://example.org/c"}` (1 row) | OK |

End-to-end round-trip: tarball → bind-mount → `CREATE EXTENSION` →
parse Turtle → SPARQL SELECT, all on a stock upstream Postgres image
with zero source build. The 872 KiB artifact is sufficient.

**Total elapsed (stage → CREATE EXTENSION → final SPARQL → teardown):
~50s** on M-series darwin, podman 4-ARM hypervisor.

**Bind-mount caveats discovered:**

- Podman on darwin runs in a VM (applehv) that does NOT auto-mount
  `/tmp`. First attempt staged the tarball under `/tmp/pgrdf-install-test`
  per the slice brief and `podman run` returned
  `statfs /tmp/.../pgrdf.so: no such file or directory`. Restaging
  under `$PWD/.smoke-install-test` (under `/Users`, auto-mounted by
  podman's machine config) resolved it. **Implication for the public
  install guide:** the existing guide already tells users to write
  artifacts to PG's actual `pkglibdir` + `sharedir` (via
  `pg_config --pkglibdir`), not to bind-mount from `/tmp`, so this
  is a smoke-test infrastructure quirk only — not a documentation gap.

- glibc version mismatch was a worry going in (tarball built on glibc
  2.36 from the slice #26 manylinux container, smoke runs against
  bookworm's glibc 2.36) — and it was a non-event, since the build +
  smoke happen to use the same glibc minor. Cross-glibc verification
  is still owed once aarch64 + amd64 release builds land on the real
  GH runner.

**Teardown:**

```bash
podman rm -f pgrdf-smoke
rm -rf "${STAGING}"           # $PWD/.smoke-install-test
```

`pgrdf-smoke` is a one-shot container, never persisted; the regular
`pgrdf-postgres` (from `just compose-up`) is unaffected — it was
stopped at start of the smoke and can be restarted by the user with
`just compose-up` at any time.

**Slice outcome: PASS.** The v0.3.0 tarball-shaped release artifact
installs cleanly into a stock Postgres image. The pre-flight group
(slices #66 → #24, 43 entries) closes here. Remaining slices #23 → #1
shift focus to feature work in the v0.3.0 / v0.4.0 scope (per the
roadmap in slice #29).

### Release pre-flight — manual tarball repack verification (slice #25)

Manually executed the `release.yml` repack step (lines 38-53) on the
slice #26 build artifacts to confirm the staging → tarball pipeline
produces the exact INSTALL §3 layout the GH Release would publish.
Goal: catch any tar/find/sha256sum corner case *before* the tagged
release runs across 4 PG majors x 2 arches.

**Procedure (aarch64 darwin, slice #26 artifacts re-used — no rebuild):**

```bash
VER=0.2.0; PG=17; ARCH=arm64
STAGING=/tmp/pgrdf-repack-test
OUT="${STAGING}/pgrdf-${VER}-pg${PG}-glibc-${ARCH}"
rm -rf "${STAGING}"
mkdir -p "${OUT}/lib" "${OUT}/share/extension"
cp compose/extensions/lib/pgrdf.so                       "${OUT}/lib/"
cp compose/extensions/share/extension/pgrdf.control       "${OUT}/share/extension/"
cp compose/extensions/share/extension/pgrdf--${VER}.sql   "${OUT}/share/extension/"
cp LICENSE NOTICE                                          "${OUT}/"
( cd "${OUT}" && find . -type f ! -name SHA256SUMS -print0 \
    | xargs -0 sha256sum > SHA256SUMS )
tar -czf "${STAGING}/$(basename ${OUT}).tar.gz" -C "${STAGING}" "$(basename ${OUT})"
```

Mirrors `release.yml` L41-52 byte-for-byte (substituting
`compose/extensions/{lib,share}` for `target/release/pgrdf-pg17/usr/{lib,share}/postgresql/17/`,
since slice #26 already copied those out — same files, different path
prefix). GNU `sha256sum` from coreutils 9.7 is installed on darwin via
brew so the `sha256sum > SHA256SUMS && sha256sum -c SHA256SUMS` flow
matches Ubuntu runner behaviour exactly.

**Tarball produced:**

| Field | Value |
|---|---|
| Name | `pgrdf-0.2.0-pg17-glibc-arm64.tar.gz` |
| Size | 872,067 B (852 KiB) |
| Contents | 6 files + 4 dirs |
| Compression ratio | ~2.6:1 vs `pgrdf.so` (2.2 MB → 870 KB) |

**`tar -tzf | sort` (every entry, in lexicographic order):**

```
pgrdf-0.2.0-pg17-glibc-arm64/
pgrdf-0.2.0-pg17-glibc-arm64/LICENSE
pgrdf-0.2.0-pg17-glibc-arm64/NOTICE
pgrdf-0.2.0-pg17-glibc-arm64/SHA256SUMS
pgrdf-0.2.0-pg17-glibc-arm64/lib/
pgrdf-0.2.0-pg17-glibc-arm64/lib/pgrdf.so
pgrdf-0.2.0-pg17-glibc-arm64/share/
pgrdf-0.2.0-pg17-glibc-arm64/share/extension/
pgrdf-0.2.0-pg17-glibc-arm64/share/extension/pgrdf--0.2.0.sql
pgrdf-0.2.0-pg17-glibc-arm64/share/extension/pgrdf.control
```

**SHA256SUMS contents (5 lines, SHA256SUMS itself absent — self-exclude OK):**

```
a6dc47dea368e1cb479f456538144939060fa72bb2a96c4eabf23477d1a5ece8  ./LICENSE
7ee0daa51a51f29729f80e96192b6df4874b02a39f131c34f486c5365b3726c8  ./NOTICE
c8c661eada2255fa85e441a50240c3eaad4e1c12197a2102a1554bf5574ab90c  ./lib/pgrdf.so
7584c499464333b53dc7bd106aafd37ffa5071cb33980bd5214e6da8c72284b4  ./share/extension/pgrdf.control
3a785b2b483bd510ecf810af029bb47cd8dab032071c548f3735e759564e7f69  ./share/extension/pgrdf--0.2.0.sql
```

`pgrdf.so` SHA matches slice #26's recorded `c8c661ea…ab90c` — same
binary, no rebuild, by construction.

**Round-trip verify (extract fresh, `sha256sum -c`):**

```
./LICENSE: OK
./NOTICE: OK
./lib/pgrdf.so: OK
./share/extension/pgrdf.control: OK
./share/extension/pgrdf--0.2.0.sql: OK
```

5 of 5 OK. SHA256SUMS does not appear in its own manifest — the
`find -type f ! -name SHA256SUMS` predicate works as intended.

**INSTALL §3 layout conformance:**

| INSTALL §3 entry | tarball entry | match |
|---|---|---|
| `lib/pgrdf.so` | `lib/pgrdf.so` | exact |
| `share/extension/pgrdf.control` | `share/extension/pgrdf.control` | exact |
| `share/extension/pgrdf--<version>.sql` | `share/extension/pgrdf--0.2.0.sql` | exact |
| `share/extension/pgrdf--<prev>--<version>.sql` (zero or more) | (none — 0.2.0 is the first cut) | n/a |
| `LICENSE` | `LICENSE` | exact |
| `SHA256SUMS` | `SHA256SUMS` | exact |
| (none) | `NOTICE` | **spec gap — see below** |

Byte-for-byte conformant with INSTALL §3 except for `NOTICE`, which
landed in the tarball via slice #28 (Apache 2.0 §4(d) compliance) but
the corresponding INSTALL §3 file list was not updated then. Adding
`NOTICE` to the spec's enumerated layout is a one-line surface edit
deferred to a separate spec-grooming slice — the tarball mechanics
themselves are correct.

**Aggregate SHA256SUMS (release.yml L67-72) — not verified this slice:**

The aggregate step runs over `pgrdf-*.tar.gz` produced by all
`(pg, arch)` matrix legs and lives in the `release` job. With only
one local tarball it'd be a single-line file; the multi-leg
aggregation is fundamentally a GH Actions concern (artifact upload +
`download-artifact merge-multiple: true`). Single-tarball spot check
mirrors the same `sha256sum pgrdf-*.tar.gz > SHA256SUMS` invocation —
no behavioural surprise expected.

**Why this matters:**

`release.yml` is a single-shot, tag-triggered workflow. A failure
inside the `Repack to INSTALL-spec layout` step would leave a
half-published release on GitHub with no artifacts attached. Dry-running
the repack locally on the same artifact tree the workflow consumes
catches `cp` glob mismatches, `find -print0` portability surprises,
and tar layout regressions before they cost a re-tag + force-push.
Combined with slice #26 (path mapping verified) and slice #27 (verify
docs + GPG defer): the v0.2.0 cut is end-to-end traced.

Status: repack mechanics verified end-to-end on aarch64 darwin. The
GH Actions runner uses identical GNU coreutils + GNU tar, so the
behaviour transfers. The lone surface gap is `NOTICE` missing from
INSTALL §3's file list — flagged for a follow-up spec edit, no
runtime impact.

### Release pre-flight — cargo pgrx package dry-run (slice #26)

Verified `cargo pgrx package` produces the artifact tree the
`release.yml` repack step (lines 38-53) consumes. Goal: catch any
path mismatch *before* `git push --tags` triggers the real workflow
across 4 PG majors x 2 arches.

**Procedure (Colima docker builder, aarch64):**

1. `rm -rf compose/extensions/{lib,share}` to clean state.
2. `DOCKER_BUILDKIT=1 docker build --target builder --no-cache
   -t pgrdf-builder-rust:pg17 -f compose/builder.Containerfile .`
   — forces a fresh `cargo pgrx package` run (busts cargo cache).
3. `just build-ext` — completes export-stage and copies artifacts
   to `compose/extensions/`.

**Live `cargo pgrx package` output (from step 2, builder stage 7/7):**

```
Installing extension
Copying control file to target/release/pgrdf-pg17/usr/share/postgresql/17/extension/pgrdf.control
Copying shared library to target/release/pgrdf-pg17/usr/lib/postgresql/17/lib/pgrdf.so
Writing SQL entities to /work/target/release/pgrdf-pg17/usr/share/postgresql/17/extension/pgrdf--0.2.0.sql
Finished installing pgrdf
```

**Path mapping (cargo pgrx package output → release.yml expectation):**

| cargo pgrx package emits | release.yml repack reads | match |
|---|---|---|
| `target/release/pgrdf-pg17/usr/lib/postgresql/17/lib/pgrdf.so` | `${PKG}/usr/lib/postgresql/${pg}/lib/*.so` (L46) | exact |
| `target/release/pgrdf-pg17/usr/share/postgresql/17/extension/pgrdf.control` | `${PKG}/usr/share/postgresql/${pg}/extension/*.control` (L47) | exact |
| `target/release/pgrdf-pg17/usr/share/postgresql/17/extension/pgrdf--0.2.0.sql` | `${PKG}/usr/share/postgresql/${pg}/extension/*.sql` (L48) | exact |

`PKG=target/release/pgrdf-pg${pg}` at release.yml L43; the inner
`usr/{lib,share}` prefix is what cargo pgrx package writes by
default and matches what release.yml then `cp`s out of.

**Artifact set produced (aarch64, glibc-bookworm, pg17):**

| File | Size | Notes |
|---|---|---|
| `lib/pgrdf.so` | 2,214,040 B | ELF 64 LSB aarch64, BuildID `80021f2c…`, not stripped |
| `share/extension/pgrdf.control` | 216 B | `default_version = '0.2.0'`, `module_pathname = '$libdir/pgrdf'` |
| `share/extension/pgrdf--0.2.0.sql` | 7,220 B | 234 lines, auto-generated by pgrx |

`pgrdf.so` SHA-256 `c8c661ea…ab90c`. Sizes will differ slightly on
amd64 vs aarch64 and across pg14/15/16/17 (different pgrx-generated
FFI shim sets), but the **path layout is invariant** — that's all
the release workflow's repack step needs.

**Build time (fresh `cargo pgrx package`, single PG major, single
arch, all cargo deps cached in BuildKit mount):**

- `--no-cache` builder rebuild: 175.30s real (~3 min)
- of which `cargo pgrx package` proper: ~80s (deps mostly
  warm-cached in `/usr/local/cargo/registry` mount, fresh
  `pgrdf` compile + SQL extraction)
- subsequent `just build-ext` (all cached, just re-runs export
  container): 1.46s

The release workflow runs each `(pg, arch)` cell on a clean GitHub
runner with no cargo cache, so 8 cells x ~10-15min cold-cache build
= ~80-120 min total wall time. Within tolerance.

**Warnings worth flagging:** none. `cargo pgrx package` exits 0,
both `pgrdf-builder-rust:pg17` (~3.35 GB) and `pgrdf-builder:pg17`
(~99 MB) images build clean, export container copies the three
artifacts into `compose/extensions/` with no errors.

**Verdict:** layout matches. Release workflow's repack step
(release.yml lines 46-48) will find every path it expects. No
follow-up needed for slice #26; downstream slice #25 (manual
tarball repack dry-run) will exercise the LICENSE + NOTICE +
SHA256SUMS aggregation end-to-end.

### Release pre-flight — SHA256SUMS verify + GPG signing defer (slice #27)

Follow-up to slice #28's `release.yml` audit. Slice #28 confirmed
SHA256SUMS coverage is already wired at **both** levels (per-tarball
internal manifest + aggregate top-level over all 8 tarballs). This
slice surfaces the orthogonal piece — the detached GPG signature
`SHA256SUMS.asc` mentioned in INSTALL §3 / LLD §5.4 step 3 — and
decides scope for it.

**SHA256SUMS state (confirmed):** the `Repack to INSTALL-spec layout`
step in the `build` job emits per-tarball internal `SHA256SUMS`
(line 51 of `release.yml`) covering `lib/pgrdf.so`,
`share/extension/*`, `LICENSE`, `NOTICE`. The downstream `release`
job's `Generate aggregate SHA256SUMS` step emits a top-level
`SHA256SUMS` covering every `pgrdf-*.tar.gz` and attaches it as a
release asset (lines 67-77). No release.yml change needed.

**GPG signing decision: defer to v0.4.** Rationale:

- No `GPG_PRIVATE_KEY` secret or release-signing key is provisioned
  for the workflow today — `grep -rn "GPG_PRIVATE_KEY\|secrets\."
  .github/` returns zero matches.
- No public-half signing key is published anywhere visible
  (keyserver, release page, repo).
- SHA256SUMS itself is the primary integrity check most extension
  consumers verify (`sha256sum -c SHA256SUMS`); the `.asc` signature
  layer is a downstream supply-chain hardening, not a v0.3-cut
  blocker.
- Wiring `.asc` properly requires (a) sourcing a real signing key
  (Peter Styk maintainer key — not in repo), (b) publishing the
  public half on a keyserver or release page, (c) adding the GitHub
  secret + a `gpg --detach-sign` step to the workflow's `release`
  job. All out of scope for a verification-and-defer slice.

**Docs edits applied:**

- `docs/09-release.md` "Aggregate checksums" section: rewrote to
  confirm SHA256SUMS is wired at both levels (per-tarball +
  aggregate) and to flag `.asc` GPG signing as v0.4 follow-up
  (previously said "not yet wired in `release.yml`" which conflated
  SHA256SUMS itself with the `.asc` signing). Added a new
  "Verification (consumer side)" subsection showing the `curl` →
  `sha256sum -c SHA256SUMS --ignore-missing` recipe plus the
  in-tarball verification path; closes with a one-liner pointing
  at what changes when `.asc` lands in v0.4
  (`gpg --verify SHA256SUMS.asc SHA256SUMS`).
- `docs/10-roadmap.md` Phase 6 step 3 bullet: split the single
  conflated bullet into a positive confirmation (SHA256SUMS wired
  per slice #28) plus an explicit v0.4 defer for `.asc` listing
  the three prerequisites (signing key, public-half publication,
  secret wiring).

**No `.github/workflows/release.yml` change.** This is a
verify-and-document slice; the workflow already does what slice
#27's original plan would have done. The actual `.asc` wiring lands
in a v0.4 slice once a signing key is sourced.

Test bar unchanged: still 93 pgrx + 39 pg_regress + 23 W3C-shape +
3 LUBM-shape = 158 across all five layers.

### Release pre-flight — release.yml audit + NOTICE inclusion fix (slice #28)

End-to-end audit of `.github/workflows/release.yml` ahead of v0.3 cut.
Verified workflow shape:

- Trigger `on: push: tags: ["v*"]`, matrix `pg14/15/16/17 × {amd64, arm64}`
  (8 tarballs), GH Release job gated on `needs: build`.
- Action pins: `actions/checkout@v4`, `dtolnay/rust-toolchain@stable`,
  `actions/upload-artifact@v4`, `actions/download-artifact@v4`,
  `softprops/action-gh-release@v2` (major-pin policy preserved).
- Auth: relies on default `GITHUB_TOKEN` with top-level
  `permissions: contents: write`. No third-party secrets referenced.
- Pre-release detection: implicit via `softprops/action-gh-release@v2`'s
  SemVer pre-release tag heuristic (e.g. `v1.0.0-rc1`); no explicit
  `prerelease:` flag — relying on action default.
- SHA256SUMS: already wired in **both** per-tarball form (inside each
  `pgrdf-<ver>-pg<PG>-glibc-<arch>.tar.gz`) and aggregate form (top-level
  `SHA256SUMS` over all 8 tarballs, attached to the GH Release).
  Supersedes the slice #36 audit note that flagged this as "not yet
  wired"; no TODO needed.

**Bug fixed (Apache 2.0 §4(d) compliance):** the `Repack to INSTALL-spec
layout` step previously copied only `LICENSE` into the staging directory.
Apache 2.0 §4(d) requires that where a `NOTICE` file exists, its
attribution notices MUST be included in distributed derivative works.
Added `cp NOTICE "${OUT}/"` directly after the existing `cp LICENSE`
line, mirroring the LICENSE pattern exactly. Also updated the layout
comment block to list `NOTICE` between `LICENSE` and `SHA256SUMS`.

Net effect: each of the 8 published tarballs now ships `LICENSE` +
`NOTICE` + `SHA256SUMS` alongside the extension binaries, satisfying
the upstream license terms inherited from `oxigraph`, `spargebra`,
`sophia`, and other Apache-2.0 dependencies whose attribution flows
through pgRDF's own `NOTICE`.

### Roadmap — v0.4 scope cohesion check (slice #29)

Bi-directional cohesion audit between `specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`
(the source of truth for v0.4 scope) and the
`## v0.4 — next milestone (forward-looking)` section in
`docs/10-roadmap.md` (added by slice #31). The LLD wins on disagreement;
this slice fixes drift in the roadmap.

Coverage table (LLD §2 + ancillary v0.4 items → roadmap section):

| LLD item                                    | Roadmap before                | Roadmap after                              | Match? |
|---|---|---|---|
| §3 named-graph + IRI mapping                | Track 1                       | Track 1                                    | ✅ already |
| §4 SPARQL UPDATE                            | Track 2                       | Track 2                                    | ✅ already |
| §5 graph-level lifecycle UDFs               | Track 3                       | Track 3                                    | ✅ already |
| §6 CONSTRUCT                                | Track 4                       | Track 4                                    | ✅ already |
| §7 property paths                           | Track 5                       | Track 5                                    | ✅ already |
| §11 SPARQL surface backlog                  | "Carried backlog"             | "Carried backlog"                          | ✅ already |
| §12 perf work (heap_multi_insert + scans)   | absent                        | NEW: "Performance work carried forward"    | ✅ added |
| §13 W3C SPARQL 1.1 manifest runner wired v0.4 | absent                      | NEW: "Conformance runner wiring (v0.4)"    | ✅ added |
| §8 reasoning profile selector → v0.5        | "Excluded from v0.4"          | "Excluded from v0.4"                       | ✅ already |
| §9 real SHACL → v0.5 (E-009)                | "Excluded from v0.4"          | "Excluded from v0.4"                       | ✅ already |
| §10 TriG / N-Quads → v0.5                   | "Excluded from v0.4"          | "Excluded from v0.4"                       | ✅ already |

Reverse direction (every roadmap v0.4 subsection → LLD anchor) — every
existing subsection (Track 1-5, Carried backlog, Excluded) maps cleanly
to a numbered LLD section. No orphans in the roadmap.

Drift entries fixed:

- **Missing-in-roadmap: LLD §12 (Performance work carried forward)** —
  the LLD explicitly says "v0.4 targets shipping this" for Phase 3 step
  3 phase B (`heap_multi_insert` / `COPY BINARY`) and "v0.4 is the
  earliest target" for Postgres custom-scan hooks. Both were absent from
  the roadmap's v0.4 milestone section even though the roadmap's
  pre-existing Phase 3 narrative already refers to phase B as v0.4 work.
  Added a "Performance work carried forward from v0.3" subsection
  pointing at v0.4-FUTURE §12.
- **Missing-in-roadmap: LLD §13 (W3C SPARQL 1.1 manifest runner wired
  in v0.4)** — the LLD §13 test-policy paragraph says the manifest
  runner "is wired in v0.4 — it gates §11's SPARQL backlog automatically
  as the deferred forms come online". The roadmap's v0.4 milestone
  section had no entry for this; the Phase 6 narrative covers the v0.3
  state but the v0.4 wiring was unsurfaced in the forward-look. Added
  a "Conformance runner wiring (v0.4)" subsection.

Framing checks (LLD wording → roadmap wording):

- LLD §2: "v0.4 ships five major tracks" → roadmap: "five major tracks
  — the full contract lives in the spec". ✅ consistent.
- LLD §11: "ship together for economy" (translator machinery shared
  with §4 + §6) → roadmap: "Shipped in the same cut because they share
  the translator machinery §4 + §6 already require". ✅ consistent.
- LLD §8/§9/§10: framed as v0.5 work, "v0.4 keeps the v0.3 surface
  unchanged" / "v0.4 does not attempt" / "v0.4 does not ship this; v0.5
  does" → roadmap: "Excluded from v0.4 (planned v0.5)". ✅ consistent.

Total fixes applied: 2 new subsections added to
`docs/10-roadmap.md` v0.4 milestone section.

Test bar unchanged: docs-only slice. The LLD was not edited — only the
navigation aid.

### Roadmap — coverage ratchet table (slice #30)

Added a `## Coverage ratchet — release-by-release targets` section to
`docs/10-roadmap.md`, placed between the new
`## v0.4 — next milestone (forward-looking)` H2 (slice #31) and the
pre-existing `## Out of scope (v0.x)` H2 so the reader's eye flows
shipped-phases → next-milestone → ratchet-trajectory →
out-of-scope.

The new section consolidates targets already declared in scattered
prose across `specs/SPEC.pgRDF.LLD.v0.3.md` §5.4 + §6.1,
`specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md` §13, and `docs/08-testing.md`
("What we don't test (yet)") into a single 7-row × 5-column table:

- Rows: pgrx integration, pg_regress golden, W3C-shape SPARQL
  harness, LUBM-shape correctness harness, W3C SPARQL 1.1
  conformance manifest, W3C SHACL conformance manifest, LUBM
  cross-engine benchmark.
- Columns: v0.3 (current) shipped baselines, v0.4 target, v0.5
  target, v1.0 target.

Every cell anchors to a documented source — none are fabricated.
Cells without a published target carry `TBD` rather than a guess
(the pgrx and pg_regress columns for v0.5 / v1.0 are TBD because
the v0.5 / v1.0 LLDs aren't drafted yet; `v0.4-FUTURE` §13 only
gives counts for v0.4).

A one-paragraph explainer below the table pins the ratchet
enforcement rule: each release's CI must hit at least that
release's column, once a target is met it becomes a floor and can
never regress, citing `docs/08-testing.md`'s "Coverage gates
ratchet but never lower" line.

Sub-edits:

- `docs/10-roadmap.md` — new H2 + table + enforcement paragraph,
  inserted between `## v0.4 — next milestone` and
  `## Out of scope (v0.x)`. Caption cross-links
  `specs/SPEC.pgRDF.LLD.v0.3.md` §6.1, `v0.4-FUTURE` §13, and
  `docs/08-testing.md`.

Test bar unchanged: no new pg_regress or pgrx fixtures, this is a
docs-only slice.

Source-citation per cell (so future contributors can verify before
ratchet'ing a column):

- pgrx v0.3 = `93` from `docs/08-testing.md` line 25 +
  `docs/10-roadmap.md` v0.3 cut row.
- pgrx v0.4 = `+ heap_multi_insert tests` from
  `docs/08-testing.md` line 25.
- pg_regress v0.3 = `39` from `docs/08-testing.md` line 26 +
  v0.3 cut row.
- pg_regress v0.4 = `~60` from `v0.4-FUTURE` §13 breakdown
  (§3 6-8 + §4 8-10 + §5 4 + §6 3-4 + §7 5-6 + §11 5-6).
- W3C-shape harness v0.3 = `23` from `docs/08-testing.md` line 27.
- W3C-shape harness v0.4+ = "superseded by TTL-manifest runner"
  from `docs/08-testing.md` line 27.
- LUBM-shape v0.3 = `3` from `docs/08-testing.md` line 28.
- LUBM-shape v0.4+ = "superseded by LUBM-1/10/100 real benchmarks"
  from `docs/08-testing.md` line 28.
- SPARQL conformance v0.3 = `not wired` from
  `docs/08-testing.md` line 30 + `LLD v0.3` §5.4.
- SPARQL conformance v0.4 = `runner wired + ≥ 30 %` from
  `LLD v0.3` §5.4 line 389 + `docs/08-testing.md` line 30, 182 +
  `v0.4-FUTURE` §13 (runner wired in v0.4).
- SPARQL conformance v0.5 = `≥ 70 %` from `LLD v0.3` §6.1
  Phase 4 column line 415.
- SPARQL conformance v1.0 = `≥ 95 %` from `LLD v0.3` §5.4
  line 389 + §6.1 Phase 6 column line 415.
- SHACL conformance v0.3 = `not wired (E-009)` from
  `LLD v0.3` §5.4 line 392-394 + `docs/08-testing.md` line 31.
- SHACL conformance v0.4 = `not wired (still E-009)` from
  `v0.4-FUTURE` §9 (E-009 still gates SHACL real integration to
  v0.5).
- SHACL conformance v0.5 = `≥ 50 %` from `LLD v0.3` §6.1
  Phase 4 column line 415 + `docs/08-testing.md` line 184.
- SHACL conformance v1.0 = `≥ 90 %` from `LLD v0.3` §5.4
  line 392 + §6.1 Phase 6 column line 415.
- LUBM benchmark v0.3 = `scaffold only` from
  `docs/08-testing.md` line 32.
- LUBM benchmark v0.4 = `LUBM-1 smoke` from `LLD v0.3` §6.1
  Phase 3 column line 416.
- LUBM benchmark v0.5 = `LUBM-10 baseline vs Apache Jena TDB /
  Apache AGE` from `LLD v0.3` §6.1 Phase 4 column line 416 +
  §5.4 line 395 + `docs/08-testing.md` line 32.
- LUBM benchmark v1.0 = `LUBM-100 vs Apache Jena TDB /
  Apache AGE` from `LLD v0.3` §6.1 Phase 6 column line 416 +
  `docs/10-roadmap.md` Phase 6 step 3 line 301.

### Roadmap — v0.4 milestone section (slice #31)

Added an explicit `## v0.4 — next milestone (forward-looking)` section
to `docs/10-roadmap.md`, placed between Phase 6 and "Out of scope" so
the reader's eye flows shipped-phases → next-milestone →
out-of-scope. The new section surfaces the five v0.4 tracks at H3
heading granularity (named-graph + IRI mapping, SPARQL UPDATE,
graph-level lifecycle UDFs, CONSTRUCT, property paths) plus the
carried SPARQL-surface backlog from v0.3 and the explicit "excluded
from v0.4 (planned v0.5)" list, with each H3 cross-linking the
specific anchor in `specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`.

The intent is navigation, not new contract material: each H3 is 2–4
lines pointing at v0.4-FUTURE for detail — the v0.4-FUTURE spec
remains the single source of truth, this section is a section-TOC
entry so readers can land on "what comes next" without spelunking
the Phase 1–6 bullets.

Sub-edits:

- `docs/10-roadmap.md` — new H2 + 7 H3s (5 tracks + carried backlog +
  excluded-from-v0.4), inserted at line ~321 just above the existing
  `## Out of scope (v0.x)` H2.
- `docs/10-roadmap.md` — "Test bar over time" preamble gains a
  one-paragraph forward note that future v0.4 rows land under
  `v0.4 cut` labels per the new section's track grouping; existing
  v0.3 rows remain frozen as the shipped baseline.

Test bar unchanged: no new pg_regress or pgrx fixtures, this is a
docs-only slice.

Anchor verification: each cross-link uses the GitHub heading-slug
rules (lowercase, spaces → `-`, drop non-alphanumeric-or-hyphen,
em-dash → empty leaving a double-hyphen at its position). Targets
are §3 (`#3-named-graph-scoping-and-iri-mapping-new`), §4
(`#4-sparql-update-new`), §5 (`#5-graph-level-lifecycle-udfs-new`),
§6 (`#6-construct-deferred-from-v03-now-in-scope`), §7
(`#7-property-paths-deferred-from-v03-now-in-scope`), §11
(`#11-sparql-surface-backlog-deferred-from-v03-now-in-scope`), §8
(`#8-reasoning-profile-selector-v05--flagged-here-for-planning`),
§9 (`#9-shacl-real-integration-v05--gated-on-errata-e-009`), §10
(`#10-trig--n-quads-ingest-v05`).

### Docs — markdown link verification (slice #32)

Final docs-group lockdown pass before release-pre-flight: a full,
mechanical sweep of every internal markdown link across the repo to
confirm zero broken targets going into v0.3 release prep.

Scope and method:

| Surface | Count | Result |
| --- | --- | --- |
| Markdown files scanned (excl. `target/`, `node_modules/`, `fixtures/ontologies/`) | 61 | inventory complete |
| Total markdown links extracted (incl. external) | 153 | parsed via `\[…\]\(…\)` with code-fence stripping |
| External links (`http`/`https`/`mailto`/etc.) | 18 | not verified (out of scope) |
| Internal relative links (the audit surface) | 135 | every target resolved on disk |
| Same-file `#anchor` links | 1 | resolves to a real H2 in `docs/10-roadmap.md` |
| Cross-file `path.md#anchor` links | 0 | none in the repo |
| Directory-style links (e.g. `guide/`, `docs/`) | 4 | every target is a real directory |
| Non-markdown internal targets (`LICENSE`, `NOTICE`, `.sql`, `.rs`, `.tsv`) | 14 distinct | every file exists on disk |

Audit table — broken links found:

| File:line | Bad target | Type of break | Fix applied |
| --- | --- | --- | --- |
| _(none)_ | _(none)_ | _(none)_ | _(none)_ |

Counts: **broken 0 / fixed 0 / left-as-flagged 0**.

Verification approach (recorded for future slices):
- Resolver walks every `[text](path)` link, splits off `#anchor` and
  `?query`, resolves `path` relative to the source file, then
  `path.exists()` against the filesystem.
- For `#anchor` suffixes on `.md` targets, the anchor is matched
  against GitHub's heading-slug rules: strip inline markdown
  (`**`, `*`, `\``), lowercase, spaces → hyphens, drop any char that
  isn't `[a-z0-9-_]`. Em-dashes and emoji collapse to nothing
  (yielding double-hyphen runs) which is exactly how GitHub renders
  them.
- Code fences (`\`\`\``) and inline-code spans (`\`…\``) are stripped
  before link extraction to avoid false-positives on documented
  example links.
- Excluded paths: `target/`, `node_modules/`, `.git/`,
  `fixtures/ontologies/` (per task spec — the W3C ontology fixtures
  carry their own internal links unrelated to repo docs).

Surprising findings: none. The repo is already link-clean. This
reflects the cumulative discipline of slices #33–#66 (each prior
slice has fixed its own anchor / path drift in-flight rather than
deferring), so the final lockdown pass finds nothing to fix. If a
permanent CI gate is wanted for v0.3 GA, the audit logic (61 files →
153 links → 0 broken in under a second) is small enough to land as
a `just check-links` recipe in a follow-up slice.

### Docs — guide intro + install audit (slice #44)

Companion pass to slice #45: walked the three user-guide entry-point files
(`guide/README.md`, `guide/00-intro.md`, `guide/01-install.md`) end-to-end
against current shipped reality — same discipline as the README audit, no
restructuring, only drift correction.

Audit scope and results:

| File | Surface | Result |
| --- | --- | --- |
| `guide/README.md` | All internal + external links, page-row blurbs, client-page targets, GitHub-issues URL | clean — every link resolves; the four pages and four client guides exist on disk |
| `guide/00-intro.md` | Status block ("Alpha, v0.3 engine feature-complete"), SPARQL feature row vs LLD §3 capability matrix, deferred-to-v0.4 list vs LLD §3 ⏳ entries, ERRATA E-009 link, naming + conventions claims | one fix — RDF-star out-of-scope citation pointed at "SPEC.pgRDF.LLD §2" but neither LLD v0.2 §2 ("High-Level Architecture") nor LLD v0.3 §2 ("What's shipped") addresses RDF-star / quoted triples. The citation is unfounded; re-pointed to ERRATA E-009 (the actual upstream feature-unification block on RDF 1.2 triple-term support, same root cause that gates the real SHACL impl) |
| `guide/00-intro.md` | SPARQL feature line: SELECT/ASK with BGP + FILTER + DISTINCT/LIMIT/OFFSET/ORDER BY + OPTIONAL + UNION + MINUS + aggregates (COUNT, SUM, AVG, type-aware MIN/MAX, GROUP_CONCAT, SAMPLE) + HAVING (alias **and** inline aggregate) + BIND | clean — matches LLD §3 verbatim and the README status pill |
| `guide/00-intro.md` | Code-block UDF signatures (`load_turtle`, `count_quads`, `sparql`, `materialize`, `add_graph`) | clean — every example matches `src/storage/`, `src/inference/`, `src/query/` current arity |
| `guide/00-intro.md` | "What's NOT" list (federated SPARQL, full OWL 2 reasoner, RDF-star, replacement for graph DB) | clean |
| `guide/01-install.md` | Path A compose flow (`build-ext`, `compose-up`, `psql`, `CREATE EXTENSION`, `pgrdf.version() → 0.2.0`) | clean — every recipe is in the `Justfile`; `compose/.env.example` exists; `pgrdf.version()` returns `env!("CARGO_PKG_VERSION") = "0.2.0"` |
| `guide/01-install.md` | PG-version range claim (`postgres:14..postgres:17`) | clean — matches ERRATA E-006 hold (pgrx 0.16.1, PG 14-17) for v0.3; PG 18 deferred to v0.4 |
| `guide/01-install.md` | Path B Kubernetes ref to `specs/SPEC.pgRDF.INSTALL.v0.2.md` | clean — spec is unchanged in v0.3 per LLD §0 |
| `guide/01-install.md` | Path C manual-install URL (`releases/download/v0.2.0/pgrdf-0.2.0-pg17-glibc-amd64.tar.gz`) | acknowledged as illustrative — no `v0.2.0` GitHub release exists yet (no git tag in the repo), but the section header is "If you have a Postgres server you control" with a "Download the matching tarball" comment that reads as a worked example for the post-release case. INSTALL spec §3 uses the same placeholder pattern with `0.4.1`. Left as-is per the conservative rule. |
| `guide/01-install.md` | Verify-install snippet (`SHOW shared_preload_libraries`, `pgrdf.stats() -> 'shmem_ready'`) | clean — `shmem_ready` is the documented field in `src/storage/stats.rs:45` |
| All three files | "Phase 3 — Extended SPARQL surface" stale-label check (slice #45 adjacent finding) | not present — the guide files don't carry the conflicting roadmap label |
| All three files | `just test-all` / new recipe coverage | not referenced — the guide intentionally points users at `compose-up` + `psql`, not the test harness |

**Result: one citation correction.** The §2 reference for the RDF-star
out-of-scope policy was unfounded — the policy stands as a project
decision rooted in ERRATA E-009 (`oxrdf` `rdf-12` feature surface
conflicting with `reasonable 0.4.1`), not in any LLD section. Re-pointed
the citation; no other text moved.

Drift sources surveyed for completeness:
- v0.3 vs v0.4 target labels — every deferred bullet in the guide files
  already says `⏳ v0.4` correctly.
- "Alpha"/"unstable"/"experimental" framing — the guide says
  "Alpha, v0.3 engine feature-complete" which is the same status the
  README pill carries. No "experimental" / "unstable" wording leaked in.
- Test counts (93/39/23/3 = 158) — guide files don't cite specific
  counts (those live in the README + `docs/08-testing.md`); no drift
  surface here.

This completes the three-file user-guide entry-point sweep.

### Docs — README audit (slice #45)

Final pre-release pass over `README.md` — every badge, every link, every
code block, every test-count claim, every status pill walked against
current shipped reality.

Audit scope and results:

| Surface | Check | Result |
| --- | --- | --- |
| Top-of-file badges (12) | URL resolves, value matches reality | clean |
| Status row pill | "v0.3 engine surface feature-complete", SPARQL feature list, Phase 3/4/5/6 labels, PG version list, deferred-to-v0.4 list | clean — Phase numbering matches `specs/SPEC.pgRDF.LLD.v0.3.md` §5; SPARQL feature list matches `docs/10-roadmap.md` §Phase 3 steps 1–12 plus the brought-forward HAVING-inline-aggregate and type-aware MIN/MAX |
| Local-file link targets (~25) | Each path exists on disk | clean — every `LICENSE`, `NOTICE`, `docs/*`, `guide/*`, `specs/*`, `tests/perf/*`, `tests/w3c-sparql/`, `TEST.ONTOLOGY-SET.md` link resolves |
| Code-block UDF signatures (`load_turtle`, `load_turtle_verbose`, `parse_turtle`, `add_graph`, `count_quads`, `materialize`, `sparql`, `sparql_parse`, `version`) | Signature in README matches `src/` | clean — every example matches current arity and return type |
| `just` recipes referenced (`build-ext`, `compose-up`, `psql`, `test`, `test-regression`, `test-w3c`, `test-lubm`, `test-all`, `test-conformance`, `test-everything`, `smoke-cold`) | Recipe exists in `Justfile` | clean |
| Test counts (93 pgrx + 39 pg_regress + 23 W3C-shape + 3 LUBM-shape = 158) | Match disk reality | clean — pgrx `#[pg_test]` grep returns 93; `tests/regression/sql/*.sql` count is 39; `tests/w3c-sparql/` has 23 test dirs; `tests/perf/lubm-shape/` has 3 query dirs |
| Smoke claim (24 ontologies, 17,134 triples) | Matches `tests/perf/smoke-ontologies.expected.tsv` | clean — 24 rows, sum of triple-count column is 17,134 |
| License section | Matches `LICENSE` + `NOTICE` (Copyright 2026 Peter Styk, Apache-2.0) | clean |
| ERRATA E-006 re-check date (2026-05-14) | Matches `specs/ERRATA.v0.2.md` | clean |
| `pgrdf.version()` return ("0.2.0") | Matches `Cargo.toml` `version` field | clean |
| CI W3C-shape + LUBM-shape wiring | Workflow runs `tests/w3c-sparql/run.sh` and `tests/perf/lubm-shape/run.sh` | clean — `.github/workflows/ci.yml` lines 119 + 128 |

**Result: zero drift.** No facts required correction; no links required
re-targeting; no signatures required updating. README is consistent with
the v0.3 LLD, the current Justfile, the current test fixtures, the
current Cargo.toml, and the current ERRATA. The audit re-establishes the
baseline before the v0.3 tag.

One adjacent finding — out of scope for this audit but noted for the
follow-on: `docs/10-roadmap.md` carries **two overlapping Phase 3
labels** (a `### Phase 3 — Extended SPARQL surface` heading at line 130
inside the `## Phase 2` section, AND a "Phase 3 storage performance" use
at the intro line 5 and the §270 test-bar-over-time table). Both are
internally consistent with the LLD §5 phase numbering (`Phase 3 =
Storage Performance`), but the in-line heading creates a local
ambiguity. Not corrected here — roadmap surgery is its own slice — and
the README correctly tracks the LLD scheme. Filed mentally for the
roadmap maintenance group.

### Hygiene — Cargo.lock freshness audit (slice #46)

Final entry in the hygiene group (54 → 46, sixty-six → forty-six). Verified
`Cargo.lock` is committed (`git ls-files Cargo.lock` returns the path) and
matches `Cargo.toml`: `cargo metadata --format-version 1` resolves clean on
the online index. **Reproducibility check** — captured the lock's MD5
(`1627cb986cfb73ca300550854b9564d5`), ran `cargo build --no-default-features
--features pg17` (via the rustup-managed `stable-aarch64-apple-darwin`
toolchain at rustc 1.95, since Homebrew's PATH-first rustc 1.88 is below
the declared MSRV); link step fails on the workstation as expected (pgrx
final link wants `pg_config` on PATH, not in scope here), but resolution
completes and the post-build MD5 is byte-for-byte identical. The lock is
stable — Cargo did not touch it during a fresh resolution pass.

**`cargo update --dry-run --verbose`** (online):

```
Updating crates.io index
 Locking 1 package to latest compatible version
Unchanged pgrx v0.16.1 (available: v0.18.0)
Unchanged pgrx-tests v0.16.1 (available: v0.18.0)
Updating winnow v1.0.2 -> v1.0.3
warning: not updating lockfile due to dry run
```

| Crate | Bump | Classification | Root |
| --- | --- | --- | --- |
| `winnow` | 1.0.2 → 1.0.3 | Safe patch of a build-time transitive | `pgrx-pg-config → cargo_toml → toml 0.9.12+spec-1.1.0 → toml_parser 1.1.2 → winnow 1.0` |
| `pgrx` | 0.16.1 → 0.18.0 (held) | Pinned root (E-006) — not eligible under current `Cargo.toml` constraint `0.16` | direct |
| `pgrx-tests` | 0.16.1 → 0.18.0 (held) | Pinned root (E-006) — same | dev-dep |

The single eligible bump (`winnow` 1.0.2 → 1.0.3) is a patch on the
inner parser used by `cargo_toml` at build time — no runtime crate
touched, no `serde_json`/`serde_core`/`tokio`/-sys edge in scope, no
pinned root moves. Under a v0.4-cycle policy this would land
automatically alongside a `just test-regression` re-run.

**Decision: skip and defer to the v0.4 hygiene cycle.** Rationale:
the lock is reproducing cleanly, the only eligible bump is a single
transitive patch with zero behavioural surface, and the v0.3 tag is
imminent. Intentional churn against `Cargo.lock` this close to a
release tag adds risk (new regression-test pass required, new
artifact hash, new container layer) without proportionate benefit.
The held bumps on `pgrx` 0.16 → 0.18 are gated by ERRATA E-006 and
will move only when E-006 resolves; that's a v0.4 work item already
on the roadmap, and `winnow` will ride along on the same `cargo
update` invocation at that point.

This closes the hygiene group (slices #54 → #46, 9 entries). Lock
is fresh, reproducible, and audited; the next intentional refresh
is owed in the v0.4 cycle.

### Hygiene — lints allowlist review (slice #47)

Audited every `#![allow(...)]` / `#[allow(...)]` attribute in `src/`
plus the `[lints.rust]` block in `Cargo.toml`. Procedure: comment each
entry, run `cargo check --no-default-features --features pg17 --tests`
(rustc 1.95.0 via rustup-installed stable, since the Homebrew rustc on
this workstation is 1.88 — below the declared MSRV), then restore.
For each entry, classify the lint as still firing (keep), masking a
single site only (narrow candidate), or no longer firing (trim
candidate).

| Allow | Location | Scope | Lint still fires? | Disposition |
| --- | --- | --- | --- | --- |
| `unreachable_patterns` | `src/lib.rs:14` | crate | Yes — 6 sites (`reasonable.rs:247`, `loader.rs:161`, `executor.rs:1861`, `executor.rs:1902`, `parser.rs:161`, `parser.rs:212`) | Keep. Rationale ("future-proof against upstream `#[non_exhaustive]` variant additions") is crate-wide design intent. |
| `clippy::doc_lazy_continuation` | `src/lib.rs:19` | crate | Yes — 4 doc sites (`lib.rs:35`, `lib.rs:36`, `executor.rs:450`, `executor.rs:451`) | Keep. Rationale ("vertically-aligned ASCII continuation lines"). |
| `clippy::useless_conversion` | `src/lib.rs:25` | crate | Yes — 1 site (`executor.rs:156` `SetOfIterator::new(rows.into_iter())`) | Keep. Single-site narrowing would invert the rationale ("don't litter call sites with annotations"). |
| `unreachable_patterns` | `src/inference/reasonable.rs:246` | item | Redundant under crate-level allow above, but documents intent at the call site | Keep. |
| `unreachable_patterns` | `src/storage/loader.rs:160` | item | Same as above. | Keep. |
| `[lints.rust] unexpected_cfgs check-cfg = ["cfg(feature, values(\"pg13\", \"pg18\"))"]` | `Cargo.toml:53` | crate | Yes — 9 `pg13` + 9 `pg18` sites under rustc 1.95 (pgrx 0.16.1's `pg_shmem_init!` / per-PG `pg_guard` shims expand cfg branches for every PG major they know about regardless of which `feature = "pgN"` we select) | Keep. |

**Result:** zero trims. Every allow on disk currently suppresses a
real lint that fires under rustc 1.95. The two item-level
`unreachable_patterns` allows are redundant under the crate-level one
but document intent at the call site, so they stay. Cargo accepts
`[lints.rust]` as written (the IDE schema linter flags it as invalid
under its TOML schema; `cargo check` parses it without complaint).
This audit re-establishes the baseline: a future slice that drops or
narrows an allow can point back here as the prior-state record.

### Hygiene — ERRATA E-006 pgrx-upstream re-check (slice #48)

Refreshed `specs/ERRATA.v0.2.md` E-006 against today's upstream state.
`crates.io` reports `pgrx.max_stable_version = "0.18.0"` (unchanged
since 2026-04-17); `develop` is one commit ahead (PR #2280, an
aarch64 `-Wl,--no-gc-sections` link-flag fix). Upstream README now
documents "pgrx supports Postgres 13 through Postgres 18" — PG 18
support has officially landed at the 0.18.0 line. Local-compile
blockers from the 2026-05-13 saga are unchanged: 0.17.0's
`non_null_from_ref` E0658 and 0.18.0's `impl_table_iter` E0716 still
reproduce on every Rust stable/nightly we tested, and `develop` has
not touched the relevant macro since the release. Additionally,
0.18.0 carries a hard breaking migration (PR #2264 /
`v18.0-MIGRATION.md`): `pgrx_embed` binary removed, `crate-type` must
drop `"lib"`, manual `SqlTranslatable` impls move from methods to
associated `const`s. pgRDF still ships `src/bin/pgrx_embed.rs` and
`crate-type = ["cdylib", "lib"]`, so the bump is non-trivial.

**Disposition:** E-006 stays open. Classification B — partially
resolved at the upstream layer (PG 18 support exists), still blocked
at the consumption layer (E0716 + breaking-migration scope). Hold
pgrx 0.16.1 + PG 14–17 matrix for v0.3; defer pgrx-0.18 migration to
v0.4 as a planned work item. README + `docs/10-roadmap.md` updated
to reflect the partial-resolution framing. Next re-check trigger:
any pgrx publish above 0.18.0 OR an E0716 fix landing on `develop`.

### Hygiene — MSRV declared (slice #49)

Added `rust-version = "1.91"` to `[package]` in `Cargo.toml`. The value
matches the CI build container (`compose/builder.Containerfile` →
`FROM rust:1.91-bookworm`), which is the only Rust version pgRDF
artifacts are actually produced against. The existing `[lints.rust]`
block already assumes Rust 1.91+'s strict `check-cfg` behavior (see
the inline comment introduced in an earlier slice), so declaring a
lower MSRV would misadvertise support. pgrx 0.16.1's
`resolver = "3"` independently imposes a 1.84 floor; this declaration
tightens that to the value CI verifies. `rust-toolchain.toml` stays
on `channel = "stable"` — pinning a specific minor for an active
project trades health for false stability.

Verification: `cargo check --no-default-features --features pg17
--ignore-rust-version` clean on the dev workstation (rustc 1.88.0
Homebrew); the `--ignore-rust-version` is required only because the
workstation toolchain is older than the declared MSRV — CI's 1.91
container is unaffected. Bump the `rust-version` in lockstep with
the Containerfile when upgrading the build floor.

### Hygiene — cargo tree duplicate-version audit (slice #50)

Ran `cargo tree --duplicates --no-default-features --features pg17`
against `Cargo.lock`. Workspace currently resolves to **182 crates**
(normal + build edges); first-order direct deps are seven: `oxrdf`,
`oxttl`, `pgrx`, `reasonable`, `serde_json`, `spargebra` (normal)
plus `pgrx-tests` (dev). Nine crates appear at two distinct versions:

| Crate | Versions | Sources | Fix attempted |
| --- | --- | --- | --- |
| `byteorder` | 0.5.3 / 1.5.0 | `reasonable → roaring 0.5.2` (0.5) vs `pgrx-tests → tokio-postgres → postgres-protocol` (1.5) | No. `reasonable` is pinned (E-009); `roaring 0.5.2`'s old `byteorder 0.5` is structural until `reasonable` bumps. |
| `getrandom` | 0.3.4 / 0.4.2 | `oxrdf/oxttl/spargebra → rand 0.9 → rand_core 0.9 → getrandom 0.3` vs `pgrx → uuid + tempfile + rand 0.10 → getrandom 0.4` | No. Both roots pinned (oxrdf/oxttl/spargebra semantic-stability; pgrx 0.16 E-006). |
| `hashbrown` | 0.15.5 / 0.17.1 | `pgrx-sql-entity-graph → petgraph 0.8.3 → hashbrown 0.15` AND same `petgraph 0.8.3 → indexmap 2.14 → hashbrown 0.17`. `petgraph 0.8.3` itself pulls two hashbrowns. | No. Internal to `petgraph 0.8.3`; not fixable downstream. |
| `itertools` | 0.8.2 / 0.13.0 | `reasonable` (0.8) vs `pgrx-bindgen → bindgen 0.71.1` (0.13) | No. Both roots pinned. |
| `rand` | 0.9.4 / 0.10.1 | `oxrdf/oxttl/spargebra` (0.9) vs `pgrx-tests → tokio-postgres → postgres-protocol` + `pgrx → uuid + tempfile` (0.10) | No. Both roots pinned. |
| `rand_core` | 0.9.5 / 0.10.1 | Follows the `rand` split. | No. Same as `rand`. |
| `thiserror` | 1.0.69 / 2.0.18 | `reasonable` + `cargo_metadata 0.18.1 (via clap-cargo via pgrx-tests)` (1.x) vs `oxrdf/oxttl/spargebra/pgrx/pgrx-pg-config/pgrx-sql-entity-graph/pgrx-tests` (2.x) | No. `thiserror` 1↔2 is an intentional major; reasonable+cargo_metadata cannot move to 2 without their own bumps. |
| `thiserror-impl` | 1.0.69 / 2.0.18 | Mirrors `thiserror`. | No. Same as `thiserror`. |
| `winnow` | 0.7.15 / 1.0.2 | `pgrx-pg-config → cargo_toml → toml 0.9.12+spec-1.1.0` — same `toml` crate uses `winnow 0.7` (top-level parser) AND `winnow 1.0` (via the inner `toml_parser` 1.1.2 helper crate). | No. Internal to `toml 0.9.12`; not fixable downstream. |

Plus six crates that `cargo tree --duplicates` flags but Cargo.lock
shows at exactly one version (`bitflags 2.11.1`, `memchr 2.8.0`,
`peg-runtime 0.8.6`, `percent-encoding 2.3.2`, `serde_core 1.0.228`,
`serde_json 1.0.149`) — these are single-version crates pulled in
through multiple distinct dep chains, which the `--duplicates` view
also surfaces. Nothing to fix on those.

**Disposition:** zero code or `Cargo.lock` changes. Every actual
duplicate roots in a pinned dep (`reasonable` 0.4.1 / `pgrx` 0.16 /
`oxttl`+`oxrdf`+`spargebra` semantic-stability) or in a transitive
internal split (`petgraph` 0.8.3, `toml` 0.9.12). No SemVer-safe
`cargo update --precise` collapse exists today. The duplicate budget
is in line with what a Rust workspace of this composition produces
and is purely informational — recorded so a future audit can diff
against the same picture once `reasonable` and `pgrx 0.16` are
unpinned (E-006, E-009).

### Hygiene — cargo audit (slice #51)

Ran `cargo audit` (v0.22.1, advisory-db 1088 advisories loaded
2026-05-14) against `Cargo.lock` (287 crate deps). **Zero security
vulnerabilities.** Four informational warnings, all in pinned-dep
subtrees (`pgrx 0.16` / `reasonable 0.4.1`) — none has a
SemVer-compatible fix without violating the pinned-core-dep
constraint (E-006 / pgrx 0.16, `reasonable` 0.4.1 RDF 1.2 saga in
E-009). Deferred to the cuts that bump those upstreams.

| ID | Kind | Crate | Source | Disposition |
| --- | --- | --- | --- | --- |
| RUSTSEC-2024-0375 | unmaintained | `atty 0.2.14` | `reasonable 0.4.1 → env_logger 0.7.1 → atty` | Defer. Fix requires `reasonable` to bump `env_logger` past 0.7; `reasonable` is pinned (see ERRATA E-009). |
| RUSTSEC-2021-0145 | unsound | `atty 0.2.14` | same path as above | Defer. Same root cause; unaligned-read CVE in `atty`'s Windows path. Unreachable on the Linux/macOS targets pgRDF builds for, but the advisory still trips on `Cargo.lock`. |
| RUSTSEC-2024-0436 | unmaintained | `paste 1.0.15` | `pgrx-tests 0.16.1 → paste` (dev-dep only) | Defer. Test-only proc-macro dep of pgrx-tests. Resolves when pgrx is unpinned (E-006). |
| RUSTSEC-2021-0127 | unmaintained | `serde_cbor 0.11.2` | `pgrx 0.16.1 → serde_cbor` | Defer. Hard transitive of pgrx 0.16. Resolves when pgrx is unpinned (E-006). |

Counts: Critical 0 / High 0 / Medium 0 / Low 0 / Yanked 0 /
Informational 4. No code or `Cargo.lock` changes — the advisories
are real but structurally unfixable in v0.3 without breaking the
pinned-dep contract. New ERRATA entry **E-010** records the
pinned-dep advisory ledger so future audits can diff cleanly.

### Hygiene — stale-docstring sweep

Audited every public `#[pg_extern]` and module-level docstring under
`src/` against actual signatures and behavior, looking for prose that
lied about current code (wrong return types, missing JSONB fields,
"Phase N backlog" claims for features that have since shipped, "still
unsupported" lists that the executor now handles). Eleven category-A
fixes landed across 7 files:

| file:line | drift | fix |
| --- | --- | --- |
| `src/storage/hexastore.rs:46` | `add_graph` SQL surface doc claimed `→ VOID` | corrected to `→ BOOLEAN`, documented return semantics (TRUE = created) |
| `src/storage/loader.rs:320-321` | `load_turtle_verbose` JSONB-field list missed `shmem_cache_hits` | added the field to the doc |
| `src/storage/stats.rs:17-29` | `stats()` JSON example showed only `shmem_*` keys; `plan_cache_*` keys (added in Phase 3 step 2) were missing | extended the example with all four `plan_cache_*` fields + explanatory note |
| `src/storage/mod.rs:3-5` | module doc said "Implementation status: skeleton" | replaced with a submodule index reflecting the v0.3 reality |
| `src/query/parser.rs:9-21` | "Today's scope" claimed OPTIONAL/UNION/non-BGP get flagged in `unsupported_algebra`; code walks through them | rewrote scope list to reflect the actual `unsupported_algebra` rejection set + added v0.4 cross-refs |
| `src/query/plan_cache.rs:103` | `plan_cache_clear` SQL surface doc said `→ integer` | corrected to `→ BIGINT` (matches `i64` return) |
| `src/query/executor.rs:14-17` | module doc claimed "dynamic SQL only carries integer constants" — incorrect post-Phase-3-step-2 (placeholders, not inlined constants) | rewrote to reflect `$N` positional parameter binding |
| `src/query/executor.rs:20-21` | "Scope today: SELECT only (no CONSTRUCT/ASK/DESCRIBE)" | ASK ships — fixed; CONSTRUCT/DESCRIBE marked with v0.4-FUTURE §6 pointer |
| `src/query/executor.rs:61-69` | scope said "HAVING and `GROUP_CONCAT` / `SAMPLE` are Phase 3 backlog" and "BIND remain unsupported" — all three landed | rewrote aggregates + BIND blocks to reflect implementation; v0.4-FUTURE pointers on the still-unsupported set |
| `src/query/executor.rs:498-499` | `parse_aggregate` doc listed only COUNT/SUM/AVG/MIN/MAX | added GROUP_CONCAT and SAMPLE; noted Custom IRI panic |
| `src/query/executor.rs:1391-1404` | `translate_filter` doc said numeric ordering, IN, REGEX were "not yet supported" — they are | rewrote both lists; left `EXISTS` + conditional `IF` as still-unsupported |
| `src/query/executor.rs:1535-1539` | `expr_to_id_sql` doc said constants → "inlined integer literal" | corrected to "`$N` parameter placeholder bound to the resolved dict id" |
| `src/query/executor.rs:2107-2109` | test docstring `sparql_unknown_predicate_returns_zero_rows` said translator "inlines `-1`" | corrected to "binds `-1` as the parameterised dict id sentinel" |
| `src/validation/shacl.rs:38-52` | `validate` JSONB schema doc listed `data_graph_exists` + `shapes_graph_exists` fields the body does not emit | removed the two fields from the doc (the stub doesn't emit existence flags — only counts) |

No behavior changes. `cargo check --features pg17` is green. Test bar
unchanged (39 pg_regress + 93 pgrx + 23 W3C + 3 LUBM = 158).

### Licensing — explicit attribution surface

`LICENSE` carries the resolved Apache 2.0 copyright notice
("Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;" + project URL) in
place of the upstream `[yyyy] [name of copyright owner]`
placeholders. A new `NOTICE` file at the repo root carries the
Apache convention header — distributions that bundle pgRDF
should preserve it per Apache 2.0 §4(d). `Cargo.toml` gains an
`authors = ["Peter Styk &lt;peter@styk.tv&gt;"]` field and a
`homepage` mirror of the `repository` URL. `README.md`'s License
section is fleshed out to name the copyright holder and link
both `LICENSE` and `NOTICE`. No code or test changes.

### Spec — SPEC.pgRDF.LLD.v0.4-FUTURE draft landed (forward-looking)

New `specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md` is a draft, forward-looking
target spec for the next cut; v0.3 remains the authoritative
shipped contract until v0.4 actually lands. The draft scopes five
new substantive tracks — named-graph scoping with an IRI ↔ graph_id
mapping table (§3), SPARQL UPDATE including the graph-scoped
variants (§4), graph-level lifecycle UDFs over the LIST-partitioned
quads table (§5), CONSTRUCT returning triple-shaped JSONB rows
(§6), and property paths `*` / `+` / `?` / `^` with
materialised-closure-aware translation (§7) — plus the v0.3-deferred
SPARQL surface backlog (multi-triple OPTIONAL, VALUES,
BIND-downstream, aggregates over UNION, DESCRIBE) which shares
enough translator machinery with §4 and §6 to ship in the same cut.
v0.3 §0 gains a one-line cross-link to the new draft.

### Coverage — error-path regression signals

New `tests/regression/sql/81-error-paths.sql` opens a sibling track
to `80-unsupported-shapes.sql`: instead of locking the failure-mode
of SPARQL translator gaps, it locks the stable error-prefix each
UDF emits when given an invalid input. The helper `_check_error`
generalises `80`'s `_check_gap` to run arbitrary SQL via `EXECUTE`
inside a plpgsql try/catch, capturing the boolean signal (`t` =
expected substring present in SQLERRM) without pinning the
volatile tail (OS-level `os error N` numbers, path strings, etc.).

This commit locks #66 of the 66 → 1 countdown toward v0.3.0:
`pgrdf.load_turtle()` against a missing path must surface the
prefix `load_turtle: failed to open` (from
`src/storage/loader.rs:315`). Downstream tooling matches that
prefix to decide retry-vs-escalate; a silent rename would break
those callers without any pgRDF-side test firing.

Locks #65 of the countdown: a syntactically invalid `base_iri`
argument must surface the prefix `load_turtle: invalid base IRI`
(from `src/storage/loader.rs::ingest_turtle_with_stats`'s
`with_base_iri().unwrap_or_else(...)`). The check fires through
`pgrdf.parse_turtle('...', 9982, 'not an iri at all')` — using
`parse_turtle` keeps the regression file fixture-free while
exercising the same shared ingest path. The panic message is
prefixed `load_turtle:` even when triggered via `parse_turtle`;
that cross-UDF prefix invariance is itself part of the contract
(downstream callers route on one substring regardless of which
UDF parses the Turtle). Empty-string `base_iri` continues to
short-circuit before `with_base_iri()` runs, so callers can
safely pass `''` to mean "no base"; only a non-empty value that
fails oxiri's IRI grammar trips the prefix.

Locks #64 of the countdown: syntactically malformed Turtle bytes
must surface the prefix `load_turtle: turtle parse error` (from
`src/storage/loader.rs:256`'s `triple_result.expect(...)` inside
the parser-iterator loop). The check fires through
`pgrdf.parse_turtle(':alice :name "Alice"', 9964)` — the
fragment uses the default `:` prefix without declaring it, so
oxttl rejects at byte 0 with `The prefix : has not been
declared`; that specific complaint is tail / volatile, the
locked substring is just the `load_turtle: turtle parse error`
prefix. The same cross-UDF prefix invariance as error-65
applies: the panic text says `load_turtle:` regardless of
whether bytes entered via `load_turtle()` or `parse_turtle()`,
so downstream tooling routes on one substring. Any malformed
Turtle variant (missing trailing dot, undeclared prefix, bad
IRI ref, RDF-star in default mode) trips the same prefix.

Locks #63 of the countdown: a syntactically malformed SPARQL
query handed to `pgrdf.sparql()` must surface the prefix
`sparql: parse error:` (from `src/query/executor.rs:142`'s
`SparqlParser::new().parse_query(sql).unwrap_or_else(...)`).
The check fires through
`SELECT * FROM pgrdf.sparql('this is not sparql at all')` —
spargebra rejects at byte 10 with `expected CONSTRUCT`; that
specific complaint plus the line:col coordinates are tail /
volatile across spargebra versions, the locked substring is
just the `sparql: parse error:` prefix. This is the user-facing
contract surface for query-parse failure, distinct from the
translator-gap prefix locked across `80-unsupported-shapes`
(`sparql: …not supported yet`, `sparql: aggregates on top of
UNION…`, etc.) and from the RDF-ingest prefixes locked in
error-66/65/64. The sibling introspection UDF
`pgrdf.sparql_parse()` routes through its own panic site with
prefix `sparql_parse:` instead — a deliberate distinction so
callers can tell which entry point the bytes came in through;
that path is covered by the `#[pg_test]`
`sparql_parse_syntax_error_panics` in `src/query/parser.rs`
and is not pinned by this regression slice.

Test bar: **93 pgrx + 34 pg_regress + 23 W3C-shape + 3 LUBM-shape
= 153 tests**, green locally.

### Coverage — edge-case correctness regression signals

New `tests/regression/sql/62-materialize-empty.sql` opens a sibling
track to the error-path file (`81-error-paths.sql`): instead of
locking the prefix a UDF emits when given an *invalid* input, it
locks the *correctness contract* on **edge-case but valid** inputs
the engine must handle without surprise. The countdown shifts from
66→63 (error-path locks) into 62→onward (edge-case locks).

Locks #55 — the final entry in the 66→1 coverage countdown — promotes
the W3C-shape and LUBM-shape harnesses to first-class Justfile
recipes and adds a cold-compose smoke that exercises every
compose-based test layer end-to-end. New recipes: `just test-w3c`
(wraps `bash tests/w3c-sparql/run.sh`), `just test-lubm` (wraps
`bash tests/perf/lubm-shape/run.sh`), `just test-conformance` (the
three compose-based layers: regression + W3C-shape + LUBM-shape),
`just test-everything` (pgrx integration + test-conformance — the
broadest sweep), and `just smoke-cold` (`compose-down` →
`build-ext` → `compose-up` → `CREATE EXTENSION` → test-conformance,
the cold-compose discipline gate). `just test-all` keeps its
original narrow shape (`test` + `test-regression`) for back-compat;
`docs/08-testing.md` and `README.md`'s Tests block point at
`test-everything` and `smoke-cold` as the new entry points. The
shift matters because two of the three compose-based harnesses
(W3C-shape, LUBM-shape) were previously discoverable only by
knowing the bash paths — a contributor running `just --list`
saw nothing about them, and `just test-all` silently skipped
them. Cold-compose smoke is the verification half: it tears the
compose stack down with `compose-down` first (no shortcuts to a
warm `compose-up`), rebuilds the extension artefacts, brings the
stack back, recreates `CREATE EXTENSION pgrdf`, and runs all
three compose-based layers against the fresh state — catching the
class of bugs that pass on a warm compose because some prior
DROP/CREATE left state behind, and break on the next cold boot.
This is the final coverage-countdown slice before the hygiene
phase opens.

Locks #62 of the 66→1 countdown: `pgrdf.materialize(N)` on a graph
with zero base triples MUST NOT panic and MUST return a JSONB stats
object with `base_triples = 0`. The UDF still emits OWL 2 RL
**axiomatic triples** (per `reasonable 0.4`, four self-statements
over `owl:Thing` / `rdfs:Class` / etc. on the empty input) — that
count is upstream-defined and NOT pinned by this slice; only
`inferred_triples_written ≥ 0` is part of the locked contract.
Idempotency carries across the empty case: a second
`materialize(N)` call wipes its own prior `is_inferred=TRUE` rows
before re-deriving, so run 2's `previous_inferred_dropped` equals
run 1's `inferred_triples_written` exactly. Both invariants
project as booleans (`base_is_zero`, `inferred_nonneg`,
`first_run_dropped_zero`, `idempotent`) so the expected output
stays `t` regardless of axiomatic-set churn from upstream
`reasonable` releases.

Locks #61 of the countdown: `pgrdf.shmem_reset()` MUST actually
invalidate the process-wide shmem dict cache. The implementation
in `src/storage/shmem_cache.rs::reset()` bumps a single
`PgAtomic<AtomicU64>` `GENERATION` counter; `lookup()` reads slots
as cold whenever `slot.generation != current`. A refactor that
drops the generation bump would silently leave stale dict ids
visible across a `DROP EXTENSION; CREATE EXTENSION` cycle (where
the dict id space resets), so the regression must catch the
omission. New `tests/regression/sql/63-shmem-reset-invalidation.sql`
warms shmem with three terms, snapshots `(shmem_hits,
shmem_inserts)` via `\gset`, re-parses the same Turtle and asserts
hits went up (sanity — cache is hot), then calls `shmem_reset()`,
re-parses one more time, and asserts (a) `shmem_hits` stayed flat
across the post-reset parse and (b) `shmem_inserts` strictly
increased. Counter VALUES are not pinned — each assertion projects
a single boolean comparing deltas, so the expected output stays
`t`-flat across cumulative-counter drift from prior tests in the
same psql session.

Locks #60 of the countdown: `pgrdf.plan_cache_clear()` MUST return
the literal count of prepared statements drained from THIS
backend's `thread_local!` plan-cache HashMap — NOT zero, NOT a
constant, NOT the cumulative shmem `plan_cache_inserts` counter.
The implementation in `src/query/plan_cache.rs::plan_cache_clear`
reads `m.len()` BEFORE calling `m.clear()` and returns that as
`i64`; a refactor that swaps `m.len()` for a constant, or hoists
the `len()` call to AFTER `m.clear()` (always returning 0), would
corrupt operator-facing telemetry. New
`tests/regression/sql/64-plan-cache-clear.sql` locks four
invariants: (a) fresh backend → `clear()` returns 0 (nothing to
drop); (b) after one `parse_turtle` + three structurally distinct
SPARQL shapes, the drained count matches the pre-clear
`plan_cache_local_size` snapshot; (c) `plan_cache_local_size = 0`
immediately after the clear; (d) a second consecutive clear
returns 0 (idempotent at zero). Empirically `size_before = 4` on
the current pgrx 0.16 / PG 17 build (1 ingest-side `flush_batch`
INSERT plan + 3 SELECT plans), but the test locks the RELATION
`drained = size_before AND size_after = 0 AND idempotent_clear =
0 AND size_before > 0` rather than the literal — an ingest-path
refactor that takes `flush_batch` off the plan-cache path leaves
the test still passing as long as the contract holds. Bare-row
`SELECT count(*) FROM pgrdf.sparql(...)` calls are the cleanest
way to drive distinct plans into the cache; `\gset` captures the
snapshots without polluting the expected-output stream.

Locks #59 of the countdown: `pgrdf.parse_turtle()` MUST accept
*triple-free* Turtle input without panicking and MUST return `0`
as the inserted-triple count. The parser path in
`src/storage/loader.rs::ingest_turtle_with_stats` drives an oxttl
`TurtleParser` iterator whose for-loop body — the only site that
interns dict ids and pushes onto `batch_s/p/o` — runs ONCE PER
TRIPLE. Inputs that contain no triples (empty string,
whitespace-only, Turtle comment lines, bare `@prefix` declaration)
yield zero iterator items: the loop body never executes,
`stats.triples` stays `0`, the trailing `flush_batch()` flushes
empty vectors (no SQL is emitted to `_pgrdf_quads`), and the
function returns `0`. New `tests/regression/sql/65-parse-turtle-empty.sql`
locks six invariants in one go: each of the four
zero-triple inputs returns `0` (four booleans), `_pgrdf_quads`
for the test graph stays empty (one boolean), and
`_pgrdf_dictionary` stays empty across all four parses (one
boolean — interning is loop-body-only, so the `@prefix` IRI in
case 4 is parser-scope state, not a dict write). This is the
orthogonal correct-path companion to the malformed-input case
noted in `81-error-paths.sql` (where the parser panics with the
literal `load_turtle: turtle parse error: …` prefix): an EMPTY
parser iterator is NOT a parse error — it returns `0` cleanly.
Guards against a refactor that wraps the loop in a "fast-path"
panicking on empty input, that seeds a placeholder dict/quad row,
or that mishandles `flush_batch()` of zero-length arrays. The
whitespace-only case uses an `E''` extended-string literal so
`\n` / `\t` reach the parser as actual whitespace rather than
literal backslash-n (which the Turtle grammar correctly rejects).

Locks #58 of the countdown: the smoke-ontologies set MUST keep
parsing AND each ontology's triple count MUST stay stable. The
existing `tests/perf/smoke-ontologies.sh` loads every `*.ttl`
under `fixtures/ontologies/` through `pgrdf.load_turtle` into its
own graph and prints `<filename>: <triples>` per file; today's
snapshot is **24 ontologies, 17,134 triples** (workflow.ttl held
out per ERRATA E-007). Slice #58 captures that snapshot as
`tests/perf/smoke-ontologies.expected.tsv` (alphabetically-sorted
`filename<TAB>triples` rows) and adds a `--check` mode to the
smoke script: it re-runs the smoke, regenerates the TSV from the
live output, and `diff -u`'s it against the lock-file, exiting
non-zero on any drift. The diff catches two regression classes
the bare smoke can't: (a) an ontology that used to parse stops
parsing — the row disappears from the actual side; (b) the
parser silently drops or duplicates triples and the count moves
even though parsing nominally succeeds. The check is NOT yet
wired into CI (the fetched ontology payloads under
`fixtures/ontologies/*.ttl` are gitignored, so CI can't run the
smoke without a fetch step that doesn't yet exist in the
workflow). Landing the lock-file + the opt-in `--check` mode
now means a future Phase 6 slice can wire `--check` once the
ontology-fetch step is added to CI. The default behaviour
(no flag → pretty-print results, exit 0) is unchanged so
existing manual runs still work. Updating the lock-file is a
deliberate maintenance step — when an upstream ontology updates
and the new count is intentional, regenerate the TSV from a
fresh smoke run and commit the delta as one explicit move; no
`--accept`-style automatic refresh.

Locks #57 of the countdown: the end-to-end round-trip from
`pgrdf.parse_turtle` ingest through `pgrdf.sparql` query MUST
preserve every triple the parser saw, across all four
object-term kinds AND the blank-node-subject case. New
`tests/regression/sql/66-parse-sparql-roundtrip.sql` parses a
single 5-shape Turtle fragment and asserts five
`bool_and(EXISTS (SELECT 1 FROM pgrdf.sparql(…) WHERE …))`
booleans, one per shape: (1) IRI object —
`ex:alice foaf:knows ex:bob` resolves with the bob IRI as the
lexical projection of `?o`; (2) plain literal —
`foaf:name "Alice"`; (3) typed literal —
`ex:age "30"^^xsd:integer` projects `"30"`; (4) lang-tagged
literal — `ex:bio "Engineer"@en` projects `"Engineer"`; (5)
blank-node subject — the anonymous `[ a foaf:Person ;
foaf:name "Anon" ]` is keyed via a sibling-property join
`?s foaf:name "Anon" . ?s foaf:name ?n` so the
parser-allocated bnode id stays out of the assertion and the
contract is "queryable via sibling property", not "this
specific bnode id". Sibling to `61-materialize-then-sparql.sql`
which locks the materialize→sparql edge; together they pin
both ends of the storage layer's visibility contract to the
SPARQL surface. Datatype URI and lang-tag echo policy are
NOT pinned by this slice — the `pgrdf.sparql` projection
emits the lexical value only; the storage-side datatype-URI
contract is locked separately by `21-typed-literals.sql` and
the lang-tag contract by `22-lang-tags.sql`. Guards against a
refactor that loses a triple from the dict→quads write path
for any one of these five term kinds.

Locks #56 of the countdown: the `pgrdf.stats()` JSONB shape MUST
NOT silently gain, lose, rename, or `null` a field — the canonical
key set is closed at the 10 keys emitted by
`src/storage/stats.rs::stats()` today (`shmem_ready`, `shmem_slots`,
`shmem_hits`, `shmem_misses`, `shmem_inserts`, `shmem_evictions`,
`plan_cache_hits`, `plan_cache_misses`, `plan_cache_inserts`,
`plan_cache_local_size`). Extends the existing `82-stats-shape.sql`
in-place (no new pg_regress file — the file is explicitly scoped to
schema-shape contract and these three new invariants are schema
shape too) with three appended assertion blocks: (a) exact field
count — `count(*) FROM jsonb_object_keys(stats()) = 10`, the
deliberate-update tripwire that fires the moment any new field
lands without a corresponding test update; (b) keys-match-canonical
— `array_agg(k ORDER BY k) = ARRAY[…literal 10-element list…]`,
catches both silent additions and silent renames in one assertion
(an addition makes the array longer; a rename swaps an element);
(c) no-null-fields — `bool_and(jsonb_typeof(value) != 'null')`,
catches a refactor that defaults an uninitialised counter to JSON
`null` rather than `0` (the type-contract block above would not
fire on a null since `jsonb_typeof(null) = 'null'` is checked
positively only on the seven existing-key assertions, not on
unknown keys). The existing "fields-that-should-be-there are
there" assertions are sibling to these "fields-that-shouldn't-be-
there ARE NOT there" assertions; together they pin the closed-set
shape contract that downstream operator tooling (CloudNativePG
operators, CI dashboards, client telemetry parsers) wires against.

Test bar: **93 pgrx + 39 pg_regress + 23 W3C-shape + 3 LUBM-shape
= 158 tests**, green locally. Slice #58 doesn't add a pg_regress
file — the smoke is a separate harness, so its lock-file (24
rows / 17,134 triples) lives alongside the script and is
enforced by `tests/perf/smoke-ontologies.sh --check`. Slice #57
adds the 39th pg_regress file (`66-parse-sparql-roundtrip.sql`).
Slice #56 extends `82-stats-shape.sql` in-place — three new
assertion blocks, three new rows in the expected baseline — no
test count bump (still 39 pg_regress files).

### Translator fix — type-aware `MIN` / `MAX`

`src/query/executor.rs::translate_aggregate` for `MIN` / `MAX`
previously emitted

    MIN(lexical_value)

which sorts lexicographically — so over the four `xsd:integer`
literals `10, 2, 100, 20` it returned `"10"` (since
`"10" < "100" < "2" < "20"` as strings). Now emits

    COALESCE(MIN(numeric_cast_subselect)::text, MIN(lexical_value))

so when any row in the group has an `xsd:numeric` datatype the
numeric MIN/MAX wins (matches the SUM/AVG path that has been
type-aware since Phase 2.2). Pure-string groups fall back to
lexicographic ordering. Mixed-type groups prefer numeric — the
SPARQL spec (§17.4) leaves mixed-type ordering
implementation-defined.

Coverage: new `tests/w3c-sparql/23-min-max-numeric/` — fixture's
`xsd:integer` literals `10/2/100/20` produce `MIN=2, MAX=100`
(would have been `MIN="10", MAX="20"` lexicographically).

Test bar: **93 pgrx + 33 pg_regress + 23 W3C-shape + 3 LUBM-shape
= 152 tests**, green locally. v0.4 deferred SPARQL surface
shrinks by one entry.

### Translator fix — inline `HAVING(SUM(?v) > c)` now supported

`src/query/executor.rs::AggregateSpec` gains a `synth_aliases:
Vec<String>` field that preserves spargebra's synthetic
intermediate-variable name even after `Extend` renames
`output_var` to the user's AS-alias.

Why: spargebra emits algebra of the form
```
Project { Filter(HAVING) { Extend(?total = $synth) {
  Group(aggregates=[($synth, SUM(?p))]) } } }
```
The `Extend` visitor previously rewrote `output_var` from `$synth`
to `?total` and dropped the `$synth` mapping. The HAVING filter
`Filter(Greater(Variable($synth), Literal(15)))` then couldn't
find its aggregate during the filter-migration step and fell
through to the non-aggregate-aware FILTER translator, producing
`sparql: FILTER expression not translatable`.

The fix:
- `AggregateSpec.synth_aliases` is initialised by
  `parse_aggregate` with the original `$synth` name and never
  modified by `Extend`.
- The filter-migration step's `agg_names` is now the union of
  every aggregate's `output_var` AND its `synth_aliases`.
- `translate_filter_with_aggregates`'s lookup helper (`find_agg`)
  consults both fields.

Effects:
- `tests/regression/sql/80-unsupported-shapes.sql::gap-1` removed
  (was negative-locked; no longer a gap).
- New positive coverage:
  `tests/w3c-sparql/22-having-inline-aggregate/` — same shape as
  `08-aggregates-having` but with the inline `HAVING(SUM(?p)>15)`
  form. Hand-computed expected output verified.
- Both forms are now first-class. `08`'s description.md updated
  to note the companion test.
- v0.4 SPARQL-surface deferred list shrinks by one entry.

Test bar: **93 pgrx + 33 pg_regress + 22 W3C-shape + 3 LUBM-shape
= 151 tests**, green locally.

### Translator-gap regression signals + Phase 6 step 3 scaffolding

Two adjacent additions, motivated by a real translator gap I hit
while expanding the W3C-shape harness (W3C 08 — inline `HAVING(SUM
(?v) > c)` falls through with `FILTER expression not translatable`
when spargebra synthesises a fresh aggregate node for the HAVING).

**1. `tests/regression/sql/80-unsupported-shapes.sql`** —
regression signals locking the failure-mode contract for every
known unsupported SPARQL shape. Each gap drives a query that MUST
fail, and asserts via plpgsql `EXCEPTION WHEN OTHERS` that
`SQLERRM` contains a stable error-prefix substring. The check
helper outputs a clean boolean (`t` = expected substring present)
rather than the raw error message — so the baseline isn't pinned
to spargebra's algebra-dump format, synthetic variable hashes, or
upstream `dataset` / `base_iri` internals.

Gaps locked in:
- `gap-1` — `HAVING(SUM(?v) > c)` inline (vs the supported
  alias form `HAVING(?total > c)`).
- `gap-2` — multi-triple OPTIONAL.
- `gap-3` — VALUES inline data block.
- `gap-4` — GRAPH named-graph clause.
- `gap-5` — CONSTRUCT query form.
- `gap-6` — DESCRIBE query form.
- `gap-7` — property path with `*` repetition.
- `gap-8` — aggregates over UNION.

If pgRDF accidentally starts producing wrong results for any of
these shapes (translator regression), the baseline diff fires
with `unexpected success`. If we genuinely add support for a
shape, this file is the single place to flip the assertion to a
positive test.

**2. `tests/perf/lubm-shape/`** — Phase 6 step 3 scaffolding.
Three hand-authored LUBM-shape queries (`Q1` class membership,
`Q2` `teacherOf`, `Q3` `takesCourse` aggregate) against a small
LUBM-shape fixture. Same directory-per-test layout + bash runner
shape as `tests/w3c-sparql/`; runs alongside the W3C harness in
the CI `regression` job. Real LUBM-1/10/100 with the Java
generator + cross-engine comparison vs Apache Jena TDB and
Apache AGE remains v0.4 work (see `tests/perf/README.md`).

Test bar: **93 pgrx + 31 pg_regress + 18 W3C-shape + 3 LUBM-shape
= 145 tests**, green locally.

### Phase 6 step 2 starter — W3C-shape SPARQL harness

- `tests/w3c-sparql/` ships a directory-per-test harness with **13
  hand-authored W3C-shape conformance tests** covering common
  spec patterns:
  - `01-basic-bgp` — §5 Basic Graph Pattern.
  - `02-distinct` — §15.4 `SELECT DISTINCT` multiset → set.
  - `03-union-disjoint` — §18.2.4 `UNION` with disjoint variables
    (unbound → null in cross-branch rows).
  - `04-optional-chain` — §6 `OPTIONAL` keeps the row when the
    optional pattern fails to match.
  - `05-minus-no-shared` — §8.3.2 `MINUS` with no shared variables
    is a no-op (the translator elides the WHERE NOT EXISTS).
  - `06-filter-isiri` — §17.4.2.1 `isIRI` term-type filter.
  - `07-aggregates-count` — §11 `COUNT(?v) GROUP BY ?s`.
  - `08-aggregates-having` — §11.5 `HAVING(?alias > c)` after `SUM`.
  - `09-order-by-desc` — §15.1 `ORDER BY DESC(?v)`.
  - `10-limit-offset` — §15.2 / §15.3 `LIMIT 2 OFFSET 2`.
  - `11-bind-concat` — §10.1 `BIND` + §17.4.3.2 `CONCAT(...)`.
  - `12-ask-true` — §16.2 `ASK` returning `true`.
  - `13-ask-false` — §16.2 `ASK` returning `false`.
  - `14-filter-regex` — §17.4.3.14 `REGEX(?v, "^A")`.
  - `15-filter-in` — §17.4.1.9 `FILTER(?v IN (...))`.
  - `16-strlen` — §17.4.3.3 `STRLEN(?v)`.
  - `17-lang-tag` — §17.4.2.4 `LANG(?v)` over language-tagged literals.
  - `18-ucase` — §17.4.3.8 `UCASE(?v)`.
- `tests/w3c-sparql/run.sh` is a bash runner: for each test it
  drops + recreates the extension, loads `data.ttl`, runs
  `query.rq` via `pgrdf.sparql`, sorts both sides
  lexicographically (bag-equivalent comparison; SPARQL solutions
  are unordered absent ORDER BY), and `diff -u`s against
  `expected.jsonl`. `ACCEPT=1` regenerates expected; every
  baseline must be hand-verified against the W3C spec.
- Each test ships a `description.md` quoting the spec section
  exercised + the hand-computed expected JSONL — load-bearing for
  reviewers and for the "never ACCEPT=1 blind" rule from v0.3 §6.2.
- Wired into CI's `regression` job (right after the pg_regress
  suite, using the same compose Postgres). The W3C harness is
  gated PR-on / push-on like the rest of the regression suite.
- `regression-w3c.yml` nightly workflow stays gated `if: false`
  — it's the destination shape for the **full W3C TTL-manifest
  runner** (`pgrdf-w3c-sparql` Rust binary parsing
  `w3c/rdf-tests/sparql/sparql11/manifest.ttl` against the
  ratcheting coverage targets `≥ 30 % → ≥ 70 % → ≥ 95 %`).
  v0.4 work item; not blocking the v0.3 release.

### Phase 6 step 1 — regression suite in CI

- `.github/workflows/ci.yml`: new `regression` job runs the
  compose-based pg_regress suite on every PR + push to main. The
  job:
  - Builds `pgrdf.so` via `compose/builder.Containerfile`
    (BuildKit, same path as the local dev loop).
  - Boots `postgres:17.4-bookworm` via `docker compose up -d` with
    the artifacts bind-mounted at the canonical paths.
  - Waits on the compose healthcheck, then drives
    `tests/regression/sql/NN-*.sql` via
    `PGRDF_RUNTIME=docker bash tests/regression/run.sh`.
  - Captures `docker logs pgrdf-postgres` on failure for triage.
  - Tears the stack down with `compose down -v` on `always()`.
- Pinned to PG 17 today (compose pin per ERRATA E-006). Widens to
  the full matrix when the PG-18 / pgrx issue clears.
- `tests/regression/run.sh` already honoured `PGRDF_RUNTIME` so no
  runner changes needed.

**Deferred (still placeholders):**
- W3C SPARQL 1.1 + SHACL conformance runners live in
  `.github/workflows/regression-w3c.yml` gated `if: false`. Need a
  Rust runner binary that reads the manifest TTL, materialises each
  test's data graph, runs the query, and diffs against the expected
  result. v0.4 work item.
- LUBM-10 / LUBM-100 perf comparison vs Jena TDB and Apache AGE.
  Needs `tests/perf/run-lubm.sh` + a normalised reporting layer.
  v0.4 work item.
- Release workflow (`release.yml`) is wired but only fires on
  `v*` tags. Tag the first release once Phase 6 step 2 (the
  conformance runners) lands.

### Phase 5 — SHACL `pgrdf.validate` ships as a STUB

- `src/validation/shacl.rs`: `pgrdf.validate(data_graph_id,
  shapes_graph_id) → JSONB` is wired with a stable response shape
  but a `{"status": "stub", "reason": "...", …}` body. The UDF
  echoes both graph IDs and reports the actual triple count in
  each — enough for downstream tooling (CloudNativePG operators,
  client libraries, CI jobs) to integrate the SQL surface today.
- 2 new pgrx tests (`validate_stub_shape`,
  `validate_stub_unknown_graphs`) lock the JSONB schema.
- New regression `70-validate-stub.sql` asserts: status = "stub",
  `data_graph_id` / `shapes_graph_id` echoed, triple counts
  matched, `conforms` is `null`, `results` is an empty array,
  `reason` field present. Hand-computed; never ACCEPT=1 baselined.
- Test bar: **91 → 93 pgrx + 29 → 30 regression**, green.

**Why a stub, not a real impl.** New ERRATA entry
[`E-009`](specs/ERRATA.v0.2.md). Briefly:
- `shacl_validation 0.2.x` (latest 0.2.12) ships an unfinished
  `iri_s` → `rudof_iri` migration; `shacl_ast 0.2.9` fails to
  compile against the resolved tree
  (`expected rudof_iri::IriS, found iri_s::IriS`).
- `shacl_validation 0.1.149` compiles in isolation but its
  transitives turn on `oxrdf`'s `rdf-12` feature, which adds
  `TermRef::Triple(_)` — a variant `reasonable 0.4.1`'s pattern
  match doesn't handle. Cargo feature unification means we can't
  have both crates in one workspace until either upstream catches
  up.
- We chose to ship Phase 4 (inference) first because it's
  load-bearing; Phase 5's real implementation is a v0.4 follow-up
  the moment upstream unblocks. The stub keeps the surface
  available so nothing downstream gets blocked on a missing UDF.
- `Cargo.toml` carries the `shacl_validation = "0.2"` line
  commented out with the full reason inline.

### Phase 4 — OWL 2 RL materialization via `reasonable`

- `Cargo.toml`: `reasonable = "0.4"` (0.4.1, 2026-05-10 publish).
  Pulls in `datafrog 2`, `disjoint-sets 0.4`, `roaring 0.5`,
  `rio_api / rio_turtle 0.7`, `farmhash 1`, `serde_sexpr 0.1`. The
  oxrdf version requirement (`^0.3.3`) matches our existing pin so
  triple types unify cleanly across the codebase.
- `src/inference/reasonable.rs`: full implementation of
  `pgrdf.materialize(graph_id BIGINT) → JSONB`. Flow:
  1. Idempotency — wipe every `is_inferred = TRUE` row in this
     graph via a single `DELETE … RETURNING 1` + count aggregate.
  2. Bulk-rehydrate base triples — one `SELECT … JOIN
     _pgrdf_dictionary × 3 + LEFT JOIN dt` round-trip builds
     `Vec<oxrdf::Triple>` directly. Datatype + language tag both
     carried; blank-node subjects + object IRIs / literals all
     supported.
  3. `Reasoner::new().load_triples(base).reason()` — OWL 2 RL
     forward chain.
  4. Set-diff against the base `HashSet<Triple>` to isolate
     entailed-but-not-asserted triples (filters out the base AND
     the OWL 2 RL axiomatic triples that match the input).
  5. Each new triple's terms intern via `put_term_full` (shmem-
     warm path from Phase 3 step 1) and INSERT with
     `is_inferred = TRUE`.
- Stats JSONB:
  `base_triples / inferred_triples_written /
  previous_inferred_dropped / reasoner_errors[] / elapsed_ms`.
- 3 new pgrx tests:
  `materialize_subclass_chain` (verifies
  `?a a :Engineer ⇒ ?a a :Person`),
  `materialize_is_idempotent` (two calls produce the same row count
  and drop the prior output),
  `materialize_pure_data_preserves_input` (base survives).
- New regression `60-materialize-owl-rl.sql` covers:
  - 2-hop subClassOf chain
    (`Engineer ⊑ Person ⊑ Agent` plus assertions →
     `alice a Person`, `alice a Agent`, `bob a Agent`).
  - Idempotence — `previous_inferred_dropped` equals the prior
    `inferred_triples_written`.
  - `owl:inverseOf` entailment
    (`:owner :owns :store` ⇒ `:store :ownedBy :owner`).
- Test bar: **88 → 91 pgrx + 28 → 29 regression**, green.

Scope honest. `reasonable` implements OWL 2 RL only. OWL 2 EL/QL
and arbitrary Datalog beyond RL are NOT covered. Pre-existing
ERRATA E-002 (LLD §2 → "reasonable Datalog reasoner") remains
correct; v0.3 LLD §5.2 already restricts the slice to RL.

### Phase 3 step 3 — bulk-ingest prepared INSERT (LLD §4.3 phase A)

- `src/storage/loader.rs`: the batch-flush SQL is a constant
  string; the per-backend `plan_cache` from Phase 3 step 2 stashes
  the prepared `INSERT … SELECT FROM unnest(…)` exactly once.
  Every flush across every load in the same backend reuses the
  cached `OwnedPreparedStatement`. Saves one parse+plan per batch
  (typically ~100–500 µs each on PG 17).
- `flush_batch` now runs inside `Spi::connect_mut(|c| {…})` and
  binds arguments as `Vec<DatumWithOid>` (three `INT8ARRAY` + one
  `INT8`), driving the cached plan via `client.update`.
- `tests/regression/sql/52-bulk-ingest-perf.sql` + new
  `fixtures/regression/synth-10k.{sh,ttl}` fixture (10 000
  triples = ≥ 10 flushes per load). Asserts:
  - Load 1 produces exactly one `plan_cache_misses` += 1 and one
    `plan_cache_inserts` += 1 (the cold prepare).
  - Two loads together produce ≥ 19 `plan_cache_hits` (the other
    flushes all hit).
  - Load 3 produces zero new inserts (cache fully warm).
  Hand-computed; never `ACCEPT=1` baselined.
- Test bar: **88 → 88 pgrx + 27 → 28 regression**, green.

**Honest framing — wall-clock target.** LLD §4.3 calls for *"ingest
throughput at least 2× the current batched-INSERT baseline"*. The
prepared-INSERT cache saves a few hundred µs per batch but the
batched-INSERT executor walk (`SELECT … FROM unnest(…)` per-tuple
construction + partition routing) still dominates per-batch wall
clock. Observed: synth-100 unchanged within noise; synth-10k
~85 ms steady-state on both before/after.

To hit the 2× bar the next slice has to bypass the executor —
either `pg_sys::heap_multi_insert` directly (skips per-tuple
projection and the partition tuple-router uses heap-bulk paths) or
the proper `BeginCopyFrom` + binary COPY-protocol feed. Both are
FFI-heavy. Tracked as **Phase 3 step 3b (deferred)** — does NOT
block Phase 4 (Inference) start.

### Phase 3 step 2 — prepared-plan cache (LLD §4.2)

- `src/query/plan_cache.rs`: per-backend `thread_local!`
  `HashMap<String, OwnedPreparedStatement>`. Cumulative
  `plan_cache_hits / misses / inserts` counters live in shmem
  (alongside the dict-cache counters) so a multi-backend view is
  available through `pgrdf.stats()`. Per-backend cache size is
  surfaced as `plan_cache_local_size`.
- `src/query/executor.rs`: every dict-id constant that used to be
  inlined into the dynamic SQL (`bind_subject/predicate/object`,
  `expr_to_id_sql`, `translate_in`, `numeric_datatype_id_list`,
  …) now becomes a `$N` positional placeholder. A thread-local
  `PARAM_BUF` collects the resolved i64s in declaration order;
  `translate()` snapshots it into `ExecPlan { sql, params }`.
  The SQL string itself is the canonical cache key — identical
  algebra shape → identical key by construction.
- `execute()` consults the per-backend cache before paying for
  parse + plan. Miss path uses `client.prepare(sql, &[INT8OID; n])`
  followed by `.keep()` to promote to `'static`-lifetime
  `OwnedPreparedStatement`. Hit path reuses the stashed statement
  with a fresh `Vec<DatumWithOid>` built from `plan.params`.
- `pgrdf.plan_cache_clear() -> bigint` returns the number of
  plans dropped from THIS backend's cache. Useful for diagnostics
  and tear-down; production workloads never need it.
- `pgrdf.stats()` JSONB now includes four new fields:
  `plan_cache_hits`, `plan_cache_misses`, `plan_cache_inserts`,
  `plan_cache_local_size`.
- **Perf regression**: new `tests/regression/sql/51-plan-cache.sql`
  exercises three blocks:
  - 5 identical queries → 1 miss + 4 hits (single shape, repeat).
  - 2 queries with same parametric shape but different IRI
    constants → 1 miss + 1 hit (parameterisation works — SQL
    string stays byte-identical despite constant change).
  - 1 structurally distinct query (FILTER added) → 1 miss + 0 hits.
  Also asserts `plan_cache_clear() >= 2` and
  `plan_cache_local_size == 0` post-clear. **All deltas
  hand-computed**.
- 2 new pgrx integration tests: `plan_cache_repeats_hit`,
  `plan_cache_clear_returns_count`.
- Test bar: **86 → 88 pgrx + 26 → 27 regression** tests, green.

### Phase 3 step 1 — shmem dict cache (LLD §4.1)

- `src/storage/shmem_cache.rs`: process-wide, cross-backend
  dictionary cache backed by `pgrx::PgLwLock<[Slot; 16_384]>` (~512
  KiB shmem). Slot carries a u128 fingerprint (two SipHash variants)
  plus dict_id + generation. Open-addressed with 8-deep linear
  probing; canonical-slot eviction on full streak.
- `_PG_init` gates `pg_shmem_init!` on
  `pg_sys::process_shared_preload_libraries_in_progress` so hook
  registration only happens in the postmaster scan. Lazy-loaded
  backends short-circuit every lookup and fall back to the per-call
  HashMap path. Compose already sets
  `shared_preload_libraries=pgrdf`; the pgrx-test harness's
  `postgresql_conf_options` now does too.
- `put_term_full` consults shmem before SELECT. On both SELECT-hit
  and INSERT it **stages** the (key → dict_id) mapping in a
  per-backend pending list; pgrx's `register_xact_callback` flushes
  to shmem on `XACT_EVENT_COMMIT` and discards on
  `XACT_EVENT_ABORT`. The deferred publish keeps shmem in lockstep
  with the dictionary table — a rolled-back INSERT never leaves an
  orphan id in the cache.
- `pgrdf.stats() -> JSONB` exposes cumulative shmem counters
  (`shmem_ready`, `shmem_slots`, `shmem_hits`, `shmem_misses`,
  `shmem_inserts`, `shmem_evictions`) — observability target for
  the LLD §4.1 acceptance criterion.
- `pgrdf.shmem_reset() -> void` atomically bumps a shmem
  generation counter so every previously-cached entry reads as
  cold on next lookup. Required after
  `DROP EXTENSION pgrdf; CREATE EXTENSION pgrdf;` (the dict id
  space resets but the cache survives) — also useful in regression
  setup. Slot generation is part of the slot record; mismatch on
  lookup is silent and equivalent to a miss.
- Per-call `load_turtle_verbose` stats gain `shmem_cache_hits` —
  the count of term references that fell through the per-call
  HashMap and were satisfied by the cross-backend shmem cache
  without touching `_pgrdf_dictionary`. Loader snapshots the
  global HITS counter around each `put_term_full` call to
  attribute hits.
- **Perf regression**: new `tests/regression/sql/50-shmem-dict-cache.sql`
  loads `fixtures/regression/synth-100.ttl` (100 triples, 115
  distinct terms) three times into successive graphs and asserts:
  load 1 has 115 db calls + 0 shmem hits; loads 2–3 have 0 db
  calls + 115 shmem hits each. Cumulative counter deltas asserted
  via `pgrdf.stats()` ≥ 230 shmem hits / ≥ 115 inserts vs pre-test
  snapshot. **All expected values hand-computed**, never
  autobaselined.
- 6 new pgrx integration tests cover the cache primitive
  (`shmem_ready_in_test`, `shmem_roundtrip_via_committed`,
  `shmem_disambiguates_keys`, `shmem_datatype_in_key`,
  `shmem_counters_advance`, `shmem_reset_invalidates_slots`).
- Test bar: **85 → 86 pgrx + 25 → 26 regression** tests, all green.

### LLD v0.3 — Refocus

- [`specs/SPEC.pgRDF.LLD.v0.3.md`](specs/SPEC.pgRDF.LLD.v0.3.md)
  shipped. Supersedes v0.2 at the contract level; v0.2 LLD is now
  historical (still referenced for §4.1–4.3 internals that haven't
  changed). INSTALL spec (`SPEC.pgRDF.INSTALL.v0.2.md`) unchanged.
- The v0.3 LLD acknowledges Phase 3 steps 1–12 (SPARQL surface)
  as substantively complete (BGP + FILTER + OPTIONAL + UNION +
  MINUS + DISTINCT/LIMIT/OFFSET/ORDER BY + aggregates + HAVING +
  GROUP_CONCAT/SAMPLE + expression richness + BIND + multi-triple
  MINUS + ASK) and re-bins forward work:
  - **Phase 3 (NEW): Storage Performance** — shmem dict cache
    (v0.2 LLD §4.1), prepared-plan cache (§4.2), COPY BINARY
    ingestion (§4.3). The single biggest remaining LLD gap.
  - **Phase 4**: Inference engine (OWL 2 RL via `reasonable`).
  - **Phase 5**: Validation engine (SHACL via `shacl_validation`).
  - **Phase 6**: W3C SPARQL/SHACL conformance + LUBM + release
    artifacts + CI matrix.
- v0.4 deferral list (none block Phase 3):
  - GRAPH `{ … }` named-graph clause (needs storage schema work)
  - VALUES inline tables
  - Property paths beyond simple sequence (`*`, `+`, `?`, `^`)
  - Multi-triple OPTIONAL (needs LATERAL refactor)
  - CONSTRUCT, DESCRIBE (different output shape)
  - Aggregates over UNION
  - BIND output referenced in later FILTER / BGP
  - Type-aware ORDER BY / MIN / MAX
- v0.3 also formalises the **empirical-verification rule**: new
  regression fixtures hand-compute their expected output; no
  `ACCEPT=1` autobaselining of new query coverage.
- Per-call `pgrdf.load_turtle_verbose` stats will gain
  `shmem_cache_hits` + `plan_cache_hits` in Phase 3 to support
  perf regression tests on the synth-100 fixture.
- Cross-references updated: `README.md`, `docs/README.md`,
  `docs/10-roadmap.md`, `specs/ERRATA.v0.2.md`.

### Phase 3 step 12 — ASK query form

- `pgrdf.sparql('ASK { … }')` now works. Returns a single JSONB
  row `{"_ask": "true"}` or `{"_ask": "false"}` reflecting whether
  the pattern has at least one solution.
- The pattern walk reuses `parse_select` so ASK transparently
  supports FILTER, OPTIONAL, UNION, MINUS, and any combination
  the SELECT executor handles. `build_ask_probe_sql` emits a
  `SELECT 1 FROM …` probe wrapped in `EXISTS(…)` in the outer
  query.
- `pgrdf.sparql_parse` now reports `form: "ASK"` with the same
  `bgp_pattern_count` / `bgp_patterns` / `unsupported_algebra`
  shape it gives SELECT, rather than `supported: false`.
- 2 new pg_tests: ASK match/no-match, ASK with FILTER.
- `tests/regression/sql/44-sparql-ask.sql` covers 6 query shapes:
  match, no-match, FILTER pass/fail, ASK with OPTIONAL, ASK with
  UNION.
- `README.md` pills: 77+24 → 79+25.
- `CONSTRUCT` and `DESCRIBE` change the output shape (triples
  instead of solutions) and are **deferred to v0.4**.

Test bar:
  pg_test:    79 passed; 0 failed  (was 77)
  regression: 25 passed; 0 failed  (was 24)

### Phase 3 step 11 — Multi-triple MINUS

- `MINUS { ?s :p ?o . ?s :q ?r . … }` now accepts arbitrary
  N-triple sub-patterns. `ParsedSelect.minuses` changed from
  `Vec<TriplePattern>` to `Vec<Vec<TriplePattern>>`; same for
  `UnionBranch.minuses`.
- `translate_minus` rewrites to emit one `NOT EXISTS (SELECT 1
  FROM q_min_1, q_min_2, … WHERE …)` per MINUS block. Each
  triple in the sub-pattern gets its own quad alias; shared
  variables with the outer query AND shared-inside-the-MINUS
  emit equality predicates automatically via `pattern_clauses`.
- SPARQL spec's "no shared variables → MINUS is identity" rule
  still applies: the translator unions all variables in the
  sub-pattern, checks intersection with outer anchors, and
  elides the block if empty.
- Single-triple MINUS continues to work (it's the
  `triples.len() == 1` case of the multi-triple path).
- 1 new pg_test: `sparql_minus_multi_triple` (alice+eve have
  both mbox+age → dropped, bob/carol/dave survive).
- `tests/regression/sql/43-sparql-minus-multi.sql` covers 4
  query shapes: 2-triple AND, 3-triple AND, chained multi-triple
  MINUSes, single-triple back-compat.
- `README.md` pills: 76+23 → 77+24.
- Multi-triple OPTIONAL is **deferred to v0.4** — the LATERAL
  refactor it needs is bigger than the MINUS rewrite (OPTIONAL
  has to EXPOSE its new bindings to the outer query, while MINUS
  is just a boolean check). Workaround: chain single-triple
  OPTIONALs.

Test bar:
  pg_test:    77 passed; 0 failed  (was 76)
  regression: 24 passed; 0 failed  (was 23)

### Phase 3 step 10 — BIND (non-aggregate)

- `BIND(expr AS ?v)` (and the equivalent `SELECT (expr AS ?v)` form
  on non-aggregate expressions) now adds a virtual column. `walk_select`'s
  Extend handler falls through to a `BindSpec` when the expression
  isn't a Variable-rename of an existing aggregate.
- Projection in `build_single_branch_outer` checks `ps.binds` before
  falling back to the BGP anchor lookup, emitting the translated
  expression with the BIND var as the column alias.
- `translate_bind_expression` covers Literal / NamedNode / Variable,
  STR / LANG / DATATYPE / UCASE / LCASE, arithmetic, STRLEN, and
  `CONCAT(?a, ?b, …)` via Postgres `concat`. All values surface as
  text in the JSONB row.
- Today's restriction: a BIND output variable referenced in a later
  FILTER / BGP isn't yet supported (would need expression substitution
  during translation). Filtering on BIND output is Phase 3 backlog.
- 3 new pg_tests + `tests/regression/sql/42-sparql-bind.sql`
  (6 query shapes: UCASE, arithmetic, CONCAT, literal-constant,
  STRLEN, two-BINDs in one query).
- `README.md` pills: 73+22 → 76+23.

Test bar:
  pg_test:    76 passed; 0 failed  (was 73)
  regression: 23 passed; 0 failed  (was 22)

### Phase 3 step 9 — Expression richness in FILTER

- `pgrdf.sparql` FILTER translator gains a much wider expression
  surface:
  - **Arithmetic**: `?a + ?b`, `?a - ?b`, `?a * ?b`, `?a / ?b`
    (with NULLIF-guarded divide-by-zero), unary `-`, unary `+`.
    All built on top of `expr_to_numeric_sql`'s CASE-cast so
    non-numeric operands NULL-propagate instead of erroring.
  - **String predicates**: `CONTAINS`, `STRSTARTS`, `STRENDS` —
    Postgres `strpos`, `left`, `right` against `lexical_value`.
  - **String-valued functions** usable inside other expressions:
    `LANG(?v)`, `DATATYPE(?v)`, `UCASE(?v)`, `LCASE(?v)`,
    `STR(?v)` (was passthrough, formalised). LANG / DATATYPE use
    chained dict lookups (datatype IRI ids → IRI lexical).
  - **`STRLEN(?v)`** is numeric-valued, plugged into
    `expr_to_numeric_sql`.
- Equality fallback: when either side of `=` / `sameTerm` is a
  function call (or otherwise can't resolve to a dict id), the
  translator falls back to lexical comparison. Lets `STR(?v) =
  "x"`, `LANG(?v) = "en"`, `DATATYPE(?v) = xsd:integer` etc.
  translate cleanly.
- `expr_to_lexical_sql` learned to emit a SQL string for
  `NamedNode` (the IRI's lexical form), making the fallback work
  for IRI constants on the right of equality.
- 6 new pg_tests: arithmetic add, mul/div, STRLEN, CONTAINS/
  STRSTARTS/STRENDS, LANG/DATATYPE equality, UCASE/LCASE case
  folding.
- `tests/regression/sql/41-sparql-expressions.sql` covers 11
  query shapes (4 arithmetic, STRLEN, 4 string predicates,
  LANG, DATATYPE).
- `README.md` pills: 67+21 → 73+22.

Test bar:
  pg_test:    73 passed; 0 failed  (was 67)
  regression: 22 passed; 0 failed  (was 21)

### Phase 3 step 8 — HAVING + GROUP_CONCAT + SAMPLE

- `pgrdf.sparql` now translates `HAVING (expr)` clauses on
  aggregate queries. `parse_select` post-processes the collected
  filters: any filter referencing an aggregate output variable
  becomes a HAVING predicate (the rest stay as WHERE).
- `translate_filter_with_aggregates` is the HAVING-aware translator:
  variable references resolve to (a) the underlying SQL aggregate
  function for aggregate-output vars, (b) the group-by expression
  for group vars, (c) literals are used directly. Supports
  identity, numeric ordering (`<`/`>`/`<=`/`>=`), boolean composition.
- `GROUP_CONCAT(?v [; SEPARATOR = "…"])` → Postgres `STRING_AGG`,
  default separator a single space per SPARQL spec.
- `SAMPLE(?v)` → `MIN(lexical_value)` as a deterministic surrogate
  (SPARQL spec says "implementation-defined element"; MIN is one
  conformant choice).
- 4 new pg_tests: HAVING with COUNT, HAVING with SUM, GROUP_CONCAT
  with custom separator, SAMPLE.
- `tests/regression/sql/40-sparql-having.sql` covers 9 query
  shapes (HAVING > N, HAVING = 1, HAVING composite, GROUP_CONCAT
  custom + default separator, SAMPLE, SUM-HAVING on non-numeric
  strings — demonstrates the numeric-awareness rule — and
  SUM-HAVING on real numeric data across two graphs).
- `README.md` pills: 63+20 → 67+21.

Test bar:
  pg_test:    67 passed; 0 failed  (was 63)
  regression: 21 passed; 0 failed  (was 20)

### Phase 3 step 7 — Aggregates + GROUP BY

- `pgrdf.sparql` handles SPARQL aggregates with or without
  `GROUP BY`:
  - `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT ?v)`.
  - `SUM(?v)`, `AVG(?v)` — numeric-aware via the same XSD-numeric
    CASE cast as FILTER ordering. Non-numeric values contribute
    `NULL` (skipped by SUM/AVG per SQL semantics, no Postgres
    cast error).
  - `MIN(?v)`, `MAX(?v)` — lexicographic on the term's
    `lexical_value`. Type-aware MIN/MAX queued.
- `GROUP BY ?vars` translates to SQL `GROUP BY` using the same
  dict-lookup expressions that drive the SELECT clause. Multiple
  aggregates per group supported.
- Aggregate output values come back as **JSON strings** in the
  `pgrdf.sparql` row, consistent with the rest of the surface.
  Callers cast with `(j ->> 'n')::int`/`::numeric` etc.
- Algebra layout: spargebra lowers `SELECT (EXPR AS ?v)` to
  `Project → Extend → Group → BGP`. `walk_select` now handles
  Extend (renames the synthesised `$agg_N` to `?v`) and Group
  (captures group_vars + AggregateSpecs). Walk order: descend
  into inner first so Group's aggregates are populated before
  Extend tries to rename them.
- Parser walks `GraphPattern::Group` and `GraphPattern::Extend`
  rather than flagging them; tests adjusted.
- 7 new pg_tests: COUNT(*), COUNT(DISTINCT), GROUP BY counting,
  SUM numeric, AVG numeric, MIN/MAX lex, multiple aggregates
  per group.
- `tests/regression/sql/39-sparql-aggregates.sql` covers 10
  query shapes: count_all, count_o, count_distinct, sum_age,
  avg_age (rounded), min/max names, group_by predicates,
  multi-aggregate, ORDER-BY-aggregate + LIMIT.
- `README.md` pills: 56+19 → 63+20; SPARQL pill adds AGGREGATES.
- `guide/03-querying.md` gains a full "Aggregates and GROUP BY"
  section covering the JSON-string output rule, the SUM/AVG
  numeric-awareness rule, the MIN/MAX lex caveat, and the
  HAVING/GROUP_CONCAT/BIND restrictions.

Today's restrictions:
- HAVING not yet translated — post-process with regular SQL.
- BIND outside aggregate aliasing not supported.
- Aggregates on top of UNION not supported (panic with clear msg).
- `GROUP_CONCAT` / `SAMPLE` not supported.

Test bar:
  pg_test:    63 passed; 0 failed  (was 56)
  regression: 20 passed; 0 failed  (was 19)

### Phase 3 step 6 — MINUS

- `pgrdf.sparql` handles `MINUS { ?s :p ?o }` and chained MINUSes.
  Each block becomes a `WHERE NOT EXISTS (SELECT 1 FROM
  pgrdf._pgrdf_quads qMIN_K WHERE …)` sub-SELECT, keyed on shared
  variables between the outer query and the MINUS triple.
- Per SPARQL spec, MINUS with no shared variables is a no-op —
  the translator detects this at translation time and emits no
  SQL for that block (different from OPTIONAL, which always
  emits a LEFT JOIN).
- Restriction: each MINUS block must be a single triple pattern
  (mirrors OPTIONAL's current restriction).
- Inside UNION branches, MINUS works the same way (scoped to the
  branch's anchor map).
- 4 new pg_tests: basic MINUS, no-shared-vars no-op, chained
  MINUSes, MINUS + outer FILTER + REGEX.
- Parser walks `GraphPattern::Minus` rather than flagging it.
  New parser pg_test for the new state + a Path-still-flagged
  test taking its place (transitive `:a*`, not simple `:a/:b`
  which spargebra desugars to BGP).
- `tests/regression/sql/38-sparql-minus.sql` covers 6 query
  shapes (basic, no-op, chained, with-FILTER, ordered survivor,
  shared-non-subject-var).
- `30-sparql-parse.sql` baseline updated: MINUS supported, Path
  (quantified) is the new unsupported representative.
- `README.md` pills: 51+18 → 56+19, SPARQL pill adds MINUS.
- `guide/03-querying.md` gains a MINUS section covering the
  shared-vars-vs-no-op rule and the OPTIONAL-asymmetry note.

Test bar:
  pg_test:    56 passed; 0 failed  (was 51)
  regression: 19 passed; 0 failed  (was 18)

### Phase 3 step 5 — UNION

- `pgrdf.sparql` handles `{ A } UNION { B }` and chained
  `A UNION B UNION C`. Each branch is its own complete sub-SELECT
  (own BGP / FILTERs / OPTIONALs / per-branch dict-id anchors).
  Branches are combined with SQL `UNION ALL`; the outer SELECT
  layers `DISTINCT` / `ORDER BY` / `LIMIT` / `OFFSET`.
- Variables bound in only some branches come back as `null` from
  the other branches (each branch SELECTs `NULL::TEXT` for vars
  it doesn't bind, so row shapes line up across `UNION ALL`).
- ORDER BY on UNION may only reference projected variables — the
  outer SELECT can't see branch-local alias columns. Executor
  panics with a clear message otherwise.
- Refactor: extracted `build_from_and_where` (shared by both the
  single-branch and per-UNION-branch paths) + `build_branch_sql`
  + `build_union_sql`. The original `build_bgp_sql` is now a
  dispatcher over `ps.union_branches.is_empty()`.
- 5 new pg_tests: basic UNION over same var, different-var
  UNION with NULL pad, three-way chain, UNION + DISTINCT,
  UNION + ORDER BY + LIMIT.
- Parser walks `GraphPattern::Union` rather than flagging it.
  New parser pg_test for the new state + a new MINUS-still-flagged
  test taking its place.
- `tests/regression/sql/37-sparql-union.sql` covers 9 query shapes
  (basic, DISTINCT, different-var, two NULL-discriminator checks,
  three-way chain, ORDER BY first, LIMIT, branch-local FILTER).
- `30-sparql-parse.sql` baseline refreshed: UNION supported,
  MINUS now the unsupported representative.
- `README.md` pills: 45+17 → 51+18, SPARQL pill adds UNION.
- `guide/03-querying.md` gains a full UNION section covering
  the cross-branch null padding, ORDER-BY-must-be-projected
  rule, and the no-nesting restriction for this slice.

Test bar:
  pg_test:    51 passed; 0 failed  (was 45)
  regression: 18 passed; 0 failed  (was 17)

### Phase 3 step 4 — OPTIONAL (LeftJoin) translation

- `pgrdf.sparql` now handles `OPTIONAL { ?s :p ?o }`. Each OPTIONAL
  block emits a `LEFT JOIN pgrdf._pgrdf_quads qOPT_i ON (…)`. Variables
  introduced inside an OPTIONAL surface as NULL (JSONB `null`) when
  the LEFT JOIN didn't match.
- `OPTIONAL { … FILTER(...) }` — the inner filter lands in the LEFT
  JOIN's ON clause, so rejected matches keep the optional variable
  NULL rather than pruning the whole row.
- Multiple chained OPTIONALs each get their own LEFT JOIN, in
  left-to-right order. Per SPARQL semantics, variables introduced
  by one OPTIONAL aren't visible to another OPTIONAL's ON clause.
- `BOUND(?v)` translation tightened: now emits `qN.col IS NOT NULL`
  regardless of whether ?v is mandatory or OPTIONAL. Mandatory
  anchors are non-NULL so it's trivially TRUE there; OPTIONAL anchors
  can be NULL so this is the spec-correct semantics.
- Internal refactor: `build_bgp_sql` switched from comma-style FROM
  (`q1, q2, q3 WHERE …`) to explicit JOIN syntax
  (`q1 INNER JOIN q2 ON … INNER JOIN q3 ON …`). Same semantics for
  INNER joins; necessary for OPTIONAL's LEFT JOIN to compose.
- Parser updated: `LeftJoin` no longer flagged in
  `unsupported_algebra` — the parser walks both arms.
- 4 new pg_tests: simple OPTIONAL, OPTIONAL with inner FILTER,
  multiple chained OPTIONALs, outer FILTER(BOUND) pruning.
- `tests/regression/sql/36-sparql-optional.sql` covers 8 query
  shapes (LEFT JOIN counts, NULL/not-NULL discrimination, inner
  filter, multi-chain, outer BOUND prune, OPTIONAL + ORDER BY).
- `30-sparql-parse.sql` baseline updated: OPTIONAL no longer
  flagged; new UNION assertion replaces it.
- `README.md` pills: 40+16 → 45+17; SPARQL pill adds OPTIONAL.
- `guide/03-querying.md` gains a full OPTIONAL section covering
  inner-FILTER semantics, chained OPTIONALs, BOUND-pruning, and
  the single-triple restriction for this slice.

Test bar:
  pg_test:    45 passed; 0 failed  (was 40)
  regression: 17 passed; 0 failed  (was 16)

### Phase 3 step 3 — Solution modifiers (DISTINCT / LIMIT / OFFSET / ORDER BY)

- The four classic SPARQL solution modifiers now land in the
  generated SQL instead of being silently stripped from the AST:
  - `SELECT DISTINCT ?vars` → `SELECT DISTINCT` in SQL.
  - `SELECT REDUCED ?vars` → also `SELECT DISTINCT` (REDUCED is a
    "dups may or may not be removed" hint per spec; over-approxing
    with DISTINCT is conformant).
  - `LIMIT N` / `OFFSET N` → `LIMIT N` / `OFFSET N`.
  - `ORDER BY ?var`, `ORDER BY ASC(?var)`, `ORDER BY DESC(?var)`,
    multi-key — sorted by the term's `lexical_value` with
    `NULLS LAST`. If the var is projected the existing column is
    reused; otherwise an extra hidden column is appended and ORDER
    BY references it by ordinal (so the JSONB output stays clean).
- ORDER BY today is **lexicographic on string form**, not SPARQL's
  full type-aware ordering. Numeric ordering through ORDER BY lands
  in step 4+; for now use FILTER for numeric range + post-SQL
  `ORDER BY (sparql->>'n')::numeric`.
- Refactor: `unwrap_select` → `parse_select` returning a richer
  `ParsedSelect` struct (projected, bgp, filters, distinct,
  order_by, limit, offset). Single recursive walk replaces the
  old two-pass extract_bgp_and_filters / unwrap_select split.
- 6 new pg_tests: distinct dedups, LIMIT caps, OFFSET skips,
  ORDER BY ASC + DESC, DISTINCT + ORDER BY interaction.
- `tests/regression/sql/35-sparql-modifiers.sql` covers 10 query
  shapes (raw count, DISTINCT, REDUCED, LIMIT 2, ORDER ASC first,
  ORDER DESC first, OFFSET 3 LIMIT 2 window, DISTINCT + ORDER,
  ORDER BY on non-projected var, LIMIT 0).
- `README.md` pills: 34+15 → 40+16, SPARQL pill adds
  DISTINCT/ORDER/LIMIT.
- `guide/03-querying.md` gains a full "Solution modifiers" section
  covering ORDER BY's lexicographic-vs-type-aware caveat, the
  DISTINCT-with-non-projected-order-by panic case, and a worked
  example.

Test bar:
  pg_test:    40 passed; 0 failed  (was 34)
  regression: 16 passed; 0 failed  (was 15)

### Phase 3 step 2 — FILTER numeric ordering + REGEX + IN

- `pgrdf.sparql` FILTER translator gains three new shapes:
  - **Numeric ordering** (`<`, `>`, `<=`, `>=`): operand resolves to
    `NUMERIC` via a CASE-guarded subselect on `_pgrdf_dictionary`.
    Only XSD numeric datatypes (integer, decimal, double, float,
    sized + unsigned + constraint subtypes — 16 IRIs total)
    contribute; everything else compares NULL → row dropped. This
    matches SPARQL's "type error → unbound" semantics without ever
    raising a Postgres cast error.
  - **`REGEX(?v, "pat" [, "flags"])`**: Postgres `~` (case-sensitive)
    or `~*` (with `i` flag) against the term's `lexical_value`.
    Pattern + flags are SPARQL literals at translation time;
    single quotes in the pattern are escaped. `STR(?v)` inside
    REGEX is a passthrough.
  - **`?term IN (e1, e2, …)`**: dict-id set membership.
- 6 new pg_tests: numeric `>` / range / non-numeric drop, regex
  case-sensitive / case-insensitive with STR(), and IN.
- `tests/regression/sql/34-sparql-filter-advanced.sql` covers 10
  query shapes (numeric `>`, range, `<` with non-numeric mixed in,
  `>= 0` over a typed-decimal row, regex `^A`, regex `ar` case-i,
  regex+STR wrap, IN over IRIs, IN over a literal, and a cross-BGP
  composition).
- `README.md` pills: tests 28+14 → 34+15, SPARQL pill adds REGEX.
- `guide/03-querying.md` gains full sections for numeric ordering,
  REGEX (with the POSIX-vs-PCRE caveat), and IN. Capability matrix
  refreshed.

Test bar:
  pg_test:    34 passed; 0 failed  (was 28)
  regression: 15 passed; 0 failed  (was 14)

### Phase 3 step 1 — FILTER expressions over BGPs

- `pgrdf.sparql` now walks `GraphPattern::Filter { expr, inner }`
  and translates a useful subset of `Expression` into SQL WHERE
  predicates appended after the BGP joins:
  - **Identity**: `=`, `!=`, `sameTerm` — both operands resolved to
    dictionary ids, compared as BIGINT. Sound because the dictionary
    deduplicates by `(term_type, lexical, datatype, language)`.
  - **Boolean**: `&&`, `||`, `!`.
  - **Term-type predicates**: `isIRI`, `isLiteral`, `isBlank` — emit
    a correlated subselect on `_pgrdf_dictionary.term_type`.
  - **`BOUND`**: trivially `TRUE` for any anchored BGP variable.
  - Untranslatable shapes (numeric `<`/`>`/`<=`/`>=`, `regex`, `str`,
    `lang`, arithmetic, `IN`, `EXISTS`) panic with a clear message
    rather than silently dropping the filter.
- `pgrdf.sparql_parse` no longer flags `Filter` in
  `unsupported_algebra` — it walks into the inner BGP. OPTIONAL,
  UNION, MINUS, Group, Path, Values, Extend (BIND), Service still
  flagged.
- 6 new pg_tests: literal equality, `!=`, `isIRI`, boolean AND
  composition, var-equals-var (self-loop), `BOUND` trivially-true.
- 1 new parser pg_test: OPTIONAL replaces the FILTER-flagged baseline.
- `tests/regression/sql/33-sparql-filter.sql` covers 9 query shapes
  end-to-end (literal eq, neg, isIRI, isLiteral, self-loop,
  boolean AND, negated isIRI, BOUND, unknown-literal-zero-rows).
- `tests/regression/sql/30-sparql-parse.sql` baseline updated: Filter
  no longer reported as unsupported; new OPTIONAL assertion added.
- `guide/03-querying.md` adds a full FILTER section with examples,
  including the `=` ↔ sameTerm-vs-value-equality caveat and how
  filters interact with multi-pattern BGPs.
- `README.md`: status pill → `phase 3 start`, test pill 21+13 → 28+14,
  SPARQL pill `SELECT/BGP` → `SELECT/BGP/FILTER`.

Test bar:
  pg_test:        28 passed; 0 failed  (was 21)
  regression:     14 passed; 0 failed  (was 13)

### Phase 2.2 step 8 — Node.js + Go client guides

- `guide/clients/typescript.md` — `pg` (node-postgres) + `postgres.js`
  + `pg-cursor` streaming + strongly-typed binding helpers. Covers
  `load_turtle`, `parse_turtle`, `load_turtle_verbose`, and the
  full `pgrdf.sparql` JSONB result shape with type narrowing.
- `guide/clients/go.md` — `pgx` v5 + `pgxpool` + sqlc integration
  + bulk-ingest pattern + the constant-time graph-drop idiom.
- `guide/README.md` index lists both new client pages.
- `README.md` clients section now points at all 4 supported clients
  (Python, Rust, TypeScript, Go).

### Phase 2.2 step 7 — User guide for SPARQL surface

- New `guide/03-querying.md`: full walkthrough of `pgrdf.sparql`
  (single + multi-pattern BGPs, constants in any position, JSONB
  output, combining with regular SQL, `pgrdf.sparql_parse` for
  introspection) plus what works / doesn't / why, and a worked
  example of the SQL translation.
- `README.md` promoted the SPARQL surface from "coming soon" to a
  live code example, bumped the test pill from 9+10 to 21+13,
  added a SPARQL pill, refreshed the status row.
- `guide/README.md` index entry for `03-querying.md`.

### Phase 2.2 step 6 — Multi-pattern BGP joins

- `pgrdf.sparql` now handles N-pattern Basic Graph Patterns. Each
  pattern becomes a `_pgrdf_quads qN` clause; shared variables across
  patterns are tracked by first-occurrence anchors and emit equality
  predicates (`q2.subject_id = q1.subject_id`) that fold into INNER
  joins.
- 2 new pg_tests: two-pattern shared-subject BGP (Alice + Carol have
  both `foaf:name` and `foaf:mbox`, Bob doesn't), three-pattern chain
  following `foaf:knows`.
- `tests/regression/sql/32-sparql-multipattern.sql` covers 5 shapes:
  shared-subject BGP, three-pattern chain, self-loop pattern (?s ?p ?s),
  bound-subject multi-pattern, and bound-predicate + bound-literal.

Test bar:
  pg_test:        21 passed; 0 failed  (was 19)
  regression:     13 passed; 0 failed  (was 12)

### Phase 2.2 step 5 — SPARQL execution: BGP → SQL

- `pgrdf.sparql(q TEXT) → SETOF JSONB` — first user-visible SPARQL
  surface. Parses via spargebra, translates a single Basic Graph
  Pattern into a dynamic SQL SELECT over `_pgrdf_quads` joined to
  `_pgrdf_dictionary`, returns one JSONB row per solution keyed by
  the projected variable names.

  ```sql
  SELECT * FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n WHERE { ?s foaf:name ?n }'
  );
  --  → {"s": "http://example.com/alice", "n": "Alice"}
  --  → {"s": "http://example.com/bob",   "n": "Bob"}
  ```

  Scope today (intentionally narrow — multi-pattern joins land in
  step 6):
  - SELECT only.
  - Exactly one BGP triple per query.
  - Constants in any position (subject IRI, predicate IRI, object
    IRI or literal). Unknown constants resolve to `-1` so the query
    correctly returns zero rows rather than erroring.
  - Variables in any position.
  - Distinct / Reduced / Slice / OrderBy wrappers are passed through.
- 4 new pg_tests covering all-three-vars BGP, bound-predicate filter,
  bound-subject filter, and unknown-predicate-returns-empty.
- `tests/regression/sql/31-sparql-bgp.sql` exercises 7 query shapes
  end-to-end through the compose Postgres.

Infrastructure:

- `compose/builder.Containerfile` rewritten with BuildKit cache
  mounts. The builder image dropped from 7.73 GB → 3.35 GB; cargo
  registry + target/ now live in build-scoped cache volumes that
  persist across rebuilds without bloating image layers.
- `Justfile build-ext` now invokes `DOCKER_BUILDKIT=1 docker build`
  so the `# syntax=docker/dockerfile:1.4` directive activates.
- `.dockerignore` excludes `target/`, `.target-linux/`,
  `compose/pg-data/`, `compose/extensions/lib|share`,
  `fixtures/ontologies/`, `.git/`. Build context dropped accordingly.

### Phase 2.2 step 4 — SPARQL parser surface

- `spargebra = "0.4"` (0.4.6 resolved). Pins `oxrdf = "=0.3.3"`, the
  same version oxttl 0.2.3 uses, so no graph split.
- New module `src/query/parser.rs`.
- `pgrdf.sparql_parse(q TEXT) -> JSONB` parses a SPARQL query via
  `spargebra::SparqlParser` and returns the high-level shape:
  - `form` — SELECT / CONSTRUCT / ASK / DESCRIBE
  - `variables` — projected vars (SELECT only)
  - `bgp_pattern_count`, `bgp_patterns` — BGP triples with
    s/p/o each rendered as `{var: …}`, `{iri: …}`, `{bnode: …}`,
    or `{literal: …, datatype/lang: …}`
  - `unsupported_algebra` — flags Filter / Union / OPTIONAL /
    Property paths / Aggregates / VALUES / SERVICE / etc., so
    callers see the AST has shape the translator doesn't yet
    cover.
- 5 new pg_tests covering basic SELECT, predicate-as-IRI BGP,
  two-pattern BGP, FILTER detection, and a syntax-error panic path.
- New regression `tests/regression/sql/30-sparql-parse.sql` asserts
  the JSONB extraction over 6 query forms.

### Phase 2.2 step 3 — Batched ingestion

(landed alongside docs split + README pills.)

- `src/storage/loader.rs`: per-call HashMap dict cache + buffered
  multi-row INSERTs via `unnest($1::bigint[], $2::bigint[], $3::bigint[])`.
  BATCH_SIZE = 1000. Reduces SPI calls from ~7/triple to roughly
  `distinct_terms + ceil(triples/1000)`.
- `pgrdf.load_turtle_verbose(path, graph_id, base_iri)` and the
  matching `pgrdf.parse_turtle_verbose(content, graph_id, base_iri)`
  return JSONB stats: `triples`, `dict_cache_hits`, `dict_db_calls`,
  `quad_batches`, `elapsed_ms`. Used to assert the cache is firing.
- `fixtures/regression/synth-100.sh` + `synth-100.ttl`: deterministic
  100-triple synthetic fixture (10 subjects × 5 predicates × 100
  objects). 115 distinct terms, 185 expected cache hits.
- `tests/regression/sql/25-bulk-ingest.sql` asserts exact stat values
  on the synth-100 fixture and verifies dict dedup across two graphs.
- One new pg_test (`parse_turtle_verbose_cache_fires`) asserts cache
  behavior at the Rust level.
- `serde_json = "1"` added as a direct dependency for the verbose UDFs.

### Phase 2.1 — Turtle ingest

- `pgrdf.load_turtle(path, graph_id, base_iri)` and
  `pgrdf.parse_turtle(content, graph_id, base_iri)` parse Turtle via
  `oxttl 0.2` and stream triples through the dictionary +
  partitioned hexastore. `base_iri` resolves relative IRIs like
  `<#>` (needed for W3C PROV).
- Internal `put_term_full(value, type, datatype_id, lang)` honours
  the full dictionary key with `IS NOT DISTINCT FROM` lookups so
  NULL datatype + language columns participate in dedup.
- Compose: read-only `./fixtures:/fixtures:ro,z` bind mount so the
  postgres process can reach test + ontology fixtures by path.
- 24 W3C / Apache Jena / ConceptKernel / ValueFlows ontologies fetch
  cleanly via `fixtures/ontologies.sh`; `tests/perf/smoke-ontologies.sh`
  loads each one through `pgrdf.load_turtle` and prints triple
  counts. 17,134 triples across the set on the 2026-05-13 fetch.
- Four checked-in regression fixtures (`typed-literals.ttl`,
  `lang-tags.ttl`, `blank-nodes.ttl`, `rdf-list.ttl`) under
  `fixtures/regression/` exercise XSD datatypes, language tags,
  blank-node dedup, and `rdf:List` desugaring. All assertions are
  scoped strictly by graph_id so prior smoke loads don't pollute
  results.
- `workflow.ttl` excluded from the iteration set: source uses
  `<ckp://Name:v0.1>` IRI form (colon in path segment, not RFC 3986
  compliant). To be re-added when the CKP source is fixed.

### Phase 2.0 — Storage CRUD UDFs

- `pgrdf.put_term(value, term_type)`,
  `pgrdf.get_term(id)`,
  `pgrdf.put_quad(s, p, o, g)`,
  `pgrdf.count_quads(g)`,
  `pgrdf.add_graph(g)` — all backed by SPI against the
  `_pgrdf_dictionary` + `_pgrdf_quads` schema declared in
  `sql/schema_v0_2_0.sql`.
- 7 `#[pg_test]` integration tests + 3 regression files.
- Justfile: `just test` runs `cargo pgrx test` inside the linux
  builder container; `just test-regression` runs pg_regress-style
  SQL fixtures against the compose Postgres. Both gate the same
  thing CI will.

### Phase 1 — Scaffold + runtime

- pgrx 0.16 extension scaffolding (PG 14-17 feature matrix,
  `pgrx_embed` bin target for schema generation).
- Compose-based local runtime: stock `postgres:17.4-bookworm` with
  per-file bind mounts at `$libdir` / `$sharedir/extension`. No init
  script, no entrypoint wrapper.
- Linux builder container (`compose/builder.Containerfile`) that
  produces glibc-bookworm artifacts on macOS hosts. Two-VM topology:
  Colima for builds (100 GB), podman for the compose stack (avoids
  filling the user's other container state).
- 10-doc engineering set under `docs/` (architecture, storage, query,
  inference, validation, install, dev, testing, release, roadmap).
- `specs/SPEC.pgRDF.LLD.v0.2.md` + `specs/SPEC.pgRDF.INSTALL.v0.2.md`
  captured verbatim alongside `specs/ERRATA.v0.2.md` cataloguing
  deltas found during implementation.
- CI / release workflow placeholders for the
  {pg14..pg17}×{amd64, arm64} matrix.

### Errata against v0.2 specs

- `shacl-rust` → `shacl_validation` (E-001).
- `reasonable` is OWL 2 RL only, not arbitrary Datalog (E-002).
- PG 18 forward path blocked on pgrx 0.17/0.18 not building on
  current Rust (E-006). Compose targets PG 17 until upstream lands
  a fix.
- See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) for the full set.
