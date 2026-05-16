# **SPEC.pgRDF.LLD.v0.6-FUTURE**

**pgRDF: A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL,
and OWL 2 RL reasoning.**

*Positioning: pgRDF — the high-performance PostgreSQL semantic-web toolkit.*

---

## 0. Document status and supersession

- **Status:** draft / forward-looking / **target: pgRDF v0.6+ /
  v1.0**. This document is aspirational, not authoritative. The
  `-FUTURE` postfix signals it is a sibling forward look, not a
  shipped contract.
- **Authoritative shipped contract:**
  [`SPEC.pgRDF.LLD.v0.5.md`](SPEC.pgRDF.LLD.v0.5.md). v0.5 LLD is
  the authoritative contract — every v0.5-gate track §3–§8 is
  shipped in pgRDF v0.5.0. This document is a sibling forward look
  beyond v0.5, not a replacement.
- **Carries forward:** [`SPEC.pgRDF.INSTALL.v0.2.md`](SPEC.pgRDF.INSTALL.v0.2.md)
  (no install-spec changes anticipated for v0.6) and
  [`ERRATA.v0.5.md`](ERRATA.v0.5.md) (carried forward into the v0.6
  cycle; v0.6 may open its own ERRATA file once a v0.6-era delta
  appears). [`ERRATA.v0.4.md`](ERRATA.v0.4.md) remains live for the
  E-011 upstream `reasonable` / crates.io-publish gate.
  [`ERRATA.v0.2.md`](ERRATA.v0.2.md) remains live for pre-v0.3
  items still open (E-006 pgrx 0.18 / PG 18). **E-009** (SHACL
  upstream conflict) is resolved in the v0.4 cycle via E-011; the
  v0.6 cycle inherits the resolved state and tracks final
  upstream-merge close-out under E-011.
- **Reason for v0.6-FUTURE:** v0.5 closes the residual v0.4-deferred
  surface (reasoning-profile selector, TriG/N-Quads ingest, SHACL
  `mode` argument, the W3C SHACL Core manifest gate, IRI lifecycle
  overloads, aggregates-over-UNION residuals) and ships v0.5.0. The
  items that remain genuinely deferred — the v1.0 engineering
  targets, the post-v0.5.0 hygiene (the `executor.rs` core-BGP
  module carve), and the two upstream-gated surfaces (RDF 1.2
  triple terms via E-011, a real SHACL-SPARQL engine via E-012) —
  collect here as the next forward look.
- **Tense discipline:** v0.6-FUTURE is forward-looking. Future
  tense ("will land", "ships with", "targets") is the default
  throughout. Where a forward item builds on shipped v0.5
  mechanism, that mechanism is named in present tense and
  explicitly cross-linked to
  [`SPEC.pgRDF.LLD.v0.5.md`](SPEC.pgRDF.LLD.v0.5.md).

## 1. Mission (carry forward from v0.5 / v0.4 / v0.3)

pgRDF is a PostgreSQL extension built entirely in Rust using `pgrx`.
It provides native storage and querying for RDF data directly
inside Postgres, with four engines:

1. **Storage Engine** — dictionary-encoded terms in
   `_pgrdf_dictionary`; quads in `_pgrdf_quads` partitioned by
   `graph_id`; hexastore covering indexes (SPO, POS, OSP).
2. **SPARQL Engine** — `pgrdf.sparql(q TEXT) → SETOF JSONB`;
   spargebra parser; dynamic-SQL executor with prepared-plan cache.
   The full SPARQL 1.1 query surface (SELECT/ASK/CONSTRUCT/DESCRIBE,
   property paths, the §11 backlog, type-aware ORDER BY) + SPARQL
   UPDATE ✅ ship in the v0.4/v0.5 cycle.
3. **Inference Engine** — OWL 2 RL materialisation via `reasonable`;
   the RDFS / OWL-RL profile selector ✅ ships in v0.5 (LLD v0.5
   §3). The reserved-future `'owl-rl-ext'` profile is named in v0.5
   §3 but not yet wired — a v0.6+ candidate (§5 below).
4. **Validation Engine** — SHACL Core via `shacl 0.3.x` (rudof
   project), with the `pgrdf.validate(…, mode)` argument ✅ shipped
   in v0.5 (LLD v0.5 §5). A real SHACL-SPARQL constraint engine is
   an upstream-gated forward item ([`ERRATA.v0.5.md`](ERRATA.v0.5.md)
   E-012; §4 below).

## 2. Scope of v0.6-FUTURE

| Section | Surface | Provenance | Gate |
|---|---|---|---|
| §3 | `executor.rs` core-BGP module carve (post-v0.5.0 hygiene) | LLD v0.5 §8 close-out / Phase H | none (internal refactor, non-gating) |
| §4 | A real SHACL-SPARQL engine | LLD v0.5 §5 / ERRATA.v0.5 E-012 | upstream rudof `IRComponent::Sparql` |
| §5 | RDF 1.2 triple terms | LLD v0.5 §9 / ERRATA.v0.2 E-009 / ERRATA.v0.4 E-011 | upstream `reasonable` (gtfierro/reasonable#50) |
| §6 | `heap_multi_insert` / `COPY BINARY` ingest phase B | LLD v0.4 §12 phase B | none (perf, non-gating) |
| §7 | Postgres custom-scan hooks | LLD v0.4 §12 / LLD v0.5 §9 | none (perf, non-gating) |
| §8 | Incremental (delta-driven) materialisation | LLD v0.5 §9 | none (optimisation, non-gating) |
| §9 | Federated `SERVICE` | LLD v0.5 §10 | explicitly v1.0 |
| §10 | v1.0 contents (forward look) | LLD v0.5 §9 | forward look |

> **None of these gate v0.5.0.** v0.5.0 ships the complete
> RDF/SPARQL/SHACL/OWL surface (LLD v0.5 §3–§8). This document
> collects the genuinely-deferred work for v0.6+ / v1.0.

## 3. `executor.rs` core-BGP module carve (post-v0.5.0 hygiene)

The SPARQL→SQL translator (`src/query/executor.rs`) accreted the
F1 derived-table refactor, the F2 downstream-BIND substitution +
aggregate-over-UNION machinery, the F3/F4 DESCRIBE + type-aware
ORDER BY, and the G2 §8 residual lifts. Correctness was the
priority through Phase F/G (per the F1/F2/F3 "correctness first"
discipline), so the core-BGP translation, the anchors/projection
entanglement, and the aggregate/UNION builders all live in one
module.

**Forward item:** carve a clean core-BGP module out of
`executor.rs` — separating the BGP→SQL lowering, the anchors model,
the aggregate/UNION builders, and the property-path lowering into
cohesive units. This is **explicitly deferred post-v0.5.0**: it is
a large, behaviour-neutral refactor that would add risk to the
v0.5.0 cut for zero user-facing benefit. It is recorded here (and
in `docs/10-roadmap.md`) as the canonical post-v0.5.0 hygiene item.

### 3.1 Acceptance criteria (forward)

- The carve is **behaviour-neutral**: the full pgrx / pg_regress /
  W3C-sparql / W3C SHACL Core / LUBM test bar passes byte-identically
  before and after, with no `ACCEPT=1` re-baselining.
- No new public UDF surface; `pgrdf.sparql_parse`'s
  `unsupported_algebra` set is unchanged.
- `executor.rs` line count drops materially with the core-BGP
  module extracted; no logic moves into a less-tested path.

## 4. A real SHACL-SPARQL engine (upstream-gated — ERRATA.v0.5 E-012)

v0.5 ✅ ships the `pgrdf.validate(data, shapes, mode TEXT DEFAULT
'native')` surface (LLD v0.5 §5). `'sparql'` mode is an honest,
forward-compatible short-circuit: `shacl 0.3.1` has **no**
SHACL-SPARQL constraint component and its `SparqlEngine` is an
`unimplemented!()` stub ([`ERRATA.v0.5.md`](ERRATA.v0.5.md) E-012,
rudof issues #21/#94/#1) — pgRDF returns a deterministic structured
report (`conforms:null` + an `error` naming the upstream gap),
never a panic.

**Forward item:** when a future `shacl` (rudof) release lands an
`IRComponent::Sparql` (or equivalent) + `sh:sparql`/`sh:select`
parsing, promote `'sparql'` from the E-012 short-circuit to a real
SHACL-SPARQL constraint engine. **No signature change** is needed —
the `mode` argument already routes to
`ShaclValidationMode::Sparql`; the E-012 guard in
`src/validation/shacl.rs` is deleted and the already-present
`validator.validate(&schema, &validation_mode)` call routes through.

### 4.1 Acceptance criteria (forward)

- `pgrdf.validate(g, g, 'sparql')` on a shape declaring
  `sh:sparql [ sh:select "…" ]` produces a `sh:Violation` for the
  matching focus node (the literal v0.5 §5.3 #1 form, promoted
  from the E-012 documented-upstream-gate state).
- `just test-shacl-manifest --sparql` is re-baselined from the
  E-012 `conforms:null`-every-fixture known state toward a real
  SHACL-SPARQL manifest full-pass.
- ERRATA.v0.5 E-012 closes (re-check trigger fired).

## 5. RDF 1.2 triple terms (upstream-gated — E-011 / E-009)

Enable `oxrdf`'s `rdf-12` feature and surface RDF 1.2 triple terms
in `pgrdf.sparql` / `pgrdf.materialize` once upstream
(`reasonable` + `shacl_validation`) supports it unanimously.
[`ERRATA.v0.2.md`](ERRATA.v0.2.md) E-009 tracks the original
conflict; the v0.4 cycle's E-011 unblocks coexistence **locally**
via the `styk-tv/reasonable` `rdf12-passthrough` fork, but full
surfacing of triple terms is gated on upstream unanimous support
(and on `gtfierro/reasonable#50` merging so the `[patch.crates-io]`
line + `publish-crate.yml.disabled` can be lifted — E-011).

The reserved-future `'owl-rl-ext'` reasoning profile (named in LLD
v0.5 §3 but not yet wired) is a sibling forward candidate: a
later cycle wires it once a concrete extended-RL rule set is
specified.

### 5.1 Acceptance criteria (forward)

- `gtfierro/reasonable#50` (or its successor) merges; the
  `[patch.crates-io] reasonable = …` line is dropped, `reasonable`
  is pinned to a released version, and `publish-crate.yml.disabled`
  is re-enabled (E-011 close-out).
- A quoted-triple SPARQL query / a triple-term in a materialised
  graph round-trips through `pgrdf.sparql` / `pgrdf.materialize`.

## 6. `heap_multi_insert` / `COPY BINARY` ingest phase B (perf, non-gating)

LLD v0.4 §12 phase B (`heap_multi_insert` / `COPY BINARY` bulk
ingest path) is a performance optimisation that does **not** gate
any v0.x surface. The v0.4/v0.5 batched-insert path
(`flush_batch` / `QUAD_INSERT_SQL` prepared plan) is correct and
sufficient for the v0.5.0 contract; phase B is a throughput
forward item only.

### 6.1 Acceptance criteria (forward)

- Bulk ingest of a large Turtle/TriG/N-Quads document is measurably
  faster than the prepared-plan batched-insert path, with no
  correctness regression on the full test bar.

## 7. Postgres custom-scan hooks (perf, non-gating)

Postgres custom-scan / FDW-style integration hooks (LLD v0.4 §12 /
LLD v0.5 §9). A planner-integration optimisation; non-gating, no
user-facing surface change. Carried forward as a v1.0 engineering
target.

## 8. Incremental (delta-driven) materialisation (optimisation, non-gating)

`pgrdf.materialize_delta(graph_id, since_xid TEXT)` — forward-chain
only over quads added since a recorded transaction id. Targets a
common optimisation pattern in reasoning pipelines but does **not**
gate any v0.x surface (a full `pgrdf.materialize` is always
correct; the delta path is a throughput optimisation).

### 8.1 Acceptance criteria (forward)

- `pgrdf.materialize_delta(g, since)` produces the same closure as
  a full `pgrdf.materialize(g)` for triples added after `since`,
  with a measurable speedup on an incremental workload.

## 9. Federated `SERVICE`

SPARQL 1.1 federated query (`SERVICE <endpoint> { … }`) is
**explicitly deferred to v1.0**; it remains out of scope for v0.x
(LLD v0.5 §10). Callers fetch externally and ingest via
`pgrdf.load_turtle` / `pgrdf.parse_trig` in the interim.

## 10. Forward look — v1.0 and beyond

**v1.0 contents (planned engineering targets):**

- **A real SHACL-SPARQL engine** (§4 — upstream-gated, E-012).
- **RDF 1.2 triple terms** (§5 — upstream-gated, E-011 / E-009).
- **`heap_multi_insert` / `COPY BINARY` ingest phase B** (§6 —
  perf, non-gating).
- **Postgres custom-scan hooks** (§7 — perf, non-gating).
- **Incremental (delta-driven) materialisation** (§8 —
  optimisation, non-gating).
- **Federated `SERVICE`** (§9 — explicitly v1.0).
- **The `executor.rs` core-BGP module carve** (§3 — post-v0.5.0
  hygiene; behaviour-neutral refactor).

No domain-specific motivation appears in this section; the items
are listed as engineering targets only.

## 11. Out of scope (carry forward from v0.5 §10)

- Streaming replication / logical decoding of RDF state.
- Full OWL 2 (EL / QL) reasoning ([`ERRATA.v0.2.md`](ERRATA.v0.2.md)
  E-002 — pgRDF ships OWL 2 RL only via `reasonable`).
- Backup/restore for opaque binary state (tracked by future
  `SPEC.pgRDF.BACKUP.v0.x`, INSTALL §11 OQ5).
- `LOAD <url>` in SPARQL UPDATE — callers fetch externally and
  invoke `pgrdf.load_turtle` or `pgrdf.parse_trig` directly.

## 12. Errata

- This document is the **draft** v0.6-FUTURE forward look. It is
  not authoritative; [`SPEC.pgRDF.LLD.v0.5.md`](SPEC.pgRDF.LLD.v0.5.md)
  is the authoritative shipped contract (v0.5.0).
- Spec corrections discovered during the v0.6 cycle will land in
  [`ERRATA.v0.5.md`](ERRATA.v0.5.md) (continued) or a new
  `ERRATA.v0.6.md` once a v0.6-era delta appears.
- **E-012** (`shacl 0.3.1` SHACL-SPARQL upstream-gate) — final for
  v0.5.0 as a documented upstream-gate; its re-check trigger (a
  rudof release shipping `IRComponent::Sparql`) is the §4 gate.
- **E-011** (upstream `reasonable` patch / crates.io-publish gate)
  — carried forward; the §5 gate. `publish-crate.yml.disabled`
  stays disabled until `gtfierro/reasonable#50` merges.
- **E-009** (SHACL upstream conflict) is resolved in the v0.4
  cycle via E-011 (patched `reasonable` fork). The v0.6 cycle
  inherits the resolved state; final close-out gates on the
  upstream `reasonable` PR merge.
- **E-006** (pgrx 0.18 / PG 18 migration) remains the largest
  deferred upstream item carried into v0.6.
