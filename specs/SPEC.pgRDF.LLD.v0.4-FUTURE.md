# **SPEC.pgRDF.LLD.v0.4-FUTURE**

**pgRDF: A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL,
and OWL 2 RL reasoning.**

*Positioning: pgRDF — the high-performance PostgreSQL semantic-web toolkit.*

---

## 0. Document status and supersession

- **Status:** draft / forward-looking / **target: pgRDF v0.4 cut**.
- **Authoritative shipped contract:** [`SPEC.pgRDF.LLD.v0.3.md`](SPEC.pgRDF.LLD.v0.3.md)
  remains the contract until v0.4 actually lands. This document is
  a sibling, not a replacement.
- **Carries forward:** [`SPEC.pgRDF.INSTALL.v0.2.md`](SPEC.pgRDF.INSTALL.v0.2.md)
  (no install-spec changes anticipated for v0.4) and
  [`ERRATA.v0.2.md`](ERRATA.v0.2.md) (still authoritative for spec
  deltas predating v0.3; ERRATA E-009 in particular gates §9).
- **Reason for v0.4:** the v0.3 cut closes Phase 2's SPARQL surface
  to the line at which a substantial class of downstream consumers
  — operators with named-graph workloads, applications performing
  atomic write-back via SPARQL, ingest pipelines consuming
  TriG / N-Quads, validation tooling, reasoners selecting between
  RDFS and OWL 2 RL profiles, and consumers traversing transitive
  class hierarchies via `rdfs:subClassOf*`-style paths — start
  running into the same handful of gaps. v0.4 closes the
  highest-leverage gaps as a coherent group; v0.5 cleans up the
  rest (see §15 for the forward look).
- **Tense discipline:** v0.4 is **forward-looking**. Future tense
  ("will land", "ships with") is the default throughout. Where
  v0.4 builds on shipped v0.3 mechanism, that mechanism is named
  in present tense and explicitly cross-linked to v0.3.

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
4. **Validation Engine** — SHACL via `shacl_validation` (stubbed in
   v0.3 per ERRATA E-009; planned for v0.5 per §9).

## 2. Scope of v0.4

v0.4 ships five major tracks, plus the SPARQL surface backlog
already enumerated in [`v0.3 §3`](SPEC.pgRDF.LLD.v0.3.md) as
"⏳ v0.4":

1. **Named-graph scoping and IRI mapping** (§3) — `GRAPH { … }` in
   SPARQL, a new `_pgrdf_graphs` system table, IRI ↔ `graph_id`
   helper UDFs.
2. **SPARQL UPDATE** (§4) — `INSERT DATA`, `DELETE DATA`,
   `INSERT … WHERE …`, `DELETE … WHERE …`,
   `DELETE … INSERT … WHERE …`, and the graph-scoped variants
   (`WITH <iri>`, inline `GRAPH <iri> { … }`).
3. **Graph-level lifecycle UDFs** (§5) — `drop_graph`, `clear_graph`,
   `copy_graph`, `move_graph` as state-management primitives over
   the LIST-partitioned `_pgrdf_quads` table.
4. **CONSTRUCT** (§6) — `pgrdf.construct(q TEXT) → SETOF JSONB`
   returning `{subject, predicate, object}`-shaped rows.
5. **Property paths** (§7) — `*`, `+`, `?`, `^`, with alternation
   `p1|p2` as a stretch. Materialised-closure-aware translation.

Plus the v0.3-deferred SPARQL surface items (§11): multi-triple
OPTIONAL, VALUES, BIND-downstream-of-FILTER, aggregates over UNION,
DESCRIBE. These share enough translator machinery with §4 and §6
that they ship in the same cut for economy.

Capability matrix for the v0.4 target:

| Capability | v0.3 status | v0.4 target |
|---|---|---|
| `GRAPH <iri> { … }` and `GRAPH ?g { … }` | ⏳ deferred | ✅ §3 |
| IRI ↔ graph_id mapping table + UDFs | not yet | ✅ §3.1/§3.2 |
| SPARQL UPDATE (INSERT DATA / DELETE DATA / INSERT/DELETE WHERE) | not yet | ✅ §4 |
| `WITH <iri>` + graph-scoped UPDATE | not yet | ✅ §4.1 |
| `pgrdf.drop_graph / clear_graph / copy_graph / move_graph` | not yet | ✅ §5 |
| `CONSTRUCT` | ⏳ deferred | ✅ §6 |
| Property paths `*`, `+`, `?`, `^` | ⏳ deferred | ✅ §7 |
| Property-path alternation `p1\|p2` | not yet | 🎯 stretch §7.1 |
| Multi-triple `OPTIONAL { BGP }` | ⏳ deferred | ✅ §11 |
| `VALUES` inline tables | ⏳ deferred | ✅ §11 |
| `BIND` output in later FILTER / BGP | ⏳ deferred | ✅ §11 |
| Aggregates over `UNION` | ⏳ deferred | ✅ §11 |
| `DESCRIBE` | ⏳ deferred | ✅ §11 |
| Reasoning profile selector (RDFS / OWL-RL) | not yet | ⏳ v0.5 §8 |
| Real SHACL output | 🚧 stub | ⏳ v0.5 §9 (gated on E-009) |
| TriG / N-Quads ingest | not yet | ⏳ v0.5 §10 |
| Incremental materialisation | not yet | ⏳ v1.0 §15 |
| RDF 1.2 triple terms | not yet | ⏳ v1.0 §15 (gated on E-009) |
| Federated `SERVICE` | ❌ | ❌ out of scope (§14) |

## 3. Named-graph scoping and IRI mapping (NEW)

Named-graph workloads are first-class in pgRDF: storage already
partitions `_pgrdf_quads` by `graph_id` (LIST partition), and the
v0.3 cut ships `pgrdf.add_graph(id BIGINT)` and `pgrdf.count_quads`
filtered by graph. What's missing is the **IRI ↔ id binding** and
the **SPARQL `GRAPH { … }` surface**. v0.4 closes both.

Downstream consumers running graph-level lifecycle operations
(drop / clear / copy / move — §5) and atomic write-back via SPARQL
UPDATE (§4) need the IRI mapping as a hard prerequisite: a SPARQL
query writes `GRAPH <http://example.org/g1>`, and pgRDF has to
resolve `<http://example.org/g1>` to the integer `graph_id` of the
backing partition without forcing the caller to track the integer
out-of-band.

### 3.1 Storage extension — `_pgrdf_graphs`

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
- **Default partition** retains `graph_id = 0` semantics from v0.3
  (catch-all for un-IRI-bound writes). Synthetic IRI:
  `urn:pgrdf:graph:0`.

### 3.2 UDF surface

| UDF | Signature | Returns | Semantics |
|---|---|---|---|
| `pgrdf.add_graph(iri TEXT)` | overload | `BIGINT` | Idempotent on the IRI: insert if absent (auto-allocated id), return existing id otherwise. |
| `pgrdf.add_graph(id BIGINT, iri TEXT)` | overload | `BIGINT` | Explicit id binding; errors if id or IRI is already bound to a different counterpart. |
| `pgrdf.add_graph(id BIGINT)` | retained | `BIGINT` | Back-compat with v0.3; assigns synthetic IRI `urn:pgrdf:graph:{id}` automatically. |
| `pgrdf.graph_id(iri TEXT)` | new | `BIGINT` | Returns `NULL` if the IRI is not bound. |
| `pgrdf.graph_iri(id BIGINT)` | new | `TEXT` | Returns `NULL` if the id is not bound. |

The integer-id and IRI surfaces are interchangeable at the UDF
boundary. `pgrdf.put_quad`, `pgrdf.count_quads`, and the lifecycle
UDFs in §5 retain their `BIGINT graph_id` argument forms; an
IRI-keyed overload may follow in v0.5 if usage warrants.

### 3.3 SPARQL GRAPH support

- **`GRAPH <iri> { … }`** resolves `<iri>` against `_pgrdf_graphs.iri`
  to a `graph_id`. Unresolved IRI ⇒ zero rows (spec-correct "no
  solutions"; no error raised).
- **`GRAPH ?g { … }`** projects `?g` as the **IRI** (NOT the integer
  id) — bound by a JOIN against `_pgrdf_graphs`. This is
  user-visible; callers see and write IRIs, never the dictionary id.
- Composition discipline:
  - `GRAPH { … }` composes inside `OPTIONAL`, `UNION`, and `MINUS`
    blocks. Translation reuses the v0.3 `build_from_and_where`
    layout, threading a new `graph_id` (or `graph_iri`) join
    constraint per pattern.
  - Nested `GRAPH` blocks resolve to the innermost scope at AST-walk
    time (per W3C SPARQL 1.1 §13.3).
  - A bare BGP outside any `GRAPH { … }` continues to mean "match in
    any graph" — same semantics as v0.3 (`pgrdf.sparql` over the
    union of all partitions).

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
surface.

### 4.1 Surface

| Form | Notes |
|---|---|
| `INSERT DATA { … }` | Direct triple insertion (single triple or BGP-style block). Constants only — no variables. |
| `DELETE DATA { … }` | Direct triple removal. Constants only. |
| `INSERT { template } WHERE { pattern }` | Pattern-driven insertion. Each solution of `WHERE` instantiates `template` once. |
| `DELETE { template } WHERE { pattern }` | Pattern-driven removal. |
| `DELETE { … } INSERT { … } WHERE { … }` | Atomic modify — both operations run against the same `WHERE` solutions snapshot. |
| `WITH <iri> …` | Graph scope for the surrounding INSERT/DELETE/WHERE. |
| `INSERT { GRAPH <iri> { … } }` | Inline graph scope on the template. |
| `DELETE { GRAPH <iri> { … } }` | Inline graph scope on the template. |

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
a vector of `GraphUpdateOperation`s. The translator walks that
algebra parallel to the SELECT translation in
`src/query/executor.rs`, dispatching per operation variant
(`InsertData`, `DeleteData`, `DeleteInsert`, `Load`, `Clear`,
`Create`, `Drop`, `Add`, `Move`, `Copy`).

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

`pgrdf.sparql_parse(q TEXT)` already parses UPDATE shapes through
the same `spargebra` entry point. v0.4 revs the JSONB return to
surface the UPDATE form variants explicitly:

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
| `DROP GRAPH <iri>` | `pgrdf.drop_graph(graph_id(iri))` |
| `CLEAR GRAPH <iri>` | `pgrdf.clear_graph(graph_id(iri))` |
| `CREATE GRAPH <iri>` | `pgrdf.add_graph(iri)` |
| `COPY <src> TO <dst>` | `pgrdf.copy_graph(graph_id(src), graph_id(dst))` |
| `MOVE <src> TO <dst>` | `pgrdf.move_graph(graph_id(src), graph_id(dst))` |
| `ADD <src> TO <dst>` | `pgrdf.copy_graph` (ADD = COPY without first-clearing dst per W3C SPARQL 1.1 Update §3.2.6) |

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
partitioning.

### 5.1 Surface

| UDF | Signature | Returns | Semantics |
|---|---|---|---|
| `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN DEFAULT TRUE)` | new | `BIGINT` | Removes the partition entirely; returns the count of triples that were in it. `cascade => FALSE` errors if inferred rows are present. |
| `pgrdf.clear_graph(id BIGINT)` | new | `BIGINT` | `TRUNCATE ONLY` the partition; the partition itself is preserved (so subsequent inserts route normally). Returns triples removed. |
| `pgrdf.copy_graph(src BIGINT, dst BIGINT)` | new | `BIGINT` | Copies all quads from `src` to `dst`. Creates the `dst` partition if absent. Returns triples copied. |
| `pgrdf.move_graph(src BIGINT, dst BIGINT)` | new | `BIGINT` | Atomic association swap: the `src` partition's `FOR VALUES IN (...)` clause rebinds to the new id. Returns triples moved (== row count at swap time). |

IRI overloads (`pgrdf.drop_graph(iri TEXT)`, etc.) may follow in
v0.5; in v0.4 callers route IRI input through `pgrdf.graph_id(iri)`
explicitly.

### 5.2 Implementation notes

- `_pgrdf_quads` is LIST-partitioned on `graph_id` per v0.3 §4. The
  lifecycle UDFs lean on Postgres's partition-management DDL:
  - `drop_graph` issues
    `ALTER TABLE _pgrdf_quads DETACH PARTITION ...` then
    `DROP TABLE`. The detach is metadata-only; the subsequent drop
    drops the partition table's own row storage and indexes. Cost
    is independent of row count (modulo btree page release).
  - `move_graph` is also metadata-only: rebind the partition's
    `FOR VALUES IN (<old_id>)` clause to `FOR VALUES IN (<new_id>)`.
    Postgres requires DETACH + ATTACH for this; the DETACH/ATTACH
    pair runs under an `ACCESS EXCLUSIVE` lock on the parent for a
    bounded window. Backing rows do not move.
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
- **Constant-time move.** `pgrdf.move_graph(src, dst)` execution
  time is independent of `_pgrdf_quads` row count in `src`
  (measured: < 100 ms for a graph of 1 000 000 quads; covered by a
  performance regression fixture).
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
diverges from the `pgrdf.sparql` JSONB row shape.

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
operators.

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
materialised under a profile (OWL-RL or RDFS — see §8) that
already entails the closure of the path's predicate, the translator
falls back to a direct BGP match against the materialised triples.
No recursion is emitted.

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

## 8. Reasoning profile selector (v0.5 — flagged here for planning)

Reasoners selecting between RDFS and OWL 2 RL per workload class
need a per-call profile selector on `pgrdf.materialize`. v0.4 keeps
the v0.3 surface (`pgrdf.materialize(graph_id) → JSONB`) unchanged
but flags this as a v0.5 work item.

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
  gains a sibling `63-materialize-rdfs.sql`. The
  `'rdfs'` regression asserts the entailed-triple count is a
  **non-strict subset** of the OWL-RL count on the same input.

**Acceptance criteria (v0.5 gate):**
- `pgrdf.materialize(g, 'rdfs')` triple count ≤
  `pgrdf.materialize(g, 'owl-rl')` triple count on a fixed input.
- The two profiles agree on the entailment of the RDFS axioms
  (subClassOf transitivity, domain/range propagation, etc.).

## 9. SHACL real integration (v0.5 — gated on ERRATA E-009)

The v0.3 `pgrdf.validate` stub remains in place. v0.4 does not
attempt to unblock it. The mechanical plan to ship the real
integration, once the upstream conflict resolves, is identical to
[`v0.3 §5.3`](SPEC.pgRDF.LLD.v0.3.md):

**Unblock checklist:**
1. Verify `shacl_validation 0.3.x` (or a 0.2.x point release)
   compiles cleanly against a single `iri_s` major.
2. Verify it does not force `oxrdf`'s `rdf-12` feature, OR confirm
   that `reasonable` ships handling for `TermRef::Triple(_)`.
3. Add `shacl_validation` back to `Cargo.toml`.
4. Replace the `pgrdf.validate` stub body with N-Triples
   serialisation of both data and shapes graphs +
   `GraphValidation::from_graph(...).validate(&schema_ir)`.
5. Map `ValidationReport.results()` → JSONB W3C
   `sh:ValidationReport` shape.

Surface stays stable: the stub already emits a versioned JSONB
shape; the real implementation preserves it.

**v0.5 enhancement:** allow `data_graph` to be a materialised
reasoning graph IRI (per §3/§8), so validation tooling consuming
W3C `sh:ValidationReport` output can validate shapes against
entailed triples, not just base ones.

**Acceptance criteria (v0.5 gate):**
- A SHACL `sh:NodeShape` with `sh:property` + `sh:class` reports
  violations on malformed input.
- The data graph IRI may resolve to either a base graph or a
  materialised reasoning graph.
- Output JSONB matches the W3C `sh:ValidationReport` shape
  byte-for-byte (a regression fixture round-trips through
  `pgrdf.load_turtle` of the JSONB-serialised report).

## 10. TriG / N-Quads ingest (v0.5)

Ingest pipelines that consume TriG and N-Quads with inline graph
declarations need a parser that honours the inline `GRAPH { … }`
blocks and resolves graph IRIs through the §3 mapping. v0.4 does
not ship this; v0.5 does.

**v0.5 surface:**

```sql
pgrdf.parse_trig(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) → JSONB
pgrdf.parse_nquads(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) → JSONB
```

- **TriG:** accept the W3C TriG grammar; honour inline
  `GRAPH <iri> { … }` blocks; resolve `<iri>` via §3.2
  `pgrdf.add_graph(iri)` (auto-allocate unknown IRIs by default).
- **N-Quads:** parse the 4-position line format; resolve the
  fourth-position IRI via §3.2.
- **`strict => TRUE`:** reject unknown graph IRIs instead of
  auto-allocating. Useful for ingest into a pre-bound graph space.
- Both UDFs reuse the v0.3 batched-insert path (same `flush_batch`
  prepared plan).

**Acceptance criteria (v0.5 gate):**
- A TriG document declaring three inline named graphs loads into
  three pgRDF graphs in a single call.
- Unknown graph IRIs auto-allocate (default) or reject under
  `strict => TRUE`.
- Round-trip: `pgrdf.parse_trig` followed by a CONSTRUCT-of-each-graph
  re-serialised back to TriG produces an isomorphic document.

## 11. SPARQL surface backlog (deferred from v0.3, now in scope)

These items were enumerated under "⏳ v0.4" in
[`v0.3 §3`](SPEC.pgRDF.LLD.v0.3.md). v0.4 ships them together with
§4-§7 because the same translator machinery they need
(LATERAL-style derived-table refactor + AST substitution) is the
same machinery §4 (UPDATE) and §6 (CONSTRUCT) need. Ship together
for economy.

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
  becomes a sub-SELECT, aggregation runs over its rows.
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
  place from v0.3 — re-measure once phase B lands.
- **Postgres custom-scan hooks** for specific quad-shape access
  patterns. v0.4 is the earliest target; may slip to v0.5 if the
  refactor cost exceeds the §4 / §6 wins. Acceptance: measurable
  wall-clock win on a single-predicate, single-graph SELECT against
  a materialised closure.

These do not gate the surface work in §3-§7; they ship in their
own slices.

## 13. Test policy (continues v0.3 §6, unchanged in spirit)

- Every new UDF lands with at least one `#[pg_test]` and at least
  one pg_regress fixture.
- No `ACCEPT=1` autobaselining of new query coverage. Expected
  outputs are hand-computed from the SQL + spec.
- The W3C SPARQL 1.1 manifest runner (Phase 6 step 2, gated `if: false`
  in v0.3) is wired in v0.4 — it gates §11's SPARQL backlog
  automatically as the deferred forms come online.
- New surface for v0.4: pg_regress files project to grow from
  the v0.3 baseline (25 at the v0.3 cut per
  [`v0.3 §2`](SPEC.pgRDF.LLD.v0.3.md); ~39 today including
  post-cut hygiene fixtures) to roughly 60-something across
  §3-§7 and §11.
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

## 15. Forward look — v0.5 and v1.0

**v0.5 contents (planned):**
- Real SHACL output (§9 — gated on ERRATA E-009).
- Reasoning profile selector (§8).
- TriG / N-Quads ingest (§10).
- Aggregates-over-UNION refinements not landed in v0.4 §11.
- IRI overloads for the §5 lifecycle UDFs.
- W3C SHACL manifest runner wired in CI.

**v1.0 contents (planned):**
- Incremental (delta-driven) materialisation:
  `pgrdf.materialize_delta(graph_id, since_xid TEXT)` — forward-chain
  only over quads added since a recorded transaction id. Targets a
  common optimisation pattern in reasoning pipelines but does not
  gate any v0.x surface.
- RDF 1.2 triple terms — enable `oxrdf`'s `rdf-12` feature once
  upstream (`reasonable` + `shacl_validation`) supports it
  unanimously. ERRATA E-009 tracks the conflict.
- Federated `SERVICE` — explicitly deferred to v1.0; remains out of
  scope for v0.x.

No domain-specific motivation appears in this section; the items
are listed as engineering targets only.

## 16. Errata

- This document is the **draft** v0.4 contract. It is not yet
  authoritative; v0.3 remains the shipped contract.
- Spec corrections discovered during v0.4 implementation will land
  in a future `ERRATA.v0.3.md` (or `ERRATA.v0.4.md` if scope
  warrants a dedicated file).
- `ERRATA.v0.2.md` remains authoritative for pre-v0.3 items.
- **E-009** (SHACL upstream conflict) is unblock-tracked in §9 of
  this document. It is the only erratum that meaningfully blocks a
  v0.5 work item; it does not block any v0.4 work item.
