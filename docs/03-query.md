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
