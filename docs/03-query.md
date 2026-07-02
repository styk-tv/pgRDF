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
| FILTER BOUND | `qN.col IS NOT NULL` | Correct for OPTIONAL vars (nullable, resolved via `qOPT.vK`); trivially TRUE for mandatory |
| OPTIONAL { BGP } | `LEFT JOIN LATERAL (SELECT <vars AS vK> FROM <inner BGP> WHERE <preds + correlation + inner FILTERs>) qOPT ON TRUE` | Phase F group F1: N-triple right side as one atomic LATERAL derived table (all-or-nothing, W3C §6.1); nested OPTIONAL recurses the same emitter; optional-only vars resolve as `qOPT.vK` |
| `VALUES (?x …) { … }` | `CROSS JOIN (VALUES (id,…),(id,…)) AS vN(vK…)` + `(vN.vK IS NULL OR vN.vK = q{anchor}.{col})` correlation | Phase F group F1: constants → dict ids ahead of execution; `UNDEF` → NULL cell (no constraint, W3C §10) |
| UNION { A } { B } | `(SELECT … FROM A) UNION ALL (SELECT … FROM B)` | Each branch SELECTs `NULL::TEXT` for vars it doesn't bind |
| `BIND(expr AS ?v)` downstream | AST substitution: `?v` rewritten to `expr` in every later FILTER / triple slot / chained BIND **before** the structural walk | Phase F group F2: no new translator surface — `FILTER(?v>10)` with `BIND(?a+?b AS ?v)` becomes `FILTER(?a+?b>10)` and the existing anchors path resolves it; unbound-var BIND → `NULL::TEXT` (not an error, W3C §18.2.5); projection still emits the bind column (no v0.3 regression) |
| Aggregate over UNION | `SELECT <agg(qU.vK)> FROM ((<branch1 dict-id projection>) UNION ALL (<branch2 …>)) qU [GROUP BY …] [HAVING …]` | Phase F group F2: each branch sub-SELECTs the agg/GROUP-BY vars' **dict ids** into the F1 `vK` pool; the EXISTING `translate_aggregate` runs over `qU` unchanged (COUNT/SUM/AVG/type-aware MIN-MAX/GROUP_CONCAT/SAMPLE, DISTINCT, GROUP BY, HAVING); group-by on a GRAPH-scope-only var (genuinely-mixed degenerate) → stable panic (LLD v0.5 §8), never a wrong count |
| MINUS { triple } | `WHERE NOT EXISTS (SELECT 1 FROM _pgrdf_quads qMIN_K WHERE …)` | Elided at translation time when there are no shared variables (SPARQL no-op) |
| `GRAPH <iri> { … }` | `qN.graph_id = <resolved>` on every triple alias inside the block | IRI resolved against `_pgrdf_graphs.iri` at translate time; unresolved IRI binds to `-1` (zero rows, spec-correct "no solutions") |
| `GRAPH ?g { … }` | `INNER JOIN _pgrdf_graphs g{S} ON g{S}.graph_id = q{first}.graph_id` + `qN.graph_id = q{first}.graph_id` for non-anchor triples | One JOIN per Variable scope; ?g projects as `g{S}.iri` (the IRI string); INNER matches W3C §13.3 — only mapped graphs bind ?g; multi-triple inner BGPs share the anchor's graph_id so triples can't stitch across graphs |
| GRAPH composition (slice 112) | Per-pattern `Option<GraphScope>`; GRAPH inside OPTIONAL/UNION/MINUS scopes only its contained triples, OPTIONAL/MINUS inside GRAPH inherits the outer scope | Mandatory Variable scopes → INNER JOIN to `_pgrdf_graphs`; OPTIONAL-born scopes → LEFT JOIN so unmatched OPTIONALs still NULL out `?g` instead of dropping outer rows; MINUS scopes stay internal to the NOT EXISTS subquery |
| DISTINCT / REDUCED | `SELECT DISTINCT …` | REDUCED → DISTINCT (safe over-approximation per spec) |
| ORDER BY ?v / expr (type-aware, §15.1) | `ORDER BY <kind-rank>, <numeric>, <timestamp>, <bool-rank>, <text COLLATE "C">` (each ASC/DESC NULLS LAST) over the underlying SQL expr | Phase F group F4: SPARQL 1.1 §15.1 value-space order — numerics numerically (`2<10`), `xsd:dateTime` chronologically, `xsd:boolean` false<true, strings by codepoint; total/stable, never raises (regex-guarded casts fall through to the codepoint tier). `DESC()` + multi-key + expression keys (`ORDER BY STRLEN(?s)`, via the BIND/FILTER translator). The four builders order over the group/aggregate/dict-lookup/BIND expr, never an output alias; `SELECT DISTINCT` + ORDER BY wraps the dedup in an outer derived table |
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

## Property paths (LLD v0.4 §7 — Phase E, fully shipped E1 → E4)

SPARQL property paths arrive in the spargebra algebra as
`GraphPattern::Path { subject, path, object }`. The shared WHERE
walker (`walk_select_scoped` / `walk_branch`) recognises `Path` at
the single chokepoint every query form routes through — SELECT, ASK,
`pgrdf.construct`, and the UPDATE WHERE bodies all inherit path
support at once (it is not special-cased per consumer).
`query::path::scoped_triple_from_path` classifies the operator: the
E1 non-recursive set lowers to an ordinary triple; the `+` / `*` /
`?` / `|` sets lower to a derived FROM relation that exposes the
same `subject_id` / `object_id` columns a quad alias does (`+` is
the recursive walk; `*` is that walk `UNION` the zero-length
node-set; `?` is the direct edge `UNION` the same zero-length set,
non-recursive; `|` is the non-reflexive single step over a
predicate **set**, no recursion). Every recursive/optional/
alternation builder centralises the predicate match as
`predicate_id IN (…)` — a 1-element set is identical to `= $P`, so
`|` (and its `(a|b)+`/`(a|b)*`/`(a|b)?` recursion compositions) is
just a wider set, the LLD §7.2 "union of per-predicate scans" done
as one scan. Either way the result flows through the existing
`pattern_clauses` / var-binder machinery — so paths compose for
free with named-graph scoping, multi-pattern BGP joins, and
OPTIONAL/UNION/MINUS.

| Operator | SPARQL | Semantics | Status |
|---|---|---|---|
| bare predicate | `?s p ?o` (as a `Path`) | direct triple | ✅ E1 — lowers to `?s p ?o` |
| `^` inverse | `?s ^p ?o` | `?o p ?s` | ✅ E1 — subject/object swap, no recursion; `^(^p)` folds by parity |
| `+` one-or-more | `?s p+ ?o` | transitive closure (non-reflexive) | ✅ E2 — `WITH RECURSIVE` CTE; cycle-safe `CYCLE`-clause dedup; `^p+`/`(^p)+` inverse-composition; depth guard enforced |
| `*` zero-or-more | `?s p* ?o` | reflexive transitive closure | ✅ E3 — the `+` cycle-safe recursive walk `UNION` the W3C §9.3 zero-length node-set; reuses E2's `CYCLE` termination + depth guard + truncation probe; `^(p*)`/`(^p)*` inverse-composition |
| `?` zero-or-one | `?s p? ?o` | equal-or-linked | ✅ E3 — non-recursive: the direct edge `UNION` the SAME W3C §9.3 zero-length node-set; no depth guard; `^(p?)`/`(^p)?` inverse-composition |
| `\|` alternation | `?s (a\|b) ?o` | per-predicate union (non-reflexive single step) | ✅ E4 — `predicate_id IN (…)`; n-ary `a\|b\|c`; the recursion compositions `(a\|b)+`/`(a\|b)*`/`(a\|b)?`; inverse `^(a\|b)`/`(^a\|^b)` |
| `!(...)` negated set | `?s !(p) ?o` | — | out of v0.4 scope (panics) |
| sequence `p1/p2` | `?s p1/p2 ?o` | — | use a multi-pattern BGP (`{ ?s p1 ?m . ?m p2 ?o }`); E1 rejects an explicit `Sequence` path-expr with a pointer to the BGP form |

The only remaining **preview-panic** is the §7.1-permitted **gated
remainder**: an alternation arm that is itself a
sequence/recursive/nested-recursive path (`(a/b|c)`, `(a+|b)`), or
a recursive operator whose inner box is a sequence (`(p1/p2)+`).
Folding these would compose a recursive CTE inside an alternation
arm — the translator balloon LLD §7.1 explicitly permits gating.
They panic with the stable nested-recursive prefix; negated sets
panic with the out-of-scope message. Substring-match the prefix;
any slice-number tail is advisory. `sparql_parse` does NOT panic on
these — it lowers the full executable set (E1 ∪ `+` ∪ `*`/`?` ∪
`|`) into the `bgp` shape and flags only the gated remainder in
`unsupported_algebra` (parse-time analysis, mirroring how Phase C
reports not-yet-shipped UPDATE forms).

**`+` chain example.** Over a `subClassOf`-style chain
`c1 → c2 → … → c11`:

```sparql
PREFIX ex: <http://example.org/>
SELECT ?x WHERE { ?x ex:sub+ ex:c11 }    -- → c1 … c10 (10 ancestors, non-reflexive)
```

`+` is the strict transitive closure: a node is **not** its own
ancestor (that is `*`, group E3). Cycles are safe — the recursive
CTE uses Postgres's `CYCLE src, dst SET is_cycle USING path` clause
(PG14+), which stops extending a path the moment a `(src,dst)` pair
repeats on it, so a cyclic graph terminates after one lap (a bare
`UNION` can't do this once the working tuple carries `depth` for the
guard). `^ex:sub+` / `(^ex:sub)+` walk the inverse edge (the inverse
of a transitive closure equals the transitive closure of the
inverse). A `p+` pattern joins to ordinary triple
patterns, GRAPH scoping, and `pgrdf.construct` exactly like a plain
triple.

**`*` / `?` and W3C §9.3 zero-length-path semantics (E3).** `*` is
the **reflexive** transitive closure — the `+` walk **plus** the
zero-length ("identity") pairs; `?` is the single direct edge plus
the same identity pairs (no recursion). Over the same chain:

```sparql
PREFIX ex: <http://example.org/>
SELECT ?x WHERE { ?x ex:sub* ex:c11 }   -- → c1 … c10 AND c11 itself (11 — reflexive)
SELECT ?o WHERE { ex:c1 ex:sub? ?o }    -- → c1 (identity) AND c2 (direct) only
```

The identity ("zero-length") pair-set the LLD §7.2 `SELECT ?s ?s`
sketch alludes to is **refined to the precise W3C SPARQL 1.1 §9.3
rules** (exactly as E2 refined §7.2's bare-`UNION` to the `CYCLE`
clause). Which `(n,n)` pairs an endpoint contributes depends on
whether that endpoint is **bound** (an IRI) or **unbound** (a var):

| Pattern | Zero-length contribution |
|---|---|
| `<x> p* ?o` (subject bound) | `{(x,x)}` **unconditionally** — even if `<x>` is in no graph — plus `{(x,o) : x p+ o}` |
| `?s p* <y>` (object bound) | symmetric: `{(y,y)}` unconditionally plus `{(s,y) : s p+ y}` |
| `<x> p* <y>` (both bound) | true iff `x == y` **or** `x p+ y` |
| `?s p* ?o` (both var) | `{(n,n)}` for every node `n` of the active scope (subject∪object position) plus `{(s,o) : s p+ o}` |
| `?s p? ?o` | same identity set, but the non-identity part is the single direct `p` edge (no recursion) |

A **bound** endpoint's self-pair holds even when the IRI is not a
term in the data — pgRDF registers the queried IRI as an RDF term
(a term reference; **no quad is added**, the graph data is
unchanged) so the opposite projected variable can resolve it
(`<lone> p* ?o` → `?o = <lone>`, 1 solution; `+` stays pure-lookup
since it has no zero-length set). An **unbound** endpoint's
node-set is the DISTINCT subject∪object of the active scope; under
`GRAPH <iri>` / `GRAPH ?g` it is **scoped to that graph's nodes**
(and is predicate-agnostic — the named graph's full term set, a
node only in another graph is NOT in the scoped identity set). `*`
inherits E2's cycle-safety and depth guard for its `+` part (the
zero-length part is a single non-recursive scan and cannot
truncate); `?` is fully non-recursive (no depth guard).

**`pgrdf.path_max_depth` GUC + depth guard.** Integer,
`GucContext::Userset`, default **64**, range **1..1024**, registered
in `_PG_init` (`query::guc`). Bounds the recursive-path walk depth.
**Enforced from E2:** the `+` CTE's recursive arm carries
`WHERE w.depth < pgrdf.path_max_depth` (read at translate time —
re-`SET`ting it mints a distinct cached plan, so a changed cap takes
effect on the next query). A query whose traversal would go beyond
the cap returns the **truncated** solution set (it does **not**
error), and `pgrdf.stats()->>'path_depth_truncations'` increments:

```sparql
SET pgrdf.path_max_depth = 3;
PREFIX ex: <http://example.org/>
SELECT ?o WHERE { ex:c1 ex:sub+ ?o }     -- → c2,c3,c4 only (truncated)
-- SELECT pgrdf.stats()->>'path_depth_truncations'  → > 0
```

`path_depth_truncations` is a cross-backend shmem counter zeroed by
`pgrdf.shmem_reset()`. The truncation detector never under-counts (a
traversal that completes under the cap leaves the counter at 0; any
path the guard actually cut bumps it); it may benignly over-count
when the cut node was already reached by a shorter path (LLD v0.4
§7.2 explicitly permits this).

**`pgrdf.on_path_truncation` GUC — fail-closed truncation (#14).**
String, `GucContext::Userset`, one of `'count' | 'warn' | 'error'`,
default **`warn`**. The counter above is cumulative and
cross-backend, so on its own a truncated walk is invisible to the
caller that ran it; this GUC upgrades the per-query signal:

* `count` — bump the counter only (the pre-#14 silent behaviour).
* `warn` (default) — counter **plus a client-visible `WARNING`** per
  truncated walk: a partial result is never silent.
* `error` — **fail the query** (stable prefix `sparql: property path
  truncated at pgrdf.path_max_depth=N`) instead of returning a
  depth-truncated result. The fail-closed mode for closure queries —
  e.g. an un-materialised `rdfs:subClassOf*` type-closure walk whose
  under-collection would silently propagate into a carve slice.

An unrecognised value warns and behaves as `warn` (a typo never
*loosens* the policy to silent). Companion carve-side report: the
neighbourhood `carve_graph` counts the **boundary** (distinct nodes
one edge beyond the `max_hops` rim that the cap kept out of the
slice) in the same statement and reports a non-zero boundary via
`NOTICE` — `max_hops` legitimately *defines* the slice, so a
continuing neighbourhood is a report, not an error. Regression
`140-truncation-fail-closed.sql` locks all four behaviours.

**`|` alternation (E4).** `?s (a|b) ?o` is the union of the
per-predicate scans — equivalently a single scan over the predicate
**set** (`predicate_id IN (a, b)`). It is a **non-reflexive single
step** (not a closure — no recursion, no zero-length identity set):

```sparql
PREFIX ex: <http://example.org/>
SELECT ?c ?who WHERE { ?c (ex:parent|ex:guardian) ?who }   -- parent ∪ guardian edges
```

The n-ary form `a|b|c` flattens to the full set; `^(a|b)` /
`(^a|^b)` fold the inverse into the same swapped-edge flag; and the
recursion compositions `(a|b)+` / `(a|b)*` / `(a|b)?` make the
alternation the recursive step's predicate set (the depth guard,
the `CYCLE` clause, the truncation probe, and the zero-length
node-set are all predicate-set-agnostic, so they are reused
verbatim). The only gated case is an alternation whose **arm** is
itself a sequence/recursive path (`(a/b|c)`) — see the gated
remainder above.

**Materialised-closure no-CTE fallback (E4, LLD v0.4 §7.2 / §7.3).**
When `pgrdf.materialize(graph_id)` has already entailed the
transitive closure of a path's predicate, a recursive CTE is wasted
work — every transitive pair is already a direct `is_inferred =
TRUE` edge. For a `+`/`*` over a **single** predicate that is one of
the well-known transitive predicates (`rdfs:subClassOf`,
`rdfs:subPropertyOf`, `owl:sameAs`), the translator probes
`EXISTS(… WHERE predicate_id = $P AND is_inferred AND <scope>)`; if
a materialised row is present it emits a **direct match instead of
the recursive CTE** — `+` becomes the non-reflexive single step,
`*` becomes that step `UNION` the W3C §9.3 zero-length set (= the
`?` relation; with the closure materialised, direct ∪ identity is
the full `*` solution set). The executed plan therefore carries no
`CTE Scan` (§7.3 acceptance, scraped via `EXPLAIN (FORMAT JSON)`).
The result set is byte-identical to the non-materialised recursive
walk — the optimisation is semantics-preserving. Detection is
per-query, not cached; `?`/`^`/`|` are unaffected (no recursion to
elide); a multi-predicate `(a|b)+` skips the fallback (the
heuristic is single-well-known-predicate only). The
`pgrdf.sparql_sql(q TEXT) → TEXT` debug hook returns the translated
SQL (dict ids inlined) so a regression can EXPLAIN-scrape it.

Implementation:
[`src/query/path.rs`](../src/query/path.rs) (classifier +
recursive-CTE builder + predicate-set generalisation + the
alternation relation builder + truncation probe — the executor only
calls into it), [`src/query/executor.rs`](../src/query/executor.rs)
(`scoped_triple_from_path` wiring + the live-dictionary
materialised-closure probe + `pgrdf.sparql_sql`),
[`src/query/guc.rs`](../src/query/guc.rs); regression coverage:
[`108-property-path-inverse.sql`](../tests/regression/sql/108-property-path-inverse.sql)
+ [`109-property-path-plus.sql`](../tests/regression/sql/109-property-path-plus.sql)
+ [`110-property-path-star-opt.sql`](../tests/regression/sql/110-property-path-star-opt.sql)
+ [`111-property-path-materialised-closure.sql`](../tests/regression/sql/111-property-path-materialised-closure.sql);
W3C-shape fixtures `36-path-inverse` … `41-path-materialised`.

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

## Surface today (complete in v0.5.0 — §3.3 GRAPH, §4 UPDATE, §6 CONSTRUCT, §7 property paths, §11 backlog incl. DESCRIBE / type-aware ORDER BY / multi-triple OPTIONAL / VALUES / downstream BIND / aggregates-over-UNION all shipped)

- ✅ Basic Graph Patterns (1..N triples)
- ✅ `SELECT` (explicit projection or `SELECT *`); `ASK`
- ✅ FILTER — identity, boolean, term-type, BOUND, numeric
      ordering, REGEX, IN, STR, LANG, DATATYPE, UCASE, LCASE,
      STRLEN, CONTAINS, STRSTARTS, STRENDS, arithmetic
- ✅ Solution modifiers — DISTINCT, REDUCED, LIMIT, OFFSET, ORDER BY
- ✅ OPTIONAL — single AND multi-triple groups, chained, nested
      (Phase F group F1). The whole N-triple right side emits as a
      `LEFT JOIN LATERAL (SELECT …) qOPT ON TRUE` so the group binds
      **atomically** (all-or-nothing, W3C §6.1): either every inner
      variable binds or every one comes back NULL. OPTIONAL-internal
      FILTER, the `OPTIONAL { … } FILTER(…)` join-FILTER, the
      optional-var outer FILTER, GRAPH scoping, and a `+`-path in
      the required part all compose; inherited by `pgrdf.construct`
      and SPARQL UPDATE WHERE.

      ```sql
      -- 2-triple OPTIONAL: alice/carol (name+age) bind both;
      -- bob (name, no age) → BOTH ?n and ?ag NULL (atomic).
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?s ?n ?ag
           WHERE { ?s a ex:Person
                   OPTIONAL { ?s ex:name ?n . ?s ex:age ?ag } }');

      -- Nested OPTIONAL — the inner optional binds ?ag
      -- independently of the outer ?n.
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?s ?n ?ag
           WHERE { ?s a ex:Person
                   OPTIONAL { ?s ex:name ?n
                              OPTIONAL { ?s ex:age ?ag } } }');
      ```
- ✅ `VALUES` inline tables (Phase F group F1) — top-level or joined
      alongside a BGP. Translates to a `(VALUES (id,…),(id,…)) AS
      vN(cols)` derived table joined on the shared variables;
      constants resolve to dictionary ids ahead of execution;
      `UNDEF` is a NULL cell that places **no** constraint on that
      variable for that row (W3C §10); typed/lang literals match
      datatype-aware. Composes with GRAPH scoping + OPTIONAL;
      inherited by `pgrdf.construct` and SPARQL UPDATE WHERE.

      ```sql
      -- Only the listed subjects that also have an ex:p.
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?x ?y WHERE { ?x ex:p ?y }
         VALUES (?x) { (ex:a) (ex:c) }');

      -- UNDEF places no constraint on ?y for that row.
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?x ?y WHERE { ?x ex:p ?y }
         VALUES (?x ?y) { (ex:a UNDEF) (ex:b 2) }');
      ```
- ✅ UNION (n-way; per-branch FILTERs / OPTIONALs / MINUSes)
- ✅ MINUS — single AND multi-triple sub-pattern, shared-var keyed
- ✅ Aggregates — `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT ?v)`,
      `SUM`, `AVG`, type-aware `MIN` / `MAX` (numeric path on
      `xsd:numeric`, lex fallback), `GROUP_CONCAT`, `SAMPLE` with
      `GROUP BY` — over a single BGP **or over a UNION** (Phase F
      group F2; see "Aggregates over UNION" below)
- ✅ `HAVING` — both by aggregate alias (`HAVING(?total > c)`)
      AND inline (`HAVING(SUM(?v) > c)`); also over an
      aggregate-of-UNION
- ✅ `BIND(expr AS ?v)` — projection (Literal / NamedNode /
      Variable, STR / LANG / DATATYPE / UCASE / LCASE / STRLEN,
      arithmetic, CONCAT) **and downstream** (Phase F group F2; the
      v0.3 projection-only limitation is lifted — see "Downstream
      BIND" below)
- ✅ Downstream `BIND` (Phase F group F2) — a `BIND`-introduced
      variable is usable in a textually-later FILTER, a later
      triple's variable position (BGP join key), and a chained BIND
      (resolved left-to-right). Realised as an AST substitution pass
      that rewrites the bind var to its expression **before** the
      structural walk, so the existing anchors-driven translator
      resolves it with no new surface. A BIND over an unbound
      variable yields an UNBOUND result (NULL), NOT a query error
      (W3C §18.2.5). Composes with GRAPH scoping + F1 OPTIONAL/VALUES;
      inherited by `pgrdf.construct` and SPARQL UPDATE WHERE.

      ```sql
      -- BIND value reused in a later FILTER (was projection-only).
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?s ?sum
           WHERE { ?s ex:x ?x . ?s ex:y ?y
                   BIND(?x + ?y AS ?sum)
                   FILTER(?sum > 10) }');

      -- Chained BIND: ?c = (?x + 1) * 2, resolved left-to-right.
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?s ?c
           WHERE { ?s ex:x ?x
                   BIND(?x + 1 AS ?b) BIND(?b * 2 AS ?c) }');
      ```
- ✅ Aggregates over UNION (Phase F group F2) — the UNION becomes a
      derived table whose branches each project the aggregate /
      GROUP BY variables' dict ids into the F1 `vK` column pool; the
      existing aggregate translator runs over `(<union>) qU`
      unchanged. COUNT/SUM/AVG/type-aware MIN-MAX/GROUP_CONCAT/SAMPLE,
      `DISTINCT`, `GROUP BY`, `HAVING`, GRAPH scoping and a
      property-path branch all compose; inherited by
      `pgrdf.construct`. A GROUP BY (or aggregate argument) on a
      variable that is ONLY ever a `GRAPH ?g`-scope var across the
      union is the genuinely-mixed degenerate of
      `SPEC.pgRDF.LLD.v0.5 §8` (the six §8 residuals proper are
      shipped in v0.5.0) and surfaces a stable panic (never a
      wrong count).

      ```sql
      -- COUNT over a 2-branch UNION.
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT (COUNT(?p) AS ?n) WHERE {
           { ?x ex:cat "books" . ?x ex:price ?p }
           UNION
           { ?x ex:cat "tools" . ?x ex:price ?p } }');

      -- SUM over a UNION with GROUP BY a union variable + HAVING.
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?c (SUM(?p) AS ?s) WHERE {
           { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "books") }
           UNION
           { ?x ex:cat ?c . ?x ex:price ?p FILTER(?c = "tools") } }
         GROUP BY ?c HAVING(SUM(?p) > 20)');
      ```
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
      multi-triple, idempotent on repeat via `WHERE NOT EXISTS`).
      The pattern-driven UPDATE forms shipped in slices 82 (INSERT
      WHERE), 81 (DELETE WHERE), and 80 (combined DELETE+INSERT
      WHERE); the lifecycle algebra (`DROP / CLEAR / CREATE GRAPH`)
      shipped in slice 78. LOAD remains out of scope (LLD v0.4 §14).
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
- ✅ SPARQL UPDATE — lifecycle algebra (`DROP / CLEAR / CREATE GRAPH`,
      `DEFAULT / NAMED / ALL` qualifiers) (Phase C slice 78, LLD v0.4
      §4.4). Closes the SPARQL UPDATE ↔ §5 lifecycle-UDF lattice.
      Spargebra-0.4.6 models the three lifecycle forms as the
      `Clear { graph: GraphTarget, silent }`, `Create { graph:
      NamedNode, silent }`, and `Drop { graph: GraphTarget, silent }`
      variants of `GraphUpdateOperation`; the dispatcher in
      `src/query/executor.rs::execute_update` routes through
      `pgrdf.drop_graph(id, true)`, `pgrdf.clear_graph(id)`, and
      `pgrdf.add_graph(iri TEXT)` (the §5 UDFs shipped at slices
      99 / 98 / 118). SQL strings (not Rust direct) — the SPARQL
      front-end and the SQL UDF front-end remain two consumers of
      the same partition-level primitives; every existence check,
      partition-DDL window, and `_pgrdf_graphs` binding update
      happens once in the UDFs. `GraphTarget::NamedNode(iri)`
      panics on not-bound unless `SILENT`; `GraphTarget::DefaultGraph`
      routes to a direct `DELETE FROM _pgrdf_quads WHERE graph_id = 0`
      for BOTH `CLEAR DEFAULT` and `DROP DEFAULT` (W3C §3.1.3
      paragraph 7 "DROP DEFAULT empties, not destroys"; routine
      default-graph inserts land in `_pgrdf_quads_default` rather
      than `_pgrdf_quads_g0`, so `pgrdf.clear_graph(0)` would miss
      them — the partition-wide DELETE catches both via Postgres
      partition routing; `pgrdf.drop_graph(0)` panics by design);
      `GraphTarget::AllGraphs` iterates every `_pgrdf_graphs` row
      INCLUDING `graph_id = 0`; `GraphTarget::NamedGraphs` excludes
      `graph_id = 0`. `CREATE GRAPH <iri>` on an already-bound IRI
      panics with `CREATE GRAPH <iri>: graph already exists` unless
      `SILENT` (W3C §3.1.3 paragraph 12). CREATE never touches row
      counts (`triples_inserted = 0`). The `_update` summary's
      `form` field reports `"CLEAR"` / `"CREATE"` / `"DROP"` for the
      single-op forms; multi-op Updates collapse to `"MIXED"` via
      the existing `form != op_name` rule. `ADD / MOVE / COPY` are
      not separate enum variants — they desugar at parse time
      (spargebra parser.rs §Add / §Move / §Copy) into compositions
      of `Drop + DeleteInsert` (or just `DeleteInsert` for ADD),
      so they ride the existing per-form dispatcher arms.
- ✅ `CONSTRUCT` — full surface (Phase D slices 59 → 52: constant /
      variable / blank-node / multi-triple templates, GRAPH-scoped
      WHERE, WHERE shorthand, round-trip ingest, `sparql_parse`
      enrichment; LLD v0.4 §6) —
      sibling UDF `pgrdf.construct(q TEXT) → SETOF JSONB`. Each row
      carries `{"subject": …, "predicate": …, "object": …}` with
      structured term cells `{"type": "iri"|"literal"|"bnode",
      "value": …, "datatype"?: …, "language"?: …}` per LLD v0.4 §6.1.
      Templates accept constants, variables, AND blank-node labels
      (`_:label`) in subject / object positions; blank nodes in
      predicate position are illegal RDF (spargebra rejects at parse
      time). Per-solution substitution resolves each variable's
      dict id through the dictionary into the same structured shape.
      Blank-node template positions mint a FRESH label per (solution,
      template-label) pair per W3C SPARQL 1.1 §16.2 ("any blank
      nodes in the template are replaced with new blank nodes" — one
      fresh label per solution). The same template label appearing
      in multiple positions of one triple within one solution
      resolves to the SAME fresh label (within-solution sameness);
      across solutions, fresh labels differ. **Multi-triple templates
      (slice 56)** widen the surface to N-triple templates: an
      N-triple `{ … . … . … }` template emits N rows per solution,
      and the same template blank-node label is SHARED across all N
      template triples WITHIN the same solution. So
      `CONSTRUCT { _:r <ex:type> <ex:Card> . _:r <ex:value> ?v .
      <ex:owner> <ex:owns> _:r }` emits three rows per solution, with
      `_:r` resolving to the SAME fresh label in subject of the type
      triple, subject of the value triple, AND object of the owns
      triple of that solution. Across solutions, `_:r` mints a NEW
      fresh label. Two distinct template labels `_:a` vs `_:b`
      within the same solution mint TWO DIFFERENT fresh labels —
      slice 56 does not conflate distinct labels. Empty templates
      `CONSTRUCT { } WHERE { … }` reject with
      `pgrdf.construct: empty template`. Variable-bound blank nodes
      from the WHERE pattern (the dictionary stored a bnode via
      Turtle ingest) pass through with the dictionary-stored label
      unchanged. Typed and language-tagged literal bindings flow
      through with full datatype IRI / `language` field preservation
      (`rdf:langString` for tagged literals per RDF 1.1 §3.3). The
      WHERE pattern accepts the full SELECT-side BGP / FILTER /
      OPTIONAL / UNION / MINUS surface (translation reuses
      `parse_select` + `build_from_and_where`). Variables that the
      WHERE pattern does not bind panic with
      `pgrdf.construct: unbound template variable ?X`. **GRAPH-scoped
      WHERE (slice 55)** widens to `GRAPH <iri> { … }` and
      `GRAPH ?g { … }` inside the WHERE block. The literal form
      filters solutions to a single named graph; the variable form
      binds `?g` per-solution to the source graph IRI (`g{S}.iri`
      from the `_pgrdf_graphs` join, projected as TEXT and shaped as
      an `iri` term — graph IRIs are NOT entered in
      `_pgrdf_dictionary`, so the construct path now projects the
      IRI text directly instead of round-tripping through a scalar
      subselect that would always return NULL for named-graph rows).
      `GRAPH ?g` ranges over named graphs only per W3C SPARQL 1.1
      §13.3 — default-graph quads never bind `?g` (the
      `_pgrdf_graphs` JOIN carries `AND g{S}.graph_id <> 0`,
      which also corrected the slice-79 / slice-87 SELECT path's
      latent default-graph bleed). All prior template surfaces
      compose: variable substitution, blank-node label sharing,
      multi-triple emission, constant constants. Empty named graphs
      and missing graphs yield zero solutions. **CONSTRUCT WHERE
      shorthand (slice 54)** admits the `CONSTRUCT WHERE { pattern }`
      form per W3C SPARQL 1.1 §16.2.4 — equivalent to `CONSTRUCT
      { pattern } WHERE { pattern }`. spargebra populates the AST's
      `template` field from the pattern's BGP at parse time, so the
      shorthand flows through the same multi-triple emission path as
      the explicit form; slice 54 reduces to (1) detecting the
      shorthand syntactically via an ASCII probe of the input query
      string (the post-parse AST is otherwise indistinguishable from
      the explicit form), and (2) enforcing the two W3C
      restrictions. The pattern must be a pure basic graph pattern
      — composites (OPTIONAL / UNION / MINUS / FILTER / GRAPH / BIND
      / VALUES) inside `CONSTRUCT WHERE { … }` are rejected at parse
      time by spargebra's grammar with `pgrdf.construct: parse
      error: …`. The pattern must contain no blank nodes — spargebra
      admits them at parse, so slice 54 enforces this rule
      semantically with `pgrdf.construct: WHERE-shorthand prohibits
      blank nodes in the pattern (W3C SPARQL 1.1 §16.2.4)`. The
      explicit `CONSTRUCT { } WHERE { … }` empty-template form
      continues to reject with `pgrdf.construct: empty template`
      (slice-56 contract preserved — the shorthand detection branch
      does not swallow the explicit-empty case). **Round-trip
      ingest (slice 53)** lands the pairing
      `pgrdf.put_construct_row(row JSONB, graph_id BIGINT DEFAULT 0)`
      and `pgrdf.put_construct_rows(rows JSONB[], graph_id BIGINT
      DEFAULT 0)`: any rowset emitted by `pgrdf.construct(q)` can be
      re-ingested to reproduce the original graph state per LLD v0.4
      §6.3 (modulo dict id reshuffles). The plural form is the
      recommended surface — it maintains a per-call
      `HashMap<String, i64>` of blank-node labels so repeated bnode
      references within one batch resolve to a single stored blank
      node, preserving the slice 56 / 57 within-solution joining
      across round-trip. Typed literals (`xsd:integer`,
      `xsd:dateTime`, …) round-trip with their datatype IRI verbatim;
      language-tagged literals carry both the `language` field and
      the implicit `rdf:langString` datatype per RDF 1.1 §3.3; plain
      strings carry the explicit `xsd:string` datatype that the
      construct emitter writes (slice 59 contract). Re-ingestion is
      idempotent via `WHERE NOT EXISTS` (set semantics matching
      `executor::insert_quad`), and a NULL input array (from
      `array_agg` over a zero-row construct) is a no-op. Literals in
      subject/predicate position panic with the stable
      `pgrdf.put_construct_row:` prefix. The canonical idiom is:
      ```sql
      SELECT pgrdf.put_construct_rows(
        (SELECT array_agg(j) FROM pgrdf.construct(
          'CONSTRUCT { ?s ?p ?o } '
          'WHERE { GRAPH <urn:src> { ?s ?p ?o } }') AS t(j)),
        dst_graph_id);
      ```
      **`sparql_parse` CONSTRUCT enrichment (slice 52)** — the
      previous placeholder `{form: "CONSTRUCT", supported: false,
      reason: "…"}` is gone. `pgrdf.sparql_parse` now returns the
      structured shape:
      ```json
      {
        "form": "CONSTRUCT",
        "template": {
          "triple_count": 1,
          "has_variables": true,
          "has_blank_nodes": false,
          "has_constants_only": false,
          "variables": ["?o", "?s"]
        },
        "where_shape": {
          "kind": "Bgp",
          "triple_count": 1,
          "named_graphs_used": [],
          "variables": ["?o", "?p", "?s"]
        },
        "shorthand": false,
        "unsupported_algebra": []
      }
      ```
      `template.has_constants_only` is true iff the template has no
      variables and no blank nodes (the all-constants case the
      slice-59 foundation supported). `where_shape.kind` is the
      W3C-facing name of the immediate top-level WHERE pattern
      variant (`Bgp` / `Optional` / `Union` / `Minus` / `Graph` /
      `Filter` / `Bind` / `Values` / `Group` / `OrderBy` /
      `Distinct` / `Service`); trivial outer `Project` / `Slice`
      wrappers that spargebra adds for the implicit all-vars
      projection are peeled before reporting. `triple_count` sums
      BGP triples recursively across composite shapes;
      `named_graphs_used` lists literal IRIs and `?var` sentinels
      under any GRAPH scope; `variables` is the distinct sorted
      list of variables in the WHERE pattern.
      `shorthand` is true when the input is in W3C SPARQL 1.1 §16.2.4
      form (`CONSTRUCT WHERE { ... }`), detected with the same ASCII
      probe `pgrdf.construct` uses. `unsupported_algebra` flags
      `Distinct` / `OrderBy` / `Group` / `Aggregate` modifiers that
      `pgrdf.construct` will panic on at execute time per LLD §6.2 —
      surfaced ahead of execution so callers can route on the JSONB
      shape alone.
      DISTINCT / ORDER BY / GROUP BY / aggregates on CONSTRUCT are
      explicitly out of scope per W3C 1.1 §16.2 — rejected with
      `pgrdf.construct: DISTINCT / ORDER BY / GROUP BY / aggregates
      not supported (W3C 1.1 §16.2)`.
- ✅ `DESCRIBE` — full surface (Phase F group F3, slices 26-24; LLD
      v0.4 §11) — sibling UDF `pgrdf.describe(q TEXT) → SETOF JSONB`,
      parallel to `pgrdf.construct` and **byte-identical** in row
      shape (`{"subject": …, "predicate": …, "object": …}` with the
      same `{"type": …, "value": …, "datatype"?: …, "language"?: …}`
      structured term cells — the same encoders, no new shaper). The
      caller signals intent at the SQL boundary (the §6.1 sibling-UDF
      rationale): a DESCRIBE through `pgrdf.sparql` panics
      `sparql: use pgrdf.describe(q) for DESCRIBE queries` (mirrors
      how `pgrdf.construct` is the CONSTRUCT entry point);
      `pgrdf.describe` on a non-DESCRIBE query panics
      `pgrdf.describe: not a DESCRIBE query`.

      DESCRIBE is **not** a CONSTRUCT template — there is no
      `{ template }`. The "description" is the **closure** of each
      described resource: for resource R (an IRI or blank node — a
      literal can't be a subject so it yields an empty description),
      every triple `(R, ?p, ?o)`, and whenever an emitted object
      `?o` is a blank node, the closure recurses into that blank
      node's triples and keeps following while the frontier object
      stays a blank node ("transitively expanded one hop on blank
      nodes" per W3C §16.4). Recursion only ever traverses
      blank-node objects (IRI / literal objects are leaves), so it
      terminates on any finite graph; a visited-set of blank-node
      ids additionally makes blank-node cycles
      (`_:b1 ex:p _:b2 . _:b2 ex:p _:b1`) terminate. Triples are
      deduplicated across the whole result (set semantics — a
      resource described twice emits its closure once; overlapping
      closures emit each triple once).

      Supported forms (spargebra normalises all of them to
      `Project { inner, variables }` where each constant
      `DESCRIBE <iri>` is a leading `Extend { …, NamedNode(iri) }`
      layer over the residual WHERE, which the executor peels):

      ```sql
      -- Constant, no WHERE — every (iri, ?p, ?o); empty IRI → 0 rows
      SELECT * FROM pgrdf.describe('DESCRIBE <http://example.com/a>');

      -- Variable form — union of the closures of every ?x binding
      SELECT * FROM pgrdf.describe(
        'PREFIX ex: <http://example.com/>
         DESCRIBE ?x WHERE { ?x a ex:Thing }');

      -- Mixed constant + variable terms
      SELECT * FROM pgrdf.describe(
        'PREFIX ex: <http://example.com/>
         DESCRIBE <http://example.com/b> ?x WHERE { ?x a ex:Thing }');

      -- DESCRIBE * — every projected variable binding
      SELECT * FROM pgrdf.describe(
        'PREFIX ex: <http://example.com/>
         DESCRIBE * WHERE { <http://example.com/a> ex:knows ?x }');

      -- Blank-node closure: <r> ex:p _:b1 ; _:b1 ex:q _:b2 ;
      -- _:b2 ex:r "leaf" → DESCRIBE <r> returns all 3 triples
      -- (follows the bnode chain to the literal leaf)
      SELECT * FROM pgrdf.describe('DESCRIBE <http://example.com/r>');

      -- GRAPH-scoped: the closure is computed within the named
      -- graph; other graphs' triples about <a> are excluded
      SELECT * FROM pgrdf.describe(
        'DESCRIBE <http://example.com/a>
           WHERE { GRAPH <http://example.com/g1> { ?s ?p ?o } }');
      ```

      An unscoped DESCRIBE scans every graph (the slice-112 pgRDF
      unscoped-BGP semantic). `pgrdf.sparql_parse` reports
      `form:"DESCRIBE"` with a `describe` block
      (`kind` ∈ `constant`/`variable`/`mixed`, `constant_iris`,
      `variable_terms`, `has_where`) and a `where_shape` over the
      residual WHERE; DESCRIBE is NOT flagged in
      `unsupported_algebra` (the LLD §11 acceptance binding;
      `80-unsupported-shapes` gap-6 retired in the same commit).
      Regression-locked: `tests/regression/sql/116-describe.sql`.
- ✅ Property paths (`^`, `+`, `*`, `?`, `\|` incl. `(a\|b)+`/`(a\|b)*`/`(a\|b)?`/`^(a\|b)`) + materialised-closure no-CTE fallback — shipped v0.4 Phase E (the §7.1 sequence-arm / sequence-inner remainder stays gated; negated sets out of v0.4 scope)
- ✅ Type-aware `ORDER BY` (SPARQL 1.1 §15.1) — shipped v0.4 Phase F
      group F4 (slices 23-22). Sort keys order across the §15.1
      value space: a leading **kind rank** (numerics, then
      `xsd:dateTime`, then `xsd:boolean`, then everything else)
      groups comparable lexical spaces, then a per-kind comparator —
      numerics compared **numerically** (so `"2"^^xsd:integer` sorts
      before `"10"^^xsd:integer`, not the old lexical `"10" < "2"`),
      `xsd:dateTime` **chronologically**, `xsd:boolean` `false<true`,
      strings / plain / lang-tagged by **Unicode codepoint**
      (`COLLATE "C"`, locale-independent) — plus a final codepoint
      tiebreak. ORDER BY is **total and never raises** (the
      numeric/dateTime casts are regex-guarded; a malformed lexical
      falls through to the codepoint tier — the §15.1 stable
      fallback), distinct from `<` in FILTER which can error.
      `DESC()` reverses; multi-key (`ORDER BY ?a DESC(?b)`) composes;
      **expression sort keys** (`ORDER BY (?a + ?b)`,
      `ORDER BY STRLEN(?s)`) translate via the shared BIND/FILTER
      expression translator. All four SQL builders (single-branch,
      aggregate, UNION, aggregate-over-UNION) and `SELECT DISTINCT`
      compose; an expression sort key on the aggregate/UNION shapes
      is a documented narrow deferral (project it with BIND, then
      ORDER BY the bound variable). Regression-locked:
      `tests/regression/sql/100-sparql-order-by-type-aware.sql`
      (+ W3C-shape `47-order-by-type-aware`).

      ```sql
      -- xsd:integer literals sort NUMERICALLY: 1, 2, 10, 100
      -- (the pre-F4 lexical sort gave 1, 10, 100, 2).
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?n WHERE { ?s ex:n ?n } ORDER BY ?n');

      -- DESC + an expression sort key.
      SELECT * FROM pgrdf.sparql(
        'PREFIX ex: <http://example.com/>
         SELECT ?s WHERE { ?x ex:s ?s } ORDER BY DESC(STRLEN(?s))');
      ```
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
