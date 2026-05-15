# 03 — Query

The query engine answers `SELECT pgrdf.sparql($1)` where `$1` is a
SPARQL 1.1 string. User-facing documentation for the full surface
lives in [`guide/03-querying.md`](../guide/03-querying.md); this
page is the engineering-side view of how it's wired.

## Pipeline (v0.3 — full Phase 3 SPARQL surface + steps 1-2 storage perf)

```
SPARQL string
     │
     ▼  spargebra::SparqlParser::new().parse_query
algebra AST (Distinct ▸ Project ▸ Slice ▸ OrderBy ▸ Filter ▸ LeftJoin ▸ Union ▸ Minus ▸ Bgp)
     │
     ▼  src/query/executor.rs::parse_select
ParsedSelect { projected, bgp, filters, optionals, minuses,
               union_branches, distinct, order_by, limit, offset }
     │
     ▼  build_bgp_sql (or build_union_sql for UNION queries)
dynamic SQL string with constant dict IDs already inlined
     │
     ▼  Spi::connect_mut(|c| c.update(sql, None, &[]))
result rows ─► SETOF JSONB
```

## Modules

- `src/query/parser.rs` — `pgrdf.sparql_parse(q TEXT) → JSONB`,
  returns the spargebra algebra shape (form, projected vars, BGP
  triples, `unsupported_algebra` tags). Used for introspection +
  for the executor's "is this expression translatable" check.
- `src/query/executor.rs` — `pgrdf.sparql(q TEXT) → SETOF JSONB`.
  Walks the wrapper algebra into `ParsedSelect`, then dispatches:
  - **Single-branch path** (`build_single_branch_outer`): one
    SELECT statement with FROM + WHERE + the hidden-column trick
    for ORDER-BY-on-unprojected-var.
  - **UNION path** (`build_union_sql`): each branch becomes its
    own SELECT via `build_branch_sql`, combined with `UNION ALL`,
    wrapped by an outer SELECT for DISTINCT / ORDER BY / LIMIT /
    OFFSET.
  - Shared subroutine `build_from_and_where` emits explicit
    `INNER JOIN` syntax for mandatory BGP patterns 2..N, then
    `LEFT JOIN` per OPTIONAL block, then `WHERE NOT EXISTS (…)`
    per MINUS block.

## Translation strategy

| SPARQL form | SQL emitted | Notes |
|---|---|---|
| Single BGP `?s :p ?o` | `FROM _pgrdf_quads q1 WHERE …` | First-occurrence anchors record `(alias, col)` per variable |
| Multi-pattern BGP | `q1 INNER JOIN q2 ON (q2.col = q1.col …)` | Shared vars become equality predicates that fold into the join |
| Constant in any position | `qN.col = <resolved dict id>` | Unknown IRIs/literals resolve to `-1` → zero rows (spec-correct "no solutions") |
| FILTER identity (`=`, `!=`, `sameTerm`) | dict-id equality | Sound because `_pgrdf_dictionary` dedups by (type, lex, datatype, lang) |
| FILTER numeric ordering | `CASE WHEN datatype_iri_id IN (…XSD numeric…) THEN lex::numeric ELSE NULL END` | Type-safe; non-numeric drops the row via NULL comparison |
| FILTER REGEX | Postgres `~` / `~*` against `lexical_value` | `i` flag → case-insensitive |
| FILTER BOUND | `qN.col IS NOT NULL` | Correct for OPTIONAL vars (nullable); trivially TRUE for mandatory |
| OPTIONAL { triple } | `LEFT JOIN _pgrdf_quads qOPT_K ON (…)` | Per-block FILTER lands in the ON clause |
| UNION { A } { B } | `(SELECT … FROM A) UNION ALL (SELECT … FROM B)` | Each branch SELECTs `NULL::TEXT` for vars it doesn't bind |
| MINUS { triple } | `WHERE NOT EXISTS (SELECT 1 FROM _pgrdf_quads qMIN_K WHERE …)` | Elided at translation time when there are no shared variables (SPARQL no-op) |
| `GRAPH <iri> { … }` | `qN.graph_id = <resolved>` on every triple alias inside the block | IRI resolved against `_pgrdf_graphs.iri` at translate time; unresolved IRI binds to `-1` (zero rows, spec-correct "no solutions") |
| `GRAPH ?g { … }` | `INNER JOIN _pgrdf_graphs g{S} ON g{S}.graph_id = q{first}.graph_id` + `qN.graph_id = q{first}.graph_id` for non-anchor triples | One JOIN per Variable scope; ?g projects as `g{S}.iri` (the IRI string); INNER matches W3C §13.3 — only mapped graphs bind ?g; multi-triple inner BGPs share the anchor's graph_id so triples can't stitch across graphs |
| GRAPH composition (slice 112) | Per-pattern `Option<GraphScope>`; GRAPH inside OPTIONAL/UNION/MINUS scopes only its contained triples, OPTIONAL/MINUS inside GRAPH inherits the outer scope | Mandatory Variable scopes → INNER JOIN to `_pgrdf_graphs`; OPTIONAL-born scopes → LEFT JOIN so unmatched OPTIONALs still NULL out `?g` instead of dropping outer rows; MINUS scopes stay internal to the NOT EXISTS subquery |
| DISTINCT / REDUCED | `SELECT DISTINCT …` | REDUCED → DISTINCT (safe over-approximation per spec) |
| ORDER BY ?v | `ORDER BY (SELECT lex …) ASC/DESC NULLS LAST` or by ordinal | Unprojected ?v → hidden trailing SELECT column |
| LIMIT N / OFFSET N | `LIMIT N` / `OFFSET N` | Postgres-native |

## Named-graph GRAPH-scope translation (LLD v0.4 §3.3, shipped — Phase A slices 114 → 112)

The single-row "GRAPH composition" entry in the translation matrix
above abbreviates the algorithm slice 112 landed; the engineering
detail follows.

**Per-pattern scope, not per-query.** Each triple, each OPTIONAL
block, and each MINUS block carries an
`Option<GraphScope>` describing the innermost enclosing GRAPH
block during the algebra walk. `GraphScope` has two arms:

- `Literal(graph_id: i64)` — resolved at translate time against
  `_pgrdf_graphs.iri`; unresolved IRI binds to `-1` (no real
  partition uses that value), so an unknown IRI yields zero rows
  per W3C SPARQL 1.1 §13.3 "no solutions".
- `Variable { name, scope_id }` — the `?g`-style variable name
  plus a globally-unique scope id (counter on
  `ParsedSelect.graph_scope_counter`). Two GRAPH blocks under the
  same query get distinct `scope_id`s even if they name the same
  variable.

**INNER vs LEFT JOIN to `_pgrdf_graphs`.** `build_from_and_where`
pre-scans the BGP and OPTIONALs to produce a `ScopePlan`:

- **Mandatory** Variable scopes (a GRAPH block at the top level
  of a BGP) get an `INNER JOIN _pgrdf_graphs g{scope_id} ON
  g{scope_id}.graph_id = q{anchor}.graph_id`. The anchor is the
  first BGP alias inside that scope. INNER matches W3C §13.3:
  only graphs present in the IRI mapping bind the variable.
- **OPTIONAL-born** Variable scopes (a GRAPH block nested inside
  an OPTIONAL with no enclosing GRAPH) get a `LEFT JOIN
  _pgrdf_graphs g{scope_id} ON g{scope_id}.graph_id =
  q{opt_anchor}.graph_id`. An unmatched OPTIONAL leaves `?g`
  NULL without dropping the outer row.
- Triples 2..N within a scope carry `qN.graph_id =
  q{anchor}.graph_id` (Variable) or `qN.graph_id = $K`
  (Literal), so a multi-triple inner BGP cannot stitch triples
  from different graphs together.
- Two GRAPH blocks binding the same `?g` get a
  `g{later}.graph_id = g{anchor}.graph_id` predicate so the
  projected variable stays consistent across the joins.

**MINUS inherits outer scope.** A MINUS block carries the GRAPH
scope active at the point its `NOT EXISTS` sub-SELECT is built;
the scope predicate lands inside the sub-SELECT so the MINUS
remains internal to the outer row's existence test. OPTIONAL /
MINUS that nest inside a GRAPH inherit the outer scope (innermost
wins at AST-walk time, per W3C §13.3).

**Projection.** When the projected variable matches a Variable
scope's `name`, the SELECT clause emits `g{scope_id}.iri` rather
than the integer `qN.graph_id` — the JSONB row value is the IRI
string, matching SPARQL semantics. `SELECT *` adds the graph
variable to the projected list even when no inner triple anchors
it (the GRAPH block itself is the anchor).

**Bare BGPs.** A triple outside any `GRAPH { … }` carries
`scope = None`, meaning "match in any graph" — unchanged from
v0.3 semantics (`pgrdf.sparql` over the union of all partitions).

Implementation in
[`src/query/executor.rs`](../src/query/executor.rs); regression
coverage:
[`78-sparql-graph-literal-iri.sql`](../tests/regression/sql/78-sparql-graph-literal-iri.sql),
[`79-sparql-graph-variable.sql`](../tests/regression/sql/79-sparql-graph-variable.sql),
[`87-sparql-graph-composition.sql`](../tests/regression/sql/87-sparql-graph-composition.sql),
plus W3C-shape fixtures 24 / 25 / 26 under
[`tests/w3c-sparql/`](../tests/w3c-sparql/).

## Prepared-plan cache (LLD §4.2, **shipped — Phase 3 step 2**)

Lives in [`src/query/plan_cache.rs`](../src/query/plan_cache.rs).
The flow:

```
parse → translate → ExecPlan { sql: "...$1...$2...", params: [..] }
                          │
                          ▼
                  Spi::connect_mut
                          │
                          ├── plan_cache.contains(sql) ?
                          │      │
                          │      └── miss → client.prepare(sql, &[INT8OID; n])
                          │                          │
                          │                          └── .keep() → OwnedPreparedStatement
                          │                                              │
                          │                          plan_cache.insert(sql, ↑)
                          │      │
                          │      └── hit  → record_hit()
                          │
                          ▼
                  client.update(&owned, None, &datums)
```

Concrete shape:
- **Parameterisation.** Every dict-id constant in the dynamic SQL
  (subject / predicate / object literals in BGP triples; constants
  in FILTER `=` `!=` `IN(…)`; the xsd:numeric dict-id list inside
  numeric-comparison sub-SELECTs) becomes a `$N` positional
  placeholder. A `thread_local!` `PARAM_BUF` collects the resolved
  i64s in declaration order. `translate()` snapshots the buffer
  into `ExecPlan { sql, params }`. The SQL string itself is the
  canonical cache key — same algebra shape ⇒ same SQL byte-for-byte
  ⇒ same key, no extra hashing layer.
- **Cache.** Per-backend `thread_local!`
  `RefCell<HashMap<String, OwnedPreparedStatement>>`. Lifetime-
  promoted via `PreparedStatement::keep()` (`SPI_keepplan`).
  Capacity is unbounded for v1; typical backends touch a few
  dozen distinct shapes per session. Eviction (bounded LRU) is a
  v0.4 polish.
- **Counters.** `plan_cache_hits / misses / inserts` live in shmem
  (`PgAtomic<AtomicU64>`) so a multi-backend benchmark reads a
  single fleet-wide view through `pgrdf.stats()`. Per-backend
  `plan_cache_local_size` is also exposed in stats — useful for
  catching unbounded growth in a misbehaving session.
- **Invalidation.** Plans are parameterised, so dict-id reshuffles
  from `DROP EXTENSION; CREATE EXTENSION` don't invalidate the SQL
  itself — only the parameter VALUES change next call. Postgres's
  own SPI cached-plan invalidation handles relation drops. For
  paranoia, `pgrdf.plan_cache_clear() -> bigint` empties THIS
  backend's cache and returns the count.
- **Acceptance criterion** (LLD §4.2): repeated structural queries
  with varying constants reuse the cached plan. `tests/regression/sql/51-plan-cache.sql`
  verifies: 5 identical queries → 1 miss + 4 hits; 2 queries with
  same shape but different IRI constants → 1 miss + 1 hit; a
  structurally distinct query → 1 miss + 0 hits.

## Surface today (v0.3 SPARQL surface complete; v0.4 §3.3 GRAPH shipped)

- ✅ Basic Graph Patterns (1..N triples)
- ✅ `SELECT` (explicit projection or `SELECT *`); `ASK`
- ✅ FILTER — identity, boolean, term-type, BOUND, numeric
      ordering, REGEX, IN, STR, LANG, DATATYPE, UCASE, LCASE,
      STRLEN, CONTAINS, STRSTARTS, STRENDS, arithmetic
- ✅ Solution modifiers — DISTINCT, REDUCED, LIMIT, OFFSET, ORDER BY
- ✅ OPTIONAL (single triple per block, chained)
- ✅ UNION (n-way; per-branch FILTERs / OPTIONALs / MINUSes)
- ✅ MINUS — single AND multi-triple sub-pattern, shared-var keyed
- ✅ Aggregates — `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT ?v)`,
      `SUM`, `AVG`, type-aware `MIN` / `MAX` (numeric path on
      `xsd:numeric`, lex fallback), `GROUP_CONCAT`, `SAMPLE` with
      `GROUP BY`
- ✅ `HAVING` — both by aggregate alias (`HAVING(?total > c)`)
      AND inline (`HAVING(SUM(?v) > c)`)
- ✅ `BIND(expr AS ?v)` for projection — Literal / NamedNode /
      Variable, STR / LANG / DATATYPE / UCASE / LCASE / STRLEN,
      arithmetic, CONCAT
- ✅ Named-graph `GRAPH <iri> { … }` — literal-IRI form (slice 114).
      Translate-time IRI → `graph_id` resolution via
      `_pgrdf_graphs.iri`; unresolved IRI binds to `-1` (zero
      rows, spec-correct).
- ✅ Named-graph `GRAPH ?g { … }` — variable form (slice 113).
      Inner BGP gains an `INNER JOIN _pgrdf_graphs g{S} ON
      g{S}.graph_id = q{first}.graph_id`; ?g projects as `g{S}.iri`
      (the IRI string, not the integer id). Triples 2..N inside the
      GRAPH block share the anchor's graph_id so a multi-triple
      inner BGP cannot stitch triples from different graphs together.
      INNER JOIN matches W3C SPARQL 1.1 §13.3 — only graphs in the
      IRI mapping bind ?g. COUNT + GROUP BY ?g works as expected.
- ✅ GRAPH composition with OPTIONAL / UNION / MINUS (slice 112).
      Per-pattern `Option<GraphScope>` decorates each triple, each
      OPTIONAL triple, and each MINUS block: a GRAPH block inside
      one of these scopes only the contained triples; an
      OPTIONAL / MINUS inside a GRAPH inherits the outer scope. Two
      GRAPH blocks binding the same `?g` variable get tied together
      with a graph_id equality so the projected variable is
      consistent. Distinct GRAPH blocks get distinct `scope_id`s and
      independent `_pgrdf_graphs` joins; OPTIONAL-born Variable
      scopes use a LEFT JOIN so an unmatched OPTIONAL leaves `?g`
      NULL without dropping the outer row.
- ✅ SPARQL UPDATE foundation — `INSERT DATA { … }` (Phase C slice 84,
      LLD v0.4 §4). `pgrdf.sparql(q)` detects UPDATE forms via a
      try-parse-then-fallback: `parse_query` first (the v0.3 path,
      unchanged), `parse_update` on query-side failure. UPDATE forms
      return a single summary row with shape `{"_update": {form,
      triples_inserted, triples_deleted, graphs_touched,
      elapsed_ms}}` paralleling the v0.3 `_ask` sentinel. Slice 84
      lands INSERT DATA end-to-end (default + named graph,
      multi-triple, idempotent on repeat via `WHERE NOT EXISTS`);
      other UPDATE forms panic with "lands in slice NN" pending
      per-form follow-ups (CLEAR/CREATE/DROP GRAPH → 71/70/69). The
      pattern-driven UPDATE forms shipped in slices 82 (INSERT WHERE),
      81 (DELETE WHERE), and 80 (combined DELETE+INSERT WHERE).
- ✅ SPARQL UPDATE — `INSERT { template } WHERE { pattern }` (Phase C
      slice 82, LLD v0.4 §4.1). Pattern-driven insertion: the WHERE
      pattern goes through the v0.3 `parse_select` walker (sharing the
      BGP/FILTER/OPTIONAL/MINUS algebra with SELECT), emits a custom
      SQL that returns each template-referenced variable's **dict id**
      (BIGINT, not lexical text — lossless internment), and Rust
      iterates the binding rows, materialises each `QuadPattern` in
      the template, and routes through the shared `insert_quad` helper
      (same `WHERE NOT EXISTS` guard as INSERT DATA, set-semantic on
      re-issue). The `_update` summary reports `form: "INSERT_WHERE"`
      so callers can discriminate from `INSERT_DATA`. Limitations
      locked for slice 82: WHERE may not carry aggregates / GROUP BY /
      UNION; template variables MUST be bound by the WHERE BGP
      (fail-fast rather than silent-skip); variable GRAPH in template
      panics (lands with slice 76 graph-scoped INSERT WHERE).
- ✅ SPARQL UPDATE — `DELETE { template } WHERE { pattern }` (Phase C
      slice 81, LLD v0.4 §4.1). Sibling of slice 82's INSERT WHERE.
      Same `parse_select` walker for the WHERE half, same dict-id
      (BIGINT) projection one row per solution, same per-row
      template instantiation. The DELETE template is modelled as
      `Vec<GroundQuadPattern>` (spargebra bakes the W3C §4.1.2 "no
      blank nodes in the DELETE clause" rule into the AST). Per-row
      DELETE uses the `WITH d AS (DELETE … RETURNING 1) SELECT
      count(*)` idiom slice 83 installed for DELETE DATA, so the
      counter reports ACTUAL rows removed (distinct from INSERT
      WHERE's per-attempt counter). Lookup-only dict path mirrors
      slice 83: missing terms in the instantiated template route to
      a per-row no-op rather than an error. The `_update` summary
      reports `form: "DELETE_WHERE"`. Limitations locked: WHERE may
      not carry aggregates / GROUP BY / UNION; template variables
      MUST be bound by the WHERE BGP (fail-fast); variable GRAPH in
      template panics (lands with slice 76).
- ✅ SPARQL UPDATE — `DELETE DATA { … }` (Phase C slice 83, LLD v0.4
      §4). Symmetric to slice 84's INSERT DATA: ground quads only,
      no variables. Default-graph + `GRAPH <iri> { … }` inline
      graph scope both supported. The dispatcher routes through a
      **lookup-only** dictionary path (no interning) — if any term
      of the quad is missing from `_pgrdf_dictionary`, the quad
      cannot exist in `_pgrdf_quads`, so the operation is a
      spec-correct no-op rather than an error. Same-shape triples
      in a different graph are NOT touched. Repeated DELETE against
      the same quad is idempotent (the second call reports
      `triples_deleted = 0`). When the Update carries multiple
      operations of mixed kinds (e.g. a future
      `DELETE DATA ; INSERT DATA`), the `form` field collapses to
      `"MIXED"` and the per-op counters aggregate.
- ✅ SPARQL UPDATE — `DELETE { … } INSERT { … } WHERE { … }` (Phase C
      slice 80, LLD v0.4 §4.1). Atomic modify form. Both halves resolve
      against the SAME WHERE solutions snapshot: the pattern is
      evaluated exactly once, the projection unions every variable
      referenced by EITHER template (DELETE-side then INSERT-side,
      first-appearance per side), and Rust iterates the binding rows
      applying DELETE then INSERT per row. Per W3C SPARQL 1.1 Update
      §3.1.3 the DELETE conceptually precedes the INSERT — important
      for status-flip patterns (`DELETE { ?x ex:status "draft" }
      INSERT { ?x ex:status "approved" } WHERE { ?x ex:status
      "draft" }`) where the DELETE removes the old row and the INSERT
      adds the new one cleanly. Atomicity is naturally provided by
      Postgres's transaction model. DELETE counter uses the
      `WITH d AS (DELETE … RETURNING 1) SELECT count(*)` idiom from
      slice 81/83 (actual rows removed); INSERT counter is per-attempt
      (slice 82 convention). Summary reports `form:
      "DELETE_INSERT_WHERE"`. Limitations inherit slices 81/82: no
      aggregates / GROUP BY / UNION in WHERE; template variables must
      be bound by the WHERE BGP; variable GRAPH in either template
      panics with the slice-76 prefix; `USING / USING NAMED` not yet
      supported.
- ✅ SPARQL UPDATE — graph-scoped variants (`WITH <iri>` +
      `GRAPH <iri> { … }` in template / WHERE) (Phase C slice 79,
      LLD v0.4 §4.1). Closes the graph-aware loop for pattern-driven
      UPDATEs. Spargebra desugars `WITH <iri>` at parse time into
      (a) per-quad `graph_name` injection on every default-graph
      template QuadPattern (the per-row instantiators
      `instantiate_template_quad` / `instantiate_ground_template_quad`
      already routed `GraphNamePattern::NamedNode` into the right
      partition since slices 80/81/82) and (b) a
      `using: Some(QueryDataset { default: [<iri>], named: None })`
      sentinel on the DeleteInsert operation. The slice-79 dispatcher
      lifts the IRI from (b) and wraps the WHERE pattern in
      `GraphPattern::Graph { name, inner }` before passing it to
      `execute_*_where` — the slice-112 walker then scopes every
      emergent BGP triple to `<iri>` (nested explicit
      `GRAPH <other> { … }` still overrides per W3C §13.3). The
      `GRAPH <iri> { … }` in WHERE pattern path was already supported
      (slice 112); the `GRAPH <iri> { … }` in template halves was
      already wired through the per-quad `graph_name` branches in
      slices 80/81/82. Cross-graph copy
      (`INSERT { GRAPH <g2> { … } } WHERE { GRAPH <g1> { … } }`) and
      scoped modify (`WITH <g1> DELETE { … } INSERT { … } WHERE { … }`)
      are now first-class. Limitations: proper `USING <iri>` /
      `USING NAMED <iri>` clauses (distinct from the WITH-injected
      sentinel — i.e. multi-default-graph or USING NAMED) panic with
      `'USING / USING NAMED' not yet supported`.
- ⏳ Lifecycle algebra (`CLEAR/CREATE/DROP GRAPH`) — Phase C slices
      71 → 69.
- ⏳ `CONSTRUCT`, `DESCRIBE` — different output shape; v0.4
- ⏳ Property paths beyond simple sequence (`*`, `+`, `?`, `^`, `\|`) — v0.4
- ⏳ `VALUES` inline data — needs derived-table refactor; v0.4
- ⏳ Aggregates over UNION; multi-triple OPTIONAL; BIND-in-FILTER — v0.4
- ❌ Federated `SERVICE` — out of scope for v0.x

## Postgres custom scan hooks

Aspirational — out of scope for v0.3. With the prepared-plan cache
now in place (Phase 3 step 2), the next performance lever after
COPY BINARY (step 3) is bypassing the standard executor for specific
quad-shape access patterns via the Postgres custom scan API.
Earliest v0.4 target.

## See also

- User-facing surface: [`guide/03-querying.md`](../guide/03-querying.md)
- Implementation: [`src/query/executor.rs`](../src/query/executor.rs)
  + [`src/query/parser.rs`](../src/query/parser.rs)
- Tests: `tests/regression/sql/3[0-8]-sparql-*.sql`
