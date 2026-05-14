# 10 — Roadmap

> **v0.3 LLD has shipped** ([`specs/SPEC.pgRDF.LLD.v0.3.md`](../specs/SPEC.pgRDF.LLD.v0.3.md)).
> Phase numbering on this page tracks the v0.3 progression: Phase 1
> done, Phase 2 (functional SPARQL) done through the sub-steps below,
> Phase 3 (storage performance) is next. See the v0.3 LLD for the
> authoritative phase map.

Within each phase, sub-steps track delivery cadence — each one is a
git commit on `main` with both pgrx + regression coverage green.

Status legend:
- ✅ shipped
- 🚧 in progress (sub-step partially delivered)
- ⏳ planned (not yet started)
- ❌ deferred (intentionally out of current scope)

---

## Phase 1 — Core Storage & Build Automation ✅

Outcome: extension registers cleanly in stock `postgres:17.4-bookworm`
and the local build produces a usable `.so` + `.control` + `.sql`.

- ✅ pgrx 0.16 scaffold compiles on PG 14–17. PG 18 deferred pending
      pgrx 0.17/0.18 fix (see `specs/ERRATA.v0.2.md` E-006).
- ✅ `_pgrdf_dictionary` + `_pgrdf_quads` schema in
      `sql/schema_v0_2_0.sql`, loaded via `extension_sql_file!`.
- ✅ Hexastore SPO/POS/OSP covering indexes
      (`INCLUDE (is_inferred)`).
- ✅ Two-VM build/run split: Colima 200 GB for builds (Linux
      container), podman for the compose stack.
- ✅ BuildKit cache mounts for `cargo` registry + `target/`; builder
      image 7.73 GB → 3.35 GB.
- ✅ `just build-ext` produces the package artifacts in
      `compose/extensions/`.
- ✅ `just compose-up` boots stock postgres:17.4 + `CREATE EXTENSION
      pgrdf` works end-to-end.

**Not shipped at this phase boundary** (carried into later phases):
- ⏳ GitHub Actions matrix green on tag push (workflow stubs exist;
      not yet wired to a real release).
- ⏳ Pre-built tarballs on a GitHub release matching INSTALL §3
      layout — Phase 4.
- ❌ COPY BINARY ingestion (LLD §4.3) — Phase 2.2 substituted
      **batched INSERT via `unnest($1::bigint[], …)`** as a
      stepping-stone delivery. COPY-BINARY tracked as a Phase 2.x
      performance follow-on.

---

## Phase 2 — Query Engine & Storage Performance 🚧

Outcome: SPARQL SELECT queries cover the practically-useful surface
end-to-end; ingestion is fast enough to load real-world ontologies.

### Phase 2.0 — Storage CRUD UDFs ✅

- ✅ `pgrdf.put_term(value, term_type)` + `pgrdf.get_term(id)` with
      `IS NOT DISTINCT FROM` dedup over (term_type, lexical_value,
      datatype_iri_id, language_tag).
- ✅ `pgrdf.put_quad(s, p, o, g)` + `pgrdf.count_quads(g)`.
- ✅ `pgrdf.add_graph(g)` — idempotent LIST partition creation, so
      `DROP TABLE _pgrdf_quads_<g>` becomes the constant-time
      whole-graph drop the LLD calls for.

### Phase 2.1 — Turtle ingest ✅

- ✅ `pgrdf.load_turtle(path, graph_id, base_iri)` and
      `pgrdf.parse_turtle(content, graph_id, base_iri)` via
      `oxttl 0.2`.
- ✅ `put_term_full(value, type, datatype_id, lang)` honours the full
      dictionary key with NULL-aware dedup.
- ✅ 24 W3C / Apache Jena / ValueFlows / ConceptKernel v3.7 ontologies
      smoke-load cleanly via `tests/perf/smoke-ontologies.sh`
      (17 134 triples on the 2026-05-13 fetch). `workflow.ttl` held
      out for non-RFC IRI form (ERRATA E-007).

### Phase 2.2 — Dict cache + batched ingest + SPARQL parser/executor ✅

- ✅ **Per-call HashMap dict cache** + buffered multi-row INSERTs
      via `unnest($1::bigint[], $2::bigint[], $3::bigint[])` with
      BATCH_SIZE = 1000. Reduces SPI calls from ~7/triple to roughly
      `distinct_terms + ceil(triples/1000)`.
- ✅ `pgrdf.load_turtle_verbose` / `parse_turtle_verbose` return
      JSONB stats (triples, dict_cache_hits, dict_db_calls,
      quad_batches, elapsed_ms).
- ✅ `pgrdf.sparql_parse(q TEXT) → JSONB` — spargebra-backed AST
      introspection.
- ✅ `pgrdf.sparql(q TEXT) → SETOF JSONB` — BGP → SQL translator.
      Single triple → N-pattern BGPs with shared-variable INNER
      JOINs via first-occurrence anchors.
- ✅ Three doc tracks split: `specs/` (authoritative) +
      `docs/` (engineering plan) + `guide/` (user docs).
- ✅ 4 client integration guides: Python, Rust, Node/TypeScript, Go.

**Phase 3 storage-perf status (v0.3 LLD):**
- ✅ **Shmem dictionary cache (LLD §4.1)** — `PgLwLock<[Slot; 16 384]>`
      cross-backend cache with u128 fingerprint, commit-deferred
      publish, generation invalidation. Per-call
      `load_turtle_verbose.shmem_cache_hits` and cumulative
      `pgrdf.stats()` counters; regression
      `50-shmem-dict-cache.sql` asserts 100 % shmem hit rate on the
      second load of `synth-100.ttl`.
- ✅ **Prepared-plan cache (LLD §4.2)** — parameterised SPARQL SQL +
      per-backend `OwnedPreparedStatement` cache keyed by the SQL
      string. `pgrdf.stats()` exposes
      `plan_cache_hits / misses / inserts / local_size`. Operator
      hook: `pgrdf.plan_cache_clear()`. Regression
      `51-plan-cache.sql` asserts the hit / miss / parametric-reuse
      arithmetic for three workload shapes.
- 🚧 **COPY BINARY ingestion (LLD §4.3)** —
      - ✅ **Phase A**: prepared `INSERT … unnest(…)` cached
        per-backend, reused across batches and across loads.
        Saves one parse+plan per batch (~100–500 µs each).
        Verified by `52-bulk-ingest-perf.sql` on synth-10k.ttl.
      - ⏳ **Phase B** (deferred to Phase 3 step 3b / v0.4): the
        2× wall-clock target from LLD §4.3 acceptance is not met
        by phase A alone — the per-tuple executor walk dominates.
        Candidate paths: `pg_sys::heap_multi_insert` per partition,
        or `BeginCopyFrom` + binary callback. Both FFI-heavy.
- ⏳ W3C SPARQL 1.1 manifest runner wired into CI; coverage target
      ≥ 30 % pass for Phase 2 completion per LLD §7.

### Phase 3 — Extended SPARQL surface 🚧 (current)

This phase wasn't called out in the v0.2 LLD — LLD Phase 2 just
said "SELECT … WHERE { BGP }". The work below extends `pgrdf.sparql`
toward a practically-useful SPARQL 1.1 surface, in tight slices
each shipping with pgrx + regression coverage.

| Step | Surface | Commit | pgrx | regression |
|---|---|---|---|---|
| 1 | FILTER — identity (`=`, `!=`, `sameTerm`), boolean (`&&`, `\|\|`, `!`), term-type (`isIRI`, `isLiteral`, `isBlank`), `BOUND` | `1ebeefc` | 28 | 14 |
| 2 | FILTER — numeric ordering (`<`/`>`/`<=`/`>=`), `REGEX`, `IN`, `STR` passthrough | `51b4d56` | 34 | 15 |
| 3 | Solution modifiers — `DISTINCT`, `REDUCED`, `LIMIT`, `OFFSET`, `ORDER BY ASC/DESC ?var` | `4bc9a87` | 40 | 16 |
| 4 | `OPTIONAL { ?s :p ?o }` → `LEFT JOIN` (with inner FILTER and chained blocks) | `6546d80` | 45 | 17 |
| 5 | `UNION` (n-way, branch-local FILTERs and OPTIONALs) | `56b7bca` | 51 | 18 |
| 6 | `MINUS` → `NOT EXISTS` keyed by shared variables | `59ee1b9` | 56 | 19 |
| 7 | Aggregates — `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT)`, `SUM`, `AVG`, `MIN`, `MAX` + `GROUP BY` | `fd40845` | 63 | 20 |
| 8 | `HAVING` (post-aggregate filter) + `GROUP_CONCAT` + `SAMPLE` | `066ce53` | 67 | 21 |
| 9 | Expression richness — arithmetic (`+`/`-`/`*`/`/`), `STRLEN`, `CONTAINS`/`STRSTARTS`/`STRENDS`, `LANG`/`DATATYPE`/`UCASE`/`LCASE` | `78df3a6` | 73 | 22 |
| 10 | `BIND(expr AS ?v)` for projection (Literal/NamedNode/Variable, STR/LANG/DATATYPE/UCASE/LCASE/STRLEN, arithmetic, CONCAT) | `99069a6` | 76 | 23 |
| 11 | Multi-triple MINUS (sub-pattern with N triples joined inside the NOT EXISTS) | `bc6d0a8` | 77 | 24 |
| 12 | `ASK { … }` query form → single JSONB row `{"_ask": "true"\|"false"}` | `fc67285` | 79 | 25 |

**SPARQL surface declared substantively complete with step 12 — the
deferred items (multi-triple OPTIONAL, VALUES, GRAPH, BIND-in-FILTER,
aggregates-over-UNION, CONSTRUCT, DESCRIBE, type-aware MIN/MAX,
property paths beyond simple sequence) move to v0.4 work; they don't
block the storage-performance Phase 3 of the v0.3 LLD.**

Phase 3 backlog (each its own slice):

- ⏳ `HAVING` (post-aggregate filter) + `GROUP_CONCAT` / `SAMPLE`
      aggregates.
- ⏳ `GRAPH { … }` named-graph clause. Needs a graph IRI → graph_id
      mapping (schema change).
- ⏳ Multi-triple OPTIONAL / MINUS — relax the current single-triple
      restriction via a derived-table refactor inside the LEFT JOIN
      / NOT EXISTS sub-SELECT.
- ⏳ Arithmetic in FILTER (`?a + ?b > 30`), `lang(?v)` /
      `datatype(?v)` functions, full string-fn surface (`STRLEN`,
      `CONTAINS`, `STRSTARTS`, `STRENDS`, `SUBSTR`).
- ⏳ Type-aware ORDER BY (sort numeric literals numerically rather
      than as strings).
- ⏳ `BIND (expr AS ?var)`, `VALUES (?x ?y) { … }`.
- ⏳ Property paths beyond simple sequence (`*`, `+`, `?`, `^`,
      alternation). Simple sequence already works because spargebra
      desugars `:a/:b` into a BGP chain.
- ⏳ `CONSTRUCT`, `ASK`, `DESCRIBE`.

---

## Phase 4 — Inference Engine 🚧 (partial)

Outcome: materialized OWL 2 RL inference works against real
ontologies; SHACL validation is its own Phase 5. Tracks LLD v0.3
§5.2.

Gates:
- ✅ `pgrdf.materialize(graph_id BIGINT) → JSONB` —
      `src/inference/reasonable.rs` rehydrates base quads via a
      single SPI scan + 3 dict-JOINs, runs `reasonable::Reasoner`
      (OWL 2 RL — see ERRATA E-002), set-diffs against the input,
      and INSERTs the entailed-but-not-asserted triples with
      `is_inferred = TRUE`. Idempotent. Verified by
      `tests/regression/sql/60-materialize-owl-rl.sql`.
- ⏳ Reasoner-coverage fixture (e.g. pizza ontology subset) with a
      golden expected-closure diff. Deferred — current regression
      uses minimal hand-authored TBoxes.
- ⏳ Loader-side writeback via `flush_batch` (depends on Phase 3
      step 3b shipping the bulk-INSERT primitive).

---

## Phase 5 — Validation Engine 🚧 (stub)

Outcome: SHACL validation works against real shapes graphs. Tracks
LLD v0.3 §5.3.

Gates:
- 🚧 `pgrdf.validate(data BIGINT, shapes BIGINT) → JSONB` —
      surface SHIPPED (`src/validation/shacl.rs`); body returns
      `{"status": "stub", …}` blocked by ERRATA E-009 (upstream
      `iri_s`/`rdf-12` dep conflict between `shacl_validation` and
      `reasonable`). Verified by `70-validate-stub.sql`.
- ⏳ Real `shacl_validation` integration once either upstream
      catches up (see `docs/05-validation.md` for the unblock
      conditions). When wired, this lands as a v0.4 follow-up.
- ⏳ W3C SHACL conformance manifest runner — paired with Phase 6.

---

## Phase 6 — CI + Conformance + Release 🚧 (step 1 shipped)

Outcome: pgRDF is consumable by external operators (CloudNativePG,
StackGres) following INSTALL spec methodology. Benchmarked. Tracks
LLD v0.3 §5.4.

**Step 1 — Regression in CI** ✅
- `.github/workflows/ci.yml` `regression` job runs the
  compose-based pg_regress suite on every PR + push to main.
  Pinned to PG 17 today (compose pin per ERRATA E-006).

**Step 2 — W3C conformance** 🚧 (starter shipped)
- ✅ `tests/w3c-sparql/` hand-authored harness — 5 starter tests
  covering basic BGP, DISTINCT, UNION-disjoint, OPTIONAL chain,
  MINUS-no-shared. Bash runner; runs alongside `tests/regression/`
  in the same CI job. Each expected output cites the W3C spec
  section it exercises.
- ⏳ Full W3C TTL-manifest runner against `w3c/rdf-tests`. The
  `pgrdf-w3c-sparql` Rust binary placeholder in
  `regression-w3c.yml::sparql11` (gated `if: false`) is the
  destination shape; lands as v0.4.
- ⏳ W3C SHACL manifest runner. Gated on ERRATA E-009 unblocking.
- ⏳ Coverage targets ratchet per release:
  SPARQL `≥ 30 % → ≥ 70 % → ≥ 95 %`; SHACL `≥ 50 % → ≥ 90 %`.

**Step 3 — Release artifacts** ⏳
- `.github/workflows/release.yml` already builds and packages on
  `v*` tags; fires the first official release once step 2 lands.
- LUBM-100 results in `target/perf-report.json` compared against
  Apache Jena TDB and Apache AGE.
- OCI artifact published at `ghcr.io/styk-tv/pgrdf-bundle:<ver>`
  (INSTALL §11 OQ1).
- INSTALL §12 conformance test in CI against a fresh K8s cluster
  (kind or k3s).
- SHA256SUMS.asc detached GPG signature attached to every release.
- Target gates: W3C SPARQL 1.1 ≥ 95 % pass; SHACL ≥ 90 % pass
  (the SHACL gate moves with ERRATA E-009 resolution).

---

## Out of scope (v0.x)

- Streaming replication / logical decoding of RDF state.
- Federated SPARQL `SERVICE`.
- Full OWL 2 (EL / QL) reasoning — ERRATA E-002.
- Backup/restore for opaque binary state (tracked by future
  `SPEC.pgRDF.BACKUP.v0.x`, INSTALL §11 OQ5).

---

## Test bar over time

A coarse cumulative view; the precise per-commit count is in the
phase 3 step table above.

| Boundary | pgrx integration | pg_regress files | Notes |
|---|---|---|---|
| Phase 1 done | 0 | 0 | smoke + scaffold only |
| Phase 2.0 done | 7 | 3 | dict + quad CRUD |
| Phase 2.1 done | 11 | 7 | + Turtle ingest, regression fixtures |
| Phase 2.2 done | 21 | 13 | + dict cache, batched ingest, SPARQL parser, BGP-to-SQL, N-pattern BGP joins, user guide |
| Phase 3 step 6 | 56 | 19 | + FILTER, modifiers, OPTIONAL, UNION, MINUS |
| Phase 3 step 7 | 63 | 20 | + aggregates (COUNT/SUM/AVG/MIN/MAX + GROUP BY) |
| Phase 3 steps 8–12 | 79 | 25 | + HAVING, GROUP_CONCAT/SAMPLE, expression richness, BIND, multi-triple MINUS, ASK |
| v0.3 Phase 3 step 1 | 86 | 26 | + shmem dict cache (LLD §4.1), `pgrdf.stats()`, perf regression `50-shmem-dict-cache.sql` |
| v0.3 Phase 3 step 2 | 88 | 27 | + prepared-plan cache (LLD §4.2), parameterised SQL, perf regression `51-plan-cache.sql` |
| v0.3 Phase 3 step 3 phase A | 88 | 28 | + bulk-ingest prepared INSERT (LLD §4.3 phase A), `synth-10k.ttl`, perf regression `52-bulk-ingest-perf.sql`. 2× wall-clock target deferred to phase B / v0.4 |
| v0.3 Phase 4 | 91 | 29 | + `pgrdf.materialize` OWL 2 RL inference via `reasonable` 0.4, set-diff isolation, idempotent re-derivation, regression `60-materialize-owl-rl.sql` |
| v0.3 Phase 5 stub | 93 | 30 | + `pgrdf.validate(data, shapes)` JSONB stub. Real `shacl_validation` integration deferred — ERRATA E-009 (upstream iri_s/rdf-12 dep block). Regression `70-validate-stub.sql` |
| v0.3 Phase 6 step 1 | 93 | 30 | + regression suite wired into CI (`.github/workflows/ci.yml` `regression` job); compose builder + runtime on every PR. W3C runners + LUBM benchmarks remain deferred |
| v0.3 Phase 6 step 2 starter | 93 | 30+5 | + W3C-shape SPARQL harness — 5 starter tests in `tests/w3c-sparql/` wired into the CI regression job. Full W3C TTL-manifest runner deferred to v0.4 |
| v0.3 Phase 6 step 2 expanded | 93 | 30+13 | + 8 more W3C-shape tests covering FILTER, COUNT/HAVING, ORDER BY DESC, LIMIT/OFFSET, BIND/CONCAT, ASK true/false |
| v0.3 Phase 6 step 2 expanded II | 93 | 30+18 | + 5 more W3C-shape tests covering REGEX, IN, STRLEN, LANG, UCASE |
| v0.3 translator-gap signals + step 3 scaffold | 93 | 31+18+3 | + 8 negative regression signals (`80-unsupported-shapes.sql`) locking the error-message contract for unsupported SPARQL shapes; + 3 LUBM-shape correctness gates (`tests/perf/lubm-shape/`) against a hand-authored fixture |
| v0.3 +3 W3C essentials + integration | 93 | 32+21+3 | + 3 more W3C-shape tests (BOUND, STR(?iri), numeric FILTER); + `61-materialize-then-sparql.sql` integration test verifying inferred triples flow back through `pgrdf.sparql` |
| v0.3 stats shape contract | 93 | 33+21+3 | + `82-stats-shape.sql` locks the `pgrdf.stats()` JSONB field set, types, and value-range invariants — schema contract for downstream operator tooling |
| v0.3 translator fix — inline HAVING aggregate | 93 | 33+22+3 | `AggregateSpec.synth_aliases` preserves spargebra's intermediate variable name post-Extend rename; HAVING migration + translation consult both `output_var` and aliases. Negative `gap-1` removed; new positive test `22-having-inline-aggregate` covers `HAVING(SUM(?v) > c)` directly |
| v0.3 translator fix — type-aware MIN/MAX | 93 | 33+23+3 | `MIN`/`MAX` emit `COALESCE(MIN(numeric)::text, MIN(lex))` — numeric ordering on `xsd:numeric` literals, lex fallback for strings. New positive test `23-min-max-numeric` over `xsd:integer` |
| v0.3 error-path signals — #66 | 93 | 34+23+3 | + `81-error-paths.sql` opens a sibling track to `80`: locks the stable error-prefix UDFs emit on invalid input. Helper `_check_error` generalises `_check_gap` via `EXECUTE`. First check: `pgrdf.load_turtle()` against a missing path surfaces `load_turtle: failed to open` |
| v0.3 edge-case signals — #62 | 93 | 35+23+3 | + `62-materialize-empty.sql` opens an edge-case correctness track (slices 62 → forward) below the error-path track (66 → 63): `pgrdf.materialize()` on a zero-triple graph stays non-panicking, returns `base_triples = 0` + non-negative inferred-count, and remains idempotent across two calls (run 2's `previous_inferred_dropped` == run 1's `inferred_triples_written`). Axiomatic OWL 2 RL triple count NOT locked — that's upstream `reasonable` internals |
| v0.3 edge-case signals — #61 | 93 | 36+23+3 | + `63-shmem-reset-invalidation.sql` locks `pgrdf.shmem_reset()`'s shmem-cache invalidation contract: after `reset()` bumps the `GENERATION` atomic, re-parsing terms that were cached pre-reset (a) does NOT advance `shmem_hits` (slot-generation mismatch reads as cold) and (b) DOES advance `shmem_inserts` (fresh inserts replace the invalidated entries). Guards against a refactor of `src/storage/shmem_cache.rs::reset()` that forgets the generation bump and leaves stale dict ids visible across a `DROP EXTENSION; CREATE EXTENSION` cycle. Asserts deltas (not absolute counter values) via `\gset`-captured booleans so the expected output survives upstream churn |
| v0.3 edge-case signals — #60 | 93 | 37+23+3 | + `64-plan-cache-clear.sql` locks the returned-count semantics of `pgrdf.plan_cache_clear()`: fresh backend → 0 dropped, after N structurally distinct queries → N dropped (matches `plan_cache_local_size` snapshot taken pre-clear), `plan_cache_local_size` falls to 0 post-clear, second consecutive clear returns 0 (idempotent at zero). Guards against a refactor of `src/query/plan_cache.rs::plan_cache_clear()` that swaps `m.len()` for a constant, hoists the `len()` after `m.clear()` (always returning 0), or accidentally muddles the per-backend count with the cumulative shmem `plan_cache_inserts` counter. Empirical `size_before` on the current pgrx 0.16 / PG 17 build is 4 (1 ingest-side `flush_batch` INSERT plan + 3 SELECT plans), but the test locks the RELATION `drained = size_before AND size_after = 0 AND idempotent_clear = 0 AND size_before > 0` rather than the literal, so an ingest-path refactor that skips the plan cache leaves the test still passing |
| v0.3 edge-case signals — #59 | 93 | 38+23+3 | + `65-parse-turtle-empty.sql` locks the boundary contract of `pgrdf.parse_turtle()` on triple-free input: empty string, whitespace-only (`E'   \n   \t  '`), comment-only (`E'# c1\n# c2\n'`), and bare `@prefix` declaration all return `0` without panicking; `_pgrdf_quads` for the graph stays empty; `_pgrdf_dictionary` stays empty (interning happens INSIDE the per-triple loop body of `src/storage/loader.rs::ingest_turtle_with_stats`, so directives that emit zero triples emit zero dict writes). Orthogonal correct-path companion to the malformed-input case in `81-error-paths.sql` (which panics with the `load_turtle: turtle parse error: …` prefix): this slice locks that an EMPTY parser iterator is NOT a parse error — it returns `0` cleanly. Guards against a refactor that wraps the loop in a "fast-path" panicking on empty input, that seeds a placeholder dict/quad row, or that mishandles the trailing `flush_batch()` of zero-length arrays |
| v0.3 edge-case signals — #58 | 93 | 38+23+3 | + `tests/perf/smoke-ontologies.expected.tsv` locks the per-ontology triple counts emitted by `tests/perf/smoke-ontologies.sh` across the current 24-ontology W3C/Apache-Jena/ValueFlows/ConceptKernel-v3.7 set (workflow.ttl held out per ERRATA E-007); snapshot today is **24 rows / 17,134 triples total**. New `tests/perf/smoke-ontologies.sh --check` mode re-runs the smoke, regenerates a TSV from the live output, and `diff -u`'s it against the lock-file (exit non-zero on any drift). Catches two regression classes invisible to the bare smoke: an ontology that used to parse stops parsing (row disappears) and the parser silently drops/duplicates triples (count moves). Not gated in CI yet — `fixtures/ontologies/*.ttl` is gitignored, so the smoke can only run locally after `fixtures/ontologies.sh`; a follow-on Phase 6 slice wires `--check` once a CI fetch step lands. Default smoke behaviour (no flag → pretty-print, exit 0) unchanged. NOT a pg_regress file — test bar unchanged at 38+23+3 |
| v0.3 edge-case signals — #57 | 93 | 39+23+3 | + `66-parse-sparql-roundtrip.sql` locks the end-to-end round-trip from `pgrdf.parse_turtle` ingest through `pgrdf.sparql` query: every triple the parser saw MUST be observable via the SPARQL executor across all four object-term kinds plus a blank-node subject. Five `bool_and(EXISTS …)` assertions over a single 5-shape Turtle fragment cover (1) IRI object (`foaf:knows`), (2) plain literal (`foaf:name "Alice"`), (3) typed literal (`ex:age "30"^^xsd:integer`), (4) language-tagged literal (`ex:bio "Engineer"@en`), and (5) blank-node subject — keyed by a sibling-property join `?s foaf:name "Anon" . ?s foaf:name ?n` so the parser-allocated bnode id stays out of the assertion. Sibling to `61-materialize-then-sparql.sql` (which locks the materialize→sparql edge); together they pin both ends of the storage layer's visibility contract to the SPARQL surface. Datatype URI and lang-tag echo policy are NOT pinned by this slice (the SPARQL projection emits the lexical only); their storage-side contracts are locked by `21-typed-literals.sql` / `22-lang-tags.sql` |
| v0.3 edge-case signals — #56 (current) | 93 | 39+23+3 | extends `82-stats-shape.sql` in-place (no new pg_regress file — the file is explicitly scoped to "schema shape only" and these three new invariants are schema shape too) with the schema-drift tripwire trio: (a) exact field count — `count(*) FROM jsonb_object_keys(stats()) = 10` pins to the literal current key count emitted by `src/storage/stats.rs::stats()` (`shmem_ready`, `shmem_slots`, `shmem_hits`, `shmem_misses`, `shmem_inserts`, `shmem_evictions`, `plan_cache_hits`, `plan_cache_misses`, `plan_cache_inserts`, `plan_cache_local_size`) so any added field forces a deliberate test update; (b) keys-match-canonical — `array_agg(k ORDER BY k) = ARRAY[…literal 10-element list…]` catches both silent additions (array gets longer) and silent renames (one element swaps); (c) no-null-fields — `bool_and(jsonb_typeof(value) != 'null')` catches a refactor that defaults an uninitialised counter to JSON `null` rather than `0`. Companions the existing "fields-that-SHOULD-be-there are there" block with the orthogonal "fields-that-SHOULDN'T-be-there ARE NOT there" guarantee — together they pin the closed-set shape contract downstream operator tooling (CloudNativePG operators, CI dashboards, telemetry parsers) wires against. Test count unchanged: still 39+23+3 — three new rows in `tests/regression/expected/82-stats-shape.out` |
