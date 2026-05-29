# pgRDF — Roadmap

> **The living, public-facing roadmap for pgRDF.** Stakeholder-oriented "what and
> why" for current and upcoming releases. Engineering "how and when" lives in
> [`docs/10-roadmap.md`](docs/10-roadmap.md) (phase + slice tracking); contract
> detail lives in `specs/SPEC.pgRDF.LLD.v0.x.md`. This document is updated on
> every release boundary and after every direction change.

Last updated: 2026-05-28 (revision 2 — Track H added; LUBM gates firmed up). Current release: **v0.5.1**. Next milestone: **v0.6**.

---

## 1. Mission

pgRDF is a Rust-native PostgreSQL extension that makes Postgres itself a
first-class RDF / SPARQL / SHACL / OWL 2 RL engine. The thesis:

> The fastest, safest, most operable semantic store is the one that lives
> **inside** a PostgreSQL you already trust — leaning on Postgres for storage,
> indexes, transactions, replication, backup, planner, partitioning, observability,
> and security, rather than shipping a parallel database to operate alongside it.

Four engines compose the extension:

| Engine | Surface | Implementation |
|---|---|---|
| **Storage** | `_pgrdf_dictionary` (interned terms) + `_pgrdf_quads` (LIST-partitioned by `graph_id`) + SPO/POS/OSP hexastore covering indexes | `src/storage/` |
| **SPARQL** | `pgrdf.sparql(q) → SETOF JSONB`, `pgrdf.construct(q)`, `pgrdf.describe(q)`, SPARQL UPDATE, IRI lifecycle UDFs | `src/query/` |
| **Inference** | `pgrdf.materialize(graph_id, profile)` — OWL 2 RL via `reasonable`; native RDFS forward-chain | `src/inference/` |
| **Validation** | `pgrdf.validate(data, shapes, mode)` — SHACL Core via rudof `shacl 0.3.x`; W3C SHACL Core suite 25/25 full-pass | `src/validation/` |

---

## 2. Where we are today — v0.5.1 (shipped)

### Shipped surface

| Track | Status | Cite |
|---|---|---|
| SPARQL 1.1 surface (SELECT/ASK/CONSTRUCT/DESCRIBE, paths, BIND, VALUES, type-aware ORDER BY) | ✅ | LLD v0.4 §11 + LLD v0.5 §8 |
| SPARQL 1.1 UPDATE (INSERT/DELETE DATA, INSERT/DELETE WHERE, lifecycle algebra) | ✅ | LLD v0.4 §4 |
| Named graphs + `GRAPH ?g { … }` scoping | ✅ | LLD v0.4 §3 |
| TriG + N-Quads ingest (`parse_trig`, `parse_nquads`) | ✅ | LLD v0.5 §4 |
| OWL 2 RL materialisation + RDFS profile selector | ✅ | LLD v0.5 §3 |
| SHACL Core validation (W3C Core suite 25/25 full-pass) | ✅ | LLD v0.5 §6 (ERRATA E-013 resolved) |
| Shmem dictionary cache + per-backend prepared-plan cache | ✅ | LLD §4.1, §4.2 |
| W3C-shape regression harness + LUBM-shape correctness fixtures | ✅ | `tests/w3c-sparql/`, `tests/perf/lubm-shape/` |
| Install spec compliance (PGXN-published v0.5.1) | ✅ | `INSTALL.md`, `README.pgxn.md` |

### Tested scale today

| Workload | Triples | Where |
|---|---:|---|
| Ontology smoke (24 real-world ontologies) | 17,134 | `tests/perf/smoke-ontologies.sh` |
| Synthetic bulk-ingest fixture | 10,000 | `tests/regression/sql/52-bulk-ingest-perf.sql` |
| LUBM-shape correctness | 3 query shapes | `tests/perf/lubm-shape/{Q1,Q2,Q3}/` |
| W3C SPARQL conformance | 35 fixtures | `tests/w3c-sparql/` |
| W3C SHACL Core conformance | 25 fixtures (full-pass) | `tests/w3c-shacl/` |

**Performance ceiling today is implicit, not measured.** No LUBM-10 / LUBM-100
baseline lives in the repo; `tests/perf/run-lubm.sh` is a target layout, not a
shipped runner (per `tests/perf/README.md` — Phase 2 LUBM-1 smoke, Phase 3 LUBM-10
baseline, Phase 4 LUBM-100 comparison all listed as phased gates, none yet met).
This is the gap v0.6 closes head-on.

### Known scale-blocking limits (carried into v0.6 as work)

| Limit | Where | Why it blocks scale |
|---|---|---|
| Row-by-row materialize writeback | `src/inference/reasonable.rs:179-191` | One `Spi::run_with_args(INSERT)` per inferred triple. At 10⁷ triples × OWL-RL expansion this is the headline scaling failure (`docs/04-inference.md:136-140`). |
| Bulk ingest is batched-`INSERT`, not `COPY BINARY` | `src/storage/loader.rs` `flush_batch` | Steady-state ~85 ms per batch on `synth-10k` — executor walk dominates (`docs/02-storage.md:418-440`). |
| `drop_graph` takes `ACCESS EXCLUSIVE` on parent | `src/storage/lifecycle.rs` | Multi-tenant scaling concern — unrelated graphs' SELECT/UPDATE traffic blocks briefly (`docs/02-storage.md:528-531`). |
| `move_graph` not metadata-only | same | Scans twice on very large graphs; spec's "metadata-only" claim is aspirational (`docs/02-storage.md:573-585`). |
| No FKs between quads and dictionary | by design (`docs/02-storage.md:702-705`) | Integrity rests on loader discipline. At 10⁷ triples, no DB-side check on orphan ids after partial failures. |
| No partition pruning on `graph_id` outside `GRAPH` blocks | `src/query/executor.rs` | A query that constrains `graph_id` via FILTER rather than `GRAPH { }` doesn't get partition pruning. |
| `WITH RECURSIVE` property paths not planner-optimisable | `src/query/executor.rs` | Materialised-closure no-CTE fallback requires a prior `materialize` call. |
| `executor.rs` is 13.4K LoC of `format!`/`push_str` | `src/query/executor.rs` | Cohesion / testability ceiling; already named in `SPEC.pgRDF.LLD.v0.6-FUTURE.md §3` as required hygiene. |
| Plan-cache `PgAtomic.get()` calls unguarded by `is_ready()` | `src/query/plan_cache.rs:72, 81, 85` | Lazy-load (non-preloaded) backends panic instead of degrading gracefully. Defense-in-depth gap. |

---

## 3. v0.6 mission

> **v0.6 takes pgRDF from thousands to tens of millions of triples by pushing
> every hot path into PostgreSQL's set-based engine, hardening internal contracts,
> and earning the PGXN-grade extension hygiene Postgres operators expect.**

Four themes:

1. **Scale to tens of millions of triples** — measured, not asserted. LUBM-10
   baseline (~1.3M triples) lands as the entry-level gate; **LUBM-100 (~13M
   triples) is the v0.6 target**; LUBM-1000 (~130M) is the stretch goal.
2. **Lean on PostgreSQL** — replace SPI per-row loops with set-based SQL;
   replace hand-built string SQL with prepared statements + planner-visible
   shapes; use `COPY BINARY`, custom aggregates, partition pruning, BRIN/GIN,
   `GENERATED` columns, parallel query.
3. **PGXN-grade extension hygiene** — `pgrdf--<from>--<to>.sql` upgrade
   scripts, `requires =` declarations, `directory =` organisation, GUC
   discipline, `search_path` correctness, optional integrations
   (`pg_partman`, `pg_stat_statements`, `pg_prewarm`).
4. **Internal hardening** — `executor.rs` core-BGP carve, plan_cache defensive
   guards (the gap identified during the SHACL stale-mount investigation),
   end-to-end lexical rehydration regression, artifact-parity testing
   tightened beyond v0.5.1's `just test-artifact-parity`.

### Scale gates

| Tier | Triples | Reference workload | v0.6 role |
|---|---:|---|---|
| **Today (v0.5.1)** | ~17K | smoke-ontologies | ✅ shipped |
| **v0.6 development gate** | ~1.3M | LUBM-10 baseline + smoke regression | 🎯 **dev-gate** — must pass throughout the v0.6 cycle; landed by Track A/B/C as each track ships |
| **v0.6.0 release gate** | ~13M | LUBM-100 + headline benchmark | 🎯 **close-out — v0.6.0 does NOT ship until this passes.** All ingest / materialise / SPARQL / SHACL targets are measured at this tier and committed to `tests/perf/lubm.expected.json`. |
| **Post-v0.6.0 directional** | ~130M | LUBM-1000 (best-effort baseline) | 🎯 **stretch** — published as a ranged benchmark in v0.6.x release notes, not a release gate |

LUBM-10 is the developer-loop gate (run in CI on every PR via the perf cron;
green throughout the cycle). LUBM-100 is the release-cut gate (the
go/no-go decision for tagging v0.6.0). LUBM-1000 is the headline scaling
demonstration but is not allowed to block the v0.6.0 cut.

Acceptance numbers (ingest throughput, materialize closure time, SPARQL
warm-cache p50/p95, memory ceiling, on-disk footprint) are decided as the v0.6
test bed lands (§5 below — provided by the user as part of the v0.6 roadmap).

---

## 4. v0.6 work tracks

Each track names: **goal**, **starting state** (with file:line cite), **target
state**, **acceptance**.

### Track A — Ingest at scale (`COPY BINARY` / `heap_multi_insert`)

- **Goal:** sub-second ingest of 1M triples; LUBM-100 ingest under 5 minutes
  on a single-node Postgres 17 baseline (final numbers set by §5 test bed).
- **Starting state:** `src/storage/loader.rs` uses prepared `INSERT … unnest($1::bigint[], …)`
  with `BATCH_SIZE = 1000`. ~85 ms per 1K-batch steady-state — the executor walk
  per-tuple projection + partition routing dominates (`docs/02-storage.md:418-440`).
- **Target state:** `heap_multi_insert` direct heap path (LLD v0.4 §12 phase B,
  already named in `SPEC.pgRDF.LLD.v0.6-FUTURE.md §6`) or `BeginCopyFrom` /
  `CopyBinary` path that bypasses the per-row executor walk. Graph-IRI
  resolution (`src/storage/loader.rs:385-401` — currently one SPI call per
  unresolved IRI) hoisted to a single set-based prepare-and-look-up before the
  ingest loop.
- **Acceptance:**
  - LUBM-1 (~100K) ingest baseline measured and committed.
  - LUBM-10 (~1.3M) ingest under N seconds (set by test bed).
  - No correctness regression on the full pgrx + pg_regress + W3C bar.
  - `parse_turtle_verbose` JSONB grows a `path: "copy" | "unnest"` field.

### Track B — Inference at scale (batched writeback, delta-driven)

- **Goal:** OWL-RL materialisation of 10M base triples completes in measured
  bounded time without per-triple SPI overhead.
- **Starting state:** `src/inference/reasonable.rs:179-191` — one
  `Spi::run_with_args(INSERT)` per inferred triple. At 10⁷ triples × OWL-RL
  expansion this is the dominant cost (`docs/04-inference.md:136-140`).
- **Target state:**
  - **B.1 Batched writeback** — reuse `flush_batch` /
    `QUAD_INSERT_SQL` prepared plan (the same path the loader already uses).
    Behaviour-neutral; pure throughput win. Already named in
    `docs/10-roadmap.md` Phase 3 step 3b.
  - **B.2 Delta materialisation** — `pgrdf.materialize_delta(graph_id, since_xid TEXT)`
    forward-chains only over quads added since a recorded transaction id.
    Designed in `SPEC.pgRDF.LLD.v0.6-FUTURE.md §8`.
- **Acceptance:**
  - LUBM-10 base graph (~1.3M) → OWL-RL closure under N seconds.
  - `materialize_delta` produces the same closure as full `materialize` for
    triples added after `since`, with measurable speedup on incremental workload.
  - `pgrdf.stats()` exposes `materialize_writeback_path: "batch" | "row"` and
    `materialize_inferred_batches`.

### Track C — Query at scale (executor carve, custom-scan, partition pruning)

- **Goal:** SPARQL SELECT over 10M-triple graph stays within a planner-pruned
  partition set; warm-cache p50 sub-100 ms on simple BGP shapes.
- **Starting state:**
  - `src/query/executor.rs` — 13,403 LoC, single module. `format!` / `push_str`
    SQL assembly across the BGP, OPTIONAL, UNION, MINUS, aggregate, CONSTRUCT
    paths. Hard to extend without regression risk.
  - GRAPH-pattern translation pushes `g{S}.graph_id = $K` onto the JOIN, which
    Postgres does prune. But a query like
    `SELECT … WHERE { ?s ?p ?o FILTER(?g = <…>) }` (no GRAPH block) doesn't.
- **Target state:**
  - **C.1 — `executor.rs` core-BGP module carve** (post-v0.5.0 hygiene per
    `SPEC.pgRDF.LLD.v0.6-FUTURE.md §3`). Behaviour-neutral refactor: separate
    BGP→SQL lowering, anchors model, aggregate/UNION builders, property-path
    lowering into cohesive sub-modules. Full test bar passes byte-identically.
  - **C.2 — Partition pruning** for `graph_id`-bound queries even when the
    binding comes from FILTER rather than `GRAPH { }`. Recognise constant-fold
    cases at SQL generation time.
  - **C.3 — Custom-scan hook** (PostgreSQL `CustomScan` / FDW-style integration
    per `SPEC.pgRDF.LLD.v0.6-FUTURE.md §7`). Specific quad-shape access patterns
    bypass the standard executor. Non-gating; pure throughput optimisation.
  - **C.4 — Property-path planner integration** — explore index-only-scan paths
    for `+`/`*` over predicates with pre-materialised closure.
- **Acceptance:**
  - `executor.rs` line count drops materially with no behaviour change.
  - `EXPLAIN` on a `WHERE { ?s ?p ?o FILTER(?g = <…>) }` query shows a single
    partition scan, not the full parent.
  - LUBM-100 Q1/Q2/Q3 (already shipped as correctness fixtures) gain published
    timing baselines.

### Track D — Push-down to PostgreSQL set semantics

This is the connective theme behind Tracks A/B/C. Concrete moves:

- **D.1 — `_pgrdf_dictionary` integrity options** — add an optional FK from
  `_pgrdf_quads.{subject_id,predicate_id,object_id,graph_id}` to the
  dictionary / graphs tables. Off by default (preserves loader pre-resolution
  assumption per `docs/02-storage.md:702-705`); on for users who prefer the
  DB-side guarantee.
- **D.2 — `GENERATED` columns audit** — surfaces where pgRDF computes
  derived state in Rust that PG could derive once at write time (e.g. term
  fingerprint for shmem cache could be a `GENERATED ALWAYS AS …` column if
  Postgres-side hash discipline matches the Rust SipHash).
- **D.3 — Index audit** — BRIN for time-shaped `graph_id` ranges if a
  workload partitions chronologically; GIN over JSONB stats outputs for
  observability queries; GIST not yet identified as useful but tracked.
- **D.4 — Custom aggregates** — explore Postgres-side custom aggregates for
  `GROUP_CONCAT`, `STRING_AGG` over dict-id sets when the lexical resolution
  could happen in a final aggregate state-function pass.
- **D.5 — Parallel query** — declare `pgrdf.sparql` / `pgrdf.construct` /
  `pgrdf.describe` `PARALLEL SAFE` where they actually are (most SELECT-shape
  SPI queries qualify); audit which UDFs trip the safety check.
- **D.6 — Set-based graph IRI resolution** — replace the per-quad SPI call in
  `src/storage/loader.rs:385-401` with a single bulk lookup + bulk allocate
  before the ingest loop.
- **D.7 — Set-based CONSTRUCT re-ingest** — `pgrdf.put_construct_rows` currently
  loops via `WHERE NOT EXISTS` insert per row. Route it through `flush_batch`
  with a final set-difference against `_pgrdf_quads` for idempotency.

### Track E — PGXN-grade extension hygiene

- **Goal:** `ALTER EXTENSION pgrdf UPDATE FROM '0.5.1' TO '0.6.0'` works
  in-place. Operators get the same upgrade ergonomics they expect from
  `pg_stat_statements`, `pg_partman`, `postgis`.
- **Starting state:** `pgrdf.control` declares only the 5 mandatory fields;
  no `requires`, no `directory`, no version-upgrade scripts. `docs/06-installation.md:72-74`
  explicitly: *"`ALTER EXTENSION pgrdf UPDATE` is not supported in v0.x and is
  deferred until v1.0."* `META.json` declares only `prereqs.runtime.requires =
  { PostgreSQL: 14.0.0 }`.
- **Target state:**
  - **E.1 — Upgrade scripts** — ship
    `sql/pgrdf--0.5.1--0.6.0.sql` for every storage / catalog
    delta. Open the door to upgrade-without-dump-and-reload (the line in
    `docs/06-installation.md` becomes "supported from v0.6 forward").
  - **E.2 — `requires =`** — declare runtime deps for any opt-in integration
    (e.g. `requires = 'plpgsql'` is implicit; if pgRDF starts to optionally
    compose with `pg_partman` for automated partition management, declare it).
  - **E.3 — `directory =`** — organise per-version SQL under a versioned
    sub-directory so the install layout stays clean as upgrade scripts
    accumulate.
  - **E.4 — Search-path discipline** — every SQL UDF body adopts
    `SET search_path = pgrdf, pg_temp` (or equivalent) so the extension is
    safe to use from any caller schema. Today the code relies on
    schema-qualified internal references; explicit discipline is
    belt-and-braces.
  - **E.5 — GUC catalogue** — extend `src/query/guc.rs` beyond
    `pgrdf.path_max_depth`. Candidates: `pgrdf.ingest_batch_size`,
    `pgrdf.materialize_writeback_path`, `pgrdf.plan_cache_max_entries`,
    `pgrdf.dict_cache_slots`. All registered via pgrx's `GucRegistry` in
    `_PG_init` so `SHOW` works.
  - **E.6 — `pg_prewarm` companion** — document a `pg_prewarm` recipe for
    warming the hexastore indexes after restart; ships as a sample, not a
    hard dep.
  - **E.7 — `pg_stat_statements` integration** — confirm pgRDF's prepared SQL
    is statement-id stable so `pg_stat_statements` aggregates per SPARQL shape.
    Document the relationship.
- **Acceptance:**
  - `ALTER EXTENSION pgrdf UPDATE FROM '0.5.1' TO '0.6.0'` on a populated
    schema completes without data loss; regression `125-extension-upgrade.sql`
    locks the contract.
  - `SHOW pgrdf.*` lists every GUC; PGXN install passes with zero warnings.

### Track F — Test bed for v0.6 (provided by user as part of this roadmap)

- **Goal:** an in-repo, CI-gated performance + correctness suite that locks
  the v0.6 scale claims and prevents regression.
- **Starting state:** `tests/perf/README.md` lists phased gates (LUBM-1 smoke,
  LUBM-10 baseline, LUBM-100 comparison) — all listed as planned, none met.
  No published timing numbers in `docs/`, `RELEASE_NOTES.md`, or
  `tests/perf/`.
- **Target state:** test bed prepared and provided by the user; the v0.6
  cycle wires it into:
  - `tests/perf/lubm-{10,100,1000}/` — generated dataset materialisation +
    SPARQL benchmark fixtures.
  - `tests/perf/run-lubm.sh` — actual runner (not just a layout). Emits
    `target/perf-report.json`.
  - CI gate: nightly cron job comparing `target/perf-report.json` against
    a `tests/perf/lubm.expected.json` baseline; pass if within tolerance.
  - New regression fixtures (per CX-002 EVAL recommendation):
    `tests/regression/sql/123-dictionary-lexical-contract.sql` and
    `124-end-to-end-lexical-rehydration.sql`.
- **Acceptance:**
  - `just test-lubm-10` exits 0 on every CI run, comparing against a
    committed baseline.
  - `just test-lubm-100` exits 0 on the nightly cron with documented timing
    tolerances.
  - Public release notes for v0.6 cite hard numbers, not "should scale".

### Track G — Internal hardening ✅

- **Status:** closed v0.5.16 (TG-1). All five Track G items shipped through
  the v0.5 cycle: G.1 plan-cache guards (v0.5.2), G.2 README preload section
  (v0.5.2), G.3 end-to-end lexical rehydration regression (v0.5.3), G.4
  artifact-parity v2 with compose-startup gate (v0.5.14), G.5 ERRATA.v0.6
  opened (v0.5.8). Two infrastructure spillovers shipped alongside as bonus:
  SLSA Build Provenance v1 attestation chain (v0.5.10) and the
  `update-latest-md.yml` automation closing PROVENANCE.md Rule 3
  (v0.5.13). Track G is the first column to fully close in the v0.6 cycle.
- **Goal:** close the small, named gaps surfaced during the v0.5 cycle so the
  v0.6 surface is a clean baseline for the scale work above.
- **Items:**
  - **G.1 — Plan-cache defensive guards** —
    `src/query/plan_cache.rs:72, 81, 85` call `PgAtomic.get()` without
    `is_ready()` guard. Other shmem module is consistently guarded; plan_cache
    is the outlier. Lazy-load (non-preloaded) backends panic instead of
    degrading gracefully. Add the guard. (Committed earlier in
    `NOTIFIES.pgRDF.0.5.1.shared-preload-required-RESPONSE.md`.)
  - **G.2 — README "Required postgresql.conf changes" section** — document the
    `shared_preload_libraries='pgrdf'` requirement prominently. (Committed
    earlier in the same NOTIFIES response.)
  - **G.3 — End-to-end lexical rehydration regression** —
    `tests/regression/sql/124-end-to-end-lexical-rehydration.sql` per CX-002
    EVAL: one seed graph through
    `parse_turtle → sparql → materialize → sparql → validate` with EXACT
    lexical IRIs asserted, not just row counts. Catches dictionary-rehydrate
    drift end-to-end.
  - **G.4 — Artifact-parity v2** — v0.5.1 added `just test-artifact-parity`.
    Tighten to a true byte-compare with hash, wire into compose-startup so the
    server refuses to start if the mounted `.so` / `.control` / `pgrdf--X.Y.Z.sql`
    don't match the source tree's expected hashes.
  - **G.5 — Honest 0.6 ERRATA** — open `specs/ERRATA.v0.6.md` once the first
    v0.6-era delta appears (per `SPEC.pgRDF.LLD.v0.6-FUTURE.md §12`).

### Track H — SHACL-SPARQL: dual-path execution

- **Goal:** ship **two interchangeable execution paths** for SHACL-SPARQL
  (`sh:sparql [ sh:select "…" ]`) constraints. Consumers (pgCK, downstream
  applications) pick per workload: portability or performance. Same input,
  same W3C `sh:ValidationReport` JSONB shape, different engine.
- **Starting state:**
  - `shacl 0.3.2` (published upstream 2026-05-26) closes both gaps from
    [`ERRATA.v0.5.md`](specs/ERRATA.v0.5.md) E-012 — `IRComponent::Sparql`
    variant + sh:sparql parser landed in commits `fa7a6c34` / `c7df40e6`;
    `SparqlEngine` target-resolution methods completed in `5445a050`.
  - pgRDF currently pins `shacl = "0.3"` (resolves to 0.3.1) in `Cargo.toml:91`
    with a short-circuit guard at `src/validation/shacl.rs:209-224` that
    intercepts `mode => 'sparql'` before reaching the previously-broken
    upstream engine.
  - All SHACL constraint evaluation rehydrates through
    `serialise_graph_to_ntriples → InMemoryGraph::from_str → Graph::try_from`
    regardless of mode — O(graph_size) per `validate` call. Fine at the
    smoke-ontology scale (17K triples); blocks at LUBM-100 (13M).
- **Target state — two paths, both fully supported:**

  | Path | Mode arg | Engine | Optimised for |
  |---|---|---|---|
  | **Upstream / portable** | `mode => 'native'` (default) or `mode => 'sparql'` | rudof `shacl 0.3.2` — Core via `NativeEngine`, SHACL-SPARQL via `SparqlEngine`, both over the rehydrated `InMemoryGraph` | Portability — shapes shared with TopBraid / Stardog / Jena; small-to-medium graphs |
  | **pgRDF-native (push-down)** | `mode => 'pgrdf'` (new) | pgRDF extracts `IRComponent::Sparql` constraints from the compiled `IRSchema` and routes each through `pgrdf.sparql` with `$this` substitution — hexastore-indexed, prepared-plan cached, parallel-safe | Performance-critical paths — large data graphs; in-pgRDF deployments; pgCK's `ckp.seal()` gate |

  Both paths return the W3C `sh:ValidationReport` JSONB shape. Same
  correctness, different performance envelope. Switching is a one-arg
  change: `pgrdf.validate(d, s)` → `pgrdf.validate(d, s, 'pgrdf')`.

- **Acceptance:**
  - `Cargo.toml:91` bumped to `shacl = { version = "0.3.2", features = ["sparql"] }` (feature-flag tracked in the dependency tree — confirm name during implementation).
  - Short-circuit guard at `src/validation/shacl.rs:209-224` is removed; `'sparql'` mode now routes correctly to rudof's working `SparqlEngine`.
  - New `'pgrdf'` mode handler lives in a sibling module (`src/validation/pgrdf_sparql_engine.rs` or similar) — extracts the SPARQL query string from each `IRComponent::Sparql`, walks the focus-node set produced by the shape's targets, substitutes `$this` per focus node, executes through `pgrdf.sparql`, maps each result row to a `sh:ValidationResult`.
  - Mode validation order: `'native'` (default), `'sparql'`, `'pgrdf'`. Unknown mode → `validate: unknown mode` (no silent fallback — same discipline as today).
  - JSONB `mode` field echoes the requested mode unchanged.
  - Regression `122-shacl-modes.sql` gains §F (sparql mode against `sh:select` shape, now real `sh:Violation` not "structured-unavailable") and §G (pgrdf mode against same shape — identical conforms/results modulo source-ordering).
  - W3C SHACL-SPARQL manifest fixtures vendored under `tests/w3c-shacl/sparql/`; `just test-shacl-manifest --sparql` re-baselines from "every fixture `conforms:null`" to real pass count under BOTH modes; `--pgrdf` sub-run added with the same gate.
  - **LUBM-10 dev-gate (Track F):** a representative SHACL-SPARQL constraint (e.g. "no two Persons share an SSN") validates against the LUBM-10 data graph; `'pgrdf'` mode timing recorded in `target/perf-report.json` alongside `'sparql'` mode for comparison.
  - **LUBM-100 release-gate:** same constraint on LUBM-100 data; `'pgrdf'` mode completes in measured bounded time set by the test bed; `'sparql'` mode allowed to be N× slower (publishes the gap as the documented justification for the dual-path design).
  - ERRATA.v0.5 **E-012 closes** — upstream gate now resolved (shacl 0.3.2), pgRDF native path shipped, both demonstrably working against the W3C manifest.
- **Why both, not one:**
  - The `'sparql'` (rudof) path delivers W3C SHACL Part 2 conformance the standard way — a shape file authored in TopBraid / Stardog / Jena loads into pgRDF and behaves identically. **Portability is real value** and worth keeping for callers who need it.
  - The `'pgrdf'` path delivers what only pgRDF can: SHACL-SPARQL evaluation directly against the hexastore, with plan-cache reuse across focus nodes, partition pruning, `pg_stat_statements` visibility. **Performance is real value** for in-pgRDF deployments at LUBM-100 scale.
  - Consumers self-select. **pgCK's `ckp.seal()` gate is expected to use `'pgrdf'`** for latency; ad-hoc validation tooling that shares shape files with non-pgRDF stacks should use `'sparql'`.

---

## 5. v0.6 test bed (you provide; we wire in)

The user is preparing and providing the test bed as part of this roadmap. The
roadmap commits pgRDF to **landing the wiring** for whatever shape arrives,
including:

- Fixture format (TTL, N-Triples, generated on demand, vendored).
- Harness (`just test-lubm-{10,100,1000}`).
- Timing emission (`target/perf-report.json`).
- CI gate (nightly cron, comparison against committed baseline, tolerance).
- Publication path (release notes link the LUBM-100 numbers; README headline
  cites the headline triple count).

When the test bed lands, this section grows file paths + a verification
checklist.

---

## 6. Out of scope for v0.6 (parked, cross-referenced)

These remain genuinely deferred and are NOT v0.6 work. They live in
[`specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md`](specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md)
(§4, §5, §9) and `ERRATA.v0.x.md`:

| Item | Why deferred | Tracking |
|---|---|---|
| ~~SHACL-SPARQL constraint mode~~ | **MOVED INTO v0.6 SCOPE (Track H).** Upstream gate closed by `shacl 0.3.2` (2026-05-26); pgRDF gains both the rudof path AND a native push-down path. | Track H above; ERRATA E-012 close-out |
| Federated SPARQL `SERVICE` | Explicitly v1.0. | v0.6-FUTURE §9 |
| PG 18 / pgrx 0.18 | Upstream `pgrx 0.18.0` fails to build; non-trivial schema-gen migration. | ERRATA.v0.2 E-006 |
| Streaming replication / logical decoding of RDF state | Out of scope for the v0.x line. | LLD v0.5 §10 |
| Full OWL 2 (EL / QL) | pgRDF ships OWL 2 RL only via `reasonable`. | ERRATA.v0.2 E-002 |
| Backup/restore for opaque binary state | Tracked separately. | `SPEC.pgRDF.BACKUP.v0.x` (future), INSTALL §11 OQ5 |

> **Note on RDF 1.2 triple terms (was E-011):** upstream gate also closed —
> `gtfierro/reasonable#50` merged into `reasonable v0.4.2` / made publishable
> as `v0.4.3` (2026-05-27). pgRDF-side close-out (drop `[patch.crates-io]`,
> pin `reasonable = "0.4.3"`, re-enable `publish-crate.yml.disabled`) is a
> small v0.5.2 patch or in-cycle landing — not v0.6 work proper. Mentioned
> here for completeness; not blocking.

---

## 7. Beyond v0.6 — v1.0 horizon

v1.0 is shaped by what the v0.6 cycle proves at scale. Current targets:

- **Federated `SERVICE`** (explicitly v1.0 per v0.6-FUTURE §9).
- **`ALTER EXTENSION … UPDATE` full coverage** for every public surface.
- **PG 18 support** (when pgrx 0.18+ stabilises).
- **Headline benchmarks vs. mainstream RDF stores** at LUBM-1000.
- **Architecture-2 SHACL evaluator** (custom `Engine<S>` impl) — opportunistic
  push-down for Core constraints, building on Track H's Architecture-1 pattern.
- **Architecture-3 `PgRdfStore: QueryRDF + NeighsRDF`** — pgRDF as a
  first-class rudof RDF backend; every future rudof feature (ShEx, custom
  DSLs) auto-runs on Postgres without rehydration.

---

## 8. Open errata snapshot

| Erratum | Status |
|---|---|
| E-002 | OWL 2 RL only; full OWL out of scope |
| E-006 | pgrx 0.18 / PG 18 upstream-gated |
| E-009 | SHACL upstream conflict — resolved in v0.4 cycle via E-011 |
| E-011 | upstream `reasonable` — **closed upstream by v0.4.3 (2026-05-27)**; pgRDF-side bump pending (v0.5.2 patch) |
| E-012 | `shacl` SHACL-SPARQL — **closed upstream by 0.3.2 (2026-05-26)**; pgRDF-side dual-path landing in v0.6 Track H |
| E-013 | W3C SHACL Core gate invariant — resolved at v0.5.0-rc1 |

---

## 9. How this document is maintained

- **Cadence:** updated on every release boundary and after every direction
  change. The `Last updated` line at the top is the canonical freshness mark.
- **Scope:** stakeholder-facing WHY and WHAT. HOW and WHEN live in
  [`docs/10-roadmap.md`](docs/10-roadmap.md) (phase + slice tracking) and
  contract detail lives in `specs/SPEC.pgRDF.LLD.v0.x.md`.
- **Edit discipline:** when a v0.6 track ships, the row in §4 moves from 🎯
  to ✅ and the matching evidence cite is added. When a track is reframed,
  the change is recorded in §10 below.
- **Anti-pattern:** this document does NOT track per-commit slice work. That
  belongs in `docs/10-roadmap.md`. If you find yourself adding a "slice 73
  landed" line here, move it.

---

## 10. Change log for this roadmap

| Date | Change |
|---|---|
| 2026-05-28 r2 | Added **Track H — SHACL-SPARQL dual-path execution** (rudof path for portability + pgRDF native path for performance). Reframed scale gates: LUBM-10 as dev-gate, **LUBM-100 as release-gate** (v0.6.0 doesn't ship without it), LUBM-1000 demoted to post-release directional. E-012 moved from "out of scope (upstream-gated)" into Track H (upstream gate closed by `shacl 0.3.2`, 2026-05-26). E-011 noted as upstream-closed by `reasonable v0.4.3` (2026-05-27); pgRDF-side bump tracked as a v0.5.2 patch separate from v0.6. v1.0 horizon gains Architecture-2 / Architecture-3 follow-ons for SHACL evaluator. |
| 2026-05-28 r1 | Initial publication. v0.5.1 shipped; v0.6 framed around scale to tens of millions of triples + PG-native push-down + PGXN-grade hygiene. |
