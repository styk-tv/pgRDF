# Changelog

All notable changes to pgRDF are tracked here. Format follows
[Keep a Changelog](https://keepachangelog.com/). Versioning is SemVer
once we cut v1.0; pre-1.0 minor bumps may include breaking changes.

## [Unreleased]

### Phase 3 step 11 — Multi-triple MINUS

- `MINUS { ?s :p ?o . ?s :q ?r . … }` now accepts arbitrary
  N-triple sub-patterns. `ParsedSelect.minuses` changed from
  `Vec<TriplePattern>` to `Vec<Vec<TriplePattern>>`; same for
  `UnionBranch.minuses`.
- `translate_minus` rewrites to emit one `NOT EXISTS (SELECT 1
  FROM q_min_1, q_min_2, … WHERE …)` per MINUS block. Each
  triple in the sub-pattern gets its own quad alias; shared
  variables with the outer query AND shared-inside-the-MINUS
  emit equality predicates automatically via `pattern_clauses`.
- SPARQL spec's "no shared variables → MINUS is identity" rule
  still applies: the translator unions all variables in the
  sub-pattern, checks intersection with outer anchors, and
  elides the block if empty.
- Single-triple MINUS continues to work (it's the
  `triples.len() == 1` case of the multi-triple path).
- 1 new pg_test: `sparql_minus_multi_triple` (alice+eve have
  both mbox+age → dropped, bob/carol/dave survive).
- `tests/regression/sql/43-sparql-minus-multi.sql` covers 4
  query shapes: 2-triple AND, 3-triple AND, chained multi-triple
  MINUSes, single-triple back-compat.
- `README.md` pills: 76+23 → 77+24.
- Multi-triple OPTIONAL is **deferred to v0.4** — the LATERAL
  refactor it needs is bigger than the MINUS rewrite (OPTIONAL
  has to EXPOSE its new bindings to the outer query, while MINUS
  is just a boolean check). Workaround: chain single-triple
  OPTIONALs.

Test bar:
  pg_test:    77 passed; 0 failed  (was 76)
  regression: 24 passed; 0 failed  (was 23)

### Phase 3 step 10 — BIND (non-aggregate)

- `BIND(expr AS ?v)` (and the equivalent `SELECT (expr AS ?v)` form
  on non-aggregate expressions) now adds a virtual column. `walk_select`'s
  Extend handler falls through to a `BindSpec` when the expression
  isn't a Variable-rename of an existing aggregate.
- Projection in `build_single_branch_outer` checks `ps.binds` before
  falling back to the BGP anchor lookup, emitting the translated
  expression with the BIND var as the column alias.
- `translate_bind_expression` covers Literal / NamedNode / Variable,
  STR / LANG / DATATYPE / UCASE / LCASE, arithmetic, STRLEN, and
  `CONCAT(?a, ?b, …)` via Postgres `concat`. All values surface as
  text in the JSONB row.
- Today's restriction: a BIND output variable referenced in a later
  FILTER / BGP isn't yet supported (would need expression substitution
  during translation). Filtering on BIND output is Phase 3 backlog.
- 3 new pg_tests + `tests/regression/sql/42-sparql-bind.sql`
  (6 query shapes: UCASE, arithmetic, CONCAT, literal-constant,
  STRLEN, two-BINDs in one query).
- `README.md` pills: 73+22 → 76+23.

Test bar:
  pg_test:    76 passed; 0 failed  (was 73)
  regression: 23 passed; 0 failed  (was 22)

### Phase 3 step 9 — Expression richness in FILTER

- `pgrdf.sparql` FILTER translator gains a much wider expression
  surface:
  - **Arithmetic**: `?a + ?b`, `?a - ?b`, `?a * ?b`, `?a / ?b`
    (with NULLIF-guarded divide-by-zero), unary `-`, unary `+`.
    All built on top of `expr_to_numeric_sql`'s CASE-cast so
    non-numeric operands NULL-propagate instead of erroring.
  - **String predicates**: `CONTAINS`, `STRSTARTS`, `STRENDS` —
    Postgres `strpos`, `left`, `right` against `lexical_value`.
  - **String-valued functions** usable inside other expressions:
    `LANG(?v)`, `DATATYPE(?v)`, `UCASE(?v)`, `LCASE(?v)`,
    `STR(?v)` (was passthrough, formalised). LANG / DATATYPE use
    chained dict lookups (datatype IRI ids → IRI lexical).
  - **`STRLEN(?v)`** is numeric-valued, plugged into
    `expr_to_numeric_sql`.
- Equality fallback: when either side of `=` / `sameTerm` is a
  function call (or otherwise can't resolve to a dict id), the
  translator falls back to lexical comparison. Lets `STR(?v) =
  "x"`, `LANG(?v) = "en"`, `DATATYPE(?v) = xsd:integer` etc.
  translate cleanly.
- `expr_to_lexical_sql` learned to emit a SQL string for
  `NamedNode` (the IRI's lexical form), making the fallback work
  for IRI constants on the right of equality.
- 6 new pg_tests: arithmetic add, mul/div, STRLEN, CONTAINS/
  STRSTARTS/STRENDS, LANG/DATATYPE equality, UCASE/LCASE case
  folding.
- `tests/regression/sql/41-sparql-expressions.sql` covers 11
  query shapes (4 arithmetic, STRLEN, 4 string predicates,
  LANG, DATATYPE).
- `README.md` pills: 67+21 → 73+22.

Test bar:
  pg_test:    73 passed; 0 failed  (was 67)
  regression: 22 passed; 0 failed  (was 21)

### Phase 3 step 8 — HAVING + GROUP_CONCAT + SAMPLE

- `pgrdf.sparql` now translates `HAVING (expr)` clauses on
  aggregate queries. `parse_select` post-processes the collected
  filters: any filter referencing an aggregate output variable
  becomes a HAVING predicate (the rest stay as WHERE).
- `translate_filter_with_aggregates` is the HAVING-aware translator:
  variable references resolve to (a) the underlying SQL aggregate
  function for aggregate-output vars, (b) the group-by expression
  for group vars, (c) literals are used directly. Supports
  identity, numeric ordering (`<`/`>`/`<=`/`>=`), boolean composition.
- `GROUP_CONCAT(?v [; SEPARATOR = "…"])` → Postgres `STRING_AGG`,
  default separator a single space per SPARQL spec.
- `SAMPLE(?v)` → `MIN(lexical_value)` as a deterministic surrogate
  (SPARQL spec says "implementation-defined element"; MIN is one
  conformant choice).
- 4 new pg_tests: HAVING with COUNT, HAVING with SUM, GROUP_CONCAT
  with custom separator, SAMPLE.
- `tests/regression/sql/40-sparql-having.sql` covers 9 query
  shapes (HAVING > N, HAVING = 1, HAVING composite, GROUP_CONCAT
  custom + default separator, SAMPLE, SUM-HAVING on non-numeric
  strings — demonstrates the numeric-awareness rule — and
  SUM-HAVING on real numeric data across two graphs).
- `README.md` pills: 63+20 → 67+21.

Test bar:
  pg_test:    67 passed; 0 failed  (was 63)
  regression: 21 passed; 0 failed  (was 20)

### Phase 3 step 7 — Aggregates + GROUP BY

- `pgrdf.sparql` handles SPARQL aggregates with or without
  `GROUP BY`:
  - `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT ?v)`.
  - `SUM(?v)`, `AVG(?v)` — numeric-aware via the same XSD-numeric
    CASE cast as FILTER ordering. Non-numeric values contribute
    `NULL` (skipped by SUM/AVG per SQL semantics, no Postgres
    cast error).
  - `MIN(?v)`, `MAX(?v)` — lexicographic on the term's
    `lexical_value`. Type-aware MIN/MAX queued.
- `GROUP BY ?vars` translates to SQL `GROUP BY` using the same
  dict-lookup expressions that drive the SELECT clause. Multiple
  aggregates per group supported.
- Aggregate output values come back as **JSON strings** in the
  `pgrdf.sparql` row, consistent with the rest of the surface.
  Callers cast with `(j ->> 'n')::int`/`::numeric` etc.
- Algebra layout: spargebra lowers `SELECT (EXPR AS ?v)` to
  `Project → Extend → Group → BGP`. `walk_select` now handles
  Extend (renames the synthesised `$agg_N` to `?v`) and Group
  (captures group_vars + AggregateSpecs). Walk order: descend
  into inner first so Group's aggregates are populated before
  Extend tries to rename them.
- Parser walks `GraphPattern::Group` and `GraphPattern::Extend`
  rather than flagging them; tests adjusted.
- 7 new pg_tests: COUNT(*), COUNT(DISTINCT), GROUP BY counting,
  SUM numeric, AVG numeric, MIN/MAX lex, multiple aggregates
  per group.
- `tests/regression/sql/39-sparql-aggregates.sql` covers 10
  query shapes: count_all, count_o, count_distinct, sum_age,
  avg_age (rounded), min/max names, group_by predicates,
  multi-aggregate, ORDER-BY-aggregate + LIMIT.
- `README.md` pills: 56+19 → 63+20; SPARQL pill adds AGGREGATES.
- `guide/03-querying.md` gains a full "Aggregates and GROUP BY"
  section covering the JSON-string output rule, the SUM/AVG
  numeric-awareness rule, the MIN/MAX lex caveat, and the
  HAVING/GROUP_CONCAT/BIND restrictions.

Today's restrictions:
- HAVING not yet translated — post-process with regular SQL.
- BIND outside aggregate aliasing not supported.
- Aggregates on top of UNION not supported (panic with clear msg).
- `GROUP_CONCAT` / `SAMPLE` not supported.

Test bar:
  pg_test:    63 passed; 0 failed  (was 56)
  regression: 20 passed; 0 failed  (was 19)

### Phase 3 step 6 — MINUS

- `pgrdf.sparql` handles `MINUS { ?s :p ?o }` and chained MINUSes.
  Each block becomes a `WHERE NOT EXISTS (SELECT 1 FROM
  pgrdf._pgrdf_quads qMIN_K WHERE …)` sub-SELECT, keyed on shared
  variables between the outer query and the MINUS triple.
- Per SPARQL spec, MINUS with no shared variables is a no-op —
  the translator detects this at translation time and emits no
  SQL for that block (different from OPTIONAL, which always
  emits a LEFT JOIN).
- Restriction: each MINUS block must be a single triple pattern
  (mirrors OPTIONAL's current restriction).
- Inside UNION branches, MINUS works the same way (scoped to the
  branch's anchor map).
- 4 new pg_tests: basic MINUS, no-shared-vars no-op, chained
  MINUSes, MINUS + outer FILTER + REGEX.
- Parser walks `GraphPattern::Minus` rather than flagging it.
  New parser pg_test for the new state + a Path-still-flagged
  test taking its place (transitive `:a*`, not simple `:a/:b`
  which spargebra desugars to BGP).
- `tests/regression/sql/38-sparql-minus.sql` covers 6 query
  shapes (basic, no-op, chained, with-FILTER, ordered survivor,
  shared-non-subject-var).
- `30-sparql-parse.sql` baseline updated: MINUS supported, Path
  (quantified) is the new unsupported representative.
- `README.md` pills: 51+18 → 56+19, SPARQL pill adds MINUS.
- `guide/03-querying.md` gains a MINUS section covering the
  shared-vars-vs-no-op rule and the OPTIONAL-asymmetry note.

Test bar:
  pg_test:    56 passed; 0 failed  (was 51)
  regression: 19 passed; 0 failed  (was 18)

### Phase 3 step 5 — UNION

- `pgrdf.sparql` handles `{ A } UNION { B }` and chained
  `A UNION B UNION C`. Each branch is its own complete sub-SELECT
  (own BGP / FILTERs / OPTIONALs / per-branch dict-id anchors).
  Branches are combined with SQL `UNION ALL`; the outer SELECT
  layers `DISTINCT` / `ORDER BY` / `LIMIT` / `OFFSET`.
- Variables bound in only some branches come back as `null` from
  the other branches (each branch SELECTs `NULL::TEXT` for vars
  it doesn't bind, so row shapes line up across `UNION ALL`).
- ORDER BY on UNION may only reference projected variables — the
  outer SELECT can't see branch-local alias columns. Executor
  panics with a clear message otherwise.
- Refactor: extracted `build_from_and_where` (shared by both the
  single-branch and per-UNION-branch paths) + `build_branch_sql`
  + `build_union_sql`. The original `build_bgp_sql` is now a
  dispatcher over `ps.union_branches.is_empty()`.
- 5 new pg_tests: basic UNION over same var, different-var
  UNION with NULL pad, three-way chain, UNION + DISTINCT,
  UNION + ORDER BY + LIMIT.
- Parser walks `GraphPattern::Union` rather than flagging it.
  New parser pg_test for the new state + a new MINUS-still-flagged
  test taking its place.
- `tests/regression/sql/37-sparql-union.sql` covers 9 query shapes
  (basic, DISTINCT, different-var, two NULL-discriminator checks,
  three-way chain, ORDER BY first, LIMIT, branch-local FILTER).
- `30-sparql-parse.sql` baseline refreshed: UNION supported,
  MINUS now the unsupported representative.
- `README.md` pills: 45+17 → 51+18, SPARQL pill adds UNION.
- `guide/03-querying.md` gains a full UNION section covering
  the cross-branch null padding, ORDER-BY-must-be-projected
  rule, and the no-nesting restriction for this slice.

Test bar:
  pg_test:    51 passed; 0 failed  (was 45)
  regression: 18 passed; 0 failed  (was 17)

### Phase 3 step 4 — OPTIONAL (LeftJoin) translation

- `pgrdf.sparql` now handles `OPTIONAL { ?s :p ?o }`. Each OPTIONAL
  block emits a `LEFT JOIN pgrdf._pgrdf_quads qOPT_i ON (…)`. Variables
  introduced inside an OPTIONAL surface as NULL (JSONB `null`) when
  the LEFT JOIN didn't match.
- `OPTIONAL { … FILTER(...) }` — the inner filter lands in the LEFT
  JOIN's ON clause, so rejected matches keep the optional variable
  NULL rather than pruning the whole row.
- Multiple chained OPTIONALs each get their own LEFT JOIN, in
  left-to-right order. Per SPARQL semantics, variables introduced
  by one OPTIONAL aren't visible to another OPTIONAL's ON clause.
- `BOUND(?v)` translation tightened: now emits `qN.col IS NOT NULL`
  regardless of whether ?v is mandatory or OPTIONAL. Mandatory
  anchors are non-NULL so it's trivially TRUE there; OPTIONAL anchors
  can be NULL so this is the spec-correct semantics.
- Internal refactor: `build_bgp_sql` switched from comma-style FROM
  (`q1, q2, q3 WHERE …`) to explicit JOIN syntax
  (`q1 INNER JOIN q2 ON … INNER JOIN q3 ON …`). Same semantics for
  INNER joins; necessary for OPTIONAL's LEFT JOIN to compose.
- Parser updated: `LeftJoin` no longer flagged in
  `unsupported_algebra` — the parser walks both arms.
- 4 new pg_tests: simple OPTIONAL, OPTIONAL with inner FILTER,
  multiple chained OPTIONALs, outer FILTER(BOUND) pruning.
- `tests/regression/sql/36-sparql-optional.sql` covers 8 query
  shapes (LEFT JOIN counts, NULL/not-NULL discrimination, inner
  filter, multi-chain, outer BOUND prune, OPTIONAL + ORDER BY).
- `30-sparql-parse.sql` baseline updated: OPTIONAL no longer
  flagged; new UNION assertion replaces it.
- `README.md` pills: 40+16 → 45+17; SPARQL pill adds OPTIONAL.
- `guide/03-querying.md` gains a full OPTIONAL section covering
  inner-FILTER semantics, chained OPTIONALs, BOUND-pruning, and
  the single-triple restriction for this slice.

Test bar:
  pg_test:    45 passed; 0 failed  (was 40)
  regression: 17 passed; 0 failed  (was 16)

### Phase 3 step 3 — Solution modifiers (DISTINCT / LIMIT / OFFSET / ORDER BY)

- The four classic SPARQL solution modifiers now land in the
  generated SQL instead of being silently stripped from the AST:
  - `SELECT DISTINCT ?vars` → `SELECT DISTINCT` in SQL.
  - `SELECT REDUCED ?vars` → also `SELECT DISTINCT` (REDUCED is a
    "dups may or may not be removed" hint per spec; over-approxing
    with DISTINCT is conformant).
  - `LIMIT N` / `OFFSET N` → `LIMIT N` / `OFFSET N`.
  - `ORDER BY ?var`, `ORDER BY ASC(?var)`, `ORDER BY DESC(?var)`,
    multi-key — sorted by the term's `lexical_value` with
    `NULLS LAST`. If the var is projected the existing column is
    reused; otherwise an extra hidden column is appended and ORDER
    BY references it by ordinal (so the JSONB output stays clean).
- ORDER BY today is **lexicographic on string form**, not SPARQL's
  full type-aware ordering. Numeric ordering through ORDER BY lands
  in step 4+; for now use FILTER for numeric range + post-SQL
  `ORDER BY (sparql->>'n')::numeric`.
- Refactor: `unwrap_select` → `parse_select` returning a richer
  `ParsedSelect` struct (projected, bgp, filters, distinct,
  order_by, limit, offset). Single recursive walk replaces the
  old two-pass extract_bgp_and_filters / unwrap_select split.
- 6 new pg_tests: distinct dedups, LIMIT caps, OFFSET skips,
  ORDER BY ASC + DESC, DISTINCT + ORDER BY interaction.
- `tests/regression/sql/35-sparql-modifiers.sql` covers 10 query
  shapes (raw count, DISTINCT, REDUCED, LIMIT 2, ORDER ASC first,
  ORDER DESC first, OFFSET 3 LIMIT 2 window, DISTINCT + ORDER,
  ORDER BY on non-projected var, LIMIT 0).
- `README.md` pills: 34+15 → 40+16, SPARQL pill adds
  DISTINCT/ORDER/LIMIT.
- `guide/03-querying.md` gains a full "Solution modifiers" section
  covering ORDER BY's lexicographic-vs-type-aware caveat, the
  DISTINCT-with-non-projected-order-by panic case, and a worked
  example.

Test bar:
  pg_test:    40 passed; 0 failed  (was 34)
  regression: 16 passed; 0 failed  (was 15)

### Phase 3 step 2 — FILTER numeric ordering + REGEX + IN

- `pgrdf.sparql` FILTER translator gains three new shapes:
  - **Numeric ordering** (`<`, `>`, `<=`, `>=`): operand resolves to
    `NUMERIC` via a CASE-guarded subselect on `_pgrdf_dictionary`.
    Only XSD numeric datatypes (integer, decimal, double, float,
    sized + unsigned + constraint subtypes — 16 IRIs total)
    contribute; everything else compares NULL → row dropped. This
    matches SPARQL's "type error → unbound" semantics without ever
    raising a Postgres cast error.
  - **`REGEX(?v, "pat" [, "flags"])`**: Postgres `~` (case-sensitive)
    or `~*` (with `i` flag) against the term's `lexical_value`.
    Pattern + flags are SPARQL literals at translation time;
    single quotes in the pattern are escaped. `STR(?v)` inside
    REGEX is a passthrough.
  - **`?term IN (e1, e2, …)`**: dict-id set membership.
- 6 new pg_tests: numeric `>` / range / non-numeric drop, regex
  case-sensitive / case-insensitive with STR(), and IN.
- `tests/regression/sql/34-sparql-filter-advanced.sql` covers 10
  query shapes (numeric `>`, range, `<` with non-numeric mixed in,
  `>= 0` over a typed-decimal row, regex `^A`, regex `ar` case-i,
  regex+STR wrap, IN over IRIs, IN over a literal, and a cross-BGP
  composition).
- `README.md` pills: tests 28+14 → 34+15, SPARQL pill adds REGEX.
- `guide/03-querying.md` gains full sections for numeric ordering,
  REGEX (with the POSIX-vs-PCRE caveat), and IN. Capability matrix
  refreshed.

Test bar:
  pg_test:    34 passed; 0 failed  (was 28)
  regression: 15 passed; 0 failed  (was 14)

### Phase 3 step 1 — FILTER expressions over BGPs

- `pgrdf.sparql` now walks `GraphPattern::Filter { expr, inner }`
  and translates a useful subset of `Expression` into SQL WHERE
  predicates appended after the BGP joins:
  - **Identity**: `=`, `!=`, `sameTerm` — both operands resolved to
    dictionary ids, compared as BIGINT. Sound because the dictionary
    deduplicates by `(term_type, lexical, datatype, language)`.
  - **Boolean**: `&&`, `||`, `!`.
  - **Term-type predicates**: `isIRI`, `isLiteral`, `isBlank` — emit
    a correlated subselect on `_pgrdf_dictionary.term_type`.
  - **`BOUND`**: trivially `TRUE` for any anchored BGP variable.
  - Untranslatable shapes (numeric `<`/`>`/`<=`/`>=`, `regex`, `str`,
    `lang`, arithmetic, `IN`, `EXISTS`) panic with a clear message
    rather than silently dropping the filter.
- `pgrdf.sparql_parse` no longer flags `Filter` in
  `unsupported_algebra` — it walks into the inner BGP. OPTIONAL,
  UNION, MINUS, Group, Path, Values, Extend (BIND), Service still
  flagged.
- 6 new pg_tests: literal equality, `!=`, `isIRI`, boolean AND
  composition, var-equals-var (self-loop), `BOUND` trivially-true.
- 1 new parser pg_test: OPTIONAL replaces the FILTER-flagged baseline.
- `tests/regression/sql/33-sparql-filter.sql` covers 9 query shapes
  end-to-end (literal eq, neg, isIRI, isLiteral, self-loop,
  boolean AND, negated isIRI, BOUND, unknown-literal-zero-rows).
- `tests/regression/sql/30-sparql-parse.sql` baseline updated: Filter
  no longer reported as unsupported; new OPTIONAL assertion added.
- `guide/03-querying.md` adds a full FILTER section with examples,
  including the `=` ↔ sameTerm-vs-value-equality caveat and how
  filters interact with multi-pattern BGPs.
- `README.md`: status pill → `phase 3 start`, test pill 21+13 → 28+14,
  SPARQL pill `SELECT/BGP` → `SELECT/BGP/FILTER`.

Test bar:
  pg_test:        28 passed; 0 failed  (was 21)
  regression:     14 passed; 0 failed  (was 13)

### Phase 2.2 step 8 — Node.js + Go client guides

- `guide/clients/typescript.md` — `pg` (node-postgres) + `postgres.js`
  + `pg-cursor` streaming + strongly-typed binding helpers. Covers
  `load_turtle`, `parse_turtle`, `load_turtle_verbose`, and the
  full `pgrdf.sparql` JSONB result shape with type narrowing.
- `guide/clients/go.md` — `pgx` v5 + `pgxpool` + sqlc integration
  + bulk-ingest pattern + the constant-time graph-drop idiom.
- `guide/README.md` index lists both new client pages.
- `README.md` clients section now points at all 4 supported clients
  (Python, Rust, TypeScript, Go).

### Phase 2.2 step 7 — User guide for SPARQL surface

- New `guide/03-querying.md`: full walkthrough of `pgrdf.sparql`
  (single + multi-pattern BGPs, constants in any position, JSONB
  output, combining with regular SQL, `pgrdf.sparql_parse` for
  introspection) plus what works / doesn't / why, and a worked
  example of the SQL translation.
- `README.md` promoted the SPARQL surface from "coming soon" to a
  live code example, bumped the test pill from 9+10 to 21+13,
  added a SPARQL pill, refreshed the status row.
- `guide/README.md` index entry for `03-querying.md`.

### Phase 2.2 step 6 — Multi-pattern BGP joins

- `pgrdf.sparql` now handles N-pattern Basic Graph Patterns. Each
  pattern becomes a `_pgrdf_quads qN` clause; shared variables across
  patterns are tracked by first-occurrence anchors and emit equality
  predicates (`q2.subject_id = q1.subject_id`) that fold into INNER
  joins.
- 2 new pg_tests: two-pattern shared-subject BGP (Alice + Carol have
  both `foaf:name` and `foaf:mbox`, Bob doesn't), three-pattern chain
  following `foaf:knows`.
- `tests/regression/sql/32-sparql-multipattern.sql` covers 5 shapes:
  shared-subject BGP, three-pattern chain, self-loop pattern (?s ?p ?s),
  bound-subject multi-pattern, and bound-predicate + bound-literal.

Test bar:
  pg_test:        21 passed; 0 failed  (was 19)
  regression:     13 passed; 0 failed  (was 12)

### Phase 2.2 step 5 — SPARQL execution: BGP → SQL

- `pgrdf.sparql(q TEXT) → SETOF JSONB` — first user-visible SPARQL
  surface. Parses via spargebra, translates a single Basic Graph
  Pattern into a dynamic SQL SELECT over `_pgrdf_quads` joined to
  `_pgrdf_dictionary`, returns one JSONB row per solution keyed by
  the projected variable names.

  ```sql
  SELECT * FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n WHERE { ?s foaf:name ?n }'
  );
  --  → {"s": "http://example.com/alice", "n": "Alice"}
  --  → {"s": "http://example.com/bob",   "n": "Bob"}
  ```

  Scope today (intentionally narrow — multi-pattern joins land in
  step 6):
  - SELECT only.
  - Exactly one BGP triple per query.
  - Constants in any position (subject IRI, predicate IRI, object
    IRI or literal). Unknown constants resolve to `-1` so the query
    correctly returns zero rows rather than erroring.
  - Variables in any position.
  - Distinct / Reduced / Slice / OrderBy wrappers are passed through.
- 4 new pg_tests covering all-three-vars BGP, bound-predicate filter,
  bound-subject filter, and unknown-predicate-returns-empty.
- `tests/regression/sql/31-sparql-bgp.sql` exercises 7 query shapes
  end-to-end through the compose Postgres.

Infrastructure:

- `compose/builder.Containerfile` rewritten with BuildKit cache
  mounts. The builder image dropped from 7.73 GB → 3.35 GB; cargo
  registry + target/ now live in build-scoped cache volumes that
  persist across rebuilds without bloating image layers.
- `Justfile build-ext` now invokes `DOCKER_BUILDKIT=1 docker build`
  so the `# syntax=docker/dockerfile:1.4` directive activates.
- `.dockerignore` excludes `target/`, `.target-linux/`,
  `compose/pg-data/`, `compose/extensions/lib|share`,
  `fixtures/ontologies/`, `.git/`. Build context dropped accordingly.

### Phase 2.2 step 4 — SPARQL parser surface

- `spargebra = "0.4"` (0.4.6 resolved). Pins `oxrdf = "=0.3.3"`, the
  same version oxttl 0.2.3 uses, so no graph split.
- New module `src/query/parser.rs`.
- `pgrdf.sparql_parse(q TEXT) -> JSONB` parses a SPARQL query via
  `spargebra::SparqlParser` and returns the high-level shape:
  - `form` — SELECT / CONSTRUCT / ASK / DESCRIBE
  - `variables` — projected vars (SELECT only)
  - `bgp_pattern_count`, `bgp_patterns` — BGP triples with
    s/p/o each rendered as `{var: …}`, `{iri: …}`, `{bnode: …}`,
    or `{literal: …, datatype/lang: …}`
  - `unsupported_algebra` — flags Filter / Union / OPTIONAL /
    Property paths / Aggregates / VALUES / SERVICE / etc., so
    callers see the AST has shape the translator doesn't yet
    cover.
- 5 new pg_tests covering basic SELECT, predicate-as-IRI BGP,
  two-pattern BGP, FILTER detection, and a syntax-error panic path.
- New regression `tests/regression/sql/30-sparql-parse.sql` asserts
  the JSONB extraction over 6 query forms.

### Phase 2.2 step 3 — Batched ingestion

(landed alongside docs split + README pills.)

- `src/storage/loader.rs`: per-call HashMap dict cache + buffered
  multi-row INSERTs via `unnest($1::bigint[], $2::bigint[], $3::bigint[])`.
  BATCH_SIZE = 1000. Reduces SPI calls from ~7/triple to roughly
  `distinct_terms + ceil(triples/1000)`.
- `pgrdf.load_turtle_verbose(path, graph_id, base_iri)` and the
  matching `pgrdf.parse_turtle_verbose(content, graph_id, base_iri)`
  return JSONB stats: `triples`, `dict_cache_hits`, `dict_db_calls`,
  `quad_batches`, `elapsed_ms`. Used to assert the cache is firing.
- `fixtures/regression/synth-100.sh` + `synth-100.ttl`: deterministic
  100-triple synthetic fixture (10 subjects × 5 predicates × 100
  objects). 115 distinct terms, 185 expected cache hits.
- `tests/regression/sql/25-bulk-ingest.sql` asserts exact stat values
  on the synth-100 fixture and verifies dict dedup across two graphs.
- One new pg_test (`parse_turtle_verbose_cache_fires`) asserts cache
  behavior at the Rust level.
- `serde_json = "1"` added as a direct dependency for the verbose UDFs.

### Phase 2.1 — Turtle ingest

- `pgrdf.load_turtle(path, graph_id, base_iri)` and
  `pgrdf.parse_turtle(content, graph_id, base_iri)` parse Turtle via
  `oxttl 0.2` and stream triples through the dictionary +
  partitioned hexastore. `base_iri` resolves relative IRIs like
  `<#>` (needed for W3C PROV).
- Internal `put_term_full(value, type, datatype_id, lang)` honours
  the full dictionary key with `IS NOT DISTINCT FROM` lookups so
  NULL datatype + language columns participate in dedup.
- Compose: read-only `./fixtures:/fixtures:ro,z` bind mount so the
  postgres process can reach test + ontology fixtures by path.
- 24 W3C / Apache Jena / ConceptKernel / ValueFlows ontologies fetch
  cleanly via `fixtures/ontologies.sh`; `tests/perf/smoke-ontologies.sh`
  loads each one through `pgrdf.load_turtle` and prints triple
  counts. 17,134 triples across the set on the 2026-05-13 fetch.
- Four checked-in regression fixtures (`typed-literals.ttl`,
  `lang-tags.ttl`, `blank-nodes.ttl`, `rdf-list.ttl`) under
  `fixtures/regression/` exercise XSD datatypes, language tags,
  blank-node dedup, and `rdf:List` desugaring. All assertions are
  scoped strictly by graph_id so prior smoke loads don't pollute
  results.
- `workflow.ttl` excluded from the iteration set: source uses
  `<ckp://Name:v0.1>` IRI form (colon in path segment, not RFC 3986
  compliant). To be re-added when the CKP source is fixed.

### Phase 2.0 — Storage CRUD UDFs

- `pgrdf.put_term(value, term_type)`,
  `pgrdf.get_term(id)`,
  `pgrdf.put_quad(s, p, o, g)`,
  `pgrdf.count_quads(g)`,
  `pgrdf.add_graph(g)` — all backed by SPI against the
  `_pgrdf_dictionary` + `_pgrdf_quads` schema declared in
  `sql/schema_v0_2_0.sql`.
- 7 `#[pg_test]` integration tests + 3 regression files.
- Justfile: `just test` runs `cargo pgrx test` inside the linux
  builder container; `just test-regression` runs pg_regress-style
  SQL fixtures against the compose Postgres. Both gate the same
  thing CI will.

### Phase 1 — Scaffold + runtime

- pgrx 0.16 extension scaffolding (PG 14-17 feature matrix,
  `pgrx_embed` bin target for schema generation).
- Compose-based local runtime: stock `postgres:17.4-bookworm` with
  per-file bind mounts at `$libdir` / `$sharedir/extension`. No init
  script, no entrypoint wrapper.
- Linux builder container (`compose/builder.Containerfile`) that
  produces glibc-bookworm artifacts on macOS hosts. Two-VM topology:
  Colima for builds (100 GB), podman for the compose stack (avoids
  filling the user's other container state).
- 10-doc engineering set under `docs/` (architecture, storage, query,
  inference, validation, install, dev, testing, release, roadmap).
- `specs/SPEC.pgRDF.LLD.v0.2.md` + `specs/SPEC.pgRDF.INSTALL.v0.2.md`
  captured verbatim alongside `specs/ERRATA.v0.2.md` cataloguing
  deltas found during implementation.
- CI / release workflow placeholders for the
  {pg14..pg17}×{amd64, arm64} matrix.

### Errata against v0.2 specs

- `shacl-rust` → `shacl_validation` (E-001).
- `reasonable` is OWL 2 RL only, not arbitrary Datalog (E-002).
- PG 18 forward path blocked on pgrx 0.17/0.18 not building on
  current Rust (E-006). Compose targets PG 17 until upstream lands
  a fix.
- See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) for the full set.
