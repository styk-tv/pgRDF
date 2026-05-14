# 03 — Query

The query engine answers `SELECT pgrdf.sparql($1)` where `$1` is a
SPARQL 1.1 string. User-facing documentation for the full surface
lives in [`guide/03-querying.md`](../guide/03-querying.md); this
page is the engineering-side view of how it's wired.

## Pipeline (as shipped, Phase 3 step 6)

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

## What is *not* yet shipped (Phase 2.x performance backlog)

The LLD §4.2 design is **prepared statements via `Spi::prepare`**
keyed by an algebra hash. We don't ship that yet — every
`pgrdf.sparql` call builds a fresh SQL string and runs `c.update`,
which forces Postgres to re-parse + re-plan. For a fixed query
shape with varying constants, that's wasted work on every call.

Plan-cache landing notes for the future slice:

- **Key:** canonical algebra hash with constants stripped (so
  `?s :p "Alice"` and `?s :p "Bob"` share a plan).
- **Value:** a `pgrx::Spi`-prepared statement + parameter shape.
- **Eviction:** bounded LRU per backend. Cross-backend sharing
  via shmem belongs with the dict-cache shmem work (LLD §4.1).

## Surface today (Phase 3 step 6)

- ✅ Basic Graph Patterns (1..N triples)
- ✅ `SELECT` with explicit projection or `SELECT *`
- ✅ FILTER (identity, boolean, term-type, BOUND, numeric ordering,
      REGEX, IN, STR)
- ✅ Solution modifiers (DISTINCT, REDUCED, LIMIT, OFFSET, ORDER BY)
- ✅ OPTIONAL (single triple per block, chained)
- ✅ UNION (n-way; per-branch FILTERs / OPTIONALs / MINUSes)
- ✅ MINUS (single triple, shared-var keyed; no-op without shared)
- ⏳ Aggregates (`GROUP BY` + COUNT/SUM/AVG/MIN/MAX) — Phase 3 backlog
- ⏳ Property paths beyond simple sequence — Phase 3 backlog
- ⏳ Named-graph `GRAPH { … }` — needs graph-IRI→graph_id mapping
- ⏳ `BIND`, `VALUES`, `CONSTRUCT`, `ASK`, `DESCRIBE` — Phase 3 backlog
- ❌ Federated `SERVICE` — out of scope for v0.x

## Postgres custom scan hooks

Aspirational — not in scope for v0.2. Once the prepared-plan cache
is in place, the next performance lever is bypassing the standard
executor for specific quad-shape access patterns via the Postgres
custom scan API. Earliest v0.3 target.

## See also

- User-facing surface: [`guide/03-querying.md`](../guide/03-querying.md)
- Implementation: [`src/query/executor.rs`](../src/query/executor.rs)
  + [`src/query/parser.rs`](../src/query/parser.rs)
- Tests: `tests/regression/sql/3[0-8]-sparql-*.sql`
