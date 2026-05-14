# 10 — Roadmap

Phase structure mirrors LLD §7 (Development Checklist & Progression).
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

**Not yet shipped at this phase boundary** (Phase 2.x backlog):
- ⏳ **Shmem dictionary cache (LLD §4.1)** — `pgrx::shmem` +
      `RwLock<LruCache<u64, i64>>` keyed by RdfTerm hash. The
      per-call HashMap pays for itself within a single ingest call
      but doesn't survive across calls or backends. Shipping this
      is the highest-leverage performance work remaining; expected
      cache-hit latency target is < 1 µs.
- ⏳ **Prepared-plan cache (LLD §4.2)** — `Spi::prepare` + algebra-hash
      keyed cache. Today the executor builds a dynamic SQL string
      per call and runs `Spi::connect_mut(|c| c.update(...))`.
      Postgres re-parses + re-plans every call; the LLD's optimization
      is to bypass both via prepared statements.
- ⏳ **COPY BINARY ingestion (LLD §4.3)** — current batched INSERT is
      ~50× faster than row-by-row INSERT but still slower than the
      LLD's stated COPY-BINARY target. Worth re-measuring against
      the synth-100 / smoke-ontologies fixtures before committing.
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
| 7 | Aggregates — `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT)`, `SUM`, `AVG`, `MIN`, `MAX` + `GROUP BY` | (pending) | 63 | 20 |

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

## Phase 4 — Semantic Engine (Inference + Validation) ⏳

Outcome: materialized OWL 2 RL inference and SHACL validation work
against real ontologies. Tracks LLD §7 Phase 3.

Gates:
- ⏳ `pgrdf.materialize(graph_id BIGINT)` — streams `_pgrdf_quads`
      through `reasonable` (OWL 2 RL — see ERRATA E-002), writes
      inferred quads with `is_inferred = TRUE`. Target ingest path
      is the same one that lands for §4.3 (COPY BINARY).
- ⏳ `pgrdf.validate(data BIGINT, shapes BIGINT) → JSONB` —
      W3C-conformant `sh:ValidationReport` via `shacl_validation`
      (per ERRATA E-001, NOT `shacl-rust`).
- ⏳ W3C SPARQL 1.1: ≥ 70 % pass. SHACL: ≥ 50 % pass.
- ⏳ Reasoner correctness gated by a small fixed OWL 2 RL fixture
      (pizza ontology subset) + diff against expected closure.

---

## Phase 5 — Release & Containerization ⏳

Outcome: pgRDF is consumable by external operators (CloudNativePG,
StackGres) following INSTALL spec methodology. Benchmarked.
Tracks LLD §7 Phase 4.

Gates:
- ⏳ LUBM-100 results in `target/perf-report.json` compared against
      Apache Jena TDB and Apache AGE.
- ⏳ OCI artifact published at `ghcr.io/styk-tv/pgrdf-bundle:<ver>`
      (INSTALL §11 OQ1).
- ⏳ INSTALL §12 conformance test in CI against a fresh K8s cluster
      (kind or k3s).
- ⏳ SHA256SUMS.asc detached GPG signature attached to every release.
- ⏳ W3C SPARQL 1.1: ≥ 95 % pass. SHACL: ≥ 90 % pass.

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
| Phase 3 step 7 (current) | 63 | 20 | + aggregates (COUNT/SUM/AVG/MIN/MAX + GROUP BY) |
