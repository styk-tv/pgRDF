# **SPEC.pgRDF.LLD.v0.5-FUTURE**

**pgRDF: A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL,
and OWL 2 RL reasoning.**

*Positioning: pgRDF — the high-performance PostgreSQL semantic-web toolkit.*

---

## 0. Document status and supersession

- **Status:** draft / forward-looking / **target: pgRDF v0.5 cut**.
- **Authoritative contract for the in-progress cycle:**
  [`SPEC.pgRDF.LLD.v0.4.md`](SPEC.pgRDF.LLD.v0.4.md). v0.4 LLD is the
  authoritative-in-progress contract; this document is a sibling
  forward look at v0.5, not a replacement.
- **Carries forward:** [`SPEC.pgRDF.INSTALL.v0.2.md`](SPEC.pgRDF.INSTALL.v0.2.md)
  (no install-spec changes anticipated for v0.5) and
  [`ERRATA.v0.4.md`](ERRATA.v0.4.md) (carried forward into the v0.5
  cycle; v0.5 may open its own ERRATA file once a delta appears).
  [`ERRATA.v0.2.md`](ERRATA.v0.2.md) remains live for pre-v0.3 items
  still open. **E-009** (SHACL upstream conflict) is resolved in
  v0.4 cycle via E-011; the v0.5 cycle inherits the resolved state
  and tracks final upstream-merge close-out under E-011.
- **Reason for v0.5:** v0.4 closes the highest-leverage gaps from
  v0.3 as a coherent group (named-graph, UPDATE, lifecycle UDFs,
  CONSTRUCT, paths, SHACL real-impl). v0.5 cleans up the residual
  surface — reasoning-profile selection, TriG/N-Quads ingest,
  SHACL-SPARQL mode, the W3C SHACL manifest runner — and adds the
  IRI-overload ergonomics that v0.4 deliberately omitted to keep its
  surface focused.
- **Tense discipline:** v0.5 is forward-looking. Future tense
  ("will land", "ships with") is the default throughout. Where
  v0.5 builds on shipped v0.4 mechanism, that mechanism is named
  in present tense and explicitly cross-linked to v0.4.

## 1. Mission (carry forward from v0.4 / v0.3)

pgRDF is a PostgreSQL extension built entirely in Rust using `pgrx`.
It provides native storage and querying for RDF data directly
inside Postgres, with four engines:

1. **Storage Engine** — dictionary-encoded terms in
   `_pgrdf_dictionary`; quads in `_pgrdf_quads` partitioned by
   `graph_id`; hexastore covering indexes (SPO, POS, OSP).
2. **SPARQL Engine** — `pgrdf.sparql(q TEXT) → SETOF JSONB`;
   spargebra parser; dynamic-SQL executor with prepared-plan cache.
3. **Inference Engine** — OWL 2 RL materialisation via `reasonable`.
   v0.5 adds the RDFS / OWL-RL / `owl-rl-ext` profile selector
   (§3).
4. **Validation Engine** — SHACL Core via `shacl 0.3.x` (rudof
   project). Real W3C-shape report ✅ shipped in v0.4 cycle (LLD
   v0.4 §9). v0.5 adds SHACL-SPARQL mode (§5), the W3C SHACL
   manifest runner (§6), and validation-against-materialised-graph
   regression coverage (§5.1).

## 2. Scope of v0.5

| Section | Surface | Provenance |
|---|---|---|
| §3 | Reasoning profile selector on `pgrdf.materialize` | was v0.4-FUTURE §8 |
| §4 | TriG / N-Quads ingest (`pgrdf.parse_trig`, `pgrdf.parse_nquads`) | was v0.4-FUTURE §10 |
| §5 | SHACL-SPARQL constraint mode + validation-against-materialised-graph | was v0.4-FUTURE §9.5 |
| §6 | W3C SHACL manifest runner wired to CI | was v0.4-FUTURE §9.5 / §13 |
| §7 | IRI overloads for lifecycle UDFs (`drop_graph(iri)`, etc.) | was v0.4-FUTURE §5.1 forward note |
| §8 | Aggregates-over-UNION refinements not landed in v0.4 §11 | was v0.4-FUTURE §11 |
| §9 | v1.0 contents (forward look) | was v0.4-FUTURE §15 |

## 3. Reasoning profile selector

Reasoners selecting between RDFS and OWL 2 RL per workload class
need a per-call profile selector on `pgrdf.materialize`. v0.4 keeps
the v0.3 surface (`pgrdf.materialize(graph_id) → JSONB`) unchanged;
v0.5 adds the selector.

**v0.5 surface:**

```sql
pgrdf.materialize(graph_id BIGINT, profile TEXT DEFAULT 'owl-rl') → JSONB
```

- Profiles: `'rdfs'`, `'owl-rl'`, future `'owl-rl-ext'`.
- The `reasonable` crate's rule set is a superset of RDFS rules; an
  `'rdfs'` path activates the appropriate subset, either via a
  rule-filter pgRDF-internal pass (if upstream does not expose
  profile selection) or via direct upstream support (preferred).
- JSONB output gains a `profile` field reflecting the requested
  profile.
- Test surface: `tests/regression/sql/60-materialize-owl-rl.sql`
  gains a sibling `63-materialize-rdfs.sql`. The `'rdfs'` regression
  asserts the entailed-triple count is a **non-strict subset** of
  the OWL-RL count on the same input.

### 3.1 Acceptance criteria (v0.5 gate)

- `pgrdf.materialize(g, 'rdfs')` triple count ≤
  `pgrdf.materialize(g, 'owl-rl')` triple count on a fixed input.
- The two profiles agree on the entailment of the RDFS axioms
  (subClassOf transitivity, domain/range propagation, etc.).
- An unknown profile string returns an error with prefix
  `materialize: unknown profile`, not a silent fallback to
  `'owl-rl'`.

## 4. TriG / N-Quads ingest

Ingest pipelines that consume TriG and N-Quads with inline graph
declarations need a parser that honours the inline `GRAPH { … }`
blocks and resolves graph IRIs through the v0.4 §3 IRI mapping.

**v0.5 surface:**

```sql
pgrdf.parse_trig(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) → JSONB
pgrdf.parse_nquads(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) → JSONB
```

- **TriG:** accept the W3C TriG grammar; honour inline
  `GRAPH <iri> { … }` blocks; resolve `<iri>` via
  [`v0.4 §3.2`](SPEC.pgRDF.LLD.v0.4.md) `pgrdf.add_graph(iri)`
  (auto-allocate unknown IRIs by default).
- **N-Quads:** parse the 4-position line format; resolve the
  fourth-position IRI via v0.4 §3.2.
- **`strict => TRUE`:** reject unknown graph IRIs instead of
  auto-allocating. Useful for ingest into a pre-bound graph space.
- Both UDFs reuse the v0.3 batched-insert path (same `flush_batch`
  prepared plan).

### 4.1 Acceptance criteria (v0.5 gate)

- A TriG document declaring three inline named graphs loads into
  three pgRDF graphs in a single call.
- Unknown graph IRIs auto-allocate (default) or reject under
  `strict => TRUE`.
- Round-trip: `pgrdf.parse_trig` followed by a CONSTRUCT-of-each-graph
  re-serialised back to TriG produces an isomorphic document.

## 5. SHACL-SPARQL constraint mode + materialised-graph coverage

v0.4 ships SHACL Core in `Native` mode only (LLD v0.4 §9). v0.5
extends the validator surface.

### 5.1 Validation against a materialised graph

Allow `data_graph_id` to be a graph that has already had
`pgrdf.materialize` run; the SHACL engine then sees the entailed
closure. Today the rehydrate selects both `is_inferred = TRUE` and
`FALSE` rows, so this works in practice; v0.5 adds documentation +
a regression covering the case.

### 5.2 SHACL-SPARQL constraint mode

`shacl 0.3` exposes a `Sparql` validation mode in addition to
`Native`. v0.5 exposes this as a third positional arg to
`pgrdf.validate`:

```sql
pgrdf.validate(
    data_graph_id   BIGINT,
    shapes_graph_id BIGINT,
    mode            TEXT DEFAULT 'native'  -- 'native' | 'sparql'
) → JSONB
```

JSONB output gains a `mode` field reflecting the requested mode.

### 5.3 Acceptance criteria (v0.5 gate)

- A shape with `sh:select` (SPARQL-based constraint) validates
  correctly under `mode => 'sparql'` and produces a
  `sh:Violation` for the matching focus node.
- Validation against a materialised data graph reports violations
  against entailed triples (regression: a shape requiring
  `rdfs:subClassOf` chain membership reports for a chain-member
  bound only by entailment).

## 6. W3C SHACL manifest runner

Wire the upstream `rudof` SHACL test suite to CI as a third
correctness gate alongside the W3C SPARQL manifest (v0.4 §13).

**Test surface:** new harness target `just test-shacl-manifest`,
running the W3C SHACL Core + SHACL-SQL manifest. Initial pass rate
target is *full* on the Core suite; partial-pass on SHACL-SPARQL is
acceptable as long as the failing-case list is enumerated and each
failure carries an entry in ERRATA.

### 6.1 Acceptance criteria (v0.5 gate)

- `just test-shacl-manifest` exits 0 on the W3C SHACL Core manifest
  on every PG major (PG14 through PG17).
- `just test-shacl-manifest --sparql` exits with a known-failing
  set, documented in ERRATA.

## 7. IRI overloads for lifecycle UDFs

v0.4 §5 ships the four lifecycle UDFs with `BIGINT graph_id`
signatures only. Callers route IRI input through
`pgrdf.graph_id(iri)` explicitly. v0.5 adds IRI-keyed overloads
for ergonomics:

```sql
pgrdf.drop_graph(iri TEXT, cascade BOOLEAN DEFAULT TRUE) → BIGINT
pgrdf.clear_graph(iri TEXT) → BIGINT
pgrdf.copy_graph(src_iri TEXT, dst_iri TEXT) → BIGINT
pgrdf.move_graph(src_iri TEXT, dst_iri TEXT) → BIGINT
```

Semantics identical to the BIGINT overloads from v0.4 §5; the IRI
overloads resolve via `_pgrdf_graphs.iri → graph_id` and dispatch
to the same partition-DDL implementation.

### 7.1 Acceptance criteria (v0.5 gate)

- `pgrdf.drop_graph('http://example.org/g1')` removes the graph
  bound to that IRI; equivalent to
  `pgrdf.drop_graph(pgrdf.graph_id('http://example.org/g1'))`.
- IRI overloads error with prefix `drop_graph: unknown iri` if the
  IRI is not bound — distinct from the BIGINT overloads' no-op
  semantics on absent ids.

## 8. Aggregates over UNION — residual refinements

v0.4 §11 ships aggregates over UNION via a derived-table refactor.
Residual cases not covered by the v0.4 cut surface in v0.5:

- Aggregates over nested UNION-of-UNION patterns.
- `HAVING` clauses over UNION-derived aggregates with cross-branch
  variable references.
- `GROUP_CONCAT(DISTINCT …)` with custom `SEPARATOR` over UNION
  branches.

### 8.1 Acceptance criteria (v0.5 gate)

- A regression fixture per residual case lands in
  `tests/regression/sql/` with the expected aggregate output
  hand-computed from the SQL + SPARQL spec semantics.

## 9. Forward look — v1.0 and beyond

**v1.0 contents (planned):**

- **Incremental (delta-driven) materialisation:**
  `pgrdf.materialize_delta(graph_id, since_xid TEXT)` — forward-chain
  only over quads added since a recorded transaction id. Targets a
  common optimisation pattern in reasoning pipelines but does not
  gate any v0.x surface.
- **RDF 1.2 triple terms** — enable `oxrdf`'s `rdf-12` feature once
  upstream (`reasonable` + `shacl_validation`) supports it
  unanimously. [`ERRATA.v0.2`](ERRATA.v0.2.md) E-009 tracks the
  conflict; the v0.4 cycle's E-011 unblocks coexistence locally via
  the styk-tv fork, but full v1.0 surfacing of triple terms in
  `pgrdf.sparql` / `pgrdf.materialize` is gated on upstream
  unanimous support.
- **Federated `SERVICE`** — explicitly deferred to v1.0; remains
  out of scope for v0.x.
- **Postgres custom-scan hooks** — if not landed in v0.4 §12 (it
  may slip per LLD v0.4 §12), v1.0 picks it up.

No domain-specific motivation appears in this section; the items
are listed as engineering targets only.

## 10. Out of scope (carry forward from v0.4 §14)

- Streaming replication / logical decoding of RDF state.
- Federated SPARQL `SERVICE` — not in v0.5 (planned for v1.0 per §9).
- Full OWL 2 (EL / QL) reasoning ([`ERRATA.v0.2.md`](ERRATA.v0.2.md)
  E-002 — pgRDF ships OWL 2 RL only via `reasonable`).
- Backup/restore for opaque binary state (tracked by future
  `SPEC.pgRDF.BACKUP.v0.x`, INSTALL §11 OQ5).
- `LOAD <url>` in SPARQL UPDATE — explicitly not in scope for v0.4
  §4 or v0.5; callers fetch externally and invoke
  `pgrdf.load_turtle` or `pgrdf.parse_trig` directly.

## 11. Errata

- This document is the **draft** v0.5 contract. It is not yet
  authoritative; v0.4 LLD remains the in-progress authoritative
  contract.
- Spec corrections discovered during v0.5 implementation will land
  in `ERRATA.v0.4.md` (continued) or a new `ERRATA.v0.5.md` once a
  v0.5-era delta appears.
- **E-009** (SHACL upstream conflict) is resolved in v0.4 cycle via
  E-011 (patched `reasonable` fork). The v0.5 cycle inherits the
  resolved state; final close-out gates on the upstream
  `reasonable` PR merge.
- **E-006** (pgrx 0.18 / PG 18 migration) remains the largest
  deferred upstream item carried into v0.5.
