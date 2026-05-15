# 10 — Roadmap

> **v0.3 LLD is the authoritative shipped contract**
> ([`specs/SPEC.pgRDF.LLD.v0.3.md`](../specs/SPEC.pgRDF.LLD.v0.3.md) §5).
> Phase numbering on this page tracks the v0.3 phase map verbatim:
> Phase 1 done, Phase 2 (Functional SPARQL Coverage) done through
> sub-steps 2.0 / 2.1 / 2.2, Phase 3 (Storage Performance) steps 1-2
> shipped + step 3 phase A shipped, Phase 4 (Inference) shipped,
> Phase 5 (Validation) stub shipped, Phase 6 (CI + Conformance +
> Release) step 1 shipped.
>
> **Forward-look:**
> [`specs/SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md)
> is the authoritative-in-progress contract for the v0.4 cycle
> (named-graph scoping, SPARQL UPDATE, lifecycle UDFs, CONSTRUCT,
> property paths, plus the SPARQL backlog deferred from v0.3 §3 —
> SHACL real-impl already landed on `main`). The next forward-look
> beyond v0.4 lives in
> [`specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md`](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md).

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

- ✅ pgrx 0.16 scaffold compiles on PG 14–17. PG 18 support has
      landed upstream in pgrx 0.18.0 (2026-04-17), but adoption is
      deferred to v0.4: 0.18.0 still trips `E0716` in its
      `impl_table_iter` macro on every Rust stable/nightly we tested,
      and its single-pass schema-gen migration (`pgrx_embed` removal,
      `crate-type` change) is a non-trivial breaking edit. See
      `specs/ERRATA.v0.2.md` E-006 (re-checked 2026-05-14).
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

## Phase 2 — Functional SPARQL Coverage ✅

Outcome: SPARQL SELECT queries cover the practically-useful surface
end-to-end; ingestion is fast enough to load real-world ontologies.
Phase 2 split into three sub-phases (2.0 storage CRUD, 2.1 Turtle
ingest, 2.2 SPARQL parser/executor) plus an extended-surface
deliverable track inside 2.2 that landed steps 1-12 below.

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

(Phase 3 storage-performance gates are tracked under
[Phase 3 — Storage Performance](#phase-3--storage-performance--steps-1-2-shipped-step-3-phase-a-shipped)
below, not here. Phase 2.2 closes with the SPARQL parser / executor
landing; perf work picks up under its own phase per v0.3 LLD §5.)

### Phase 2.2 (extended) — SPARQL surface deliverables ✅

Sub-track inside Phase 2.2 that extended `pgrdf.sparql` from the
v0.2 LLD's minimal "SELECT … WHERE { BGP }" toward a practically-useful
SPARQL 1.1 surface, in tight slices each shipping with pgrx +
regression coverage. (Phase 3 in the v0.3 LLD is **Storage
Performance** — see the next section. The "extended SPARQL surface"
label that previously hung off this table was pre-v0.3 framing and
has been retired.)

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

**SPARQL surface declared substantively complete with step 12.** The
backlog below (every item deferred to v0.4 per
[`SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md))
does not block Phase 3 (Storage Performance) of the v0.3 LLD:

- ⏳ `GRAPH { … }` named-graph clause — needs a graph IRI → graph_id
      mapping (schema change). LLD v0.4 §3.
- ⏳ Multi-triple OPTIONAL — relax the current single-triple
      restriction via a derived-table refactor inside the LEFT JOIN.
      (Multi-triple MINUS shipped step 11.) LLD v0.4 §11.
- ⏳ Arithmetic in FILTER (`?a + ?b > 30`), `BIND` inside FILTER,
      `SUBSTR`, aggregates-over-UNION. LLD v0.4 §11.
      (`lang(?v)` / `datatype(?v)` and the `STRLEN` / `CONTAINS` /
      `STRSTARTS` / `STRENDS` surface shipped step 9; `BIND (expr AS ?v)`
      for projection shipped step 10; type-aware `MIN`/`MAX` over
      `xsd:numeric` shipped post-step-12 — translator slice
      `7de9c17`.)
- ⏳ Type-aware ORDER BY (sort numeric literals numerically rather
      than as strings). LLD v0.4 §11.
- ⏳ `VALUES (?x ?y) { … }`. LLD v0.4 §11.
- ⏳ Property paths beyond simple sequence (`*`, `+`, `?`, `^`,
      alternation). Simple sequence already works because spargebra
      desugars `:a/:b` into a BGP chain. LLD v0.4 §7.
- 🚧 `CONSTRUCT` — slice 59 landed (foundation, constant-only
      templates); slice 58 landed (variable substitution — subject /
      predicate / object positions); slice 57 landed (blank-node
      template positions with per-solution fresh-label minting +
      within-solution label sameness per W3C SPARQL 1.1 §16.2;
      single-triple scope); slice 56 landed (multi-triple templates:
      N-triple templates emit N rows per solution, with blank-node
      labels SHARED across all N template triples within the same
      solution; empty templates `{ }` reject cleanly); ✅ slice 55
      landed (GRAPH-scoped WHERE: `GRAPH <iri> { … }` and
      `GRAPH ?g { … }` inside the WHERE block, composing with all
      prior template surfaces; variable-GRAPH binds `?g` to the
      source graph IRI per solution; default-graph quads excluded
      per W3C SPARQL 1.1 §13.3 — the JOIN to `_pgrdf_graphs` now
      carries `g{S}.graph_id <> 0`, which also corrected the
      slice-79 / slice-87 SELECT-side latent bleed); ✅ slice 54
      landed (CONSTRUCT WHERE shorthand: `CONSTRUCT WHERE { pattern }`
      ≡ `CONSTRUCT { pattern } WHERE { pattern }` per W3C SPARQL 1.1
      §16.2.4; pure-BGP-only, blank-node-free pattern; spargebra
      populates `template` from the BGP at parse so the shorthand
      reuses the multi-triple emission path; composite patterns
      (FILTER/OPTIONAL/UNION/MINUS/GRAPH/BIND/VALUES) reject at parse
      time, blank nodes reject semantically with the W3C-citing
      message). ✅ slice 53 landed (**round-trip preservation**:
      `pgrdf.put_construct_row(row JSONB, graph_id BIGINT DEFAULT 0)`
      and `pgrdf.put_construct_rows(rows JSONB[], graph_id BIGINT
      DEFAULT 0)` re-ingest captured construct rowsets into the
      hexastore, closing LLD v0.4 §6.3's round-trip acceptance
      criterion; typed literals, language tags, and within-batch
      blank-node label joining are all preserved; re-ingestion is
      idempotent via `WHERE NOT EXISTS`; NULL array input is a no-op
      so the `(SELECT array_agg(j) FROM pgrdf.construct(...))`
      idiom works for empty-result queries too).
      `pgrdf.sparql_parse` enrichment (slice 50) still pending.
      `DESCRIBE` carried forward. (`ASK` shipped step 12.) LLD v0.4
      §6.

---

## Phase 3 — Storage Performance 🚧 (steps 1-2 shipped, step 3 phase A shipped)

Outcome: shmem-resident dictionary cache + prepared-plan cache +
bulk-ingest primitive — tracks v0.3 LLD §5.1 / §4.1 / §4.2 / §4.3.

Gates:
- ✅ **Step 1 — Shmem dictionary cache (LLD §4.1)** —
      `PgLwLock<[Slot; 16 384]>` cross-backend cache with u128
      fingerprint, commit-deferred publish, generation invalidation.
      Per-call `load_turtle_verbose.shmem_cache_hits` and cumulative
      `pgrdf.stats()` counters; regression `50-shmem-dict-cache.sql`
      asserts 100 % shmem hit rate on the second load of
      `synth-100.ttl`. Edge-cases locked by
      `63-shmem-reset-invalidation.sql` (slice #61) — `shmem_reset()`
      generation bump + slot-mismatch read-as-cold contract.
- ✅ **Step 2 — Prepared-plan cache (LLD §4.2)** — parameterised
      SPARQL SQL + per-backend `OwnedPreparedStatement` cache keyed
      by the SQL string. `pgrdf.stats()` exposes
      `plan_cache_hits / misses / inserts / local_size`. Operator
      hook: `pgrdf.plan_cache_clear()`. Regression
      `51-plan-cache.sql` asserts the hit / miss / parametric-reuse
      arithmetic for three workload shapes; edge-cases locked by
      `64-plan-cache-clear.sql` (slice #60) — returned-count
      semantics, idempotent-at-zero, post-clear size invariant.
- 🚧 **Step 3 — COPY BINARY ingestion (LLD §4.3)** —
      - ✅ **Phase A**: prepared `INSERT … unnest(…)` cached
        per-backend, reused across batches and across loads.
        Saves one parse+plan per batch (~100–500 µs each).
        Verified by `52-bulk-ingest-perf.sql` on synth-10k.ttl.
      - ⏳ **Phase B** (deferred to v0.4 per
        [`SPEC.pgRDF.LLD.v0.4.md §12`](../specs/SPEC.pgRDF.LLD.v0.4.md)):
        the 2× wall-clock target from LLD §4.3 acceptance is not
        met by phase A alone — the per-tuple executor walk
        dominates. Candidate paths: `pg_sys::heap_multi_insert` per
        partition, or `BeginCopyFrom` + binary callback. Both
        FFI-heavy.
- ⏳ W3C SPARQL 1.1 manifest runner wired into CI; coverage target
      `≥ 30 %` pass for the v0.3 Phase 6 step 2 gate (LLD §5.4).
      Hand-authored W3C-shape harness (23 tests, lock-in slice #55)
      stands in until the full TTL-manifest runner lands.

---

## Phase 4 — Inference Engine ✅ (shipped; loader-writeback deferred)

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
      `tests/regression/sql/60-materialize-owl-rl.sql`. Round-trip
      to SPARQL locked by `61-materialize-then-sparql.sql`;
      zero-triple edge locked by `62-materialize-empty.sql` (slice
      #62).
- ⏳ Reasoner-coverage fixture (e.g. pizza ontology subset) with a
      golden expected-closure diff. Deferred — current regression
      uses minimal hand-authored TBoxes.
- ⏳ Loader-side writeback via `flush_batch` (depends on Phase 3
      step 3 phase B shipping the bulk-INSERT primitive in v0.4 per
      [`SPEC.pgRDF.LLD.v0.4.md §12`](../specs/SPEC.pgRDF.LLD.v0.4.md)).

---

## Phase 5 — Validation Engine ✅ (v0.4)

Outcome: SHACL validation works against real shapes graphs. Tracks
LLD v0.3 §5.3 and LLD v0.4 §9.

Gates:
- ✅ `pgrdf.validate(data BIGINT, shapes BIGINT) → JSONB` —
      real W3C-shape SHACL Core report, replacing the v0.3 stub.
      Backed by `shacl 0.3.x` (rudof project). Verified by
      `70-validate-stub.sql` (basic shape) and
      `71-shacl-real.sql` (LLD §9 violations).
- ✅ Upstream-dep unblock — `shacl 0.3.1` consolidated the
      `iri_s` → `rudof_iri` migration; the `rdf-12 /
      TermRef::Triple` half cleared via the patched
      `styk-tv/reasonable` fork branch `rdf12-passthrough`
      (ERRATA.v0.4 E-011). Fork wired via `[patch.crates-io]`
      until upstream `gtfierro/reasonable` merges.
- ⏳ W3C SHACL conformance manifest runner — paired with Phase 6,
      targets v0.5 (see [`SPEC.pgRDF.LLD.v0.5-FUTURE.md §6`](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md)).

---

## Phase 6 — CI + Conformance + Release 🚧 (step 1 shipped)

Outcome: pgRDF is consumable by external operators (CloudNativePG,
StackGres) following INSTALL spec methodology. Benchmarked. Tracks
LLD v0.3 §5.4.

**Step 1 — Regression in CI** ✅
- `.github/workflows/ci.yml` `regression` job runs the
  compose-based pg_regress suite on every PR + push to main.
  Pinned to PG 17 today (compose pin per ERRATA E-006).

**Step 2 — W3C conformance** 🚧 (starter shipped, expanded II)
- ✅ `tests/w3c-sparql/` hand-authored harness — **23 tests** across
  three expansion waves (5 starter + 8 expanded + 5 expanded II +
  3 essentials + 2 translator-fix gates), covering BGP, DISTINCT,
  UNION, OPTIONAL, MINUS, FILTER (isIRI/REGEX/IN/numeric),
  aggregates + HAVING, ORDER BY DESC, LIMIT/OFFSET, BIND/CONCAT,
  ASK true/false, STRLEN, LANG, UCASE, BOUND-after-OPTIONAL,
  STR(?iri), inline HAVING-aggregate, type-aware MIN/MAX. Plus
  3 LUBM-shape correctness gates in `tests/perf/lubm-shape/`.
  Bash runner; runs alongside `tests/regression/` in the same CI
  job. Each expected output cites the W3C spec section it exercises.
  Justfile entry points (`just test-w3c`, `just test-lubm`,
  `just test-conformance`) added in slice #55.
- ⏳ Full W3C TTL-manifest runner against `w3c/rdf-tests`. The
  `pgrdf-w3c-sparql` Rust binary placeholder in
  `regression-w3c.yml::sparql11` (gated `if: false`) is the
  destination shape; lands as v0.4.
- ⏳ W3C SHACL manifest runner. Real SHACL output landed in v0.4
  via ERRATA.v0.4 E-011; the manifest runner is the remaining
  half and targets v0.5 (per
  [`SPEC.pgRDF.LLD.v0.5-FUTURE.md §6`](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md)).
- ⏳ Coverage targets ratchet per release:
  SPARQL `≥ 30 % → ≥ 70 % → ≥ 95 %`; SHACL `≥ 50 % → ≥ 90 %`.

**Step 3 — Release artifacts** ⏳
- `.github/workflows/release.yml` already builds and packages on
  `v*` tags; fires the first official release once step 2 lands.
  Matrix is `{14,15,16,17} × {amd64, arm64}` = 8 tarballs per cut
  (PG 18 deferred per ERRATA E-006, slice #36 audit).
- LUBM-100 results in `target/perf-report.json` compared against
  Apache Jena TDB and Apache AGE.
- OCI artifact published at `ghcr.io/styk-tv/pgrdf-bundle:<ver>`
  (INSTALL §11 OQ1).
- INSTALL §12 conformance test in CI against a fresh K8s cluster
  (kind or k3s).
- SHA256SUMS is wired in `release.yml` at both per-tarball and
  aggregate levels (slice #28 audit; supersedes the older slice #36
  "not yet wired" note). The detached GPG signature
  `SHA256SUMS.asc` (INSTALL OQ4) is **deferred to v0.4** — no
  `GPG_PRIVATE_KEY` secret or release-signing key is yet provisioned
  for the workflow. v0.3 ships SHA256SUMS-only integrity; the `.asc`
  follow-up requires sourcing a signing key, publishing the public
  half, and wiring the secret. See `docs/09-release.md` "Aggregate
  checksums" for the consumer-side verification recipe.
- License attribution surface (Apache 2.0 / 2026) declared at
  repo root; NOTICE distribution in the release tarball flagged
  as workflow follow-up (slice #36 adjacent finding).
- MSRV declared `rust-version = "1.91"` in `Cargo.toml` (slice
  #49).
- Target gates: W3C SPARQL 1.1 ≥ 95 % pass; SHACL ≥ 90 % pass.
  Real SHACL output landed in v0.4 (ERRATA E-011); the SHACL
  manifest gate is the remaining lever and targets v0.5 per
  [`SPEC.pgRDF.LLD.v0.5-FUTURE.md §6`](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md).

---

## v0.4 — next milestone (forward-looking)

v0.4 is the next major cut, drafted in
[`SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md).
What follows summarises the six major tracks — the full contract
lives in the spec. Acceptance criteria, schema deltas, and
translator-level wiring are NOT duplicated here; this section is a
navigation aid only.

### Track 1 — Named-graph scoping + IRI mapping ✅ (Phase A countdown slices 120 → 110 shipped)
`GRAPH { … }` SPARQL surface plus a new `_pgrdf_graphs` system table
mapping graph IRIs to the existing integer `graph_id` (LIST-partition
key of `_pgrdf_quads`). `GRAPH ?g { … }` projects `?g` as the IRI,
not the integer. All four §3.4 acceptance criteria verified
end-to-end. See
[LLD v0.4 §3](../specs/SPEC.pgRDF.LLD.v0.4.md#3-named-graph-scoping-and-iri-mapping-new).
Phase A continues with the docs-sync + close-out slices 109 → 100
toward a v0.4.1 tag.

- ✅ **Slice 120 — `_pgrdf_graphs` table lands.** Schema in
  [`sql/schema_v0_4_0_graphs.sql`](../sql/schema_v0_4_0_graphs.sql),
  wired via the second `extension_sql_file!` in
  [`src/lib.rs`](../src/lib.rs), seed row `(0, 'urn:pgrdf:graph:0')`
  for the default partition. Regression coverage:
  [`tests/regression/sql/72-graphs-table-shape.sql`](../tests/regression/sql/72-graphs-table-shape.sql)
  + `#[pg_test]` in
  [`src/storage/graphs.rs`](../src/storage/graphs.rs). No UDF
  surface change; existing `pgrdf.add_graph(id BIGINT)` retains
  its v0.3 signature.
- ✅ **Slice 119 — synthetic-IRI binding for the existing integer
  `pgrdf.add_graph(id)`.** The v0.3 UDF in
  [`src/storage/hexastore.rs`](../src/storage/hexastore.rs) now
  inserts `(id, 'urn:pgrdf:graph:' || id::text)` into
  `_pgrdf_graphs` after creating the partition, wrapped in
  `ON CONFLICT (graph_id) DO NOTHING` so re-calls stay idempotent.
  No signature or return-value change; v0.3 callers automatically
  populate the IRI mapping for every graph they create. Regression
  coverage:
  [`tests/regression/sql/73-add-graph-populates-iri.sql`](../tests/regression/sql/73-add-graph-populates-iri.sql)
  + `#[pg_test] add_graph_populates_synthetic_iri` in
  [`src/storage/graphs.rs`](../src/storage/graphs.rs).
- ✅ **Slice 118 — `pgrdf.add_graph(iri TEXT) → BIGINT` overload.**
  Idempotent on the IRI: a repeat call returns the existing
  `graph_id` without creating a second partition. On a fresh IRI
  the overload auto-allocates the next id (smallest unused positive
  integer via `COALESCE(MAX(graph_id), 0) + 1` under a
  `LOCK TABLE _pgrdf_graphs IN SHARE ROW EXCLUSIVE MODE` to
  serialise concurrent callers), pre-INSERTs the user-supplied IRI
  into `_pgrdf_graphs` (which the slice-119 synthetic-IRI insert
  inside the integer overload then no-ops on via
  `ON CONFLICT (graph_id) DO NOTHING`, preserving the user IRI),
  and re-enters through the integer overload to create the LIST
  partition. Empty / whitespace-only IRI panics with the stable
  `add_graph: iri must be non-empty` prefix. RFC-3987 syntax
  validation deferred to a later slice. Pgrx surfaces both Rust
  functions under the SQL name `add_graph` via
  `#[pg_extern(name = "add_graph")]`; Postgres dispatches on the
  argument types. Regression coverage:
  [`tests/regression/sql/74-add-graph-iri.sql`](../tests/regression/sql/74-add-graph-iri.sql)
  + two `#[pg_test]`s in
  [`src/storage/graphs.rs`](../src/storage/graphs.rs)
  (`add_graph_iri_idempotent` + `add_graph_iri_empty_rejected`).
- ✅ **Slice 117 — `pgrdf.add_graph(id BIGINT, iri TEXT) → BIGINT`
  explicit-binding overload.** Caller supplies both halves;
  idempotent on a matching `(id, iri)`. UPDATEs in place when `id`
  is currently bound to its synthetic placeholder
  `urn:pgrdf:graph:{id}` (the slice-119 seed) and the requested IRI
  is unbound elsewhere — the upgrade path covering
  `add_graph(42)` → `add_graph(42, 'http://example.org/g42')`.
  Panics with the stable `add_graph:` prefix on conflicts:
  `add_graph: graph_id <N> is bound to a different IRI (<existing>)`
  when `id` is bound to a non-synthetic IRI different from the
  request, or
  `add_graph: iri <iri> is bound to a different graph_id (<existing>)`
  when the IRI is already bound to a different graph_id. Negative
  `id` and empty IRI rejected with the same stable prefixes shared
  with the other two overloads. Concurrent writers serialised by
  `LOCK TABLE _pgrdf_graphs IN SHARE ROW EXCLUSIVE MODE` (same idiom
  as slice 118). Regression coverage:
  [`tests/regression/sql/75-add-graph-id-iri.sql`](../tests/regression/sql/75-add-graph-id-iri.sql)
  + four `#[pg_test]`s in
  [`src/storage/graphs.rs`](../src/storage/graphs.rs)
  (`add_graph_id_iri_fresh_pair`,
  `add_graph_id_iri_synthetic_upgrade`,
  `add_graph_id_iri_id_conflict`,
  `add_graph_id_iri_iri_conflict`).
- ✅ **Slice 116 — `pgrdf.graph_id(iri TEXT) → BIGINT` lookup.**
  Read-only resolution of an IRI back to its integer `graph_id`
  in `_pgrdf_graphs`, or `NULL` when the IRI is not bound. Marked
  `#[pg_extern(strict)]` so a NULL argument short-circuits to NULL
  output without invoking the function body; the `&str` body
  therefore never observes a NULL input. The scalar-subquery
  `SELECT (subquery)` wrapper keeps SPI on the "exactly one row"
  path (NULL on miss, id otherwise), the same idiom the IRI-keyed
  `add_graph` overload uses to dodge the
  `SpiTupleTable positioned before the start` empty-result trip.
  No panic on miss — NULL is the documented lookup-miss signal
  (LLD v0.4 §3.2). Regression coverage:
  [`tests/regression/sql/76-graph-id-lookup.sql`](../tests/regression/sql/76-graph-id-lookup.sql)
  + four `#[pg_test]`s in
  [`src/storage/graphs.rs`](../src/storage/graphs.rs)
  (`graph_id_seed_lookup`, `graph_id_after_iri_add`,
  `graph_id_miss_returns_null`,
  `graph_id_null_input_null_output`).
- ✅ **Slice 115 — `pgrdf.graph_iri(id BIGINT) → TEXT` symmetric
  lookup.** Read-only resolution of an integer `graph_id` back to
  its bound IRI in `_pgrdf_graphs`, or `NULL` when the id is not
  bound. Marked `#[pg_extern(strict)]` so a NULL argument short-
  circuits to NULL output without invoking the function body. The
  same scalar-subquery `SELECT (subquery)` wrapper discipline as
  slice 116 keeps SPI on the "exactly one row" path. No panic on
  miss — NULL is the documented lookup-miss signal (LLD v0.4 §3.2).
  Symmetric inverse of slice 116's `pgrdf.graph_id(iri)` — together
  they close the §3.2 UDF surface (all five rows now ✅). Regression
  coverage:
  [`tests/regression/sql/77-graph-iri-lookup.sql`](../tests/regression/sql/77-graph-iri-lookup.sql)
  + five `#[pg_test]`s in
  [`src/storage/graphs.rs`](../src/storage/graphs.rs)
  (`graph_iri_seed_lookup`, `graph_iri_direct_insert_lookup`,
  `graph_iri_miss_returns_null`, `graph_iri_null_input_null_output`,
  `graph_iri_roundtrip`). With slice 115 done, the Phase A §3.2 UDF
  surface is complete (slices 120-115); the SPARQL surface lands
  next (slices 114-110).
- ✅ **Slice 114 — SPARQL `GRAPH <iri> { … }` literal-IRI form
  translation.** The executor's pattern walk now handles
  `GraphPattern::Graph { NamedNode(iri), inner }` by resolving the
  IRI to a `graph_id` via `_pgrdf_graphs.iri` at translate time and
  threading the constraint through `ParsedSelect` /
  `build_from_and_where` so every triple alias inside the GRAPH
  block carries an additional `qN.graph_id = $K` WHERE clause.
  Unresolved IRI binds to the sentinel `-1` (no real partition
  uses that value) ⇒ zero rows, spec-correct "no solutions"; no
  error raised. The parser's `unsupported_algebra` walk now drops
  the "Graph (named graph clause)" tag for the literal-IRI form
  (it walks `inner` so the contained BGP triples are still
  counted); the variable form `GRAPH ?g { … }` keeps a fresh
  `"Graph (variable IRI; slice 113)"` tag. Regression coverage:
  [`tests/regression/sql/78-sparql-graph-literal-iri.sql`](../tests/regression/sql/78-sparql-graph-literal-iri.sql)
  + one `#[pg_test]` (`sparql_graph_literal_iri_scopes_to_graph`
  in [`src/query/executor.rs`](../src/query/executor.rs)).
  Slice-114 limitation (lifted in slice 112): the original
  implementation kept a single graph constraint covering the entire
  single-branch BGP. Slice 112 moved the constraint to per-pattern
  scope so GRAPH composes correctly with OPTIONAL / UNION / MINUS.
- ✅ **Slice 113 — SPARQL `GRAPH ?g { … }` variable form
  translation.** The executor's pattern walk now handles
  `GraphPattern::Graph { Variable(?g), inner }` by recording the
  variable name in `ParsedSelect.graph_var` (or
  `UnionBranch.graph_var`) and threading it into
  `build_from_and_where`, which appends an
  `INNER JOIN pgrdf._pgrdf_graphs g0 ON g0.graph_id = q1.graph_id`
  (exactly one per inner BGP) and adds `qN.graph_id = q1.graph_id`
  for every additional mandatory / OPTIONAL / MINUS alias inside
  the GRAPH block — so a multi-triple inner BGP cannot stitch
  triples from different graphs together. The projection layer
  emits `g0.iri` for the graph var (IRI string, not the integer
  id). INNER JOIN matches W3C SPARQL 1.1 §13.3: only graphs present
  in the IRI mapping bind ?g. The parser's `unsupported_algebra`
  walk drops the "Graph (variable IRI; slice 113)" tag and walks
  `inner` like the literal-IRI form. Regression coverage:
  [`tests/regression/sql/79-sparql-graph-variable.sql`](../tests/regression/sql/79-sparql-graph-variable.sql)
  + one `#[pg_test]` (`sparql_graph_variable_projects_iri` in
  [`src/query/executor.rs`](../src/query/executor.rs)). Slice-113
  limitation (lifted in slice 112): the original implementation
  kept a single graph var covering the entire single-branch BGP.
  Slice 112 moved scope to per-pattern.
- ✅ **Slice 112 — GRAPH composition with OPTIONAL / UNION / MINUS
  across different graph scopes.** Refactored the executor's graph
  constraint from per-`ParsedSelect` (one literal id + one var
  name, shared by the whole single-branch BGP) to per-pattern
  `Option<GraphScope>` carried by each triple, each OPTIONAL
  triple, and each MINUS block. A new `GraphScope` enum holds
  either `Literal(graph_id)` (resolved at translate time) or
  `Variable { name, scope_id }` (with a globally-unique scope_id
  per GRAPH block instance). `build_from_and_where` builds a
  `ScopePlan` describing which Variable scopes need an INNER JOIN
  to `_pgrdf_graphs` (mandatory side) vs LEFT JOIN (OPTIONAL-born
  side), anchors each scope's JOIN to the first BGP alias in scope,
  and emits per-triple `qN.graph_id = …` constraints based on
  scope. Two GRAPH blocks binding the same `?g` are tied together
  with a `g{later}.graph_id = g{anchor}.graph_id` so the projected
  variable stays consistent. The OPTIONAL/MINUS that nest inside a
  GRAPH inherit the outer scope (W3C SPARQL 1.1 §13.3). Coverage in
  [`tests/regression/sql/87-sparql-graph-composition.sql`](../tests/regression/sql/87-sparql-graph-composition.sql)
  + four pgrx `#[pg_test]`s in
  [`src/query/executor.rs`](../src/query/executor.rs)
  (`sparql_graph_composition_with_{optional,union,minus}` +
  `sparql_optional_inside_graph_variable`).
- ✅ **Slice 111 — W3C-shape conformance fixtures for `GRAPH { … }`.**
  Three new directories under `tests/w3c-sparql/`:
  [`24-graph-named-iri/`](../tests/w3c-sparql/24-graph-named-iri/)
  (literal-IRI form, green against slice 114),
  [`25-graph-var-projection/`](../tests/w3c-sparql/25-graph-var-projection/)
  (variable form `?g` projection, green once slice 113 merges),
  [`26-graph-var-groupby/`](../tests/w3c-sparql/26-graph-var-groupby/)
  (variable form + `COUNT(*)` + `GROUP BY ?g` + `ORDER BY ?g`, also
  gated on slice 113). Also extends
  [`tests/w3c-sparql/run.sh`](../tests/w3c-sparql/run.sh) with optional
  per-test `setup.sql` support — needed because the default
  single-graph `add_graph(gid) + parse_turtle(data.ttl, gid)` path
  cannot express §13.3's multi-graph fixtures. Backward-compatible:
  tests 01–23 retain a non-empty `data.ttl` and no `setup.sql`, and
  their SQL stream is unchanged.
- ✅ **Slice 110 — pg_dump round-trip for `_pgrdf_graphs`.** New
  shell-orchestrated regression
  [`tests/regression/scripts/pg-dump-roundtrip.sh`](../tests/regression/scripts/pg-dump-roundtrip.sh)
  verifies LLD v0.4 §3.1's acceptance criterion ("`pg_dump` of a
  pgRDF database round-trips the IRI mapping verbatim") end-to-end:
  seed two `add_graph(id::bigint, iri)` bindings, `pg_dump` the
  database, drop + restore, then re-query `_pgrdf_graphs` plus a
  symmetric `pgrdf.graph_iri(101)` lookup. Cannot live as a plain
  `.sql` fixture because `pg_dump` is an external binary not
  callable from `psql -c`. New `just test-pg-dump-roundtrip` recipe;
  folded into `just test-conformance` so cold-compose sweeps catch
  it. Empirical verification deferred to the parent merge agent
  (compose-stack contention with the parallel slice-112 worktree
  during the slice's authorship).

### Track 2 — SPARQL UPDATE (Phase C countdown 84 → 67 toward v0.4.3)
`INSERT DATA`, `DELETE DATA`, pattern-driven `INSERT/DELETE … WHERE`,
the atomic `DELETE … INSERT … WHERE` modify, plus `WITH <iri>` and
inline `GRAPH <iri> { … }` graph scope. Overloads `pgrdf.sparql(q)`
to dispatch by query form; UPDATE forms return an `_update` JSONB
summary row. See
[LLD v0.4 §4](../specs/SPEC.pgRDF.LLD.v0.4.md#4-sparql-update-new).

- ✅ **Slice 78 — SPARQL UPDATE lifecycle algebra (`DROP / CLEAR /
  CREATE GRAPH`).** Closes the LLD v0.4 §4.4 lattice between the
  SPARQL UPDATE lifecycle forms and the §5 SQL UDF surface. The
  three `GraphTarget`-bearing `spargebra::GraphUpdateOperation`
  variants (`Drop`, `Clear`, `Create`) now route through
  `pgrdf.drop_graph(id, true)`, `pgrdf.clear_graph(id)`, and
  `pgrdf.add_graph(iri TEXT)` (§5 slices 99 / 98 / 118
  respectively). Routing through SQL strings (not direct Rust
  calls into the `#[pg_extern]` functions) keeps the SPARQL and
  SQL UDF front-ends as two consumers of the same partition-level
  primitives — every existence check, partition-DDL window
  (`DETACH PARTITION` / `DROP TABLE` / `TRUNCATE ONLY`), inferred-
  row cascade guard, and `_pgrdf_graphs` binding update happens
  once in the UDFs. `GraphTarget` enum coverage: `NamedNode(iri)`
  → bigint-id lookup + panic-or-no-op on not-bound per `SILENT`;
  `DefaultGraph` → direct `DELETE FROM _pgrdf_quads WHERE
  graph_id = 0` for BOTH `CLEAR DEFAULT` AND `DROP DEFAULT` (W3C
  §3.1.3 paragraph 7 "DROP DEFAULT empties, not destroys";
  `pgrdf.drop_graph(0)` panics by design under the slice-99
  guard; `pgrdf.clear_graph(0)` only handles `_pgrdf_quads_g0`
  which most default-graph inserts never touch — they land in
  `_pgrdf_quads_default` via LIST-partition catch-all routing, so
  the partition-wide DELETE is the only correct shape); `AllGraphs` → enumerate every `_pgrdf_graphs`
  row INCLUDING `graph_id = 0`; `NamedGraphs` → enumerate every
  `graph_id <> 0` (default excluded per W3C). `CREATE GRAPH <iri>`
  panics with `CREATE GRAPH <iri>: graph already exists` when the
  IRI is bound + not SILENT (the underlying
  `pgrdf.add_graph(iri TEXT)` is idempotent on its own, so the
  pre-check happens in the SPARQL dispatcher); CREATE never
  touches row counts (`triples_inserted = 0` always). ADD / MOVE
  / COPY are NOT separate variants — spargebra parser.rs §Add /
  §Move / §Copy desugars them at parse time into compositions of
  `Drop + DeleteInsert` (for COPY) / `Drop + DeleteInsert + Drop`
  (for MOVE) / `DeleteInsert` (for ADD); they ride the existing
  per-form dispatcher arms (slice 78 + slice 80). The `_update`
  summary's `form` field reports `"CLEAR"` / `"CREATE"` / `"DROP"`
  for single-op shapes; multi-op Updates collapse to `"MIXED"`.
  Regression coverage: `tests/regression/sql/99-update-lifecycle-algebra.sql`
  locks eight invariants — DROP GRAPH counter + binding removal,
  CLEAR GRAPH counter + binding preservation, CREATE GRAPH happy
  path + SILENT idempotency, DROP GRAPH not-bound panic without
  SILENT (via `_check_error` from 81-error-paths), DROP SILENT
  GRAPH not-bound no-op, CLEAR DEFAULT counter + post-state row
  count, CLEAR ALL summed counter + binding preservation. Three
  `#[pg_test]`s in `src/query/executor.rs`:
  `sparql_update_drop_graph_named_happy_path`,
  `sparql_update_clear_graph_named_preserves_binding`,
  `sparql_update_create_graph_idempotent_silent` (named graph
  seeding via `INSERT DATA { GRAPH <g> { … } }` to bypass
  `add_graph`'s parallel-test flake — same pattern as slice 79).
  Test bar after slice 78: 159 pgrx integration + 61 pg_regress +
  26 W3C-shape + 3 LUBM-shape = 249 automated tests (up from 245
  at slice 79: +3 pgrx, +1 pg_regress).

- ✅ **Slice 79 — SPARQL UPDATE graph-scoped variants (`WITH <iri>` +
  `GRAPH <iri> { … }` in template / WHERE).** Closes the graph-aware
  loop for pattern-driven UPDATEs. Spargebra-0.4.6 desugars
  `WITH <iri>` at parse time (parser.rs §Modify) into (a) per-quad
  `graph_name` injection on every default-graph template
  QuadPattern/GroundQuadPattern AND (b) a
  `using: Some(QueryDataset { default: [<iri>], named: None })`
  sentinel on the DeleteInsert operation. The per-row instantiators
  `instantiate_template_quad` / `instantiate_ground_template_quad`
  already routed `GraphNamePattern::NamedNode` into the right
  partition since slices 80/81/82 — that half was a free regression
  test. (b) is new: the slice-79 dispatcher (in
  `src/query/executor.rs::execute_update`'s three DeleteInsert
  arms) calls a small `with_iri_from_using(using, form_label)`
  helper that returns `Some(iri)` for the single-default-graph
  WITH-injected shape, panics on multi-default-or-USING-NAMED with
  the stable `'USING / USING NAMED' not yet supported` prefix, and
  returns `None` for `using.is_none()`. When `Some(iri)` is
  returned, `scope_pattern_to_graph(pattern, iri)` wraps the WHERE
  pattern in `GraphPattern::Graph { name: NamedNodePattern::
  NamedNode(iri), inner: Box::new(pattern) }` before passing it to
  `execute_*_where`. The slice-112 walker then scopes every
  emergent BGP triple (incl. via OPTIONAL/UNION/MINUS) to `<iri>`,
  and nested explicit `GRAPH <other> { … }` overrides per W3C
  §13.3. The `GRAPH <iri> { … }` in WHERE pattern path was already
  supported (slice 112); the `GRAPH <iri> { … }` in template halves
  was already wired through the per-quad `graph_name` branches in
  slices 80/81/82. Cross-graph copy
  (`INSERT { GRAPH <g2> { … } } WHERE { GRAPH <g1> { … } }`),
  scoped wipe (`DELETE { GRAPH <g> { … } } WHERE { GRAPH <g> { … } }`),
  and scoped atomic modify
  (`WITH <g> DELETE { … } INSERT { … } WHERE { … }`) are now
  first-class. Limitations: proper `USING <iri>` /
  `USING NAMED <iri>` clauses (distinct from the WITH-injected
  sentinel — i.e. multi-default-graph or USING NAMED) panic with
  `'USING / USING NAMED' not yet supported`. Regression coverage:
  `tests/regression/sql/98-update-graph-scoped.sql` locks six
  invariants — GRAPH-in-data partition isolation, cross-graph
  INSERT WHERE counter (3 inserts ⇒ all in `<g2>`), DELETE WHERE
  scoped to `<g1>` leaves the default partition intact,
  `WITH <g1> INSERT WHERE` shrinks the WHERE matches from 4
  (bare-BGP global) to 2 (g1-only) — the load-bearing proof that
  the pattern-wrapping took effect, `WITH <g1> DELETE+INSERT
  WHERE` flips two g1 rows draft→approved without touching the
  default-graph draft, and `DELETE DATA { GRAPH <g2> { … } }`
  scoped removal. Hand-authored expected output. Three `#[pg_test]`s
  in `src/query/executor.rs`
  (`sparql_update_with_insert_where_scopes_both_halves`,
  `sparql_update_cross_graph_insert_where`,
  `sparql_update_with_delete_insert_where_scopes_modify`) bypass
  the parallel-`add_graph` deadlock flake by routing graph
  allocation through `INSERT DATA { GRAPH <g> { … } }` calls
  (single-step quad+graph allocation) and inspecting the named
  partitions directly via `pgrdf.graph_id(<iri>)`.
  Test bar after slice 79: 156 pgrx integration + 60 pg_regress +
  26 W3C-shape + 3 LUBM-shape = 245 automated tests (up from 241
  at slice 80: +3 pgrx, +1 pg_regress).

- ✅ **Slice 80 — SPARQL UPDATE DELETE+INSERT WHERE (combined modify).**
  The atomic "modify" form. The DeleteInsert dispatcher arm
  `(true, true)` now routes through `execute_delete_insert_where`
  rather than panicking with the slice-77 "lands" prefix. Both halves
  resolve against the SAME WHERE solutions snapshot: the pattern is
  evaluated exactly once, the projection unions every variable
  referenced by EITHER template (DELETE-side then INSERT-side,
  first-appearance per side, so adding an INSERT-only var doesn't
  reshuffle DELETE-side columns), and Rust iterates the binding rows
  via SPI applying DELETE then INSERT per row. Per W3C SPARQL 1.1
  Update §3.1.3 the DELETE conceptually precedes the INSERT — this
  matters for status-flip patterns (`DELETE { ?x ex:status "draft" }
  INSERT { ?x ex:status "approved" } WHERE { ?x ex:status "draft" }`)
  where the DELETE removes the old row and the INSERT adds the new
  one cleanly. Atomicity is naturally provided by Postgres's
  transaction model (the whole UDF call is one transaction → DELETE
  and INSERT either both land or neither does). DELETE counter uses
  the `WITH d AS (DELETE … RETURNING 1) SELECT count(*)` idiom from
  slice 81/83 (actual rows removed); INSERT counter is per-attempt
  (slice 82 convention — the `WHERE NOT EXISTS` guard silently
  dedupes but the attempt count surfaces). The `_update` summary
  reports `form: "DELETE_INSERT_WHERE"` (the discriminator
  `update_op_name` already routed combined templates to this label
  per slice 82 — no shape change). Limitations inherit slices 81/82:
  WHERE may not carry aggregates / GROUP BY / UNION; template
  variables MUST be bound by the WHERE BGP (panics with
  `DELETE/INSERT WHERE template feature 'unbound template variable`
  stable prefix); variable GRAPH in either template panics (lands
  with slice 76); `USING / USING NAMED` not yet supported (gated in
  the dispatcher arm). Regression coverage:
  `tests/regression/sql/97-update-delete-insert-where.sql` locks
  five invariants — status-flip counters (2 deletes + 2 inserts),
  idempotent termination (re-issue against flipped state ⇒ 0/0),
  multi-template (1 DELETE quad + 2 INSERT quads × 2 solutions =
  2 deletes + 4 inserts), zero-match no-op (unrelated WHERE ⇒ 0/0),
  post-state round-trip (SELECT confirms table state matches counter
  trail). Hand-authored expected output. Three `#[pg_test]`s in
  `src/query/executor.rs`
  (`sparql_update_delete_insert_where_happy_path`,
  `sparql_update_delete_insert_where_idempotent_termination`,
  `sparql_update_delete_insert_where_multi_template`). The
  slice-77 "lands" panic assertions in regressions 93 / 94 / 95 (the
  `update-delete-insert-where-lands-82-77` `_check_error` lines)
  were replaced with smoke assertions that the dispatcher now
  returns a well-formed `form = "DELETE_INSERT_WHERE"` row.
  Test bar after slice 80: 153 pgrx integration + 59 pg_regress +
  26 W3C-shape + 3 LUBM-shape = 241 automated tests (up from 238 at
  slice 81: +2 pgrx — 3 new slice-80 cases minus 1 dropped panic
  assertion — and +1 pg_regress).

- ✅ **Slice 81 — SPARQL UPDATE DELETE WHERE (pattern-driven).**
  Sibling of slice 82's INSERT WHERE. The DeleteInsert dispatcher
  arm `(true, false)` now routes through `execute_delete_where`
  rather than panicking with the slice-78 "lands" prefix (the
  panic was removed when slice 81 shipped; the slice number was
  also renumbered from 78 to 81 to keep the countdown spacing
  consistent — see CHANGELOG). Same strategy as slice 82: the
  WHERE pattern goes through the v0.3 `parse_select` walker
  (sharing BGP/FILTER/OPTIONAL/MINUS algebra with SELECT); a
  custom projection returns each template-referenced variable's
  **dict id** (BIGINT, not lexical text); Rust iterates the
  binding rows via SPI and materialises each template's
  `GroundQuadPattern` per row. The DELETE template type
  (`Vec<GroundQuadPattern>` rather than `Vec<QuadPattern>` for
  INSERT) bakes the W3C SPARQL 1.1 §4.1.2 rule "blank nodes are
  not allowed in the DELETE clause" into the spargebra AST — the
  helper-pair `collect_ground_template_vars` /
  `instantiate_ground_template_quad` mirrors slice 82's INSERT-
  side helpers but matches `GroundTermPattern` (no blank-node
  arm). Per-row DELETE uses the same `WITH d AS (DELETE …
  RETURNING 1) SELECT count(*)` idiom slice 83 installed for
  DELETE DATA, so `triples_deleted` counts ACTUAL rows removed
  (not template instantiations attempted) — a critical
  distinction from INSERT WHERE's "attempted insert" counter,
  which the WHERE NOT EXISTS guard silently dedupes. Lookup-only
  dict path mirrors slice 83's DELETE DATA: if any term in the
  instantiated template is absent from `_pgrdf_dictionary`, the
  per-row delete is a spec-correct no-op rather than an error.
  The `_update` summary reports `form: "DELETE_WHERE"` (distinct
  from `DELETE_DATA`); `update_op_name`'s DeleteInsert label was
  already split by slice 82, so no shape change there.
  Slice-81 limitations locked (mirroring slice 82): WHERE may
  not carry aggregates / GROUP BY / UNION; template variables
  MUST be bound by the WHERE BGP (panics with `DELETE WHERE
  template feature 'unbound template variable` stable prefix);
  variable GRAPH in template panics (lands with slice 76);
  `USING / USING NAMED` not yet supported. Regression coverage:
  `tests/regression/sql/96-update-delete-where.sql` locks five
  invariants (filtered-DELETE counter, broad-DELETE counter,
  zero-match no-op, post-state round-trip, set-semantics on
  re-issue). Three `#[pg_test]`s in `src/query/executor.rs`
  (`sparql_update_delete_where_happy_path`,
  `sparql_update_delete_where_broad_and_idempotent`,
  `sparql_update_delete_where_zero_match_noop`). En passant
  fix: tightened the `error =` strings on slice 82's two
  negative-path pgrx tests
  (`sparql_update_insert_where_unbound_template_var_panics`,
  `sparql_update_delete_insert_combined_still_panics`) to
  include the full panic suffix — pgrx-tests does an exact
  string match on the error attribute, not a substring match,
  so the trimmed forms were silently failing.

- ✅ **Slice 82 — SPARQL UPDATE INSERT WHERE (pattern-driven).**
  Builds on slice 84's UPDATE foundation to land
  `INSERT { template } WHERE { pattern }` end-to-end. Strategy:
  the WHERE pattern goes through the v0.3 `parse_select` walker
  (sharing BGP/FILTER/OPTIONAL/MINUS algebra with SELECT); a
  custom projection returns each template-referenced variable's
  **dict id** (BIGINT, not lexical text — keeps internment
  lossless so the binding's term_type / datatype / language tag
  stay attached to the existing dict row); Rust iterates the
  binding rows via SPI and materialises each template QuadPattern
  per row, routing through the shared `insert_quad` helper with
  the same `WHERE NOT EXISTS` set-semantic guard as INSERT DATA.
  The `_update` summary reports `form: "INSERT_WHERE"` (not
  `INSERT_DATA`) so callers can discriminate which UPDATE variant
  ran. Slice-82 limitations locked: WHERE may not carry
  aggregates / GROUP BY / UNION (the SQL builder's output shape
  doesn't carry dict ids in those branches); template variables
  MUST be bound by the WHERE BGP — an unbound template variable
  panics with the stable `INSERT WHERE template feature 'unbound
  template variable` prefix (fail-fast rather than the spec's
  silent-skip, which lands later as an enhancement when CONSTRUCT
  ships); a variable GRAPH in the template
  (`INSERT { GRAPH ?g { … } }`) panics with the slice-76 prefix.
  Per-form panic table updated for slice 84's siblings: pure
  DELETE WHERE → slice 78 (subsequently renumbered to slice 81
  and shipped), combined DELETE+INSERT WHERE → slice 77 (the
  contiguous substring `UPDATE form 'DELETE/INSERT WHERE' lands`
  is preserved across the new dispatcher so slice 84's
  regression locks still hold). Regression coverage:
  `tests/regression/sql/95-update-insert-where.sql` locks five
  happy-path invariants (form discriminator, multi-row template
  instantiation, zero-match no-op, multi-triple template, set-
  semantics on re-issue) plus three negative-path "INSERT WHERE
  template feature 'X' not yet supported" prefix locks. Five
  pgrx integration tests in `src/query/executor.rs` cover the
  executor path under the `pg_test` harness.

- ✅ **Slice 84 — SPARQL UPDATE foundation + INSERT DATA.** Opens
  Phase C toward v0.4.3. `pgrdf.sparql(q)` now detects UPDATE
  queries via a **try-parse-then-fallback** at the entry point:
  `parse_query` first (the v0.3 SELECT/ASK path, unchanged), then
  `parse_update` on query-side failure. UPDATE forms route to
  `execute_update(&spargebra::Update)`, which walks
  `update.operations` (a `Vec<GraphUpdateOperation>`) and dispatches
  per variant. `InsertData` lands end-to-end:
  default-graph + `GRAPH <iri> { … }` inline graph scope, multi-
  triple blocks, mixed-IRI-and-literal payload (typed literals get
  their datatype IRI interned first per the loader convention),
  unknown IRIs auto-allocate via `pgrdf.add_graph(iri TEXT)` (slice
  118). Idempotency: `_pgrdf_quads` has no UNIQUE constraint, so
  the INSERT routes through a `WHERE NOT EXISTS` guard against the
  SPO covering index — set-semantics per LLD v0.4 §4 honoured
  without the `ON CONFLICT` shape Postgres can't support against
  the unconstrained table. Return shape: a single summary row of
  `{"_update": {form, triples_inserted, triples_deleted,
  graphs_touched, elapsed_ms}}` paralleling the v0.3 `_ask`
  sentinel. Per-form panics with stable "lands in slice NN" prefixes
  for the variants that follow-up slices will land: DELETE DATA →
  83, DELETE/INSERT WHERE → 82-77, CLEAR/CREATE/DROP GRAPH →
  71/70/69, LOAD → out of scope for v0.4 (LLD v0.4 §14). The
  `pgrdf.sparql_parse(q)` UDF mirrors the detection strategy and
  reports `form: "UPDATE"` with a per-op summary array; unimplemented
  ops are NOT flagged in `unsupported_algebra` (that array stays
  reserved for genuinely-out-of-scope shapes). Regression coverage:
  `tests/regression/sql/93-update-insert-data.sql` locks six
  invariants (default-graph, named-graph, multi-triple, idempotent
  on repeat, typed-literal round-trip, sparql_parse integration)
  plus six negative-path "lands in slice NN" prefix locks via the
  `_check_error` plpgsql helper. Eight pgrx integration tests cover
  the executor + parser paths under the `pg_test` harness.

- ✅ **Slice 83 — SPARQL UPDATE DELETE DATA.** Symmetric companion
  to slice 84's INSERT DATA: `DELETE DATA { … }` removes ground
  quads (no variables, no WHERE clause) one-by-one from
  `_pgrdf_quads`. spargebra emits
  `GraphUpdateOperation::DeleteData { data: Vec<GroundQuad> }`;
  each `GroundQuad` carries a `NamedNode` subject + `NamedNode`
  predicate + `GroundTerm` object (no blank nodes — enforced by
  spargebra at parse time) + `GraphName` scope. The dispatcher
  walks each ground quad through a **lookup-only** dictionary
  path (`lookup_iri_id` for subject/predicate, new
  `lookup_ground_term_id` for object) — no interning. If any term
  is missing from `_pgrdf_dictionary`, the quad cannot be in the
  store, so the form is a spec-correct no-op (LLD v0.4 §4.1
  set-semantics). Same for an unbound named-graph IRI: the
  partition can't exist, so the operation produces zero rows.
  Default-graph + `GRAPH <iri> { … }` inline graph scope both
  supported; same-shape triples in a different graph are NOT
  touched. Multi-op form discriminator: if every op in the Update
  shares the same variant name, that name carries through to the
  summary's `form` field; otherwise `form` collapses to
  `"MIXED"` (forward-looking compatibility with a future
  `DELETE DATA ; INSERT DATA` composition). The post-slice 84
  panic test in `executor.rs` retargets to `DELETE/INSERT WHERE`
  (slices 82-77); the corresponding regression assertion in
  `93-update-insert-data.sql` is removed. Regression coverage:
  `tests/regression/sql/94-update-delete-data.sql` locks six
  invariants (default-graph removal, missing-term no-op, named-
  graph scope, SELECT round-trip, idempotency on repeat, typed-
  literal payload) plus one negative-path sample. Three new
  `#[pg_test]`s in `src/query/executor.rs`
  (`sparql_update_delete_data_removes_existing`,
  `sparql_update_delete_data_missing_term_is_noop`,
  `sparql_update_delete_data_named_graph`).

### Track 3 — Graph-level lifecycle UDFs (Phase B countdown 99 → 96)
`pgrdf.drop_graph`, `clear_graph`, `copy_graph`, `move_graph` as
partition-level primitives over `_pgrdf_quads` — constant-time
`move_graph` via DETACH/ATTACH metadata swap, `TRUNCATE ONLY` for
`clear_graph`. Also wires the corresponding SPARQL UPDATE forms
(`DROP/CLEAR/CREATE/COPY/MOVE/ADD GRAPH`) to these UDFs. See
[LLD v0.4 §5](../specs/SPEC.pgRDF.LLD.v0.4.md#5-graph-level-lifecycle-udfs-new).

Phase B countdown opens with **slices 99 + 98 as parallel batch 1**
(`drop_graph` + `clear_graph`), continuing with slices 97 + 96
(`copy_graph` + `move_graph`) in the next batch. All four lifecycle
UDFs land in `src/storage/graphs.rs` against the §5.1 surface
table.

- ✅ **Slice 99 — `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN
  DEFAULT TRUE) → BIGINT`.** Removes the LIST partition
  `_pgrdf_quads_g<id>` from the parent `_pgrdf_quads` via
  `ALTER TABLE ... DETACH PARTITION` followed by `DROP TABLE`,
  deletes the matching `_pgrdf_graphs` row, returns the pre-drop
  triple count. `cascade => FALSE` errors with the stable
  `drop_graph: inferred rows present` prefix when any
  `is_inferred = TRUE` row exists. Default partition (graph_id = 0)
  rejected with `drop_graph: cannot drop default partition`;
  negative ids rejected with `drop_graph: graph_id must be >= 0`.
  Idempotent: dropping an absent graph returns 0 (and prunes any
  stranded `_pgrdf_graphs` binding so the IRI mapping converges
  with reality). Post-drop, `pgrdf.graph_iri(id)` and
  `pgrdf.graph_id(iri)` both return NULL — closes the
  `_pgrdf_graphs` invalidation clause of LLD v0.4 §5.2.
  Regression: `88-drop-graph.sql` locks six invariants (idempotent
  absent, happy path with triple count, cascade-FALSE-inferred
  guard, cascade-TRUE-inferred override, default-partition guard,
  negative-id guard). Pgrx integration tests cover the absent +
  happy + cascade-FALSE + default-partition + negative-id paths.

- ✅ **Slice 98 — `pgrdf.clear_graph(id BIGINT) → BIGINT`.**
  `TRUNCATE ONLY pgrdf._pgrdf_quads_g<id>` against the per-graph
  partition; returns rows removed (== pre-clear row count).
  Partition shell + `_pgrdf_graphs` IRI binding both survive,
  so subsequent inserts route normally and `graph_iri(id)`
  keeps resolving. Idempotent on absent / empty graphs (returns
  0 without erroring). `clear_graph(0)` is permitted (contrast
  with `drop_graph(0)`, sibling slice 99, which rejects).
  Negative id panics with stable
  `clear_graph: graph_id must be >= 0, got <N>` prefix.
  Regression coverage:
  [`tests/regression/sql/89-clear-graph.sql`](../tests/regression/sql/89-clear-graph.sql)
  + three `#[pg_test]`s in `src/storage/graphs.rs`
  (`clear_graph_absent_returns_zero`,
  `clear_graph_returns_row_count`,
  `clear_graph_twice_second_returns_zero`).

- ✅ **Slice 97 — `pgrdf.copy_graph(src BIGINT, dst BIGINT) →
  BIGINT`.** `INSERT INTO pgrdf._pgrdf_quads_g<dst> SELECT … FROM
  pgrdf._pgrdf_quads_g<src>` with the `graph_id` projection rebound
  to `dst`; returns rows copied (== src row count at INSERT time).
  Both `is_inferred = FALSE` and `is_inferred = TRUE` rows carry
  forward verbatim — entailment state is preserved per LLD v0.4
  §5.2. Destination partition auto-created via
  `pgrdf.add_graph(dst)` if absent. Idempotent on absent src
  (returns 0). `src == dst` rejected with stable
  `copy_graph: src and dst must differ` prefix.

- ✅ **Slice 96 — `pgrdf.move_graph(src BIGINT, dst BIGINT) →
  BIGINT`.** Migrates every quad in `src` to `dst` and removes
  `src`. v0.4.2 implementation composes slices 97 + 99:
  `pgrdf.copy_graph(src, dst)` then
  `pgrdf.drop_graph(src, cascade => TRUE)`. Returns rows moved
  (== src row count at copy time). The LLD §5.2 "metadata-only
  DETACH/ATTACH rebind" spec turned out to require an interim
  UPDATE of every row's `graph_id` column (the LIST partition
  constraint demands it), so the metadata-only claim is
  aspirational and downgraded to a v0.5 perf optimisation. Guards:
  `src == dst`, `dst` non-empty, negative id all rejected with
  stable prefixes. Idempotent: absent `src` returns 0. Regression:
  `91-move-graph.sql` locks five invariants; five `#[pg_test]`s
  exercise the same paths.

**Phase B §5 lifecycle UDF surface complete** at slice 96. The
SPARQL UPDATE lifecycle algebra (`DROP/CLEAR/CREATE/COPY/MOVE/ADD
GRAPH`) wiring lands in Phase C's SPARQL UPDATE track.

### Track 4 — CONSTRUCT
`pgrdf.construct(q TEXT) → SETOF JSONB` returning structured
`{subject, predicate, object}`-shaped rows via the existing term
shaper. Sibling UDF rather than overloading `pgrdf.sparql` — callers
signal intent at the SQL boundary. See
[LLD v0.4 §6](../specs/SPEC.pgRDF.LLD.v0.4.md#6-construct-deferred-from-v03-now-in-scope).

### Track 5 — Property paths
`*`, `+`, `?`, `^`, with alternation `p1|p2` as a stretch goal.
Translates to recursive Postgres CTEs with a `pgrdf.path_max_depth`
GUC; falls back to direct BGP match when the predicate's closure is
already materialised. See
[LLD v0.4 §7](../specs/SPEC.pgRDF.LLD.v0.4.md#7-property-paths-deferred-from-v03-now-in-scope).

### Carried backlog — SPARQL surface gaps from v0.3
Multi-triple `OPTIONAL { BGP }` (LATERAL-style derived-table refactor),
`VALUES` inline tables, `BIND` output usable in later FILTER/BGP,
aggregates over `UNION`, and `DESCRIBE`. Shipped in the same cut
because they share the translator machinery §4 + §6 already require.
See
[LLD v0.4 §11](../specs/SPEC.pgRDF.LLD.v0.4.md#11-sparql-surface-backlog-deferred-from-v03-now-in-scope).

### Performance work carried forward from v0.3
Phase 3 step 3 phase B — `heap_multi_insert` / `COPY BINARY` ingest
path — targets v0.4 (the 2× wall-clock target from v0.3 LLD §4.3
acceptance is not met by phase A alone; the per-tuple executor walk
dominates). Postgres custom-scan hooks for specific quad-shape access
patterns are also flagged at v0.4 as the earliest target, may slip to
v0.5 if the refactor cost exceeds the §4 / §6 wins. These do not gate
the surface work in tracks 1-5; they ship in their own slices. See
[LLD v0.4 §12](../specs/SPEC.pgRDF.LLD.v0.4.md#12-performance-work-carried-forward-from-v03).

### Conformance runner wiring (v0.4)
The W3C SPARQL 1.1 manifest runner (Phase 6 step 2, gated `if: false`
in v0.3) is wired in v0.4 — it gates the §11 SPARQL backlog
automatically as the deferred forms come online. See
[LLD v0.4 §13](../specs/SPEC.pgRDF.LLD.v0.4.md#13-test-policy-continues-v03-6-unchanged-in-spirit).

### Track 6 — Real SHACL validation (✅ landed)
`pgrdf.validate(data, shapes)` ships the real W3C `sh:ValidationReport`-shape
JSONB, backed by `shacl 0.3.x` (rudof). Unblocked via the patched
`reasonable` fork tracked in ERRATA.v0.4 E-011. Regression
fixtures `70-validate-stub.sql` (basic shape) and
`71-shacl-real.sql` (LLD §9 violations). See
[LLD v0.4 §9](../specs/SPEC.pgRDF.LLD.v0.4.md#9-shacl-real-integration-shipped-in-v04-cycle).

### Excluded from v0.4 (planned v0.5)
The reasoning profile selector (`pgrdf.materialize(graph_id, profile)`
— RDFS vs OWL-RL), TriG / N-Quads ingest, IRI overloads for the §5
lifecycle UDFs, the W3C SHACL manifest runner, and the SHACL-SPARQL
constraint mode. See
[LLD v0.5-FUTURE §3](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md#3-reasoning-profile-selector),
[§4](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md#4-trig--n-quads-ingest),
[§5](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md#5-shacl-sparql-constraint-mode--materialised-graph-coverage),
[§6](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md#6-w3c-shacl-manifest-runner),
[§7](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md#7-iri-overloads-for-lifecycle-udfs).

---

## Coverage ratchet — release-by-release targets

Per-release floor for every CI-enforced test layer plus the two
external-standard pass-rate gates (W3C SPARQL 1.1, W3C SHACL) and the
LUBM cross-engine benchmark. Cells anchor to
[`specs/SPEC.pgRDF.LLD.v0.3.md` §6.1](../specs/SPEC.pgRDF.LLD.v0.3.md)
(test-layer matrix),
[`specs/SPEC.pgRDF.LLD.v0.4.md` §13](../specs/SPEC.pgRDF.LLD.v0.4.md#13-test-policy-continues-v03-6-unchanged-in-spirit)
(v0.4 test policy), and
[`docs/08-testing.md`](08-testing.md) (test strategy doc); nothing
here is new contract, only a consolidated view of the targets already
declared in those sources.

| Layer                                 | v0.3 (current) | v0.4 target                                 | v0.5 target                              | v1.0 target                                            |
|---|---|---|---|---|
| pgrx integration (`cargo pgrx test`)  | 93 ✅           | + `heap_multi_insert` tests                 | TBD                                      | TBD                                                    |
| pg_regress golden                     | 39 ✅           | ~60 (§3 + §4 + §5 + §6 + §7 + §11)          | TBD                                      | TBD                                                    |
| W3C-shape SPARQL harness              | 23 ✅           | superseded by TTL-manifest runner outputs   | superseded by TTL-manifest runner        | superseded by TTL-manifest runner                      |
| LUBM-shape correctness harness        | 3 ✅            | superseded by LUBM-1 real benchmark         | superseded by LUBM-10 real benchmark     | superseded by LUBM-100 real benchmark                  |
| W3C SPARQL 1.1 conformance (manifest) | not wired ⏳   | runner wired + ≥ 30 % pass                  | ≥ 70 % pass                              | ≥ 95 % pass                                            |
| W3C SHACL conformance (manifest)      | not wired ⏳ (E-009) | not wired (still E-009)               | ≥ 50 % pass (E-009 cleared, real output) | ≥ 90 % pass                                            |
| LUBM cross-engine benchmark           | scaffold only ⏳ | LUBM-1 smoke                                | LUBM-10 baseline vs Apache Jena TDB / Apache AGE | LUBM-100 vs Apache Jena TDB / Apache AGE       |

**Ratchet enforcement.** Each release's CI must hit at least its
column's targets; once a target is met it becomes a floor and can
never regress (`docs/08-testing.md` "Regression discipline":
"Coverage gates ratchet but never lower."). A green build on `main`
that drops below a previously-met floor is a CI failure. Cells
marked **TBD** have no published target in the LLD or FUTURE specs
yet — they'll get filled in as v0.5 / v1.0 LLDs draft, not
fabricated here.

---

## Out of scope (v0.x)

(Carries forward unchanged from
[`SPEC.pgRDF.LLD.v0.4.md §14`](../specs/SPEC.pgRDF.LLD.v0.4.md).)

- Streaming replication / logical decoding of RDF state.
- Federated SPARQL `SERVICE` — explicitly deferred to v1.0 per
  [v0.5-FUTURE §9](../specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md#9-forward-look--v10-and-beyond).
- Full OWL 2 (EL / QL) reasoning — ERRATA E-002.
- Backup/restore for opaque binary state (tracked by future
  `SPEC.pgRDF.BACKUP.v0.x`, INSTALL §11 OQ5).
- `LOAD <url>` in SPARQL UPDATE — callers fetch externally and
  invoke `pgrdf.load_turtle` / `pgrdf.parse_trig` directly
  (LLD v0.4 §14).

---

## Test bar over time

A coarse cumulative view; the precise per-commit count is in the
Phase 2.2 (extended) SPARQL-surface step table above.

(Rows labelled `Phase 3 step N` below this table's first block are
pre-v0.3 framing — they correspond to the Phase 2.2 (extended)
SPARQL surface steps 1-12, not to the v0.3 LLD's Phase 3 Storage
Performance. Test counts are unaffected; the labels are kept here
for git-archaeology fidelity.)

(Once v0.4 work begins, new rows land under `v0.4 cut` labels per
the per-track grouping in the "v0.4 — next milestone" section
above; the v0.3 rows below remain frozen as the shipped baseline.)

| Boundary | pgrx integration | pg_regress files | Notes |
|---|---|---|---|
| Phase 1 done | 0 | 0 | smoke + scaffold only |
| Phase 2.0 done | 7 | 3 | dict + quad CRUD |
| Phase 2.1 done | 11 | 7 | + Turtle ingest, regression fixtures |
| Phase 2.2 done | 21 | 13 | + dict cache, batched ingest, SPARQL parser, BGP-to-SQL, N-pattern BGP joins, user guide |
| Phase 2.2 (extended) step 6 | 56 | 19 | + FILTER, modifiers, OPTIONAL, UNION, MINUS |
| Phase 2.2 (extended) step 7 | 63 | 20 | + aggregates (COUNT/SUM/AVG/MIN/MAX + GROUP BY) |
| Phase 2.2 (extended) steps 8–12 | 79 | 25 | + HAVING, GROUP_CONCAT/SAMPLE, expression richness, BIND, multi-triple MINUS, ASK |
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
| v0.3 edge-case signals — #56 | 93 | 39+23+3 | extends `82-stats-shape.sql` in-place (no new pg_regress file — the file is explicitly scoped to "schema shape only" and these three new invariants are schema shape too) with the schema-drift tripwire trio: (a) exact field count — `count(*) FROM jsonb_object_keys(stats()) = 10` pins to the literal current key count emitted by `src/storage/stats.rs::stats()` (`shmem_ready`, `shmem_slots`, `shmem_hits`, `shmem_misses`, `shmem_inserts`, `shmem_evictions`, `plan_cache_hits`, `plan_cache_misses`, `plan_cache_inserts`, `plan_cache_local_size`) so any added field forces a deliberate test update; (b) keys-match-canonical — `array_agg(k ORDER BY k) = ARRAY[…literal 10-element list…]` catches both silent additions (array gets longer) and silent renames (one element swaps); (c) no-null-fields — `bool_and(jsonb_typeof(value) != 'null')` catches a refactor that defaults an uninitialised counter to JSON `null` rather than `0`. Companions the existing "fields-that-SHOULD-be-there are there" block with the orthogonal "fields-that-SHOULDN'T-be-there ARE NOT there" guarantee — together they pin the closed-set shape contract downstream operator tooling (CloudNativePG operators, CI dashboards, telemetry parsers) wires against. Test count unchanged: still 39+23+3 — three new rows in `tests/regression/expected/82-stats-shape.out` |
| v0.3 harness lock-in — #55 | 93 | 39+23+3 | promotes the W3C-shape + LUBM-shape harnesses to first-class Justfile recipes (`just test-w3c`, `just test-lubm`), introduces `just test-conformance` (regression + W3C-shape + LUBM-shape — every compose-based layer) and `just test-everything` (pgrx integration + test-conformance — the broadest sweep), and lands `just smoke-cold` (`compose-down` → `build-ext` → `compose-up` → `CREATE EXTENSION` → test-conformance) as the cold-compose discipline gate. `just test-all` keeps its narrow `test + test-regression` shape for back-compat. `docs/08-testing.md` and `README.md`'s Tests block point at the new entry points. The shift matters because two of the three compose-based harnesses were previously discoverable only by knowing the bash paths — `just --list` showed nothing about them, and `just test-all` silently skipped them. Cold-compose smoke is the verification half: it catches the bug class that passes on a warm compose because some prior `DROP/CREATE` left state behind, and breaks on the next cold boot. Test count unchanged — the new recipes are wrappers, not new tests. Final entry in the 66→1 coverage countdown; the next phase opens the hygiene cycle |
| **v0.3 cut** | **93** | **39 + 23 + 3 = 65** | **Total 158 tests across all five layers** (93 pgrx integration + 39 pg_regress + 23 W3C-shape SPARQL + 3 LUBM-shape). v0.3 LLD §5 phase status: Phase 1 ✅, Phase 2 ✅ (2.0/2.1/2.2 + extended SPARQL surface steps 1-12), Phase 3 🚧 (steps 1-2 ✅, step 3 phase A ✅, phase B → v0.4), Phase 4 ✅, Phase 5 🚧 stub (real impl → v0.4 per LLD v0.4 §9 — landed ✅ in commit `ac40bc2` post-v0.3.0), Phase 6 🚧 (step 1 ✅, step 2 starter + expansions + essentials ✅, step 3 ⏳). License attribution (Apache 2.0 / 2026), MSRV (1.91), ERRATA E-006 re-check (2026-05-14), ERRATA E-010 (cargo audit informational). Forward look: [`SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md) is canonical for v0.4 scope |
| **v0.4.0 cut (current)** | **94** | **40 + 23 + 3 = 66** | **Total 160 tests across all five layers** (94 pgrx integration + 40 pg_regress + 23 W3C-shape SPARQL + 3 LUBM-shape). Key delta vs v0.3 cut: real SHACL Core validation lands — `pgrdf.validate(data, shapes)` returns a W3C `sh:ValidationReport`-shape JSONB via `shacl 0.3.1` (commit `ac40bc2`), replacing the v0.3.0 stub. Unblocked via `[patch.crates-io]` to the `styk-tv/reasonable@rdf12-passthrough` fork ([ERRATA.v0.4 E-011](../specs/ERRATA.v0.4.md); upstream PR [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50) pending — v0.4.1 drops the patch on merge). New regression `71-shacl-real.sql`, three new pgrx integration tests. v0.4 LLD §5 phase status: Phase 1 ✅, Phase 2 ✅, Phase 3 🚧 (phase B still → v0.4.x), Phase 4 ✅, Phase 5 ✅, Phase 6 🚧 (step 3 ⏳). Named-graph + SPARQL UPDATE + lifecycle UDFs + CONSTRUCT + property paths + heap_multi_insert phase B + W3C SPARQL 1.1 manifest runner all 🚧 — slated for subsequent v0.4.x point releases or a refreshed v0.5.0 cut |
| **Phase A §3 named-graph shipped** | **118** | **49 + 26 + 3 = 78** | **Total 196 tests across all five layers** (118 pgrx integration + 49 pg_regress + 26 W3C-shape SPARQL + 3 LUBM-shape). Cumulative landings of Phase A countdown slices 120 → 110 against the v0.4.0 cut: `_pgrdf_graphs` system table + `pg_extension_config_dump` registration (slice 120), the five-UDF `add_graph` / `graph_id` / `graph_iri` surface (slices 119 → 115), SPARQL `GRAPH <iri>` literal and `GRAPH ?g` variable forms (slices 114 / 113), per-pattern GRAPH composition with OPTIONAL/UNION/MINUS (slice 112), three W3C-shape conformance fixtures for §13.3 (slice 111: `24-graph-named-iri` / `25-graph-var-projection` / `26-graph-var-groupby`), and the shell-driven `tests/regression/scripts/pg-dump-roundtrip.sh` (slice 110, wired into `just test-pg-dump-roundtrip` + `just test-conformance`). New pg_regress files: `72-77`, `78`, `79`, `87` (+9 vs v0.4.0). All four §3.4 LLD acceptance criteria verified end-to-end. v0.4 LLD §5 phase status: Phase 1 ✅, Phase 2 ✅, Phase 3 🚧 (phase B still → v0.4.x), Phase 4 ✅, Phase 5 ✅, **§3 named-graph ✅** (Track 1 closed), Phase 6 🚧 (step 3 ⏳). Phase A continues with the docs-sync + close-out slices 109 → 100 toward a v0.4.1 tag; SPARQL UPDATE + lifecycle UDFs + CONSTRUCT + property paths + heap_multi_insert phase B + W3C SPARQL 1.1 manifest runner carry forward |
