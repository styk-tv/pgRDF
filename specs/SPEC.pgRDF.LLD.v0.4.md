# **SPEC.pgRDF.LLD.v0.4**

**pgRDF: A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL,
and OWL 2 RL reasoning.**

*Positioning: pgRDF — the high-performance PostgreSQL semantic-web toolkit.*

---

## 0. Document status and supersession

- **Status:** in-progress authoritative contract for the v0.4 cycle.
  Items mark as shipped (✅) when they land on `main`, regardless of
  whether the v0.4.0 tag has cut yet. Items still in progress are
  marked 🚧. The document is authoritative now — not aspirational —
  even though the cycle is mid-flight.
- **§3 named-graph track — COMPLETE within the v0.4 cycle.** The
  Phase A countdown closed the §3 surface across slices 120 → 110
  (cumulative): `_pgrdf_graphs` table + `pg_extension_config_dump`
  registration (slice 120), the five-UDF surface
  (`add_graph(id)` / `add_graph(iri)` / `add_graph(id, iri)` /
  `graph_id(iri)` / `graph_iri(id)`, slices 119 → 115), SPARQL
  `GRAPH <iri>` literal + `GRAPH ?g` variable forms (slices 114 /
  113), per-pattern GRAPH composition with OPTIONAL / UNION /
  MINUS (slice 112), three W3C-shape conformance fixtures (slice
  111), and the `pg_dump` round-trip script (slice 110). All four
  §3.4 acceptance criteria verified end-to-end. The remaining v0.4
  tracks (§4 UPDATE, §5 lifecycle, §6 CONSTRUCT, §7 paths, §11
  backlog) carry forward.
- **Supersedes:** [`SPEC.pgRDF.LLD.v0.3.md`](SPEC.pgRDF.LLD.v0.3.md)
  at the contract level for surfaces shipped in the v0.4 cycle.
  v0.3 LLD remains the verbatim record of the v0.3.0-cut surface.
- **Forward-looking sibling:** [`SPEC.pgRDF.LLD.v0.5-FUTURE.md`](SPEC.pgRDF.LLD.v0.5-FUTURE.md)
  is the draft target spec for the next cut beyond v0.4. The
  `-FUTURE` postfix on that sibling signals it is aspirational.
- **Carries forward:** [`SPEC.pgRDF.INSTALL.v0.2.md`](SPEC.pgRDF.INSTALL.v0.2.md)
  (no install-spec changes anticipated for v0.4) and
  [`ERRATA.v0.4.md`](ERRATA.v0.4.md) (authoritative for v0.4-era spec
  deltas). [`ERRATA.v0.2.md`](ERRATA.v0.2.md) remains live for
  pre-v0.3 items.
- **Reason for v0.4:** the v0.3 cut closes Phase 2's SPARQL surface
  to the line at which a substantial class of downstream consumers
  — operators with named-graph workloads, applications performing
  atomic write-back via SPARQL, validation tooling, reasoners
  selecting between RDFS and OWL 2 RL profiles, and consumers
  traversing transitive class hierarchies via `rdfs:subClassOf*`-style
  paths — start running into the same handful of gaps. v0.4 closes
  the highest-leverage gaps as a coherent group; the residual items
  carried over move to
  [`SPEC.pgRDF.LLD.v0.5-FUTURE.md`](SPEC.pgRDF.LLD.v0.5-FUTURE.md).
- **Tense discipline:** v0.4 is authoritative-in-progress. Shipped
  items (✅) describe reality in present tense. In-progress items
  (🚧) use future tense ("will land", "ships with") until they
  actually land — at which point this document is updated to flip
  the marker and the tense in the same slice that lands the work.

## 1. Mission (unchanged from v0.3)

pgRDF is a PostgreSQL extension built entirely in Rust using `pgrx`.
It provides native storage and querying for RDF data directly
inside Postgres, with four engines:

1. **Storage Engine** — dictionary-encoded terms in
   `_pgrdf_dictionary`; quads in `_pgrdf_quads` partitioned by
   `graph_id`; hexastore covering indexes (SPO, POS, OSP).
2. **SPARQL Engine** — `pgrdf.sparql(q TEXT) → SETOF JSONB`;
   spargebra parser; dynamic-SQL executor with prepared-plan cache.
3. **Inference Engine** — OWL 2 RL materialisation via `reasonable`.
4. **Validation Engine** — SHACL Core via `shacl 0.3.x` (rudof
   project). Real W3C-shape report ✅ shipped in v0.4 cycle (§9),
   replacing the v0.3 stub. Unblock vehicle:
   [`ERRATA.v0.4`](ERRATA.v0.4.md) E-011.

## 2. Scope of v0.4

v0.4 ships six major tracks, plus the SPARQL surface backlog
already enumerated in [`v0.3 §3`](SPEC.pgRDF.LLD.v0.3.md) as
"⏳ v0.4":

1. **Named-graph scoping and IRI mapping** (§3) — `GRAPH { … }` in
   SPARQL, a new `_pgrdf_graphs` system table, IRI ↔ `graph_id`
   helper UDFs. ✅ shipped (Phase A countdown slices 120 → 110).
2. **SPARQL UPDATE** (§4) — `INSERT DATA`, `DELETE DATA`,
   `INSERT … WHERE …`, `DELETE … WHERE …`,
   `DELETE … INSERT … WHERE …`, and the graph-scoped variants
   (`WITH <iri>`, inline `GRAPH <iri> { … }`). 🚧
3. **Graph-level lifecycle UDFs** (§5) — `drop_graph`, `clear_graph`,
   `copy_graph`, `move_graph` as state-management primitives over
   the LIST-partitioned `_pgrdf_quads` table. 🚧
4. **CONSTRUCT** (§6) — `pgrdf.construct(q TEXT) → SETOF JSONB`
   returning `{subject, predicate, object}`-shaped rows. 🚧
5. **Property paths** (§7) — `*`, `+`, `?`, `^`, with alternation
   `p1|p2` as a stretch. Materialised-closure-aware translation. 🚧
6. **SHACL real validation** (§9) — `pgrdf.validate(data, shapes)`
   ships the real W3C-shaped report; the v0.3 stub is gone. ✅
   shipped on `main` in commit
   [`ac40bc2`](https://github.com/styk-tv/pgRDF/commit/ac40bc2). See
   ERRATA.v0.4 E-011.

Plus the v0.3-deferred SPARQL surface items (§11): multi-triple
OPTIONAL, VALUES, BIND-downstream, aggregates over UNION, DESCRIBE.
These share enough translator machinery with §4 and §6 that they
ship in the same cut for economy. 🚧

Capability matrix for the v0.4 target:

| Capability | v0.3 status | v0.4 target | v0.4 status |
|---|---|---|---|
| `GRAPH <iri> { … }` | ⏳ deferred | §3.3 | ✅ slice 114 |
| `GRAPH ?g { … }` | ⏳ deferred | §3.3 | ✅ slice 113 |
| GRAPH composition (OPTIONAL/UNION/MINUS) | ⏳ deferred | §3.3 | ✅ slice 112 |
| IRI ↔ graph_id mapping table + UDFs | not yet | §3.1/§3.2 | ✅ slices 120-115 |
| SPARQL UPDATE (INSERT DATA / DELETE DATA / INSERT/DELETE WHERE) | not yet | §4 | 🚧 |
| `WITH <iri>` + graph-scoped UPDATE | not yet | §4.1 | ✅ slice 79 |
| Lifecycle algebra (`DROP / CLEAR / CREATE GRAPH`, plus `DEFAULT / ALL / NAMED`) | not yet | §4.4 | ✅ slice 78 |
| `pgrdf.drop_graph / clear_graph / copy_graph / move_graph` | not yet | §5 | ✅ all four shipped (slices 99 / 98 / 97 / 96) |
| `CONSTRUCT` | ⏳ deferred | §6 | 🚧 |
| Property paths `*`, `+`, `?`, `^` | ⏳ deferred | §7 | 🚧 |
| Property-path alternation `p1\|p2` | not yet | 🎯 stretch §7.1 | 🚧 |
| Multi-triple `OPTIONAL { BGP }` | ⏳ deferred | §11 | 🚧 |
| `VALUES` inline tables | ⏳ deferred | §11 | 🚧 |
| `BIND` output in later FILTER / BGP | ⏳ deferred | §11 | 🚧 |
| Aggregates over `UNION` | ⏳ deferred | §11 | 🚧 |
| `DESCRIBE` | ⏳ deferred | §11 | 🚧 |
| Real SHACL output | 🚧 stub | §9 | ✅ shipped `ac40bc2` |
| Reasoning profile selector (RDFS / OWL-RL) | not yet | — | ⏳ v0.5-FUTURE §3 |
| TriG / N-Quads ingest | not yet | — | ⏳ v0.5-FUTURE §4 |
| Incremental materialisation | not yet | — | ⏳ v1.0 (v0.5-FUTURE §9) |
| RDF 1.2 triple terms | not yet | — | ⏳ v1.0 (v0.5-FUTURE §9; gated on E-009) |
| Federated `SERVICE` | ❌ | — | ❌ out of scope (§14) |

## 3. Named-graph scoping and IRI mapping (NEW)

Named-graph workloads are first-class in pgRDF: storage already
partitions `_pgrdf_quads` by `graph_id` (LIST partition), and the
v0.3 cut ships `pgrdf.add_graph(id BIGINT)` and `pgrdf.count_quads`
filtered by graph. What's missing is the **IRI ↔ id binding** and
the **SPARQL `GRAPH { … }` surface**. v0.4 closes both. ✅ shipped
across Phase A countdown slices 120 → 110 (table + UDF surface +
SPARQL literal/variable forms + composition + W3C-shape fixtures +
pg_dump round-trip).

Downstream consumers running graph-level lifecycle operations
(drop / clear / copy / move — §5) and atomic write-back via SPARQL
UPDATE (§4) need the IRI mapping as a hard prerequisite: a SPARQL
query writes `GRAPH <http://example.org/g1>`, and pgRDF has to
resolve `<http://example.org/g1>` to the integer `graph_id` of the
backing partition without forcing the caller to track the integer
out-of-band.

### 3.1 Storage extension — `_pgrdf_graphs`

**Status: landed (Phase A countdown slice 120).** Schema lives in
[`sql/schema_v0_4_0_graphs.sql`](../sql/schema_v0_4_0_graphs.sql),
wired into `CREATE EXTENSION pgrdf` via the second
`extension_sql_file!` call in
[`src/lib.rs`](../src/lib.rs). Regression coverage:
[`tests/regression/sql/72-graphs-table-shape.sql`](../tests/regression/sql/72-graphs-table-shape.sql)
+ `#[pg_test]` in
[`src/storage/graphs.rs`](../src/storage/graphs.rs).

```sql
CREATE TABLE _pgrdf_graphs (
    graph_id BIGINT PRIMARY KEY,
    iri      TEXT NOT NULL UNIQUE
);
```

- New system table, shipped via the same `extension_sql_file!`
  mechanism that lands the rest of the v0.3 schema.
- **Migration discipline:** on `CREATE EXTENSION pgrdf UPDATE`, any
  pre-existing `graph_id` rows already populated by
  `pgrdf.add_graph(id BIGINT)` gain a synthetic IRI of the form
  `urn:pgrdf:graph:{id}`. This preserves round-trip identity for
  v0.3 databases without requiring the caller to assign IRIs
  retroactively.
- **pg_dump round-trip:** the table is `pg_dump`-included
  unconditionally; the mapping survives backup and restore.
  Acceptance criterion in §3 verified end-to-end by
  [`tests/regression/scripts/pg-dump-roundtrip.sh`](../tests/regression/scripts/pg-dump-roundtrip.sh)
  (Phase A countdown slice 110): the script seeds two IRI
  bindings, runs `pg_dump`, drops the extension, restores from
  the dump, and verifies both rows + a symmetric `graph_iri(id)`
  lookup survive. Wired into `just test-pg-dump-roundtrip` and
  `just test-conformance`.
- **Default partition** retains `graph_id = 0` semantics from v0.3
  (catch-all for un-IRI-bound writes). Synthetic IRI:
  `urn:pgrdf:graph:0`.

### 3.2 UDF surface

| UDF | Signature | Returns | Semantics |
|---|---|---|---|
| `pgrdf.add_graph(iri TEXT)` | overload ✅ slice 118 | `BIGINT` | Idempotent on the IRI: insert if absent (auto-allocated id via `COALESCE(MAX(graph_id), 0) + 1` under `LOCK TABLE _pgrdf_graphs IN SHARE ROW EXCLUSIVE MODE`), return existing id otherwise. Empty / whitespace-only IRI panics with the stable `add_graph: iri must be non-empty` prefix; IRI syntax (RFC 3987) is not validated in v0.4.1. |
| `pgrdf.add_graph(id BIGINT, iri TEXT)` | overload ✅ slice 117 | `BIGINT` | Explicit id binding. Idempotent on a matching `(id, iri)` pair; panics with the stable `add_graph:` prefix if `id` is bound to a different non-synthetic IRI (`add_graph: graph_id <N> is bound to a different IRI (<existing>)`) or `iri` is bound to a different graph_id (`add_graph: iri <iri> is bound to a different graph_id (<existing>)`). A synthetic placeholder `urn:pgrdf:graph:{id}` (the slice-119 seed for the integer overload) is treated as upgradable: the row is UPDATEd in place when the requested IRI is unbound elsewhere. Negative id and empty IRI rejected with the same stable prefixes shared by the other two overloads. |
| `pgrdf.add_graph(id BIGINT)` | retained | `BIGINT` | Back-compat with v0.3; assigns synthetic IRI `urn:pgrdf:graph:{id}` automatically as of slice 119 (`ON CONFLICT (graph_id) DO NOTHING` keeps idempotency intact). |
| `pgrdf.graph_id(iri TEXT)` | new ✅ slice 116 | `BIGINT` | Read-only lookup: returns the integer `graph_id` bound to `iri` in `_pgrdf_graphs`, or `NULL` if the IRI is not bound. Marked `STRICT` — NULL input short-circuits to NULL output without invoking the function body. No panic on miss; an actual SPI error still propagates with the stable `graph_id:` prefix. |
| `pgrdf.graph_iri(id BIGINT)` | new ✅ slice 115 | `TEXT` | Read-only lookup: returns the IRI bound to `graph_id` in `_pgrdf_graphs`, or `NULL` if the id is not bound. Marked `STRICT` — NULL input short-circuits to NULL output without invoking the function body. No panic on miss; an actual SPI error still propagates with the stable `graph_iri:` prefix. Symmetric inverse of `pgrdf.graph_id(iri)` above. |

With slice 115 landed the §3.2 UDF surface is now fully shipped
(all five rows ✅). The integer-id and IRI surfaces are
interchangeable at the UDF boundary. `pgrdf.put_quad`,
`pgrdf.count_quads`, and the lifecycle UDFs in §5 retain their
`BIGINT graph_id` argument forms; an IRI-keyed overload moves to
[`v0.5-FUTURE §7`](SPEC.pgRDF.LLD.v0.5-FUTURE.md). SPARQL
`GRAPH { … }` translation lands next in slices 114-110.

### 3.3 SPARQL GRAPH support

- **`GRAPH <iri> { … }`** ✅ landed (Phase A countdown slice 114).
  The executor's pattern walk resolves `<iri>` against
  `_pgrdf_graphs.iri` at translate time and adds `qN.graph_id = $K`
  to every triple alias inside the GRAPH block. Unresolved IRI binds
  to the sentinel `-1` (no real partition uses that value) ⇒ zero
  rows, spec-correct "no solutions"; no error raised. Implementation
  threads a `graph_id_constraint: Option<i64>` from the
  `GraphPattern::Graph { NamedNode, inner }` walk through
  `ParsedSelect` / `UnionBranch` into `build_from_and_where`, where
  the constraint is appended to mandatory BGP triples, OPTIONAL
  LEFT-JOIN ON clauses, and `translate_minus` sub-SELECTs. The
  parser's `unsupported_algebra` list no longer carries the
  "Graph (named graph clause)" tag for the literal-IRI form; it
  walks `inner` so the contained BGP triples are still counted.
  Regression: [`tests/regression/sql/78-sparql-graph-literal-iri.sql`](../tests/regression/sql/78-sparql-graph-literal-iri.sql)
  + one `#[pg_test]` (`sparql_graph_literal_iri_scopes_to_graph`).
  Slice-114 lifted by slice 112: the constraint is now per-pattern,
  not per-query (see §3.3 composition entry below).
- **`GRAPH ?g { … }`** ✅ landed (Phase A countdown slice 113). The
  executor records the variable name on `ParsedSelect.graph_var`
  (or `UnionBranch.graph_var`) during the algebra walk; in
  `build_from_and_where` the inner BGP gains an `INNER JOIN
  pgrdf._pgrdf_graphs g0 ON g0.graph_id = q{first}.graph_id`
  (exactly one such JOIN per inner BGP) — INNER matches W3C SPARQL
  1.1 §13.3, so only graphs present in the IRI mapping bind ?g.
  Triples 2..N inside the GRAPH block carry an additional
  `qN.graph_id = q{first}.graph_id` so a multi-triple inner BGP
  cannot stitch triples from different graphs together; the same
  shared-graph predicate flows into OPTIONAL aliases and MINUS
  sub-patterns nested under the variable-form GRAPH. The projection
  layer emits `g0.iri` whenever the projected variable matches
  `graph_var` — the JSONB row value is the IRI string (NOT the
  integer id). `SELECT *` adds the graph var to the projected list
  even when no inner triple anchors it. The parser's
  `unsupported_algebra` list no longer carries the
  "Graph (variable IRI; slice 113)" tag — slice 113 walks `inner`
  like the literal-IRI form. Regression:
  [`tests/regression/sql/79-sparql-graph-variable.sql`](../tests/regression/sql/79-sparql-graph-variable.sql)
  + one `#[pg_test]` (`sparql_graph_variable_projects_iri`).
  Slice-113 lifted by slice 112: per-pattern scope (see composition
  entry below).
- **GRAPH composition with OPTIONAL / UNION / MINUS** ✅ landed
  (Phase A countdown slice 112). The graph constraint is now PER
  TRIPLE PATTERN, not per query / per branch. A new `GraphScope`
  enum carries either `Literal(graph_id)` or
  `Variable { name, scope_id }`; the walk attaches an
  `Option<GraphScope>` to each `ScopedTriple` in the mandatory BGP,
  to each `OptionalBlock`, and to each `MinusBlock`. Distinct GRAPH
  blocks within one query get distinct `scope_id`s. The SQL builder
  pre-scans BGP + OPTIONAL scopes to produce a `ScopePlan`:
  mandatory Variable scopes get an `INNER JOIN _pgrdf_graphs g{S}`
  anchored on the scope's first BGP alias; OPTIONAL-born Variable
  scopes get a `LEFT JOIN _pgrdf_graphs g{S}` anchored on the
  OPTIONAL's alias so an unmatched OPTIONAL leaves `?g` NULL
  without dropping the outer row. Two GRAPH blocks binding the same
  `?g` are tied together with a `g{later}.graph_id = g{anchor}.graph_id`
  so the projected variable is consistent. OPTIONAL / MINUS that
  nest inside a GRAPH inherit the outer scope (W3C SPARQL 1.1 §13.3
  — innermost wins at AST-walk time). Regression:
  [`tests/regression/sql/87-sparql-graph-composition.sql`](../tests/regression/sql/87-sparql-graph-composition.sql)
  + four `#[pg_test]`s
  (`sparql_graph_composition_with_{optional,union,minus}` +
  `sparql_optional_inside_graph_variable`).
- Composition discipline:
  - `GRAPH { … }` composes inside `OPTIONAL`, `UNION`, and `MINUS`
    blocks via the per-pattern `Option<GraphScope>` decoration.
    Each contained triple carries the scope of its innermost
    enclosing GRAPH block (or `None` = scan every partition).
  - Nested `GRAPH` blocks resolve to the innermost scope per W3C
    SPARQL 1.1 §13.3 — the walk mints a fresh scope when it enters
    a Graph node and rebinds `current_scope` for the inner walk.
  - A bare BGP outside any `GRAPH { … }` continues to mean "match in
    any graph" — same semantics as v0.3 (`pgrdf.sparql` over the
    union of all partitions).

#### 3.3.1 W3C-shape conformance coverage (Phase A slice 111)

The W3C-shape harness at `tests/w3c-sparql/` carries three §13.3
fixtures landed in slice 111:

| Test dir | Form | Spec | First green when |
|---|---|---|---|
| `24-graph-named-iri` | `GRAPH <iri>` literal | §13.3 | slice 114 (literal-IRI translator) |
| `25-graph-var-projection` | `GRAPH ?g` projection | §13.3 | slice 113 (variable-form translator) |
| `26-graph-var-groupby` | `GRAPH ?g` + `COUNT(*)` + `GROUP BY ?g` + `ORDER BY ?g` | §13.3 + §11 + §15.1 | slice 113 |

Tests 25 + 26 are authored at slice 111 (parallel with slice 113);
they pass once both 111 and 113 land on `main`. Slice 111 also
extends `tests/w3c-sparql/run.sh` with optional per-test `setup.sql`
support — needed because the default single-graph
`add_graph(gid) + parse_turtle(data.ttl, gid)` path cannot express
the multi-graph fixtures §13.3 requires. The extension is
backward-compatible: tests 01–23 retain a non-empty `data.ttl` and
no `setup.sql`, and their SQL stream is byte-identical pre/post the
extension.

### 3.4 Acceptance criteria (v0.4 gate)

- `SELECT ?s WHERE { GRAPH <http://example.org/g1> { ?s ?p ?o } }`
  returns only the triples in the graph mapped to
  `<http://example.org/g1>`. Triples in other graphs are not
  surfaced.
- `SELECT ?g (COUNT(*) AS ?n) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?g`
  groups by IRI; `?g` projects as a `NamedNode` JSONB term, not as
  an integer.
- `pgrdf.add_graph('http://example.org/g1')` is idempotent — a
  second call against the same IRI returns the same integer id.
- `pgrdf.add_graph(42, 'http://example.org/g42')` followed by
  `pgrdf.graph_id('http://example.org/g42')` returns `42`.
- `pg_dump` of a pgRDF database carrying the mapping round-trips
  the mapping verbatim (covered by a regression fixture that
  dumps, drops, restores, and re-queries).

## 4. SPARQL UPDATE (NEW)

Applications running INSERT / DELETE / MODIFY against pgRDF need
the operations to land **inside a single Postgres transaction** —
the same transaction context as any surrounding SQL the caller has
open. v0.3 supports SELECT and ASK only. v0.4 adds the UPDATE
surface. 🚧

### 4.1 Surface

| Form | Notes |
|---|---|
| `INSERT DATA { … }` | ✅ slice 84 — direct triple insertion (single triple or BGP-style block). Constants only — no variables. Default-graph + `GRAPH <iri> { … }` inline graph scope both supported; unknown IRIs auto-allocate via `pgrdf.add_graph(iri)`. Idempotent on repeat via `WHERE NOT EXISTS` guard (the `_pgrdf_quads` table carries no `UNIQUE` constraint, so `ON CONFLICT` is unavailable). |
| `DELETE DATA { … }` | ✅ slice 83 — direct triple removal (ground quads only, no variables). Constants only. Default-graph + `GRAPH <iri> { … }` inline graph scope both supported. **Lookup-only** path against `_pgrdf_dictionary` — if any term is absent, the quad cannot be in `_pgrdf_quads`, so the operation is a spec-correct no-op (never errors). Repeated DELETE DATA against the same triple is idempotent (the second call deletes zero rows). |
| `INSERT { template } WHERE { pattern }` | ✅ slice 82 — pattern-driven insertion. Each solution row of `WHERE` instantiates `template` once. The WHERE pattern goes through `parse_select` (sharing BGP/FILTER/OPTIONAL/MINUS algebra with SELECT); a custom projection returns each template-referenced variable's dict id (BIGINT, not text — lossless internment); per-row template materialisation routes through the shared `insert_quad` helper with the same `WHERE NOT EXISTS` set-semantic guard as INSERT DATA. Slice-82 limitations: WHERE may not carry aggregates / GROUP BY / UNION; template variables MUST be bound by the WHERE BGP (fail-fast); variable GRAPH in template panics (lands with slice 76). |
| `DELETE { template } WHERE { pattern }` | ✅ slice 81 — pattern-driven removal. Sibling of slice 82's INSERT WHERE: same `parse_select` walker for the WHERE half, same dict-id (BIGINT) projection one row per solution, same per-row template instantiation. Spargebra models the DELETE template as `Vec<GroundQuadPattern>` — the type bakes the W3C SPARQL 1.1 §4.1.2 rule "blank nodes are not allowed in the DELETE clause" into the AST (no `BlankNode` arm in `GroundTermPattern`). Per-row template-quad DELETE uses the same `WITH d AS (DELETE … RETURNING 1) SELECT count(*)` idiom slice 83 installed for DELETE DATA, so `triples_deleted` counts ACTUAL rows removed (not template instantiations attempted) — important distinction from INSERT WHERE's "attempted insert" counter, which the WHERE NOT EXISTS guard silently dedupes. Lookup-only dict path mirrors slice 83's DELETE DATA: if any instantiated term is absent from `_pgrdf_dictionary` the per-row delete is a spec-correct no-op (never errors). Slice-81 limitations: WHERE may not carry aggregates / GROUP BY / UNION; template variables MUST be bound by the WHERE BGP (panics with `DELETE WHERE template feature 'unbound template variable` prefix); variable GRAPH in template panics (lands with slice 76). |
| `DELETE { … } INSERT { … } WHERE { … }` | ✅ slice 80 — atomic modify form. Both halves resolve against the SAME `WHERE` solutions snapshot: the pattern is evaluated exactly once, the projection unions every variable referenced by EITHER template (DELETE-side then INSERT-side, first-appearance per side), and Rust iterates the binding rows applying DELETE then INSERT per row. Per W3C SPARQL 1.1 Update §3.1.3 ordering, the DELETE half conceptually precedes the INSERT — this matters for status-flip patterns (`DELETE { ?x ex:status "draft" } INSERT { ?x ex:status "approved" } WHERE { ?x ex:status "draft" }`) where the DELETE removes the old row and the INSERT adds the new one cleanly. Atomicity is naturally provided by Postgres's transaction model (the whole UDF call is one transaction). DELETE counter uses the `WITH d AS (DELETE … RETURNING 1) SELECT count(*)` idiom from slice 81/83 (actual rows removed); INSERT counter is per-attempt (slice 82 convention — `WHERE NOT EXISTS` guard silently dedupes but the attempt count surfaces). `_update` summary reports `form: "DELETE_INSERT_WHERE"`. Limitations locked: inherits slice 81/82's restrictions (no aggregates / GROUP BY / UNION in WHERE; template variables must be bound by the WHERE BGP; variable GRAPH in either template panics with slice-76 prefix); `USING / USING NAMED` not yet supported. |
| `WITH <iri> …` | ✅ slice 79 — graph scope for the surrounding INSERT/DELETE/WHERE. Spargebra desugars `WITH <iri>` at parse time into (a) per-quad `graph_name` injection on every default-graph template QuadPattern/GroundQuadPattern (handled by `instantiate_template_quad` / `instantiate_ground_template_quad`'s `GraphNamePattern::NamedNode` branches — already in place since slices 80/81/82) and (b) a `using: Some(QueryDataset { default: [<iri>], named: None })` sentinel on the DeleteInsert operation. The dispatcher lifts the IRI from (b) and wraps the WHERE pattern in `GraphPattern::Graph { name, inner }`, so the slice-112 walker scopes every emergent BGP triple to `<iri>` (nested explicit `GRAPH <other> { … }` still overrides per W3C §13.3). Limitations: `USING <iri>` / `USING NAMED <iri>` proper (distinct from WITH-injected sentinel) panics with `'USING / USING NAMED' not yet supported`. |
| `INSERT { GRAPH <iri> { … } }` | ✅ slice 84 (for the `INSERT DATA` variant); ✅ slice 82 for the `INSERT … WHERE` variant with a **literal** IRI graph scope. Slice 79 locks the cross-graph `INSERT { GRAPH <g2> { … } } WHERE { GRAPH <g1> { … } }` shape end-to-end. The `INSERT { GRAPH ?g { … } }` variable-graph variant lands with slice 76. |
| `DELETE { GRAPH <iri> { … } }` | ✅ slice 79 — inline graph scope on the template. Spargebra preserves `graph_name` on every `GroundQuadPattern` (the DELETE template type); `instantiate_ground_template_quad` routes the per-row delete through `lookup_graph_id(<iri>)` rather than `graph_id = 0`. Combines with `GRAPH <iri> { … }` in the WHERE half (slice 112) and `WITH <iri>` to scope the entire operation. |

The graph-scoped variants compose with §3's IRI mapping: every
`<iri>` resolves to a `graph_id` via `_pgrdf_graphs.iri`. Unknown
IRIs auto-allocate (default behaviour, matching `add_graph(iri)`).

### 4.2 Wiring

**UDF surface decision.** v0.4 overloads `pgrdf.sparql(q TEXT)` to
dispatch by query form rather than introducing a sibling
`pgrdf.sparql_update`. Rationale: clients shouldn't need to know
which UDF to call before parsing the query string. The return type
remains `SETOF JSONB`; UPDATE forms return a single summary row of
shape:

```json
{
  "_update": {
    "form": "DELETE_INSERT_WHERE",
    "triples_inserted": 1000,
    "triples_deleted": 1000,
    "graphs_touched": ["http://example.org/g1"],
    "elapsed_ms": 12
  }
}
```

The `_update` JSONB sentinel parallels the v0.3 `_ask` sentinel for
ASK queries — callers discriminate on the leading key.

**Algebra.** `spargebra` already parses UPDATE forms via
`SparqlParser::new().parse_update(q)` returning `spargebra::Update`,
a vector of `GraphUpdateOperation`s. Detection at the
`pgrdf.sparql(q)` entry point is **try-parse-then-fallback**:
`parse_query` first (the SELECT/ASK path, unchanged from v0.3), then
`parse_update` on query-side failure. Both failing yields the stable
`sparql: parse error:` prefix (slice #63 contract — the query-side
error message is surfaced because that's what downstream tooling
scrapes). The dispatcher walks `update.operations` and routes per
operation variant. The variants enumerated by spargebra 0.4.6 are
`InsertData`, `DeleteData`, `DeleteInsert`, `Load`, `Clear`,
`Create`, `Drop`; SPARQL `ADD`, `MOVE`, `COPY` are desugared by the
parser into combinations of the above (e.g. `MOVE` becomes
`DeleteInsert` + `Clear`). The §4.4 lifecycle-algebra mapping below
flows through the desugared shape rather than the surface keyword.

**Transaction discipline.** The UDF runs inside the calling
Postgres transaction. One `pgrdf.sparql(q)` call is one transaction
unit (no implicit commits). The caller controls commit boundaries
via standard SQL `BEGIN / COMMIT / ROLLBACK`. Operations that
straddle multiple UPDATE forms (e.g. `DELETE_INSERT_WHERE`) execute
as a single SQL statement-equivalent: the `WHERE` is evaluated
once, both DELETE and INSERT templates resolve against the same
solution set, and they apply atomically.

**INSERT-from-WHERE.** The synthesised inserts route through the
existing batched `flush_batch` prepared-plan path from
[`v0.3 §4.3`](SPEC.pgRDF.LLD.v0.3.md). Throughput on bulk
`INSERT { … } WHERE { … }` matches bulk `pgrdf.load_turtle` to
within a constant factor.

### 4.3 `pgrdf.sparql_parse` integration

`pgrdf.sparql_parse(q TEXT)` mirrors the executor's detection
strategy: `parse_query` first, `parse_update` on failure. UPDATE
queries surface as `form: "UPDATE"` with a per-operation summary
array. UPDATE operations that the executor doesn't translate yet
(slices 82, 81, 80 now ship the pattern-driven forms) are NOT
flagged in
`unsupported_algebra` — that array is reserved for genuinely-out-
of-scope shapes (e.g. `LOAD <url>`, see §14). Sample JSONB shape
for the slice-84-shipped INSERT DATA form:

```json
{
  "form": "UPDATE",
  "operations": [
    {"op": "InsertData", "triples": 5, "graphs": ["http://example.org/g1"]},
    {"op": "DeleteInsert", "delete_template_size": 1, "insert_template_size": 1, "where_pattern_size": 3}
  ],
  "unsupported_algebra": []
}
```

Callers running multi-statement UPDATE preview translatability per
operation; partial-support cases (e.g. `LOAD <url>` — explicitly
out of scope, see §14) surface in `unsupported_algebra`.

### 4.4 Lifecycle algebra ↔ §5 UDFs

`spargebra::Update` includes `Drop`, `Clear`, `Create`, `Add`,
`Move`, and `Copy` operations. v0.4 wires these to the lifecycle
UDFs from §5 — the SPARQL UPDATE surface and the SQL UDF surface
are two front-ends to the same partition-level primitives. Both
honour the IRI mapping from §3.

| SPARQL form | Backing UDF |
|---|---|
| `DROP GRAPH <iri>` | ✅ slice 78 — `pgrdf.drop_graph(graph_id(iri), true)`; not-bound IRI panics with `DROP GRAPH <iri>: graph not bound` unless `SILENT`. |
| `CLEAR GRAPH <iri>` | ✅ slice 78 — `pgrdf.clear_graph(graph_id(iri))`; preserves the `_pgrdf_graphs` binding (W3C §3.1.3 "the named graph itself is preserved"); not-bound IRI panics with `CLEAR GRAPH <iri>: graph not bound` unless `SILENT`. |
| `CREATE GRAPH <iri>` | ✅ slice 78 — `pgrdf.add_graph(iri TEXT)`; already-bound IRI panics with `CREATE GRAPH <iri>: graph already exists` unless `SILENT`. No row-count change (CREATE only allocates a partition + binding). |
| `DROP DEFAULT` / `CLEAR DEFAULT` | ✅ slice 78 — both route to a direct `DELETE FROM _pgrdf_quads WHERE graph_id = 0`. `pgrdf.clear_graph(0)` only handles the explicit `_pgrdf_quads_g0` partition (if `add_graph(0)` ever ran); routine default-graph inserts land in `_pgrdf_quads_default`, which the direct DELETE catches via partition routing. `pgrdf.drop_graph(0)` panics by design (slice 99 guard); W3C §3.1.3 paragraph 7 makes `DROP DEFAULT` an "empty, not destroy" anyway. |
| `DROP ALL` / `CLEAR ALL` | ✅ slice 78 — enumerate every `graph_id` in `_pgrdf_graphs` (including `0`) and dispatch per-id. `CLEAR ALL` empties every partition with bindings preserved; `DROP ALL` removes every named partition + binding, with the default routed to clear. |
| `DROP NAMED` / `CLEAR NAMED` | ✅ slice 78 — enumerate every `graph_id <> 0` (default excluded per W3C §3.1.3) and dispatch per-id. |
| `COPY <src> TO <dst>` | `pgrdf.copy_graph(graph_id(src), graph_id(dst))` — desugars at parse time (spargebra parser.rs §Copy) into `Drop + DeleteInsert`; lands via the slice 78 + slice 80 dispatcher arms. |
| `MOVE <src> TO <dst>` | `pgrdf.move_graph(graph_id(src), graph_id(dst))` — desugars at parse time (spargebra §Move) into `Drop + DeleteInsert + Drop`; lands via slice 78 + 80. |
| `ADD <src> TO <dst>` | `pgrdf.copy_graph` (ADD = COPY without first-clearing dst per W3C SPARQL 1.1 Update §3.2.6) — desugars at parse time (spargebra §Add) into a plain `DeleteInsert`; lands via slice 80. |

### 4.5 Acceptance criteria (v0.4 gate)

- One `INSERT DATA { … }` of N triples ≡ one Postgres transaction,
  observable via `pg_stat_xact_user_tables`.
- `INSERT { ?x ex:tag "t" } WHERE { ?x rdf:type ex:Item }` over
  1 000 matched items adds 1 000 triples in one round trip (no
  per-row UDF re-entry).
- Round-tripping through `pgrdf.sparql_parse` and `pgrdf.sparql`
  for the same UPDATE produces equivalent state to direct
  `pgrdf.put_quad` calls reproducing the same set.
- `DELETE { … } INSERT { … } WHERE { … }` is atomic — a rollback
  on the surrounding transaction leaves the graph state unchanged.
- `pgrdf.sparql('DROP GRAPH <http://example.org/g1>')` is
  equivalent to `pgrdf.drop_graph(pgrdf.graph_id('http://example.org/g1'))`.

## 5. Graph-level lifecycle UDFs (NEW)

Consumers running graph-level lifecycle operations as part of
their state-management need drop / clear / copy / move primitives
that operate at the **partition level** — not as N-row DELETE
loops. v0.4 lands four UDFs that exploit `_pgrdf_quads`'s LIST
partitioning. 🚧

### 5.1 Surface

| UDF | Signature | Returns | Semantics |
|---|---|---|---|
| `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN DEFAULT TRUE)` | ✅ slice 99 | `BIGINT` | Removes the partition entirely; returns the count of triples that were in it. `cascade => FALSE` errors if inferred rows are present. |
| `pgrdf.clear_graph(id BIGINT)` | ✅ slice 98 | `BIGINT` | `TRUNCATE ONLY` the partition; the partition itself is preserved (so subsequent inserts route normally). Returns triples removed. |
| `pgrdf.copy_graph(src BIGINT, dst BIGINT)` | ✅ slice 97 | `BIGINT` | Copies all quads from `src` to `dst`. Creates the `dst` partition if absent. Returns triples copied. |
| `pgrdf.move_graph(src BIGINT, dst BIGINT)` | ✅ slice 96 | `BIGINT` | Migrates every quad from `src` to `dst`, removes `src`. v0.4.2 implementation is a compose: `pgrdf.copy_graph(src, dst)` then `pgrdf.drop_graph(src, cascade => TRUE)`. Returns triples moved (== row count at copy time). The §5.2 "metadata-only DETACH/ATTACH rebind" claim is aspirational; tractable metadata-only is a v0.5 perf optimisation. |

IRI overloads (`pgrdf.drop_graph(iri TEXT)`, etc.) deferred to
[`v0.5-FUTURE §7`](SPEC.pgRDF.LLD.v0.5-FUTURE.md); in v0.4 callers
route IRI input through `pgrdf.graph_id(iri)` explicitly.

### 5.2 Implementation notes

- `_pgrdf_quads` is LIST-partitioned on `graph_id` per v0.3 §4. The
  lifecycle UDFs lean on Postgres's partition-management DDL:
  - `drop_graph` issues
    `ALTER TABLE _pgrdf_quads DETACH PARTITION ...` then
    `DROP TABLE`. The detach is metadata-only; the subsequent drop
    drops the partition table's own row storage and indexes. Cost
    is independent of row count (modulo btree page release).
  - `move_graph` was specified as metadata-only — rebind the
    partition's `FOR VALUES IN (<old_id>)` clause to
    `FOR VALUES IN (<new_id>)` under an `ACCESS EXCLUSIVE` lock
    on the parent. In practice the LIST partition constraint
    requires every row's `graph_id` column to match the bound
    value, so the rebind needs an interim UPDATE of every row —
    a row scan, not metadata-only. The **v0.4.2 implementation
    (slice 96)** therefore composes `copy_graph(src, dst)` +
    `drop_graph(src, cascade => TRUE)` instead, which is
    correctness-preserving but scans the rows twice. A truly
    metadata-only swap is flagged as a v0.5 perf optimisation
    (drop the `graph_id` column from `_pgrdf_quads` and key
    routing purely on the partition association would make the
    rebind free — at the cost of losing the column-level
    `graph_id` reference for downstream filters).
  - `clear_graph` is `TRUNCATE ONLY` on the partition — bulk row
    discard with the partition shell preserved.
  - `copy_graph` is the only one that touches every row:
    `INSERT INTO _pgrdf_quads SELECT subject_id, predicate_id, object_id, <dst>, is_inferred FROM _pgrdf_quads WHERE graph_id = <src>`.
- **`is_inferred` semantics:**
  - `drop_graph` removes both base and inferred rows (cascade is
    the default).
  - `clear_graph` removes both base and inferred.
  - `copy_graph` copies both — `is_inferred = TRUE` rows carry
    forward as `is_inferred = TRUE` in the destination.
  - `move_graph` is metadata-only — `is_inferred` flags are
    preserved trivially.
- **`_pgrdf_graphs` invalidation:** `drop_graph` removes the
  matching `(graph_id, iri)` row. `move_graph` rebinds the IRI to
  the new id (the source id becomes unbound). `copy_graph` allocates
  a fresh IRI for `dst` if `dst` is not already bound (synthetic
  `urn:pgrdf:graph:{dst}`).
- **Idempotency:** every UDF returns 0 (no-op) on inputs that name
  an empty or absent graph — never an error.
- **Concurrency:** the partition-DDL UDFs (`drop_graph`,
  `move_graph`) take an `ACCESS EXCLUSIVE` lock on `_pgrdf_quads`
  for the metadata window. Concurrent SELECT/UPDATE traffic on
  unrelated graphs blocks for the duration; this is documented in
  the guide as part of the "long-running maintenance" workflow.

### 5.3 Acceptance criteria (v0.4 gate)

- **Idempotency.** Re-calling any of the four UDFs with the same
  input is a no-op and returns `0`.
- **Constant-time move (deferred to v0.5).** Originally specified
  as `pgrdf.move_graph(src, dst)` execution time independent of
  `_pgrdf_quads` row count in `src`. The v0.4.2 implementation
  (slice 96) is a `copy_graph + drop_graph` compose — O(N) in the
  src row count, not metadata-only. Tractable constant-time move
  is a v0.5 perf optimisation that drops the `graph_id` column
  from `_pgrdf_quads` and keys routing on partition association
  alone.
- **Cascade guard.** `pgrdf.drop_graph(id, cascade => FALSE)`
  errors with prefix `drop_graph: inferred rows present` if any
  `is_inferred = TRUE` row exists in the graph.
- **IRI mapping consistency.** After `pgrdf.drop_graph(id)`,
  `pgrdf.graph_iri(id)` returns `NULL` and the row is gone from
  `_pgrdf_graphs`. After `pgrdf.move_graph(src, dst)`,
  `pgrdf.graph_iri(src)` returns `NULL` and `pgrdf.graph_iri(dst)`
  returns the IRI previously bound to `src` (or synthetic if
  unbound).

## 6. CONSTRUCT (deferred from v0.3, now in scope)

`CONSTRUCT` is the canonical SPARQL form for graph snapshot export,
Turtle output, and sub-graph extraction. v0.3 lists it as
deferred-to-v0.4 because its return shape (triples, not solutions)
diverges from the `pgrdf.sparql` JSONB row shape. 🚧

### 6.1 Surface decision

v0.4 adds a sibling UDF rather than overloading `pgrdf.sparql`:

```sql
pgrdf.construct(q TEXT) → SETOF JSONB
```

Each row has the shape `{"subject": ..., "predicate": ..., "object": ...}`,
where each value is itself a structured JSONB term using the same
term-encoding shaper as `pgrdf.sparql`:

```json
{
  "subject":   {"type": "iri",     "value": "http://example.org/Alice"},
  "predicate": {"type": "iri",     "value": "http://example.org/name"},
  "object":    {"type": "literal", "value": "Alice", "datatype": "http://www.w3.org/2001/XMLSchema#string"}
}
```

**Rationale for not overloading.** The caller signals intent at the
SQL boundary, which simplifies result-set typing in client
libraries. A future `pgrdf.construct_turtle(q TEXT) → TEXT`
convenience wrapper will compose via `oxttl` serialisation —
serialising the same row shape into Turtle is a one-pass
projection and slots cleanly into the v0.3 ingest pipeline
(`pgrdf.load_turtle(pgrdf.construct_turtle(...), graph_id)`).

### 6.2 Translation

`CONSTRUCT { template } WHERE { pattern }` compiles to:

```sql
SELECT subject_id, predicate_id, object_id
FROM (<BGP translation of pattern>)
```

…then projects each `(subject_id, predicate_id, object_id)` triple
via the term-encoding shaper used by `pgrdf.sparql`. The
`template` may contain constants, variables, or blank nodes;
constants resolve to dictionary ids ahead of execution, variables
substitute per solution, and blank nodes generate fresh per-solution
ids via `oxrdf::BlankNode::default()`.

Reuse of v0.3 machinery:
- BGP translation: existing `build_from_and_where`.
- Term shaping: existing `JsonBuilder` from `src/query/executor.rs`.
- Aggregates / DISTINCT / etc. on CONSTRUCT are explicitly not in
  scope — per W3C SPARQL 1.1 §16.2 CONSTRUCT's solution sequence is
  the BGP's, no modifiers.

### 6.3 Acceptance criteria (v0.4 gate)

- `CONSTRUCT { ?s ex:tag "x" } WHERE { ?s rdf:type ex:Item }`
  returns one row per matched subject, each row carrying the
  fully-instantiated triple.
- **Round-trip.** `pgrdf.construct(q)` followed by re-inserting the
  rows via `pgrdf.put_quad` produces the same graph state (modulo
  dictionary id reshuffles, which are not user-visible).
- Constant-only templates resolve dictionary ids once, not per row
  (verified by `pgrdf.stats() → dict_db_calls` not increasing
  monotonically with output cardinality).
- Blank nodes in the template generate fresh ids per solution
  (covered by a regression that hashes the output and asserts no
  duplicate blank-node label appears across solutions).

## 7. Property paths (deferred from v0.3, now in scope)

Consumers traversing transitive class hierarchies via
`rdfs:subClassOf*`-style patterns hit the v0.3 limitation: only
direct predicate matches are supported. v0.4 adds the core path
operators. 🚧

### 7.1 Surface

| Operator | SPARQL syntax | Semantics |
|---|---|---|
| `*` zero-or-more | `?s ex:knows* ?o` | Reflexive transitive closure of `ex:knows`. |
| `+` one-or-more | `?s ex:knows+ ?o` | Transitive closure (non-reflexive). |
| `?` zero-or-one | `?s ex:knows? ?o` | Either equal or directly linked. |
| `^` inverse | `?s ^ex:knows ?o` | Equivalent to `?o ex:knows ?s`. |
| `\|` alternation | `?s (ex:a\|ex:b) ?o` | Stretch goal — included if the translator refactor is cheap; explicitly gated. |

Sequence paths (`p1/p2`) are already representable as multi-pattern
BGPs and do not need new translator support.

### 7.2 Translation strategy

Property paths translate to recursive Postgres CTEs:

```sql
WITH RECURSIVE walk(src, dst, depth) AS (
    SELECT subject_id, object_id, 1
    FROM _pgrdf_quads
    WHERE predicate_id = $P AND graph_id = $G
  UNION
    SELECT w.src, q.object_id, w.depth + 1
    FROM walk w
    JOIN _pgrdf_quads q ON q.subject_id = w.dst
    WHERE q.predicate_id = $P AND w.depth < $MAX_DEPTH
)
SELECT ...
```

Path-operator mapping:
- `+`: the CTE as written.
- `*`: union with `SELECT ?s ?s FROM _pgrdf_quads` (reflexive base case)
  per spec.
- `?`: union of direct match and identity.
- `^`: swap `subject_id` and `object_id` in the base case (and join
  predicate), no recursion needed.
- `|` (if shipped): the base case becomes a union of per-predicate
  scans.

**Materialised-closure detection.** If the graph has been
materialised under a profile (OWL-RL or RDFS — see
[`v0.5-FUTURE §3`](SPEC.pgRDF.LLD.v0.5-FUTURE.md)) that already
entails the closure of the path's predicate, the translator falls
back to a direct BGP match against the materialised triples. No
recursion is emitted.

Heuristic for v0.4: if `_pgrdf_quads` carries `is_inferred = TRUE`
rows whose `predicate_id` corresponds to one of the well-known
transitive predicates (`rdfs:subClassOf`, `rdfs:subPropertyOf`,
`owl:sameAs`), the translator prefers a direct match over the CTE.
The detection is per-query, not cached; a future refinement would
record materialised-closure metadata on `_pgrdf_graphs`.

**Depth-guard.** `$MAX_DEPTH` defaults to 64. Configurable via a
new GUC `pgrdf.path_max_depth` (range 1..1024). Queries whose
solution path exceeds the depth are truncated, not errored — a
warning surfaces on `pgrdf.stats()` as `path_depth_truncations`.

### 7.3 Acceptance criteria (v0.4 gate)

- `?c rdfs:subClassOf* <http://example.org/Top>` traverses the full
  chain on a non-materialised graph (regression: chain of length 10
  resolves all 10 subclasses).
- The same query on a materialised graph (post-`pgrdf.materialize`)
  emits no recursive CTE in the executed plan — verified by a
  regression test that scrapes the `EXPLAIN (FORMAT JSON)` output
  for the absence of `CTE Scan`.
- `?s ^p ?o` round-trips: the result set is identical to
  `?o p ?s` over the same graph.
- A query that would traverse beyond
  `pgrdf.path_max_depth` returns the truncated solution set and
  bumps `path_depth_truncations` in `pgrdf.stats()`.

## 8. Reasoning profile selector — moved to v0.5-FUTURE

The reasoning-profile selector on `pgrdf.materialize` (RDFS vs OWL-RL
vs `owl-rl-ext`) is deferred to v0.5. See
[`SPEC.pgRDF.LLD.v0.5-FUTURE §3`](SPEC.pgRDF.LLD.v0.5-FUTURE.md) for
the surface sketch and acceptance criteria. v0.4 keeps
`pgrdf.materialize(graph_id) → JSONB` unchanged from v0.3.

## 9. SHACL real integration (✅ shipped in v0.4 cycle)

`pgrdf.validate(data_graph_id, shapes_graph_id) → JSONB` ships as a
real W3C-shape SHACL Core validator in v0.4. The v0.3 stub is gone.
The SQL surface signature is unchanged from v0.3 — only the JSONB
body's keys shifted from `{status: "stub", reason: …}` to a W3C
`sh:ValidationReport`-shape document. Landed on `main` in commit
[`ac40bc2`](https://github.com/styk-tv/pgRDF/commit/ac40bc2);
covered by regression
[`tests/regression/sql/71-shacl-real.sql`](../tests/regression/sql/71-shacl-real.sql)
and three `#[pg_test]` integration tests in
`src/validation/shacl.rs` (conforming, violations, unknown graphs).
Unblock vehicle: [`ERRATA.v0.4`](ERRATA.v0.4.md) E-011.

### 9.1 Body shape

```json
{
  "conforms":        <bool>,
  "results":         [ ValidationResult, ... ],
  "data_graph_id":   <i64>,
  "shapes_graph_id": <i64>,
  "data_triples":    <i64>,
  "shapes_triples":  <i64>,
  "elapsed_ms":      <f64>
}
```

Each `ValidationResult` is:

```json
{
  "focusNode":      "<iri-or-bnode-or-literal-encoded>",
  "resultPath":     "<iri-or-null>",
  "sourceShape":    "<iri-or-bnode-or-null>",
  "resultMessage":  "<string-or-null>",
  "resultSeverity": "sh:Violation|sh:Warning|sh:Info|sh:Trace|sh:Debug",
  "value":          "<term-encoded-or-null>",
  "sourceConstraintComponent": "<iri>"
}
```

### 9.2 Engine + pipeline

The implementation in `src/validation/shacl.rs`:

1. Rehydrates both graphs from `_pgrdf_quads` JOIN
   `_pgrdf_dictionary` (same shape as `pgrdf.materialize`).
2. Serialises each graph to N-Triples via `oxttl::NTriplesSerializer`.
3. Builds `rudof_rdf::rdf_impl::InMemoryGraph::from_str` instances
   from the N-Triples text.
4. Compiles the shapes graph into a SHACL `IRSchema` via
   `shacl::validator::store::ShaclDataManager::load`.
5. Wraps the data graph as a `shacl::validator::store::Graph` and
   constructs a `GraphValidation` validator.
6. Runs `validator.validate(&schema, &ShaclValidationMode::Native)`.
7. Maps the resulting `ValidationReport.results()` into JSONB,
   normalising severities to the canonical `sh:` constants and
   rendering literals in Turtle-ish form.

### 9.3 Unblock vehicle (ERRATA.v0.4 E-011)

Two upstream-side preconditions cleared during the v0.4 cycle:

1. **rudof 0.3.1 consolidation (2026-05-12).** `shacl_ast` +
   `shacl_validation` 0.2.x merged into a single `shacl 0.3.1`
   crate, closing the `iri_s` → `rudof_iri` migration half of
   ERRATA.v0.2 E-009.
2. **Patched `reasonable` fork (styk-tv branch
   `rdf12-passthrough`).** Adds a `#[cfg(feature = "rdf-12")]
   TermRef::Triple(_) => panic!(...)` arm in
   `lib/src/common.rs:140` plus a passthrough feature
   `rdf-12 = ["oxrdf/rdf-12"]`. Strictly additive; lets pgRDF
   compose `shacl 0.3` (which hard-enables `rdf-12` via
   `rudof_rdf`) with `reasonable` 0.4.x in the same workspace.
   See [`ERRATA.v0.4`](ERRATA.v0.4.md) E-011 for the full patch
   summary.

The fork is wired via `[patch.crates-io]` in
[`Cargo.toml`](../Cargo.toml). Once `gtfierro/reasonable` merges
the upstream PR (held in the fork as `PR-DRAFT.md`), drop the
patch and pin the released `reasonable` version.

### 9.4 Acceptance criteria (v0.4 gate — landed ✅)

- A SHACL `sh:NodeShape` with `sh:property` + `sh:datatype`
  reports a violation on a focus node whose data is missing the
  required property. Regression:
  [`tests/regression/sql/71-shacl-real.sql`](../tests/regression/sql/71-shacl-real.sql).
- The report's `conforms` flag is `false` iff `results[]` is
  non-empty, and `true` otherwise.
- Each violation carries a `sh:Violation` severity by default;
  shape-author `sh:severity` declarations override.
- The data + shapes graphs are rehydrated from pgRDF's storage —
  no external file IO, no external SPARQL endpoint — so the
  validator runs inside the calling Postgres transaction.

Sample violation output (from `71-shacl-real.sql`, Alice missing
required `ex:age`):

```json
{
  "conforms": false,
  "results": [{
    "focusNode": "http://example.org/alice",
    "resultPath": "http://example.org/age",
    "sourceShape": "_:b887c79907df332dbd793b0bc80edbd5",
    "resultMessage": "MinCount(1) not satisfied",
    "resultSeverity": "sh:Violation",
    "sourceConstraintComponent": "http://www.w3.org/ns/shacl#MinCountConstraintComponent",
    "value": null
  }],
  "data_graph_id": 8971,
  "shapes_graph_id": 8972,
  "data_triples": 5,
  "shapes_triples": 10,
  "elapsed_ms": 1.68
}
```

### 9.5 Forward look — moved to v0.5-FUTURE

Validation-against-materialised-graph, SHACL-SPARQL constraint mode,
and the W3C SHACL manifest runner all move forward to
[`SPEC.pgRDF.LLD.v0.5-FUTURE §5/§6`](SPEC.pgRDF.LLD.v0.5-FUTURE.md).
v0.4 ships the Core-`Native` mode only.

## 10. TriG / N-Quads ingest — moved to v0.5-FUTURE

TriG and N-Quads ingest UDFs (`pgrdf.parse_trig`, `pgrdf.parse_nquads`)
move forward to
[`SPEC.pgRDF.LLD.v0.5-FUTURE §4`](SPEC.pgRDF.LLD.v0.5-FUTURE.md).
v0.4 retains Turtle-only ingest from v0.3.

## 11. SPARQL surface backlog (deferred from v0.3, now in scope)

These items were enumerated under "⏳ v0.4" in
[`v0.3 §3`](SPEC.pgRDF.LLD.v0.3.md). v0.4 ships them together with
§4-§7 because the same translator machinery they need
(LATERAL-style derived-table refactor + AST substitution) is the
same machinery §4 (UPDATE) and §6 (CONSTRUCT) need. Ship together
for economy. 🚧

- **Multi-triple `OPTIONAL { BGP }`.** The v0.3 OPTIONAL handler
  supports a single-triple right side. v0.4 extends it to N-triple
  BGPs by emitting a LATERAL-style derived-table inside the LEFT
  JOIN.
- **`VALUES` inline tables.** Translates to a derived-table /
  CTE that materialises the inline rows; the BGP joins against it
  on the bound variables.
- **`BIND` output downstream.** AST substitution pass: every
  reference to a `BIND`-introduced variable in a later FILTER or
  BGP rewrites to the bound expression. The v0.3 limitation
  (BIND projection-only) lifts.
- **Aggregates over `UNION`.** Derived-table refactor: the UNION
  becomes a sub-SELECT, aggregation runs over its rows. Residual
  refinements after the v0.4 cut move to
  [`v0.5-FUTURE §8`](SPEC.pgRDF.LLD.v0.5-FUTURE.md).
- **`DESCRIBE`.** Like CONSTRUCT but returning the closure around
  the described subject (every triple where the subject is the
  named term, transitively expanded one hop on blank nodes per
  W3C §16.4). Routes through `pgrdf.construct` internally for the
  triple-output shape.

Acceptance criteria for each carry the v0.3 LLD's existing wording:
the relevant regression file gains the deferred shape, the
`unsupported_algebra` entry for that form disappears from
`pgrdf.sparql_parse` output.

## 12. Performance work carried forward from v0.3

- **Phase 3 step 3b** — `heap_multi_insert` / `COPY BINARY` ingest
  path. The 2× wall-clock target from
  [`v0.3 §4.3`](SPEC.pgRDF.LLD.v0.3.md) phase B remains unmet. v0.4
  targets shipping this. Acceptance fixture
  (`tests/regression/sql/52-bulk-ingest-perf.sql`) is already in
  place from v0.3 — re-measure once phase B lands. 🚧
- **Postgres custom-scan hooks** for specific quad-shape access
  patterns. v0.4 is the earliest target; may slip to v0.5 if the
  refactor cost exceeds the §4 / §6 wins. Acceptance: measurable
  wall-clock win on a single-predicate, single-graph SELECT against
  a materialised closure. 🚧

These do not gate the surface work in §3-§7; they ship in their
own slices.

## 13. Test policy (continues v0.3 §6, unchanged in spirit)

- Every new UDF lands with at least one `#[pg_test]` and at least
  one pg_regress fixture.
- No `ACCEPT=1` autobaselining of new query coverage. Expected
  outputs are hand-computed from the SQL + spec.
- The W3C SPARQL 1.1 manifest runner (Phase 6 step 2, gated `if: false`
  in v0.3) is wired in v0.4 — it gates §11's SPARQL backlog
  automatically as the deferred forms come online. 🚧
- Test bar at the start of the v0.4 cycle (post-SHACL slice):
  **94 pgrx + 40 pg_regress + 23 W3C + 3 LUBM = 160 tests** green.
  The v0.4 cut targets pg_regress growth to roughly 60-something
  files across §3-§7 and §11.
  Approximate breakdown:
  - §3 named-graph + IRI mapping: 6-8 files
    (`70-graph-iri-map.sql`, `71-graph-scoped-select.sql`,
    `72-graph-var-projection.sql`, `73-pg-dump-roundtrip.sql`, …).
  - §4 UPDATE: 8-10 files
    (`74-update-insert-data.sql`, `75-update-delete-data.sql`,
    `76-update-where.sql`, `77-update-delete-insert-where.sql`,
    `78-update-graph-scoped.sql`, …).
  - §5 lifecycle: 4 files
    (`82-drop-graph.sql`, `83-clear-graph.sql`,
    `84-copy-graph.sql`, `85-move-graph.sql`).
  - §6 CONSTRUCT: 3-4 files.
  - §7 paths: 5-6 files (one per operator + materialised-fallback
    detection).
  - §11 SPARQL backlog: 5-6 files.

The exact numbering will be set during implementation; the policy
is that every UDF and every translator path takes its own file.

## 14. Out of scope (carry forward, unchanged from v0.3)

- Streaming replication / logical decoding of RDF state.
- Federated SPARQL `SERVICE` — not in v0.4, v0.5, or v1.0 as
  currently scoped.
- Full OWL 2 (EL / QL) reasoning (`ERRATA.v0.2.md` E-002 — pgRDF
  ships OWL 2 RL only via `reasonable`).
- Backup/restore for opaque binary state (tracked by future
  `SPEC.pgRDF.BACKUP.v0.x`, INSTALL §11 OQ5).
- `LOAD <url>` in SPARQL UPDATE — explicitly not in scope for §4;
  callers fetch externally and invoke `pgrdf.load_turtle` or
  `pgrdf.parse_trig` directly.

## 15. Forward look — see v0.5-FUTURE

The detailed forward look (reasoning-profile selector, TriG/N-Quads
ingest, SHACL-SPARQL mode, W3C SHACL manifest runner, lifecycle IRI
overloads, aggregates-over-UNION refinements, RDF 1.2 triple terms,
incremental materialisation, federated `SERVICE`) lives in
[`SPEC.pgRDF.LLD.v0.5-FUTURE.md`](SPEC.pgRDF.LLD.v0.5-FUTURE.md).

## 16. Errata

- This document is the authoritative-in-progress v0.4 contract.
  Items shipped on `main` are marked ✅; items still in flight are
  marked 🚧. The document is updated in the same slice that
  changes a status marker.
- [`ERRATA.v0.4.md`](ERRATA.v0.4.md) is the v0.4-era spec-deltas
  log. E-011 tracks the upstream `reasonable` patch that unblocked
  §9 SHACL real-impl.
- [`ERRATA.v0.2.md`](ERRATA.v0.2.md) remains authoritative for
  pre-v0.3 items still live (E-006 pgrx 0.18, E-007
  `extension_control_path`, E-008 Linux builder, E-010 cargo audit).
  **E-009** (SHACL upstream conflict) is resolved in v0.4 cycle via
  E-011; final close-out gates on the upstream `reasonable` PR merge.
