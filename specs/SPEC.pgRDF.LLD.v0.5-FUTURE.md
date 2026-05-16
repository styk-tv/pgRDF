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

| Section | Surface | Provenance | Status |
|---|---|---|---|
| §3 | Reasoning profile selector on `pgrdf.materialize` | was v0.4-FUTURE §8 | ✅ shipped — Phase G group G1 (slices 21-18) |
| §4 | TriG / N-Quads ingest (`pgrdf.parse_trig`, `pgrdf.parse_nquads`) | was v0.4-FUTURE §10 | 🚧 Phase G group G2 |
| §5 | SHACL-SPARQL constraint mode + validation-against-materialised-graph | was v0.4-FUTURE §9.5 | 🚧 Phase G group G3 |
| §6 | W3C SHACL manifest runner wired to CI | was v0.4-FUTURE §9.5 / §13 | 🚧 Phase G group G3 |
| §7 | IRI overloads for lifecycle UDFs (`drop_graph(iri)`, etc.) | was v0.4-FUTURE §5.1 forward note | ✅ shipped — Phase G group G1 (slices 21-18) |
| §8 | Aggregates-over-UNION refinements not landed in v0.4 §11 | was v0.4-FUTURE §11 | 🚧 Phase G group G2 |
| §9 | v1.0 contents (forward look) | was v0.4-FUTURE §15 | forward look |

## 3. Reasoning profile selector ✅ shipped (Phase G group G1)

> **Status: ✅ shipped — Phase G group G1 (slices 21-18).** All
> three §3.1 acceptance criteria met (strictly, not approximated).
> See "§3 implementation route" below for the chosen route + the
> precise `'rdfs'` semantics. Closes the last ONTOSYS P1 capability
> gap.

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

### 3.1 Acceptance criteria (v0.5 gate) — ✅ all met

- ✅ `pgrdf.materialize(g, 'rdfs')` triple count ≤
  `pgrdf.materialize(g, 'owl-rl')` triple count on a fixed input.
  *(Regression `117-materialize-rdfs.sql`: rdfs writes exactly 6,
  owl-rl writes 15, on the shared 7-triple seed.)*
- ✅ The two profiles agree on the entailment of the RDFS axioms
  (subClassOf transitivity, domain/range propagation, etc.). *(All
  6 hand-derived RDFS entailments present under BOTH profiles —
  invariant B.)*
- ✅ An unknown profile string returns an error with prefix
  `materialize: unknown profile`, not a silent fallback to
  `'owl-rl'`. *(Validated BEFORE any side effect — the
  idempotency wipe — so an unknown profile can't perturb state.
  The reserved future `'owl-rl-ext'` is treated as unknown.)*

### 3.2 §3 implementation route (shipped — route 2, strict)

**Route chosen: route 2 — a pgRDF-internal RDFS forward-chain pass**
(`src/inference/reasonable.rs::rdfs_closure`). The patched
`styk-tv/reasonable` fork (branch `rdf12-passthrough`) exposes only
a single fused OWL-RL datalog fixpoint (`Reasoner::reason()` /
`reason_full()`) with **no upstream RDFS-only rule selection**, so
route 1 (direct upstream profile support, the spec's preferred
option) is unavailable.

Route 2 is implemented as a **strict, sound, complete RDFS rule
engine** — *not* a lossy post-hoc filter of the OWL-RL output. It
forward-chains the six application-visible RDFS entailment rules
(W3C RDF 1.1 Semantics §9.2.1) to a fixed point:

| Rule | Entailment |
|---|---|
| rdfs5  | `subPropertyOf` transitivity |
| rdfs11 | `subClassOf` transitivity |
| rdfs7  | `subPropertyOf` application: `p ⊑ q ∧ s p o ⇒ s q o` |
| rdfs9  | `subClassOf` application: `c ⊑ d ∧ s a c ⇒ s a d` |
| rdfs2  | `rdfs:domain`: `p rdfs:domain c ∧ s p o ⇒ s a c` |
| rdfs3  | `rdfs:range`:  `p rdfs:range  c ∧ s p o ⇒ o a c` |

The axiomatic reflexive-typing rules (rdfs1/4a/4b/6/8/10/12/13 —
the universal `… rdf:type rdfs:Resource` / `rdfs:Class` /
`rdf:Property` triples) are **deliberately not emitted**: they add
only tautological triples that inflate the count, and `reasonable`
(OWL-RL) does not emit the universal `rdfs:Resource` typing either —
so emitting them on the `rdfs` side would *violate* §3.1 #1
(non-strict subset). Restricting to the six productive rules keeps
`rdfs` a **true subset** of `owl-rl` (RDFS rules ⊂ OWL 2 RL rules),
which is exactly why §3.1 #1 and #2 hold *by construction*, not by
coincidence.

The JSONB stats object gains a `profile` field reflecting the
requested profile (the default-arg call reports
`"profile":"owl-rl"`). The reserved future `'owl-rl-ext'` is
**not yet supported** — §3 names it as a future profile only; it
returns the same `materialize: unknown profile` error until a later
cycle wires it (documented choice; the spec's future-reservation
does not require it to work yet).

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

## 7. IRI overloads for lifecycle UDFs ✅ shipped (Phase G group G1)

> **Status: ✅ shipped — Phase G group G1 (slices 21-18).** Both
> §7.1 acceptance criteria met. The IRI overloads resolve
> `iri → graph_id` via `_pgrdf_graphs.iri` and dispatch to the
> EXISTING BIGINT UDFs (no partition-DDL logic duplicated — the
> overload re-enters through the SQL surface, the same single-
> sourcing pattern `add_graph_iri` uses).

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

### 7.1 Acceptance criteria (v0.5 gate) — ✅ all met

- ✅ `pgrdf.drop_graph('http://example.org/g1')` removes the graph
  bound to that IRI; equivalent to
  `pgrdf.drop_graph(pgrdf.graph_id('http://example.org/g1'))`.
  *(Regression `118-lifecycle-iri-overloads.sql` invariant G;
  resolution agrees with `pgrdf.graph_id(iri)`.)*
- ✅ IRI overloads error with prefix `drop_graph: unknown iri` if
  the IRI is not bound — distinct from the BIGINT overloads' no-op
  semantics on absent ids. *(All four overloads:
  `drop_graph: unknown iri` / `clear_graph: unknown iri` /
  `copy_graph: unknown iri` / `move_graph: unknown iri` —
  invariant J; the BIGINT `drop_graph(99999)` → 0 no-op
  re-asserted unchanged.)*

The IRI overloads compose with the v0.4 §4 SPARQL UPDATE lifecycle
algebra: dropping a graph via the IRI overload then issuing a
`CREATE GRAPH <same-iri>` SPARQL UPDATE rebinds the IRI cleanly to
a fresh partition (invariant K).

## 8. Aggregates over UNION — residual refinements

v0.4 §11 (Phase F group F2, slices 30-27) ships aggregates over
UNION via a derived-table refactor: each branch becomes a sub-SELECT
projecting the aggregate / GROUP BY variables' dict ids into the F1
`vK` column pool, and the existing aggregate translator runs over
`(<union>) qU`. Residual cases not covered by the v0.4 cut surface
in v0.5:

- **GROUP BY (or aggregate argument) on a variable that is ONLY ever
  a `GRAPH ?g`-scope var across the union.** Such a var has no dict
  id in the per-branch derived table (it is resolved as
  `g{S}.iri`, a text IRI, not a `BIGINT` id). The v0.4 build emits
  a **stable panic** (`sparql: GROUP BY / aggregate over a
  GRAPH-scope variable ?… across a UNION is deferred to
  v0.5-FUTURE §8`) rather than a wrong count. v0.5 fix: project a
  parallel text lane (or resolve the IRI back to its dict id) so
  the group key is consistent across branches.
- **A *computed* BIND expression used as a triple join key**
  (`BIND(?a + 1 AS ?k) . ?k :p ?o`). F2's AST substitution
  substitutes variable/term BIND aliases into a triple slot but
  leaves a computed-expression alias as the original variable
  (v0.3-degenerate behaviour preserved). v0.5 fix: emit the bind as
  a derived `SELECT expr AS col` lateral the triple correlates on.
- **A BIND variable used directly in a CONSTRUCT/DESCRIBE *template*
  output position** (`CONSTRUCT { ?s :total ?sum } WHERE { …
  BIND(?x+?y AS ?sum) }`). F2's substitution makes a BIND var
  usable in the construct's *WHERE* (FILTER/BGP/chained) — that is
  the inherited guarantee — but the construct emitter projects
  per-template-var **dict ids** and resolves them through
  `_pgrdf_dictionary`, whereas a BIND value is a query-time computed
  lexical value with no pre-interned id. v0.5 fix: project the bind
  expression as a lexical value and shape it as a literal term (or
  intern it on the fly) in the construct row encoder.
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
