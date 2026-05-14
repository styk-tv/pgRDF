# Changelog

All notable changes to pgRDF are tracked here. Format follows
[Keep a Changelog](https://keepachangelog.com/). Versioning is SemVer
once we cut v1.0; pre-1.0 minor bumps may include breaking changes.

## [Unreleased]

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
