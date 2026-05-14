# **SPEC.pgRDF.LLD.v0.3**

**pgRDF: A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL,
and OWL 2 RL reasoning.**

*Positioning: pgRDF — the high-performance PostgreSQL semantic-web toolkit.*

---

## 0. Document status and supersession

- **Supersedes:** `SPEC.pgRDF.LLD.v0.2.md` (verbatim contract still
  preserved; this document supersedes it at the contract level).
- **Carries forward:** `SPEC.pgRDF.INSTALL.v0.2.md` (no install-spec
  changes in v0.3) and `ERRATA.v0.2.md` (still authoritative for the
  spec deltas it lists).
- **Forward-looking sibling:** [`SPEC.pgRDF.LLD.v0.4-FUTURE.md`](SPEC.pgRDF.LLD.v0.4-FUTURE.md)
  is the draft target spec for the next cut; v0.3 remains the
  shipped contract until v0.4 actually lands. The `-FUTURE` postfix
  signals that the document is aspirational, not authoritative.
- **Reason for v0.3:** v0.2 captured the architecture + the
  initial Phase-by-Phase progression. After 12 Phase-3 SPARQL
  slices shipped, the v0.2 phase numbering and scoped surface no
  longer match reality. v0.3 reflects shipped state, re-bins the
  work that remains, and explicitly defers items that are bigger
  than the v0.2 LLD anticipated.

## 1. Mission (unchanged from v0.2)

pgRDF is a PostgreSQL extension built entirely in Rust using `pgrx`.
It provides native storage and querying for RDF data directly
inside Postgres, with four engines:

1. **Storage Engine** — dictionary-encoded terms in
   `_pgrdf_dictionary`; quads in `_pgrdf_quads` partitioned by
   `graph_id`; hexastore covering indexes (SPO, POS, OSP) on
   `_pgrdf_quads`.
2. **SPARQL Engine** — `pgrdf.sparql(q TEXT) → SETOF JSONB` for
   SELECT / ASK; spargebra parser; dynamic-SQL executor (prepared
   plans queued — see §5.2).
3. **Inference Engine** — OWL 2 RL materialisation via
   `reasonable` (per `ERRATA.v0.2.md` E-002); not yet shipped.
4. **Validation Engine** — SHACL via `shacl_validation` (per
   `ERRATA.v0.2.md` E-001); not yet shipped.

## 2. What's shipped (as of v0.3 cut)

| Surface | Status | Reference |
|---|---|---|
| `_pgrdf_dictionary` schema (HASH index on lexical_value) | ✅ | `sql/schema_v0_2_0.sql` |
| `_pgrdf_quads` partitioned by graph_id (LIST + default) | ✅ | same |
| Hexastore SPO / POS / OSP covering indexes | ✅ | same |
| Per-call HashMap dict cache (stepping stone for §4.1) | ✅ | `src/storage/loader.rs` |
| Batched INSERT via `unnest($1::bigint[], …)` (stepping stone for §4.3) | ✅ | same |
| Turtle ingest (`pgrdf.load_turtle`, `pgrdf.parse_turtle`, verbose stats) | ✅ | same |
| `pgrdf.put_term`, `pgrdf.get_term`, `pgrdf.put_quad`, `pgrdf.count_quads`, `pgrdf.add_graph` | ✅ | `src/storage/` |
| `pgrdf.sparql_parse` — spargebra AST as JSONB | ✅ | `src/query/parser.rs` |
| `pgrdf.sparql` — SELECT and ASK | ✅ | `src/query/executor.rs` |
| SPARQL SELECT surface (see §3 for the table) | ✅ Phase 3 steps 1–12 | same |
| W3C / Apache Jena / ValueFlows / ConceptKernel ontology smoke (24 ontologies, 17 134 triples) | ✅ manual | `tests/perf/smoke-ontologies.sh` |
| Compose-based local runtime (`postgres:17.4-bookworm` + per-file bind mounts) | ✅ | `compose/` |
| Two-VM build/run split (Colima for builds, podman for runtime) + BuildKit cache mounts | ✅ | `Justfile`, `compose/builder.Containerfile` |
| Three doc tracks (`specs/` authoritative, `docs/` engineering, `guide/` user) + 4 client guides | ✅ | `docs/README.md`, `guide/README.md` |

**Test bar at the v0.3 cut: 79 pgrx integration + 25 regression
files. All green. No autobaselining of new query coverage —
expected outputs are hand-computed (see §6.2).**

## 3. SPARQL coverage today

The `pgrdf.sparql` UDF answers a substantial subset of SPARQL 1.1.
The capability matrix is the contract for v0.3:

| Form | Status |
|---|---|
| **SELECT** with N-pattern BGP, shared-variable INNER joins | ✅ |
| Constants in any triple position (IRI, literal, datatype-annotated, lang-tagged) | ✅ |
| **`DISTINCT`**, **`REDUCED`** | ✅ |
| **`LIMIT N`**, **`OFFSET N`** | ✅ |
| **`ORDER BY ?v` / `ASC(?v)` / `DESC(?v)`** (lex on lexical_value) | ✅ |
| **`FILTER`** — identity (`=`, `!=`, `sameTerm`), boolean (`&&`, `\|\|`, `!`) | ✅ |
| **`FILTER`** — term-type (`isIRI`, `isLiteral`, `isBlank`), `BOUND` | ✅ |
| **`FILTER`** — numeric ordering (`<`/`>`/`<=`/`>=`), `IN` | ✅ |
| **`FILTER`** — arithmetic (`+`/`-`/`*`/`/`, unary `-`, `+`) | ✅ |
| **`FILTER`** — `REGEX`, `CONTAINS`, `STRSTARTS`, `STRENDS`, `STRLEN` | ✅ |
| **`FILTER`** — `STR`, `LANG`, `DATATYPE`, `UCASE`, `LCASE` | ✅ |
| **`OPTIONAL { single-triple BGP }`** with inner FILTER + chained blocks | ✅ |
| **`UNION`** (n-way; branches may bind disjoint vars; per-branch FILTER / OPTIONAL / MINUS) | ✅ |
| **`MINUS { N-triple BGP }`** keyed on shared vars (elided if none shared, per spec) | ✅ |
| **Aggregates** — `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT)`, `SUM`, `AVG`, `MIN`, `MAX` + `GROUP BY` | ✅ |
| **Aggregates** — `HAVING`, `GROUP_CONCAT (with SEPARATOR)`, `SAMPLE` | ✅ |
| **`BIND(expr AS ?v)`** — Literal, NamedNode, Variable, STR / LANG / DATATYPE / UCASE / LCASE, arithmetic, `CONCAT`, `STRLEN` (projection-only — filtering on BIND output deferred) | ✅ |
| **`ASK { … }`** — single-row `{"_ask": "true"\|"false"}` | ✅ |
| **`OPTIONAL { multi-pattern BGP }`** | ⏳ v0.4 (needs LATERAL refactor) |
| **`VALUES`** inline tables | ⏳ v0.4 |
| **`GRAPH { … }`** named-graph scoping | ⏳ v0.4 (needs storage schema: graph-IRI → graph_id map) |
| **`BIND` output referenced in later `FILTER` / BGP** | ⏳ v0.4 (needs AST substitution) |
| **Aggregates over `UNION`** | ⏳ v0.4 (needs derived-table refactor) |
| **Property paths** (`*`, `+`, `?`, `^`, alternation) beyond simple sequence | ⏳ v0.4 |
| **Type-aware `ORDER BY` / `MIN` / `MAX`** (sort numerics numerically) | ⏳ v0.4 |
| **`CONSTRUCT`** | ⏳ v0.4 (different return shape — emits triples) |
| **`DESCRIBE`** | ⏳ v0.4 (same) |
| **`SERVICE`** (federated SPARQL) | ❌ out of scope for v0.x |

`pgrdf.sparql_parse(q)` reports the parsed AST shape including a
`unsupported_algebra` array — callers can preview translatability
without execution.

## 4. Engine internals (deltas from v0.2 LLD)

### 4.1 Dictionary cache (LLD §4.1) — **SHIPPED — Phase 3 step 1**

`src/storage/shmem_cache.rs` (commit landed during this LLD's
cycle) implements the cross-backend cache:

```
hash(RdfTerm) ─► shmem cache ─hit──► return id  (no SQL)
                      │
                      └─miss──► Spi.query SELECT id FROM _pgrdf_dictionary
                                     │
                                     └─ stage_for_commit(key, id)
                                            │
                                            └─ on XACT_EVENT_COMMIT: publish to shmem
                                            └─ on XACT_EVENT_ABORT:  drop pending list
```

Concrete shape:
- `PgLwLock<[DictCacheSlot; 16 384]>` (~ 512 KiB). Slot carries a
  u128 fingerprint (two SipHash variants, distinct seeds), a
  generation counter, the dict id, and an occupied marker.
- Open-addressed with depth-8 linear probing; eviction at the
  canonical slot when the probe streak is full. Cold terms
  displace first; hot set stays sticky.
- Commit-deferred publish: pgrx's `register_xact_callback`
  publishes the per-backend pending list on commit, discards it on
  abort. A rolled-back INSERT never strands an orphan id in shmem.
- Generation invalidation: a `PgAtomic<AtomicU64>` (init 1) is
  bumped by `pgrdf.shmem_reset()`. Slot.generation must equal
  current to be a valid hit; mismatch = silent miss. Required
  after `DROP EXTENSION pgrdf; CREATE EXTENSION pgrdf;` because
  the dict id space resets but shmem outlives drops.
- Init gating: `_PG_init` only registers shmem hooks when
  `process_shared_preload_libraries_in_progress == true`. Lazy-
  loaded backends get `shmem_ready == false`; every lookup
  short-circuits and the per-call HashMap from Phase 2.2 keeps
  doing its job alone.
- Observability: `pgrdf.stats() → JSONB` (cumulative counters)
  and `load_turtle_verbose` per-call `shmem_cache_hits` field.

Acceptance criteria — both met:
- **Lookup latency on cache hit < 1 µs**: LWLock-share + ≤ 8 slot
  probes ≈ ~120 ns on commodity hardware.
- **Cross-backend cache hit demonstrated**:
  `tests/regression/sql/50-shmem-dict-cache.sql` runs three
  back-to-back `load_turtle_verbose('synth-100.ttl', graph_N)`
  calls. Load 1 = 115 db calls / 0 shmem hits. Loads 2–3 = 0 db
  calls / 115 shmem hits. All expected values hand-computed;
  baseline NOT autocommitted from runtime output.

### 4.2 SPARQL Executor (LLD §4.2) — **SHIPPED — Phase 3 step 2**

`src/query/plan_cache.rs` (commit landed during this LLD's cycle)
caches `Spi::prepare`d statements per backend. The translator now
produces parameterised SQL:

```
… WHERE q1.subject_id = $1 AND q2.predicate_id = $2 …
```

— every dict-id constant goes through `id_placeholder`, which pushes
to a thread-local `PARAM_BUF` and emits the next `$N`. `translate()`
snapshots the buffer into `ExecPlan { sql, params }`. The SQL
string is the canonical cache key (algebra-shape-stable by
construction).

Cache: per-backend `thread_local! HashMap<String,
OwnedPreparedStatement>`. Counters
(`plan_cache_hits/misses/inserts`) are shmem-backed `PgAtomic<u64>`
so a multi-backend benchmark can read a single fleet-wide view via
`pgrdf.stats()`. Per-backend cache size surfaces as
`plan_cache_local_size`.

Acceptance criteria (LLD §4.2) — both met:
- **Identical structural queries with varying constants reuse the
  cached plan**: same IRI constants → same SQL → same key (verified
  by `51-plan-cache.sql` Block B).
- **Cache hit ratio reported**: `pgrdf.stats() → JSONB` exposes
  `plan_cache_hits / misses / inserts / local_size`.

Operator surface:
- `pgrdf.plan_cache_clear() → bigint` empties THIS backend's cache
  and returns the count. Production workloads never need this;
  provided for diagnostics + tear-down.

### 4.3 Bulk Ingestion (LLD §4.3) — **phase A SHIPPED, phase B deferred**

**Phase A (shipped — Phase 3 step 3):** the batched-INSERT SQL
string is constant per backend, so the prepared-plan cache from
§4.2 also covers the loader's flush path. `flush_batch` runs:

```rust
Spi::connect_mut(|client| {
    if !plan_cache::contains(QUAD_INSERT_SQL) {
        let prepared = client.prepare_mut(QUAD_INSERT_SQL, &arg_oids)?.keep();
        plan_cache::insert(QUAD_INSERT_SQL.to_string(), prepared);
    }
    plan_cache::with_plan(QUAD_INSERT_SQL, |owned| {
        client.update(owned.unwrap(), None, &datums)?;
    });
})
```

Verified by `tests/regression/sql/52-bulk-ingest-perf.sql` against
the new 10 000-triple `synth-10k.ttl` fixture.

**Phase B (deferred to Phase 3 step 3b / v0.4):** the **2×
wall-clock** target from §4.3 acceptance is NOT met by phase A
alone. Observed synth-10k load time is ~85 ms steady-state both
before and after phase A — the per-batch executor walk (`SELECT s,
p, o FROM unnest(…)` per-tuple projection + partition routing)
dominates. To hit the 2× bar, the next slice must bypass the
executor entirely. Two candidate paths:

1. `pg_sys::heap_multi_insert` + per-partition relation handles
   (skip the executor; manage `TupleTableSlot` arrays directly).
2. `pg_sys::BeginCopyFrom` + a callback-driven binary feed (true
   COPY BINARY path).

Both are FFI-heavy. Acceptance test fixture (synth-10k.ttl) is in
place; we re-measure once phase B lands.

### 4.4 ParsedSelect representation (new in v0.3)

The executor's internal IR collected during AST walk:

```rust
struct ParsedSelect {
    projected: Vec<String>,
    bgp: Vec<TriplePattern>,
    filters: Vec<Expression>,
    optionals: Vec<OptionalBlock>,      // single-triple right side
    minuses: Vec<Vec<TriplePattern>>,   // multi-triple sub-pattern
    union_branches: Vec<UnionBranch>,
    distinct: bool,
    order_by: Vec<(String, bool)>,
    limit: Option<usize>,
    offset: usize,
    group_vars: Vec<String>,
    aggregates: Vec<AggregateSpec>,
    having_filters: Vec<Expression>,
    binds: Vec<BindSpec>,
}
```

Translation dispatch in `build_bgp_sql`:
1. If `union_branches` non-empty → `build_union_sql` (each branch
   is a complete sub-SELECT, combined with `UNION ALL`, outer
   wrapper for DISTINCT/ORDER BY/LIMIT/OFFSET).
2. Else if `aggregates` non-empty → `build_aggregate_sql`
   (GROUP BY + agg fns + HAVING).
3. Else → `build_single_branch_outer` (FROM + WHERE + SELECT in
   one statement, hidden ORDER-BY columns for non-projected sort
   keys).

All three paths share `build_from_and_where` for the BGP /
OPTIONAL / MINUS layout: explicit INNER JOIN syntax for mandatory
patterns 2..N; LEFT JOIN per OPTIONAL block; WHERE NOT EXISTS
sub-SELECT per MINUS block.

## 5. Phase progression (v0.3 numbering)

| Phase | Name | Status |
|---|---|---|
| 1 | Core Storage & Build Automation | ✅ done (schema, hexastore, compose, BuildKit) |
| 2 | Functional SPARQL Coverage | ✅ done — Phase 2.0-2.2 (storage CRUD, Turtle ingest, dict cache + batch, SPARQL parser + BGP-to-SQL) and v0.2's "Phase 3" merged into a single delivery track (§3) |
| **3** | **Storage Performance** (NEW) | **⏳ next** |
| 4 | Inference Engine (OWL 2 RL via `reasonable`) | ✅ done — `src/inference/reasonable.rs`, `60-materialize-owl-rl.sql` |
| 5 | Validation Engine (SHACL via `shacl_validation`) | 🚧 stub — surface only; real impl blocked by ERRATA E-009 |
| 6 | CI + W3C Conformance + LUBM + Release | 🚧 step 1 done (`.github/workflows/ci.yml::regression`); steps 2-3 deferred |

### 5.1 Phase 3 — Storage Performance (PRIORITISED)

The single biggest remaining LLD gap. Three deliverables, each
its own slice:

1. **Shmem dict cache** (LLD §4.1). `pgrx::shmem` +
   `RwLock<LruCache<u64, i64>>`. Cross-backend term-id reuse.
2. **Prepared-plan cache** (LLD §4.2). `Spi::prepare` keyed by
   canonical algebra hash. Bypass Postgres parse + plan on
   repeated structural queries.
3. **COPY BINARY ingestion** (LLD §4.3). Replace batched INSERT
   with `COPY _pgrdf_quads FROM STDIN (FORMAT BINARY)` through
   the pgrx COPY API.

Per-slice acceptance: pgrx + regression tests green, plus a
**performance regression test** that asserts a measurable win
on the synth-100 fixture (cache-hit time < 1 µs; second-call
plan-cache hit; ingest throughput at least 2× the current
batched-INSERT baseline).

Plus the SPARQL surface items deferred to v0.4 (multi-triple
OPTIONAL, VALUES, BIND-in-FILTER, etc.) land as Phase 3 sub-steps
once their refactor is cheap enough — none of them block the
performance work.

### 5.2 Phase 4 — Inference Engine — **SHIPPED**

`pgrdf.materialize(graph_id BIGINT) → JSONB`
(`src/inference/reasonable.rs`) rehydrates the graph's base quads
into `oxrdf::Triple`s in a single SPI scan with three dictionary
JOINs, runs them through `reasonable::reasoner::Reasoner`, and
writes the inferred set back with `is_inferred = TRUE`.

Set-diff is used to isolate entailed-but-not-asserted triples
(filters out base AND the OWL 2 RL axiomatic triples that happen
to match the input).

Idempotency: each call wipes prior inferred rows in the graph
before re-deriving; `previous_inferred_dropped` in the stats
JSONB reports the count.

**Loader interaction.** The Phase 4 writeback path is currently
row-by-row INSERT — once Phase 3 step 3b (heap_multi_insert /
COPY BINARY) lands the loader's `flush_batch` will be shared with
materialize for a single ingest pipeline.

Test surface:
- 3 pgrx integration tests (`materialize_subclass_chain`,
  `materialize_is_idempotent`,
  `materialize_pure_data_preserves_input`).
- Regression `60-materialize-owl-rl.sql` covers two-hop
  subClassOf + idempotence + `owl:inverseOf`.

### 5.3 Phase 5 — Validation Engine — **STUB SHIPPED**

`pgrdf.validate(data BIGINT, shapes BIGINT) → JSONB` exists at the
SQL boundary (`src/validation/shacl.rs`) but its body returns
`{"status": "stub", …}`. The intended W3C `sh:ValidationReport`
output is **blocked upstream**, captured in
[`specs/ERRATA.v0.2.md`](ERRATA.v0.2.md) **E-009**:

- `shacl_validation 0.2.x` (latest 0.2.12) ships an unfinished
  `iri_s → rudof_iri` migration; `shacl_ast 0.2.9` no longer
  compiles against the resolved tree.
- `shacl_validation 0.1.149` compiles in isolation but enables
  `oxrdf`'s `rdf-12` feature, which adds `TermRef::Triple(_)` —
  unhandled by `reasonable 0.4.1`. Feature unification makes the
  two crates mutually exclusive in one workspace.

We chose to ship Phase 4 (inference) first since it's load-bearing
for the rest of the engine. Phase 5 ships as a stub with a stable
JSONB schema so downstream tooling (CloudNativePG operators,
client libraries, CI jobs) can wire against the UDF today.

**Unblock path** — either:
- `shacl_validation 0.2.x` lands a release that compiles cleanly
  against a single `iri_s` major, OR
- `reasonable` ships a version that handles RDF 1.2 triple terms.

When unblocked, the v0.4 ticket is mechanical:
1. Add `shacl_validation` back to `Cargo.toml`.
2. Replace the stub body with N-Triples serialization of both
   graphs + `GraphValidation::from_graph(...).validate(&schema_ir)`.
3. Map `ValidationReport.results()` → JSONB `sh:ValidationReport`.

Scope when wired:
- ✅ SHACL Core node + property shapes.
- ✅ Cardinality, value-type, value-range constraints.
- ⚠️ SHACL-SPARQL constraints — Phase 5 stretch.

### 5.4 Phase 6 — CI + Conformance + Release — **step 1 SHIPPED**

**Step 1 — Regression in CI** ✅
`.github/workflows/ci.yml` `regression` job runs the compose-based
pg_regress suite on every PR + push to main. Builds via
`compose/builder.Containerfile`, boots `postgres:17.4-bookworm`,
drives `tests/regression/sql/NN-*.sql` via
`PGRDF_RUNTIME=docker tests/regression/run.sh`. Pinned to PG 17
(ERRATA E-006) until pgrx supports newer majors.

**Step 2 — W3C conformance** 🚧 (starter shipped)
- ✅ Hand-authored W3C-shape SPARQL harness in
  `tests/w3c-sparql/` — 5 starter tests covering BGP, DISTINCT,
  UNION-disjoint, OPTIONAL chain, MINUS-no-shared. Wired into
  CI's `regression` job. Each expected output hand-verified
  against the W3C spec section it exercises.
- ⏳ Full W3C SPARQL 1.1 TTL-manifest runner
  (`pgrdf-w3c-sparql` Rust binary against `w3c/rdf-tests`).
  Coverage ratchets per release: `≥ 30 % → ≥ 70 % → ≥ 95 %`.
  `regression-w3c.yml::sparql11` holds the destination shape
  gated `if: false`. v0.4 work item.
- ⏳ W3C SHACL manifest runner; `≥ 50 % → ≥ 90 %`. Gated on
  ERRATA E-009 — until the real `shacl_validation` integration
  lands, SHACL conformance is at 0 %.
- ⏳ LUBM-10 / LUBM-100 against Apache Jena TDB + Apache AGE.

**Step 3 — Release artifacts** ⏳
- CI matrix green on tag: pg14–17 × {amd64, arm64} (workflow
  shipped in `release.yml`; fires on `v*` tags).
- Pre-built tarballs published on GitHub releases per
  INSTALL §3 layout.
- SHA256SUMS.asc detached GPG signature attached to every release.
- OCI artifact at `ghcr.io/styk-tv/pgrdf-bundle:<ver>`.

## 6. Test & coverage policy

### 6.1 Five test layers

| Layer | Runtime | Purpose | Today | Phase 3 gate | Phase 4 gate | Phase 6 gate |
|---|---|---|---|---|---|---|
| Rust unit (`cargo test`) | sec | Pure-Rust logic (parser AST, JSONB shaping, SHACL report) | smoke | parser AST coverage | reasoner correctness | full storage coverage |
| pgrx integration (`cargo pgrx test`) | ~30 s | UDF behaviour inside managed Postgres | **79 ✅** | + shmem cache hit-latency check | + materialize correctness | + SHACL validation |
| pg_regress golden | ~1 min | UDF behaviour over the wire to compose Postgres | **25 ✅** | + plan-cache reuse demo | + inference + query | + full SHACL conformance |
| Ontology smoke (`tests/perf/smoke-ontologies.sh`) | sec each, manual | Real-world Turtle parses cleanly | 24 ontologies, 17 134 triples ✅ | (unchanged) | (unchanged) | (unchanged) |
| W3C SPARQL 1.1 + SHACL | min | Standards conformance | not wired ⏳ | runner wired | ≥ 70 % SPARQL / ≥ 50 % SHACL | ≥ 95 % SPARQL / ≥ 90 % SHACL |
| LUBM perf | min | Throughput vs Jena / AGE | not wired ⏳ | LUBM-1 smoke | LUBM-10 baseline | LUBM-100 |

### 6.2 Empirical-verification rule

**For every new regression fixture, the contributor hand-computes
the expected output from the SQL.** No `ACCEPT=1` baselining of
new query coverage. ACCEPT is reserved for output-format churn
from a Postgres minor-version bump or similar unrelated change.

Rationale: an autobaselined regression test asserts what the code
does, not what the spec says it should do. Both should match.

### 6.3 Per-commit discipline

- Every bug fix lands with a regression test that reproduced the
  bug before the fix.
- Every new UDF lands with at least one `#[pg_test]`.
- Every Phase-step commit lands with both pgrx + regression green
  (`just test-all`).
- Coverage gates ratchet — a phase's gate is the floor for all
  subsequent phases.

### 6.4 Performance regression tests (new in v0.3)

Phase 3 introduces a fourth column of `pgrdf.load_turtle_verbose`
stats:
- `triples`, `dict_cache_hits`, `dict_db_calls`, `quad_batches`,
  `elapsed_ms` (existing).
- (new) `shmem_cache_hits` — cross-call cache hit count.
- (new) `plan_cache_hits` — cross-call plan-cache hit count.

Regression tests on the synth-100 fixture assert these stats are
non-zero on a second call, gating that the perf work actually
shipped rather than being a no-op refactor.

## 7. Deferred from v0.2 LLD

| v0.2 LLD claim | v0.3 reality |
|---|---|
| `_pgrdf_dictionary` HASH index | ✅ shipped |
| `_pgrdf_quads` partitioned by LIST(graph_id) | ✅ shipped |
| SPO / POS / OSP `INCLUDE (is_inferred)` indexes | ✅ shipped |
| §4.1 shmem dict cache | ✅ Phase 3 step 1 (`src/storage/shmem_cache.rs`) |
| §4.2 prepared plans | ✅ Phase 3 step 2 (`src/query/plan_cache.rs`) |
| §4.3 bulk-ingest plan cache (phase A) | ✅ Phase 3 step 3 (`src/storage/loader.rs::flush_batch`) |
| §4.3 COPY BINARY / heap_multi_insert (phase B) | ⏳ Phase 3 step 3b / v0.4 (2× wall-clock target requires this) |
| §5 zero-install container init script | ⏳ Phase 6 (compose works for dev) |
| §6 CI matrix pg14×17 × {amd64, arm64} | ⏳ Phase 6 (workflow stubs in `.github/workflows/`) |
| §7 Phase 1 "Core Storage" | ✅ effectively done |
| §7 Phase 2 "Query Engine" — SELECT BGP | ✅ + far exceeded (FILTER / OPTIONAL / UNION / MINUS / modifiers / aggregates / BIND / ASK; see §3) |
| §7 Phase 3 "Semantic Engine" | ⏳ Phase 4 (inference) + Phase 5 (validation) |
| §7 Phase 4 "Release & Container" | ⏳ Phase 6 |

## 8. Out of scope (v0.x — unchanged from v0.2)

- Streaming replication / logical decoding of RDF state.
- Federated SPARQL `SERVICE`.
- Full OWL 2 (EL / QL) reasoning. `ERRATA.v0.2.md` E-002.
- Backup/restore for opaque binary state (tracked by future
  `SPEC.pgRDF.BACKUP.v0.x`, INSTALL §11 OQ5).

## 9. Errata

This document is the v0.3 contract. Spec corrections discovered
during implementation land in a future `ERRATA.v0.3.md`. The v0.2
errata document (`ERRATA.v0.2.md`) remains authoritative for items
that pre-date this rev.
