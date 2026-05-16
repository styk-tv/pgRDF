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
> [`specs/SPEC.pgRDF.LLD.v0.5.md`](../specs/SPEC.pgRDF.LLD.v0.5.md)
> is the authoritative shipped contract for the **v0.5.0** cut
> (reasoning-profile selector, TriG/N-Quads ingest, SHACL `mode`
> argument, the W3C SHACL Core manifest gate, IRI lifecycle
> overloads, aggregates-over-UNION residuals).
> [`specs/SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md)
> remains the verbatim record of the v0.4.x-cut surface. The next
> forward-look beyond v0.5 lives in
> [`specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md`](../specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md).

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
      landed upstream in pgrx 0.18.0 (2026-04-17), but adoption stays
      deferred past v0.5: 0.18.0 still trips `E0716` in its
      `impl_table_iter` macro on every Rust stable/nightly we tested,
      and its single-pass schema-gen migration (`pgrx_embed` removal,
      `crate-type` change) is a non-trivial breaking edit. See
      `specs/ERRATA.v0.2.md` E-006.
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
- ✅ Multi-triple OPTIONAL — shipped Phase F group F1 (slices
      34-31). N-triple right side as a LATERAL-style derived table
      inside the LEFT JOIN (atomic, W3C §6.1); nested OPTIONAL,
      OPTIONAL-internal FILTER, GRAPH scoping, `+`-path-in-required
      compose; inherited by `pgrdf.construct` + UPDATE WHERE.
      (Multi-triple MINUS shipped step 11.) LLD v0.4 §11.
- ✅ Downstream `BIND` + aggregates-over-UNION — shipped Phase F
      group F2 (slices 30-27). A `BIND`-introduced variable is now
      usable in a later FILTER, a BGP join key, and a chained BIND
      (AST substitution pass, the LLD §11-named approach);
      aggregates run over a UNION via a derived-table refactor
      reusing the F1 `vK` column pool. Both compose with GRAPH
      scoping + F1 OPTIONAL/VALUES and are inherited by
      `pgrdf.construct` + UPDATE WHERE. LLD v0.4 §11.
      (`lang(?v)` / `datatype(?v)` and the `STRLEN` / `CONTAINS` /
      `STRSTARTS` / `STRENDS` surface shipped step 9; `BIND (expr AS ?v)`
      for projection shipped step 10; type-aware `MIN`/`MAX` over
      `xsd:numeric` shipped post-step-12 — translator slice
      `7de9c17`. The six residual aggregate-over-UNION refinements
      — incl. GROUP BY on a GRAPH-scope-only var — shipped Phase G
      group G2, `SPEC.pgRDF.LLD.v0.5 §8`.)
- ✅ `DESCRIBE` — shipped Phase F group F3 (slices 26-24). Sibling
      UDF `pgrdf.describe(q TEXT) → SETOF JSONB`, byte-identical row
      shape to `pgrdf.construct`. The description is the closure of
      each described resource (every triple with it as subject)
      transitively expanded one hop through blank-node objects per
      W3C §16.4 (cycle-safe via a visited-set, dedup'd across the
      whole result). `DESCRIBE <iri>` / `DESCRIBE ?v WHERE {…}` /
      mixed constant+variable / `DESCRIBE *`; composes with GRAPH
      scoping (closure within the named graph); unscoped scans every
      graph. `pgrdf.sparql_parse` reports `form:"DESCRIBE"` and no
      longer flags it in `unsupported_algebra`; DESCRIBE via
      `pgrdf.sparql` redirect-panics to `pgrdf.describe`. LLD v0.4
      §11. (`80-unsupported-shapes` gap-6 retired same commit.)
- ✅ Type-aware ORDER BY — shipped Phase F group F4 (slices 23-22).
      `ORDER BY` now sorts across the SPARQL 1.1 §15.1 value space: a
      kind rank (numeric < dateTime < boolean < other) + per-kind
      comparator (numerics **numerically** so `2 < 10`, `xsd:dateTime`
      chronologically, `xsd:boolean` false<true, strings by Unicode
      codepoint via `COLLATE "C"`) + codepoint tiebreak; total and
      stable, never raises (regex-guarded casts fall through to the
      codepoint tier). `DESC()`, multi-key, and expression sort keys
      (`ORDER BY (?a+?b)`, `ORDER BY STRLEN(?s)`) all work; all four
      SQL builders + `SELECT DISTINCT` compose (expression keys on
      aggregate/UNION shapes a documented narrow deferral). ORDER BY
      was already an unflagged SELECT modifier (no
      `unsupported_algebra`/`80-unsupported-shapes` entry to retire).
      Regression-locked
      `tests/regression/sql/100-sparql-order-by-type-aware.sql`
      + W3C-shape `47-order-by-type-aware`; the `111` property-path
      closure expected output was corrected to §15.1 codepoint order.
      LLD v0.4 §11 — **§11 SPARQL backlog now complete**.
- ✅ `VALUES (?x ?y) { … }` — shipped Phase F group F1 (slices
      34-31). `(VALUES …) AS vN(cols)` derived table joined on
      shared vars; constants → dict ids ahead of execution; `UNDEF`
      → NULL cell (no constraint, W3C §10); typed/lang literals
      datatype-aware; composes with GRAPH + OPTIONAL; inherited by
      `pgrdf.construct` + UPDATE WHERE. LLD v0.4 §11.
- ✅ Property paths (`^`, `+`, `*`, `?`, `|` incl.
      `(a|b)+`/`(a|b)*`/`(a|b)?`/`^(a|b)`) + materialised-closure
      no-CTE fallback — shipped v0.4 Phase E (E1 → E4). Simple
      sequence already works because spargebra desugars `:a/:b`
      into a BGP chain; the §7.1 sequence-arm / sequence-inner
      remainder stays gated, negated sets out of v0.4 scope. LLD
      v0.4 §7.
- ✅ `CONSTRUCT` — full surface shipped across Phase D countdown
      slices 59 → 51 (slice 50 = the v0.4.4 release cut remains).
      slice 59 landed (foundation, constant-only
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
      idiom works for empty-result queries too). ✅ slice 52 landed
      (`pgrdf.sparql_parse` CONSTRUCT enrichment: returns `form:
      "CONSTRUCT"` with a `template` block reporting `triple_count` /
      `has_variables` / `has_blank_nodes` / `has_constants_only` /
      `variables`, and a `where_shape` block reporting `kind` (Bgp /
      Optional / Union / Minus / Graph / Filter / Bind / Values /
      Group / OrderBy / Distinct / Service) / `triple_count` /
      `named_graphs_used` / `variables`. Shorthand form
      (`CONSTRUCT WHERE { … }`) surfaces via the `shorthand` flag,
      detected with the same ASCII probe `pgrdf.construct` uses
      (slice 54). `unsupported_algebra` flags `Distinct` / `OrderBy` /
      `Group` / `Aggregate` wrappings — `pgrdf.construct` panics on
      these at execute time per LLD §6.2.). ✅ slice 51 landed
      (W3C-shape CONSTRUCT conformance fixtures 30-35 in
      `tests/w3c-sparql/` — basic bnode+var+multi-triple §16.2.1,
      WHERE shorthand §16.2.4, constant-template multiplicity §16.2,
      variable-GRAPH §13.3, typed/lang-literal term shaping, and
      round-trip via `pgrdf.put_construct_rows`; the harness gained a
      surgical per-fixture `kind: construct` selector routing through
      `pgrdf.construct` instead of `pgrdf.sparql` — plus a docs /
      spec / guide coherence sweep). Phase D is feature- and
      test-complete; only slice 50 (the v0.4.4 release cut) remains.
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
- ✅ W3C SHACL Core conformance manifest runner — shipped in
      v0.5.0 (genuine 25/25 full-pass; see
      [`SPEC.pgRDF.LLD.v0.5.md §6`](../specs/SPEC.pgRDF.LLD.v0.5.md)).

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
- ✅ W3C SHACL Core manifest runner. Real SHACL output landed in
  v0.4 via ERRATA.v0.4 E-011; the manifest runner shipped in
  v0.5.0 (genuine 25/25 full-pass, per
  [`SPEC.pgRDF.LLD.v0.5.md §6`](../specs/SPEC.pgRDF.LLD.v0.5.md)).
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
  Real SHACL output landed in v0.4 (ERRATA E-011); the SHACL Core
  manifest gate shipped in v0.5.0 (genuine 25/25 full-pass) per
  [`SPEC.pgRDF.LLD.v0.5.md §6`](../specs/SPEC.pgRDF.LLD.v0.5.md).

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

### Track 5 — Property paths (Phase E countdown — groups E1 ✅ E2 ✅ E3 ✅ E4 ✅, **CLOSED**)

`*`, `+`, `?`, `^`, plus the alternation `p1|p2` stretch (shipped).
Recursive operators translate to recursive Postgres CTEs with a
`pgrdf.path_max_depth` GUC; the materialised-closure fast path
falls back to a direct BGP match when the predicate's closure is
already materialised (no recursive CTE emitted). Phase E was
grouped into four dispatches, all landed:

- ✅ **Group E1 (slices 49 → 46) — foundation + `^` inverse.**
  Property-path AST detection in the shared WHERE walker + the
  `query::path::translate_property_path` dispatcher. `^` inverse
  fully supported (`?s ^p ?o` ≡ `?o p ?s`; nested `^(^p)` folds by
  parity; bare-predicate degenerate `Path` lowers to a triple).
  Composes with GRAPH scoping / BGP joins / OPTIONAL-UNION-MINUS /
  `pgrdf.construct` (shared walker → inherited, not special-cased).
  New GUC `pgrdf.path_max_depth` (Userset, default 64, range
  1..1024) registered in `_PG_init`; new `pgrdf.stats()` field
  `path_depth_truncations` (cross-backend shmem counter, 0 in E1,
  zeroed by `shmem_reset()` — depth enforcement + the increment land
  with the recursive CTE in group E2). Recursive `*`/`+`/`?` and `|`
  preview-panic with stable rollout-schedule prefixes. New
  regression `108-property-path-inverse.sql` (+4 pgrx tests, +8
  host-only `query::path` unit tests). Sequence paths rejected with
  a pointer to the equivalent multi-pattern BGP; negated property
  sets out of v0.4 scope.
- ✅ **Group E2 (slices 45 → 42) — `+` (one-or-more) + depth guard +
  the `src/query/path.rs` carve.** First recursive CTE: `+` lowers to
  the LLD v0.4 §7.2 `WITH RECURSIVE walk(src, dst, depth)` as a
  derived FROM relation (exposes `subject_id`/`object_id` like a quad
  alias, so it joins through the unchanged BGP machinery — composes
  with GRAPH scoping / BGP joins / OPTIONAL-UNION-MINUS /
  `pgrdf.construct` for free). Postgres's `CYCLE src, dst` clause
  makes cyclic graphs terminate after one lap (a bare `UNION` can't,
  once the working tuple carries the depth column); `^p+`/`(^p)+`
  walk the inverse edge. **Depth guard now
  enforced:** the recursive arm caps at `pgrdf.path_max_depth`
  (truncate, never error) and a per-`+` post-execution probe bumps
  `path_depth_truncations` when the cap actually cut a continuable
  path (never under-counts; benign over-count per §7.2). All
  property-path SQL generation now lives in `src/query/path.rs`
  (classifier + recursive-CTE builder + truncation probe +
  preview-panics); `executor.rs` only calls `path::…`. New
  regression `109-property-path-plus.sql`; the E1 `+`-preview pgrx
  test is replaced by chain / cycle / depth-guard `#[pg_test]`s +
  a `*`-still-panics negative; `108`'s `+`-related asserts re-targeted
  to E2 reality. `*`/`?` (E3), `|` (E4), nested-recursive `+` (E4),
  and negated sets still preview-panic with stable prefixes.
- ✅ **Group E3 (slices 41 → 38) — `*` / `?` + full W3C SPARQL 1.1
  §9.3 zero-length-path semantics.** `*` lowers to the E2 cycle-safe
  recursive `+` walk `UNION` the zero-length node-set; `?` is
  non-recursive — the single direct edge `UNION` the SAME zero-length
  set (no depth guard). The LLD §7.2 `SELECT ?s ?s` reflexive sketch
  is refined to the precise W3C §9.3 `ZeroLengthPath` rules (exactly
  as E2 refined §7.2's bare-`UNION` to the `CYCLE` clause): a **bound**
  endpoint's self-pair `(x,x)` holds unconditionally — the queried
  IRI is registered as an RDF term (term reference, no quad added) so
  the opposite projected variable resolves it; an **unbound**
  endpoint's node-set is the DISTINCT subject∪object of the active
  scope, scoped to the active `GRAPH` (predicate-agnostic). Inverse
  composition (`^(p*)`/`(^p)*`/`^(p?)`/`(^p)?`), GRAPH `<iri>`/`?g`
  scoping, BGP joins and `pgrdf.construct` all inherited.
  `*`/`?` logic lives in `src/query/path.rs`
  (`build_zero_or_more_relation_sql` / `build_zero_or_one_relation_sql`
  + the shared `zero_length_node_set_sql`); `executor.rs` only wires.
  New regression `110-property-path-star-opt.sql` (invariants A–K, all
  hand-computed); the E2 `*`-still-panics pgrx negative is replaced by
  reflexive-chain / both-var-exact-set / isolated-bound-identity /
  `?`-direct-∪-identity `#[pg_test]`s + a `|`-still-panics negative.
  `|` (E4), nested-recursive (E4) and negated sets still preview-panic.
- ✅ **Group E4 (slices 37 → 35) — `|` alternation +
  materialised-closure no-CTE fallback + Phase E W3C-shape
  consolidation + the v0.4.5 release.** The §7.1 alternation stretch
  shipped in full: the predicate match was generalised from a
  single `predicate_id = $P` to a predicate **set**
  (`predicate_id IN (…)` — a 1-element set is identical, so plain
  `+`/`*`/`?` are unchanged), a cheap uniform one-line change at
  each builder. Top-level `a|b` (non-reflexive single step), the
  n-ary `a|b|c`, the recursion compositions `(a|b)+`/`(a|b)*`/
  `(a|b)?`, and the inverse `^(a|b)`/`(^a|^b)` all execute. The
  §7.1-permitted gated remainder (an alternation arm that is itself
  a sequence/recursive path; a recursive op whose inner box is a
  sequence) stays preview-panicking with the stable nested-recursive
  prefix — folding it composes a recursive CTE inside an alternation
  arm (the balloon §7.1 explicitly permits gating). The
  materialised-closure no-CTE fallback landed: for a `+`/`*` over a
  single well-known transitive predicate (`rdfs:subClassOf` /
  `rdfs:subPropertyOf` / `owl:sameAs`) with `is_inferred` rows
  present, the translator emits a direct match (no `CTE Scan` in
  the executed plan — §7.3 acceptance, EXPLAIN-scraped via the new
  `pgrdf.sparql_sql` debug hook). The deferred-all-phase Phase E
  W3C-shape consolidation landed: 6 fixtures `36-path-inverse` …
  `41-path-materialised` (35 → 41). New regression
  `111-property-path-materialised-closure.sql` (invariants A–F);
  the 108/109/110/80/30 alternation-panic locks re-targeted to the
  gated remainder; the parser path-flag pgrx test updated (`|` no
  longer flagged, the sequence-arm form is). v0.4.5 release cut.

See
[LLD v0.4 §7](../specs/SPEC.pgRDF.LLD.v0.4.md#7-property-paths-deferred-from-v03-now-in-scope).

### Carried backlog — SPARQL surface gaps from v0.3 (✅ CLOSED — v0.4.6)
Multi-triple `OPTIONAL { BGP }` (LATERAL-style derived-table refactor),
`VALUES` inline tables, `BIND` output usable in later FILTER/BGP,
aggregates over `UNION`, `DESCRIBE`, and type-aware `ORDER BY`. All
shipped across the Phase F countdown (F1 → F4) and released in
**v0.4.6** — they share the translator machinery §4 + §6 already
require. **§11 is complete.** Residual aggregate-over-UNION
refinements ✅ shipped in v0.5.0 —
[`LLD v0.5 §8`](../specs/SPEC.pgRDF.LLD.v0.5.md) (the six residuals
lifted; the genuinely-mixed degenerate stays a stable panic, never
a wrong answer). See
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

### Conformance runner wiring (v0.4) ✅
The W3C SPARQL 1.1 shape-conformance runner (`tests/w3c-sparql/`,
gated `if: false` in v0.3) is wired in v0.4 and gates the §11 SPARQL
backlog. All §11 forms are online and W3C-shape-locked: the suite
stands at **47 fixtures** (Phase E added 36-41 for property paths;
Phase F group F4 added 42-47 — optional-multi-triple, values-inline,
bind-downstream, aggregate-over-union, describe, order-by-type-aware,
the `describe` fixture introducing a `describe` per-fixture kind
alongside the slice-51 `construct` kind). See
[LLD v0.4 §13](../specs/SPEC.pgRDF.LLD.v0.4.md#13-test-policy-continues-v03-6-unchanged-in-spirit).

### Track 6 — Real SHACL validation (✅ landed)
`pgrdf.validate(data, shapes)` ships the real W3C `sh:ValidationReport`-shape
JSONB, backed by `shacl 0.3.x` (rudof). Unblocked via the patched
`reasonable` fork tracked in ERRATA.v0.4 E-011. Regression
fixtures `70-validate-stub.sql` (basic shape) and
`71-shacl-real.sql` (LLD §9 violations). See
[LLD v0.4 §9](../specs/SPEC.pgRDF.LLD.v0.4.md#9-shacl-real-integration-shipped-in-v04-cycle).

### v0.5 cycle — Phase G (→ v0.5.0-rc1)

**Phase G started.** Grouped dispatches: **G1 ✅ shipped** (§3
reasoning-profile selector + §7 IRI lifecycle overloads — slices
21-18); **G2 ✅ shipped** (§4 TriG / N-Quads + §8 agg-over-UNION
residuals — slices 17-14); G3 (§5 / §6 SHACL + the v0.5.0-rc1 cut).

- ✅ **G1 (§3 + §7)** — `pgrdf.materialize(graph_id, profile TEXT
  DEFAULT 'owl-rl')` adds the `'rdfs'` profile (a strict, sound
  RDFS rule subset; route 2 — pgRDF-internal RDFS forward-chain,
  since `reasonable` has no upstream RDFS-only mode) alongside
  `'owl-rl'`; JSONB gains a `profile` field; unknown profiles error
  (no silent fallback). Closes the **last ONTOSYS P1 capability
  gap**. IRI-keyed overloads `pgrdf.{drop,clear,copy,move}_graph(iri
  TEXT, …)` dispatch to the v0.4 §5 partition-DDL path (error
  `<fn>: unknown iri` on an unbound IRI). The bare
  `pgrdf.materialize(g)` form is unchanged.

- ✅ **G2 (§4 + §8)** — `pgrdf.parse_trig(content, default_graph_id,
  strict)` + `pgrdf.parse_nquads(...)` ingest TriG / N-Quads,
  honouring inline / 4th-position graph IRIs (auto-allocate via
  v0.4 §3.2, or reject under `strict` with the stable
  `parse_{trig,nquads}: unknown graph iri` prefix), reusing the
  v0.3 batched-insert path partition-routed per graph. The six
  LLD v0.5 §8 aggregate-over-UNION residuals are closed (the F2
  stable panics are lifted, correct answers returned): GRAPH-scope
  group key (text-lane), computed-BIND join key (correlation
  predicate), BIND in CONSTRUCT/DESCRIBE template (lexical-literal
  encoder), nested UNION-of-UNION (UNION-over-JOIN distribution),
  cross-branch HAVING (qU-pooled, lanes-aware), GROUP_CONCAT
  DISTINCT+SEPARATOR. Closes LLD v0.5 §4 + §8 (shipped in v0.5.0).

- ✅ **G3 (§5 + §6) — v0.5.0-rc1 cut.** `pgrdf.validate(data,
  shapes, mode TEXT DEFAULT 'native')` adds the `mode` argument
  (fully wired + validated; JSONB gains a `mode` field; unknown
  mode → `validate: unknown mode`, no silent fallback). §5.3 #2
  (validation against a `pgrdf.materialize`-d graph reports
  violations against entailed triples) is **fully met**. §5.3 #1
  is **adjusted per ERRATA.v0.5 E-012**: `shacl 0.3.1` has no
  SHACL-SPARQL constraint component AND its `SparqlEngine` is an
  upstream stub (`unimplemented!()`), so `'sparql'` returns a
  clean deterministic structured report (no panic),
  forward-compatible. New `just test-shacl-manifest` harness
  (`tests/w3c-shacl/`, vendored W3C SHACL Core, hermetic) wired
  into CI on every PG major — Core **25/25 full-pass** on the
  `sh:conforms` invariant (ERRATA.v0.5 E-013, corrected: no
  upstream `sh:nodeKind` bug; `prop-nodeKind-001` restored to
  `fixtures/core/` and PASSing — genuine 25/25, no exclusion).
  Phase G W3C-sparql consolidation: 4 new fixtures 48-51 (RDFS/
  OWL-RL profile + TriG/N-Quads loaded GRAPH queries; 47 → 51).
  New pg_regress `122-shacl-modes` (84 → 85); +4 pgrx tests
  (270 → 274). Closes LLD v0.5 **§5 + §6 — all v0.5-gate
  tracks §3-§8 complete**; cut as **v0.5.0-rc1**, then released
  **final as v0.5.0** after Phase H hygiene (E-012 documented
  upstream-gate, E-013 resolved).

**Phase G complete; the v0.5-gate scope is COMPLETE (§3-§8 all
shipped) and RELEASED as v0.5.0.** Phase H hygiene: spec
promotion (the v0.5 LLD promoted from forward-looking to
authoritative; `SPEC.pgRDF.LLD.v0.6-FUTURE.md` opened), the new
`oci-publish.yml` workflow, and the final **v0.5.0** cut+tag. The **`executor.rs` core-BGP module
carve** is **explicitly DEFERRED post-v0.5.0** to
[`SPEC.pgRDF.LLD.v0.6-FUTURE.md §3`](../specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md)
(a large behaviour-neutral refactor — too risky to gate the
v0.5.0 cut on, zero user-facing benefit). E-012 is a documented
upstream-gate (final for v0.5.0); E-013 is resolved (no upstream
bug, §6 genuine 25/25).

See
[LLD v0.5 §3](../specs/SPEC.pgRDF.LLD.v0.5.md#3-reasoning-profile-selector--shipped-phase-g-group-g1),
[§4](../specs/SPEC.pgRDF.LLD.v0.5.md#4-trig--n-quads-ingest--shipped-phase-g-group-g2),
[§5](../specs/SPEC.pgRDF.LLD.v0.5.md#5-shacl-sparql-constraint-mode--materialised-graph-coverage),
[§6](../specs/SPEC.pgRDF.LLD.v0.5.md#6-w3c-shacl-manifest-runner--shipped-phase-g-group-g3),
[§7](../specs/SPEC.pgRDF.LLD.v0.5.md#7-iri-overloads-for-lifecycle-udfs--shipped-phase-g-group-g1).

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
  [LLD v0.6-FUTURE §9](../specs/SPEC.pgRDF.LLD.v0.6-FUTURE.md#9-federated-service).
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
| **Phase E property paths shipped (v0.4.5 cut)** | **230** | **73 + 41 + 3 = 117** | **Total 347 tests across all five layers** (230 pgrx integration + 73 pg_regress + 41 W3C-shape SPARQL + 3 LUBM-shape). Phase E countdown 49 → 35 closed against the v0.4.4 cut: E1 `^` inverse + `pgrdf.path_max_depth` GUC (slices 49 → 46), E2 `+` recursive CTE + depth guard + the `src/query/path.rs` carve (slices 45 → 42), E3 `*`/`?` with full W3C SPARQL 1.1 §9.3 zero-length semantics (slices 41 → 38), **E4 `\|` alternation (incl. `(a\|b)+`/`(a\|b)*`/`(a\|b)?`/`^(a\|b)`) + materialised-closure no-CTE fallback + Phase E W3C-shape consolidation (slices 37 → 35)**. New pg_regress `108`–`111` (+4 vs v0.4.4's 69; net 73 after intervening UPDATE/CONSTRUCT slices); new W3C-shape fixtures `36-path-inverse` … `41-path-materialised` (35 → 41); new `pgrdf.sparql_sql` debug hook (the §7.3 EXPLAIN-scrape acceptance). The §7.1-permitted gated remainder (alternation arm = sequence/recursive; recursive op inner = sequence) stays preview-panicking by spec allowance; negated property sets out of v0.4 scope. v0.4 LLD §5 phase status: Phase 1 ✅, Phase 2 ✅, Phase 3 🚧 (heap_multi_insert phase B still → v0.4.x), Phase 4 ✅, Phase 5 ✅, **§7 property paths ✅** (Track 5 closed), Phase 6 🚧 (step 3 ⏳). Phase F next |
| **Phase F group F1 shipped (multi-triple OPTIONAL + VALUES)** | **235** | **75 + 41 + 3 = 119** | **Total 354 tests across all five layers** (235 pgrx integration + 75 pg_regress + 41 W3C-shape SPARQL + 3 LUBM-shape). Phase F countdown slices 34 → 31 closed: the LATERAL-style derived-table translator refactor (34), multi-triple `OPTIONAL { BGP }` end-to-end (33), `VALUES` inline tables (32), compose-with-surface + `sparql_parse` unsupported-narrowing (31). N-triple OPTIONAL emits as `LEFT JOIN LATERAL (SELECT … ) qOPT ON TRUE` (atomic, W3C §6.1) — nested OPTIONAL, OPTIONAL-internal FILTER, `LeftJoin.expression` join-FILTER, optional-var outer FILTER, GRAPH `<iri>`/`?g` scoping, `+`-path-in-required all compose; `VALUES` → `(VALUES …) AS vN(cols)` derived table joined on shared vars (constants → dict ids ahead of execution, `UNDEF` → NULL no-constraint per W3C §10, typed/lang literals datatype-aware). Both inherited by `pgrdf.construct` + SPARQL UPDATE WHERE (shared BGP walker). `pgrdf.sparql_parse` no longer flags either in `unsupported_algebra` (LLD §11 acceptance; regression-locked). New pg_regress `112-optional-multi-triple` / `113-values-inline` (+2; retired 80's gap-2/gap-3); +5 pgrx tests (230 → 235). Translator stays in `src/query/executor.rs` (`build_optional_block`/`emit_optional_lateral`/`emit_values_table`) — core BGP translation, too entangled with `anchors`/`ScopePlan`/projection for a clean carve; flagged as a Phase H carve candidate. v0.4 LLD §5 phase status: Phase 1 ✅, Phase 2 ✅, Phase 3 🚧, Phase 4 ✅, Phase 5 ✅, Phase 6 🚧. §11 still 🚧 (F2 BIND-downstream + aggregates-over-UNION, F3 DESCRIBE, F4 W3C+docs+v0.4.6 cut pending) |
| **Phase F group F2 shipped (downstream BIND + aggregates-over-UNION)** | **240** | **77 + 41 + 3 = 121** | **Total 361 tests across all five layers** (240 pgrx integration + 77 pg_regress + 41 W3C-shape SPARQL + 3 LUBM-shape). Phase F countdown slices 30 → 27 closed: BIND-downstream substitution machinery (30), BIND usable in later FILTER + BGP join + chained BIND (29), aggregates-over-UNION derived-table refactor (28), compose-with-surface + `sparql_parse` unsupported-narrowing + 80-gap-8 retirement (27). **Downstream BIND** = an AST substitution pass (`substitute_binds`/`subst_expr`/`subst_triple`, LLD §11's named approach): a BIND var is rewritten to its expression in every later FILTER / triple-slot join key / chained BIND **before** the structural walk, so the existing anchors-driven translator resolves it with zero new surface; unbound-var BIND → NULL not error (W3C §18.2.5). **Aggregates over UNION** = a derived-table refactor reusing F1's `vK` column pool — each branch sub-SELECTs the agg/GROUP-BY vars' dict ids, branches `UNION ALL` into `(<union>) qU`, the existing `translate_aggregate` runs over it unchanged (COUNT/SUM/AVG/type-aware MIN-MAX/GROUP_CONCAT/SAMPLE, DISTINCT, GROUP BY, HAVING, GRAPH scoping, property-path branch). Both compose with GRAPH + F1 OPTIONAL/VALUES and are inherited by `pgrdf.construct` + SPARQL UPDATE WHERE (WHERE-side; a BIND var in a CONSTRUCT *template* output position → LLD v0.5 §8). New pg_regress `114-bind-downstream` / `115-aggregate-over-union` (+2; retired 80's gap-8); +5 pgrx tests (235 → 240). `pgrdf.sparql_parse` no longer flags either form in `unsupported_algebra` (LLD §11 acceptance; regression-locked). Translator stays in `src/query/executor.rs` (`substitute_binds`/`build_aggregate_over_union_sql`) — same anchors/projection entanglement as F1, Phase H carve candidate. Residual: GROUP-BY-on-GRAPH-scope-only-var over UNION + computed-BIND-as-join-key + BIND-in-construct-template → LLD v0.5 §8 (stable panics, never wrong answers). §11 still 🚧 (F3 DESCRIBE, F4 W3C+docs+v0.4.6 cut pending) |
| **Phase F group F3 shipped (DESCRIBE)** | **248** | **78 + 41 + 3 = 122** | **Total 370 tests across all five layers** (248 pgrx integration + 78 pg_regress + 41 W3C-shape SPARQL + 3 LUBM-shape). Phase F countdown slices 26 → 24 closed: DESCRIBE parse + the `pgrdf.describe` UDF + bare `DESCRIBE <iri>` closure (26), variable / mixed / `DESCRIBE *` forms + blank-node transitive-one-hop expansion (25), compose-with-surface — GRAPH scoping, `sparql_parse` `form:"DESCRIBE"` + `unsupported_algebra` removal, `80`-gap-6 retirement, `pgrdf.sparql` redirect-panic — + full sweep (24). **`pgrdf.describe(q TEXT) → SETOF JSONB`** is the sibling UDF to `pgrdf.construct` (the §6.1 sibling-UDF rationale: the caller signals intent at the SQL boundary; a DESCRIBE through `pgrdf.sparql` panics `sparql: use pgrdf.describe(q) for DESCRIBE queries`). Output is **byte-identical** to `pgrdf.construct`'s `{subject,predicate,object}` structured-term JSONB (same encoders — no new shaper). The description is the **closure** of each described resource: every `(R, ?p, ?o)` triple, transitively expanded one hop through blank-node objects per W3C §16.4 (recursion only ever traverses blank-node objects so it terminates on any finite graph; a visited-set of bnode ids additionally guards bnode cycles; triples dedup'd across the whole result — set semantics). Forms: `DESCRIBE <iri>` (constant, no WHERE — empty IRI → 0 rows, not an error), `DESCRIBE ?v WHERE {…}`, mixed constant+variable, `DESCRIBE *`; composes with `GRAPH <iri>` scoping (closure within the named graph), unscoped scans every graph (slice-112 semantic). spargebra normalises every form to `Project { inner, variables }` (constant terms as leading `Extend { …, NamedNode(iri) }` layers the executor peels). `pgrdf.sparql_parse` reports `form:"DESCRIBE"` + a `describe` block (`kind` ∈ constant/variable/mixed) + `where_shape`, NOT flagged in `unsupported_algebra` (LLD §11 acceptance; regression-locked). New pg_regress `116-describe` (+1; retired 80's gap-6); +8 pgrx tests (240 → 248). The `describe`/closure machinery sits next to `pgrdf.construct` in `src/query/executor.rs` (`describe`/`collect_describe_var_bindings`/`describe_closure`/`closure_one_hop`/`literal_graph_scope`/`lookup_iri_dict_id`) — additive, no core-BGP carve attempted (the entangled core-BGP carve stays a Phase H candidate, as in F1/F2). §11 surface backlog now functionally complete EXCEPT type-aware ORDER BY (F4 assesses/closes that + W3C consolidation + v0.4.6 cut). §11 still 🚧 (F4 pending) |
| **Phase F group F4 shipped — §11 complete (v0.4.6 cut)** | **250** | **79 + 47 + 3 = 129** | **Total 379 tests across all five layers** (250 pgrx integration + 79 pg_regress + 47 W3C-shape SPARQL + 3 LUBM-shape). Phase F countdown slices 23 → 22 closed, **§11 SPARQL backlog complete**, released as **v0.4.6**. **Type-aware ORDER BY** (the last §11 item — investigation confirmed pre-F4 ORDER BY was a real lexical-only gap: xsd:integer literals sorted `"1","10","100","2"`). F4 expands every sort key into the SPARQL 1.1 §15.1 value-space term list: a kind rank (numeric < dateTime < boolean < other) groups comparable lexical spaces, then a per-kind comparator — numerics **numerically** (`2 < 10`), `xsd:dateTime` chronologically, `xsd:boolean` false<true, strings by Unicode codepoint (`COLLATE "C"`, locale-independent) — + a final codepoint tiebreak; total/stable, **never raises** (regex-guarded numeric/dateTime casts fall through to the codepoint tier — the §15.1 stable fallback). `DESC()` + multi-key + **expression sort keys** (`ORDER BY (?a+?b)`, `ORDER BY STRLEN(?s)`, via the shared BIND/FILTER translator); all four SQL builders (single-branch / aggregate / UNION / aggregate-over-UNION) order over the underlying SQL expr (group/aggregate/dict-lookup/BIND), never an output alias buried in an expression (Postgres rejects that); `SELECT DISTINCT` + ORDER BY wraps the dedup in an outer derived table. Expression sort keys on the aggregate/UNION shapes are a documented narrow deferral (BIND it, then ORDER BY the var) — stable panic, never a wrong answer. ORDER BY was already an unflagged SELECT modifier — no `unsupported_algebra` / `80-unsupported-shapes` entry to retire. New pg_regress `100-sparql-order-by-type-aware` (+1; `111` expected output corrected to §15.1 codepoint order — uppercase IRIs now sort before lowercase, spec-mandated); +2 pgrx tests (248 → 250). **Phase F W3C-shape consolidation**: 6 new fixtures `42-optional-multi-triple` / `43-values-inline` / `44-bind-downstream` / `45-aggregate-over-union` / `46-describe` / `47-order-by-type-aware` (41 → 47); the `46-describe` fixture introduced a `describe` per-fixture kind in `tests/w3c-sparql/run.sh` alongside the slice-51 `construct` kind. **Compose infra-debt fix**: `compose/compose.yml` per-version SQL bind-mounts (`pgrdf--0.4.1.sql` … `pgrdf--0.4.5.sql`) replaced with one version-agnostic `extensions/share/extension` directory mount — `just build-ext` no longer requires a hand-created stale-version copy + cold restart. Residual aggregate-over-UNION refinements tracked in `SPEC.pgRDF.LLD.v0.5 §8` (stable panics). v0.4 LLD §5 phase status: Phase 1 ✅, Phase 2 ✅, Phase 3 🚧 (heap_multi_insert phase B still → v0.4.x), Phase 4 ✅, Phase 5 ✅, **§11 SPARQL backlog ✅ (Track closed)**, Phase 6 🚧 (step 3 ⏳). **Phase F complete; Phase G next** (reasoning-profile selector / TriG / N-Quads → v0.5.0-rc1) |
| **Phase G group G1 shipped (reasoning-profile selector + IRI lifecycle overloads)** | **259** | **81 + 47 + 3 = 131** | **Total 390 tests across all five layers** (259 pgrx integration + 81 pg_regress + 47 W3C-shape SPARQL + 3 LUBM-shape). Phase G countdown slices 21 → 18 closed (the 1st of 3 grouped G dispatches; G2 = §4 TriG/N-Quads + §8 agg-UNION residuals, G3 = §5/§6 SHACL + the v0.5.0-rc1 cut). **§3 reasoning-profile selector** (the last ONTOSYS P1 capability gap): `pgrdf.materialize(graph_id BIGINT, profile TEXT DEFAULT 'owl-rl') → JSONB`. The bare `pgrdf.materialize(g)` form is **unchanged** (defaults `profile => 'owl-rl'`, byte-identical to the v0.3/v0.4 OWL-RL path — `60-materialize-owl-rl`, LUBM, materialize pgrx tests all green unchanged). `'rdfs'` adds the RDFS entailment-rule subset; route chosen = **route 2 (pgRDF-internal RDFS forward-chain)** since the patched `reasonable` fork exposes only a fused OWL-RL fixpoint (no upstream RDFS-only mode), implemented as a *strict, sound, complete* RDFS rule engine (rdfs2/3/5/7/9/11 — the six productive rules; the axiomatic `rdfs:Resource`-typing rules deliberately omitted so `rdfs` stays a true subset of `owl-rl`, making the §3.1 subset + agreement criteria hold by construction). JSONB gains a `profile` field. Unknown profiles (incl. the reserved-future `'owl-rl-ext'`) error `materialize: unknown profile …` validated **before** any side effect — no silent fallback. **§7 IRI-keyed lifecycle overloads**: `pgrdf.{drop,clear,copy,move}_graph(iri TEXT, …) → BIGINT` resolve `iri → graph_id` via `_pgrdf_graphs.iri` and dispatch to the **existing v0.4 §5 BIGINT UDFs** (no partition-DDL logic duplicated — re-enter through the SQL surface, same single-sourcing as `add_graph_iri`); the one intentional difference vs BIGINT is `<fn>: unknown iri` on an unbound IRI (distinct from the BIGINT no-op-returns-0). New pg_regress `117-materialize-rdfs` / `118-lifecycle-iri-overloads` (+2; 79 → 81); +9 pgrx tests (250 → 259: 5 §3 + 4 §7). W3C stays **47** (Phase G W3C consolidation is G3), LUBM stays **3**. `pgrdf.materialize` + the `rdfs_closure` engine live in `src/inference/reasonable.rs`; the IRI overloads in `src/storage/graphs.rs`. LLD v0.5 §3 + §7 ✅ shipped; §4/§5/§6/§8 still 🚧 (G2/G3). **Phase G group G2 next** (§4 TriG/N-Quads + §8 agg-UNION residuals) |
| **Phase G group G2 shipped (TriG/N-Quads + agg-over-UNION residuals)** | **270** | **84 + 47 + 3 = 134** | **Total 404 tests across all five layers** (270 pgrx integration + 84 pg_regress + 47 W3C-shape SPARQL + 3 LUBM-shape). Phase G countdown slices 17 → 14 closed (G2 of 3). **§4 TriG / N-Quads ingest**: `pgrdf.parse_trig(content, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE)` + `pgrdf.parse_nquads(...)` honour inline `GRAPH <iri> { … }` / 4th-position graph IRIs, resolved via v0.4 §3.2 (auto-allocate, or reject under `strict` with the stable `parse_{trig,nquads}: unknown graph iri` prefix), reusing the v0.3 batched-insert path partition-routed per graph. **§8 aggregate-over-UNION residuals**: the six F2 stable panics are lifted (correct answers, not wrong): GRAPH-scope group key (parallel text-lane), computed-BIND join key (correlation predicate), BIND in CONSTRUCT/DESCRIBE template (lexical-literal encoder), nested UNION-of-UNION (UNION-over-JOIN distribution), cross-branch HAVING (qU-pooled lanes-aware), GROUP_CONCAT DISTINCT+SEPARATOR. New pg_regress `119-parse-nquads`/`120-parse-trig`/`121-agg-union-residual` (+3; 81 → 84); +11 pgrx tests (259 → 270). W3C stays **47** (consolidation is G3), LUBM **3**. Closes LLD v0.5 §4 + §8. **Phase G group G3 next** (§5/§6 SHACL + the v0.5.0-rc1 cut) |
| **Phase G group G3 shipped — v0.5-gate COMPLETE (v0.5.0-rc1 cut)** | **274** | **85 + 51 + 24 + 3 = 163** | **Total 437 tests across SIX layers** (274 pgrx integration + 85 pg_regress + 51 W3C-shape SPARQL + **24 W3C SHACL Core** + 3 LUBM-shape). Phase G countdown slices 13 → 12 closed, **all v0.5 v0.5-gate tracks §3-§8 complete**, released as **v0.5.0-rc1** (a release *candidate*). **§5 SHACL-SPARQL mode**: `pgrdf.validate(data, shapes, mode TEXT DEFAULT 'native') → JSONB` — the `mode` arg ships fully (accepted, validated, echoed in a new JSONB `mode` field); unknown mode → `validate: unknown mode` raised before any work (no silent fallback). §5.3 #2 (validation against a `pgrdf.materialize`-d graph reports violations against entailed triples) **fully met** (`122-shacl-modes.sql` §E, RDFS profile). §5.3 #1 **adjusted per ERRATA.v0.5 E-012**: `shacl 0.3.1` has no SHACL-SPARQL constraint component (parser silently drops `sh:sparql`) AND its `SparqlEngine` is an upstream stub (`unimplemented!()` in every target-resolution method — invoking it panics); `'sparql'` therefore returns a clean deterministic structured report (`conforms:null` + an `error` naming the gap), never a panic, forward-compatible (one guard deleted the day rudof ships the engine — no signature change). **§6 W3C SHACL manifest gate**: new `just test-shacl-manifest` harness (`tests/w3c-shacl/`, structured like `tests/w3c-sparql/`; vendored W3C SHACL Core subset, hermetic — checked in, never fetched), wired into `ci.yml` on every PG major (14-17) as a REAL matrix gate (no `continue-on-error`/`if:false`). W3C SHACL **Core 24/24 full-pass** on the `sh:conforms` invariant (ERRATA.v0.5 **E-013** — `conforms`, not violation count, since pgRDF's dictionary rehydrate relabels blank-node focus nodes, a serialization artifact not a conformance error); one W3C Core fixture `prop-nodeKind-001` documented-excluded for a true upstream `sh:nodeKind` multi-value bug (E-013) — the **one honest §6.1 #1 caveat for rc1**, carried to Phase H+I for the final v0.5.0. `--sparql` asserts the E-012 known state (`conforms:null` every fixture). **Phase G W3C-sparql consolidation**: 4 new fixtures `48-reasoning-profile-rdfs`/`49-reasoning-profile-owl-rl`/`50-trig-graph-scoped`/`51-nquads-loaded` (47 → 51; hand-computed expected, never ACCEPT=1). New pg_regress `122-shacl-modes` (+1; 84 → 85); +4 pgrx tests (270 → 274); new `specs/ERRATA.v0.5.md` (E-012, E-013). LLD v0.5 §2 scope: **§3-§8 ALL ✅ shipped** — v0.5-gate scope COMPLETE (the v0.5.0-rc1 headline). **Phase G complete; Phase H+I next** (final hygiene + ERRATA close-outs + executor.rs carve catch-up + E-012/E-013 SHACL follow-ups + the final **v0.5.0** cut — one release event) |
