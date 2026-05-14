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

### 4.1 Dictionary cache (LLD §4.1) — **NOT yet shipped**

v0.2 specified a process-instance-wide
`RwLock<LruCache<u64, i64>>` keyed by RDF-term hash, backed by
`pgrx::shmem`. Today's implementation is **per-call HashMap**
inside `src/storage/loader.rs::ingest_turtle_with_stats` — gives
within-call benefit but doesn't survive across calls or backends.

Forward target (unchanged from v0.2):

```
hash(RdfTerm) ─► shmem cache ─hit──► return id
                      │
                      └─miss──► Spi.query SELECT id FROM _pgrdf_dictionary
                                     │
                                     └─ insert into shmem ─► return id
```

Acceptance criterion: lookup latency on cache hit < 1 µs; cross-
backend cache hit demonstrated by a multi-backend benchmark.

### 4.2 SPARQL Executor (LLD §4.2) — partial

v0.2 specified `Spi::prepare`-cached parameterised execution plans
keyed by a canonical algebra hash. Today's executor builds a
**dynamic SQL string per call** and runs `Spi::connect_mut(|c|
c.update(sql, None, &[]))`. Postgres re-parses + re-plans every
call.

The translator already produces algebra-stable SQL up to the
inlined constant dict ids; the prepared-plan slice swaps the
constants for `$1..$N` parameters and caches the prepared
statement.

Acceptance criterion: identical structural queries with varying
constants reuse the cached plan; cache hit ratio reported via a
new `pgrdf.stats()` UDF.

### 4.3 Bulk Ingestion (LLD §4.3) — **NOT yet shipped as COPY BINARY**

v0.2 specified `COPY _pgrdf_quads FROM STDIN (FORMAT BINARY)`.
Today's loader uses batched multi-row INSERT via
`unnest($1::bigint[], $2::bigint[], $3::bigint[])` with
BATCH_SIZE = 1000.

The batched-INSERT path gets us ~50× faster than row-by-row
INSERT, but COPY BINARY is typically another 2–5× faster on
commodity hardware. Worth re-measuring against the synth-100
and smoke-ontologies fixtures before committing to the rewrite.

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
| 4 | Inference Engine (OWL 2 RL via `reasonable`) | ⏳ |
| 5 | Validation Engine (SHACL via `shacl_validation`) | ⏳ |
| 6 | W3C Conformance + LUBM Perf + Release | ⏳ |

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

### 5.2 Phase 4 — Inference Engine

`pgrdf.materialize(graph_id BIGINT)` streams the graph's quads
through `reasonable`'s OWL 2 RL evaluator and writes inferred
quads back into the same partition with `is_inferred = TRUE`.

Streaming uses the COPY BINARY path from Phase 3 step 3 (so
Phase 4 effectively depends on that slice landing first).

Idempotency: repeated calls re-derive from scratch; covering
indexes prevent duplicates.

### 5.3 Phase 5 — Validation Engine

`pgrdf.validate(data BIGINT, shapes BIGINT) → JSONB` returns a
W3C-conformant `sh:ValidationReport` via `shacl_validation`.

Scope:
- SHACL Core node + property shapes.
- Cardinality, value-type, value-range constraints.
- SHACL-SPARQL constraints — Phase 5 stretch.

### 5.4 Phase 6 — W3C Conformance + Release

- W3C SPARQL 1.1 manifest runner wired into CI; coverage targets
  ratchet per release: ≥ 30 % → ≥ 70 % → ≥ 95 %.
- W3C SHACL manifest runner; ≥ 50 % → ≥ 90 %.
- LUBM-10 / LUBM-100 against Apache Jena TDB + Apache AGE.
- CI matrix green on tag: pg14–17 × {amd64, arm64}.
- Pre-built tarballs published on GitHub releases per
  INSTALL §3 layout.
- SHA256SUMS.asc detached GPG signature attached to every release.

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
| §4.1 shmem dict cache | ⏳ Phase 3 (per-call HashMap is the stepping stone) |
| §4.2 prepared plans | ⏳ Phase 3 (dynamic SQL is current) |
| §4.3 COPY BINARY | ⏳ Phase 3 (batched INSERT via unnest is current) |
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
