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
| DISTINCT / REDUCED | `SELECT DISTINCT …` | REDUCED → DISTINCT (safe over-approximation per spec) |
| ORDER BY ?v | `ORDER BY (SELECT lex …) ASC/DESC NULLS LAST` or by ordinal | Unprojected ?v → hidden trailing SELECT column |
| LIMIT N / OFFSET N | `LIMIT N` / `OFFSET N` | Postgres-native |

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

## Surface today (v0.3 SPARQL surface complete)

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
- ⏳ `CONSTRUCT`, `DESCRIBE` — different output shape; v0.4
- ⏳ Property paths beyond simple sequence (`*`, `+`, `?`, `^`, `\|`) — v0.4
- ⏳ Named-graph `GRAPH { … }` — needs graph-IRI→graph_id mapping; v0.4
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
