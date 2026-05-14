# 03 — Querying with SPARQL

`pgrdf.sparql(q TEXT) → SETOF JSONB` runs a SPARQL SELECT against
everything in the database and returns one JSON row per solution.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?person ?name WHERE { ?person foaf:name ?name }'
);
--  → {"person": "http://example.com/alice", "name": "Alice"}
--  → {"person": "http://example.com/bob",   "name": "Bob"}
```

Each row is a `JSONB` object keyed by the SELECT-clause variable
names. Lexical values come back as strings. The `pgrdf.sparql`
function is set-returning, so you can use it anywhere a normal
SETOF Postgres function would go — `FROM`, `LATERAL`, CTEs, etc.

## What works today

| Form | Status |
|---|---|
| `SELECT ?vars WHERE { BGP }` with 1 or more triple patterns | ✅ |
| Constants in subject, predicate, or object position (IRIs, literals) | ✅ |
| Multi-pattern BGPs with shared variables → INNER joins | ✅ |
| `DISTINCT`, `REDUCED` → `SELECT DISTINCT` | ✅ |
| `LIMIT N`, `OFFSET N` | ✅ |
| `ORDER BY ?var`, `ORDER BY ASC(?var)`, `ORDER BY DESC(?var)` — lexicographic on `lexical_value` | ✅ |
| `ORDER BY <complex expression>` | ⏳ Phase 3 (next slice) |
| `FILTER` — identity (`=`, `!=`, `sameTerm`), boolean (`&&`, `\|\|`, `!`), term-type (`isIRI`, `isLiteral`, `isBlank`), `BOUND` | ✅ |
| `FILTER` — numeric ordering (`<`/`>`/`<=`/`>=`), `REGEX`, `IN`, `STR` passthrough | ✅ |
| `FILTER` — arithmetic, `lang`, `datatype`, `STRLEN`, `CONTAINS`, full string-fn surface | ⏳ Phase 3 (next slice) |
| `OPTIONAL { single-triple BGP }` → LEFT JOIN (with inner FILTER honoured) | ✅ |
| `OPTIONAL { multi-pattern BGP }`, nested OPTIONALs | ⏳ Phase 3 (next slice) |
| `UNION` (n-way, branches may bind different vars) | ✅ |
| `MINUS { single-triple }` keyed by shared vars (no-op when no shared vars per spec) | ✅ |
| Aggregates — `COUNT(*)`, `COUNT(?v)`, `COUNT(DISTINCT ?v)`, `SUM`, `AVG`, `MIN`, `MAX` with `GROUP BY` | ✅ |
| `MINUS { multi-pattern }`, property paths, `VALUES`, `BIND`, HAVING, `GROUP_CONCAT`, `SAMPLE` | ⏳ Phase 3 |
| `CONSTRUCT`, `ASK`, `DESCRIBE` | ⏳ Phase 3 |
| Named-graph `GRAPH { … }` clauses | ⏳ Phase 3 |
| `SERVICE` (federated SPARQL) | Out of scope for v0.x |

`pgrdf.sparql_parse(q)` reports the parsed shape as JSONB and flags
`unsupported_algebra` for everything not yet translated — use it to
preview whether the translator will handle your query (see further down).

## Examples

### Single-pattern BGP

```sql
-- Every triple in the database
SELECT * FROM pgrdf.sparql('SELECT ?s ?p ?o WHERE { ?s ?p ?o }');

-- All FOAF names
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?name WHERE { ?_ foaf:name ?name }'
);

-- What does this specific subject have?
SELECT * FROM pgrdf.sparql(
  'SELECT ?p ?o WHERE { <http://example.com/alice> ?p ?o }'
);
```

### Multi-pattern BGP — shared variables become joins

```sql
-- People who have BOTH name and mbox
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?p ?n ?m
     WHERE { ?p foaf:name ?n .
             ?p foaf:mbox ?m }'
);
--  → {"p": "http://example.com/alice", "n": "Alice", "m": "mailto:a@x"}
--  → {"p": "http://example.com/carol", "n": "Carol", "m": "mailto:c@x"}
--  (Bob excluded — no mbox.)

-- Three-pattern chain: "name of A, name of someone A knows"
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?an ?bn
     WHERE { ?a foaf:knows ?b .
             ?a foaf:name  ?an .
             ?b foaf:name  ?bn }'
);
--  → {"an": "Alice", "bn": "Bob"}
```

### Constants in any position

```sql
-- Bound predicate
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE { ?s a foaf:Person }'
);

-- Bound subject
SELECT * FROM pgrdf.sparql(
  'SELECT ?p ?o WHERE { <http://example.com/alice> ?p ?o }'
);

-- Bound literal object — exact value + datatype match
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?p WHERE { ?p foaf:name "Alice" }'
);

-- Typed literal
SELECT * FROM pgrdf.sparql(
  'PREFIX xsd: <http://www.w3.org/2001/XMLSchema#>
   PREFIX ex:  <http://example.com/>
   SELECT ?p WHERE { ?p ex:age "30"^^xsd:integer }'
);
```

### FILTER expressions

```sql
-- Identity: literal equality (compared as dict ids — sameTerm semantics)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE { ?s foaf:name ?n FILTER(?n = "Alice") }'
);

-- Identity: IRI equality (also against ?vars)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?o
     WHERE { ?s ?p ?o FILTER(?p = foaf:knows) }'
);

-- Negation
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE { ?s foaf:name ?n FILTER(?n != "Alice") }'
);

-- Term-type predicates
SELECT * FROM pgrdf.sparql(
  'SELECT ?s ?o WHERE { ?s ?p ?o FILTER(isIRI(?o)) }'
);
SELECT * FROM pgrdf.sparql(
  'SELECT ?s ?o WHERE { ?s ?p ?o FILTER(isLiteral(?o)) }'
);
SELECT * FROM pgrdf.sparql(
  'SELECT ?s WHERE { ?s ?p ?o FILTER(isBlank(?s)) }'
);

-- Boolean composition
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?o
     WHERE { ?s ?p ?o FILTER(isIRI(?o) && ?p = foaf:knows) }'
);

-- Self-loop detection via ?s = ?o
SELECT * FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o FILTER(?s = ?o) }');
```

#### What `=` actually means here

pgRDF's FILTER `=` is implemented by comparing **dictionary ids**.
Two terms compare equal iff their `(term_type, lexical, datatype,
language)` quadruple matches exactly — that's RDF `sameTerm`
semantics, which is also what SPARQL's `=` reduces to for IRIs and
blank nodes, and matches `=` for strings of the same datatype.

The XSD-value-equality cases (`"1"^^xsd:integer = "01"^^xsd:integer`,
`"a" = "a"^^xsd:string`) currently compare as *not equal* because
the lexical forms differ. Numeric / lexical / value comparison lands
with the rest of the ordering operators in the next Phase 3 slice.

#### `BOUND` in a BGP context

`BOUND(?v)` is trivially `TRUE` for any variable `?v` that's used in
the BGP (every BGP variable is bound on every result row) and
`FALSE` for any variable that isn't. Useful once `OPTIONAL` lands;
today it's a no-op marker you can leave in a query that previously
relied on it.

#### Combining FILTER with multi-pattern BGPs

```sql
-- All people with both name + mbox, excluding Alice
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?p ?n ?m
     WHERE { ?p foaf:name ?n .
             ?p foaf:mbox ?m
             FILTER(?n != "Alice") }'
);
```

Filters apply after the BGP joins — they're appended to the
`WHERE` clause of the generated SQL.

### Numeric ordering

```sql
-- Adults only
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?age
     WHERE { ?s foaf:age ?age FILTER(?age >= 18) }'
);

-- Age range
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s
     WHERE { ?s foaf:age ?age FILTER(?age >= 30 && ?age < 65) }'
);
```

Both sides are cast to Postgres `NUMERIC` if and only if their
dictionary entry's datatype is one of the XSD numeric IRIs
(`xsd:integer`, `xsd:decimal`, `xsd:double`, `xsd:float`, the
sized variants and unsigned variants, and the constraint subtypes).
Anything else — `xsd:string`, untyped, IRI, blank node — compares
NULL and is dropped from the result, matching SPARQL's "type
error → unbound" semantics. Comparing two strings as if they were
numbers does not raise an error; it just yields no rows.

If you need string ordering (lexicographic), wait for the
`STR` + `<`/`>` overload in Phase 3 step 3, or post-process in SQL:

```sql
SELECT j ->> 's' AS s, j ->> 'n' AS n
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?s ?n WHERE { ?s foaf:name ?n }'
  ) AS j
 ORDER BY j ->> 'n';
```

### REGEX

```sql
-- Case-sensitive (Postgres ~ operator)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE { ?s foaf:name ?n FILTER(REGEX(?n, "^A")) }'
);

-- Case-insensitive (i flag → Postgres ~* operator)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE { ?s foaf:name ?n FILTER(REGEX(?n, "^a", "i")) }'
);

-- STR() wrapper is a no-op (every term's lexical form IS its string)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE { ?s foaf:name ?n FILTER(REGEX(STR(?n), "ar", "i")) }'
);
```

The regex pattern is a SPARQL literal at translation time and is
embedded as a Postgres regex literal (single quotes are escaped).
Anchors (`^`, `$`), character classes, quantifiers — anything
Postgres POSIX regex supports. The `i` flag toggles case-insensitive;
other flags are accepted but currently ignored (Postgres POSIX
doesn't have a direct PCRE-flag equivalent for `x`/`m`/`s`).

### IN — set membership

```sql
-- Find persons in a named set
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE {
     ?s foaf:name ?n
     FILTER(?s IN (<http://example.com/alice>,
                   <http://example.com/carol>,
                   <http://example.com/dave>))
   }'
);

-- Literal membership
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE { ?s foaf:name ?n FILTER(?n IN ("Alice", "Bob")) }'
);
```

`IN` is dict-id set membership — emits `qN.col IN (id_1, id_2, …)`
where each id is resolved upfront. Unknown terms resolve to `-1`
so they can never match, matching SPARQL's "not in the set" outcome.

### OPTIONAL

`OPTIONAL { ?s :p ?o }` translates to a `LEFT JOIN` against the
mandatory BGP. Variables introduced inside the OPTIONAL come back
NULL (as `JSON null` in the JSONB output) for rows where the
optional pattern didn't match.

```sql
-- Names + mbox if available
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?m
     WHERE { ?s foaf:name ?n
             OPTIONAL { ?s foaf:mbox ?m } }'
);
--  → {"s": "...alice", "n": "Alice", "m": "mailto:a@x"}
--  → {"s": "...bob",   "n": "Bob",   "m": null}
--  → {"s": "...carol", "n": "Carol", "m": "mailto:c@x"}
```

#### OPTIONAL with an inner FILTER

```sql
-- Bring back age only if >= 18; otherwise the row still surfaces
-- with ?a = null (filter rejects the optional match, not the row)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?a
     WHERE { ?s foaf:name ?n
             OPTIONAL { ?s foaf:age ?a FILTER(?a >= 18) } }'
);
```

The OPTIONAL's filter lands in the LEFT JOIN's `ON` clause, so when
it rejects a candidate match, `?a` comes back as `null` (rather
than the whole row being pruned).

#### Multiple chained OPTIONALs

```sql
-- name (mandatory), mbox + age both OPTIONAL
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?m ?a
     WHERE { ?s foaf:name ?n
             OPTIONAL { ?s foaf:mbox ?m }
             OPTIONAL { ?s foaf:age  ?a } }'
);
```

Each OPTIONAL becomes its own LEFT JOIN. Variables introduced in
one OPTIONAL aren't visible to another OPTIONAL's join condition
(per SPARQL semantics).

#### Pruning with outer FILTER(BOUND(?v))

```sql
-- Persons who DO have an mbox — outer FILTER removes the unbound rows
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?m
     WHERE { ?s foaf:name ?n
             OPTIONAL { ?s foaf:mbox ?m }
             FILTER(BOUND(?m)) }'
);
```

`BOUND(?v)` translates to `qN.col IS NOT NULL`, so it correctly
returns FALSE for OPTIONAL vars that didn't match. (For mandatory
vars it's always TRUE since INNER joins guarantee non-null.)

#### Today's restrictions

- **Each OPTIONAL block must hold exactly one triple pattern.**
  Multi-pattern OPTIONALs require a derived-table refactor that
  lands in the next slice. The executor panics with a clear
  message if you give it `OPTIONAL { a . b . }`.
- **Nested OPTIONAL inside OPTIONAL** isn't supported yet — only
  flat chains at the same level.
- **OPTIONAL's inner FILTER** sees only that OPTIONAL's variables
  and the mandatory anchors, not other OPTIONAL groups' variables.

### UNION

`{ A } UNION { B }` combines two branches with SQL `UNION ALL`.
Each branch is a complete sub-SELECT — its own BGP, FILTERs, and
OPTIONALs. Variables only bound in one branch come back as
`null` in the JSONB rows from the other branch.

```sql
-- Same projected var across branches (names from either property)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n
     WHERE { { ?s foaf:name ?n }
             UNION
             { ?s foaf:nick ?n } }'
);

-- Different vars per branch
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?m
     WHERE { { ?s foaf:name ?n }
             UNION
             { ?s foaf:mbox ?m } }'
);
--  → {"s": "...alice", "n": "Alice", "m": null}
--  → {"s": "...bob",   "n": null,    "m": "mailto:b@x"}

-- N-way chain: A UNION B UNION C flattens to 3 branches
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?o
     WHERE { { ?s foaf:name ?o }
             UNION
             { ?s foaf:nick ?o }
             UNION
             { ?s foaf:mbox ?o } }'
);
```

#### How UNION composes with the rest

- **FILTER inside a branch** is branch-local — it only prunes that
  branch's rows.
- **OPTIONAL inside a branch** works the same as in a non-UNION
  query, scoped to that branch.
- **DISTINCT / ORDER BY / LIMIT / OFFSET** apply to the union
  result as a whole. ORDER BY on UNION may only reference
  **projected** variables (the outer SELECT can't see a branch's
  internal alias columns); the executor panics with a clear
  message if you try.
- Each branch is translated independently with its own `q1, q2, …`
  alias namespace — there's no cross-branch join.

#### Today's restriction

- Each UNION branch is one of: BGP, FILTERed BGP, BGP with
  OPTIONALs. Nested UNION inside a branch, or UNION inside an
  OPTIONAL, isn't supported in this slice.

### MINUS

`{ A } MINUS { B }` removes rows of `A` whose shared variables are
compatible with some row of `B`. The translator emits a
`WHERE NOT EXISTS (SELECT 1 FROM pgrdf._pgrdf_quads qMIN WHERE …)`
sub-SELECT keyed on those shared variables.

```sql
-- Persons who DON'T have an mbox
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n
     WHERE { ?s foaf:name ?n
             MINUS { ?s foaf:mbox ?m } }'
);

-- Persons with neither mbox nor age (chained MINUSes)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s
     WHERE { ?s foaf:name ?n
             MINUS { ?s foaf:mbox ?m }
             MINUS { ?s foaf:age  ?a } }'
);
```

#### The shared-variables rule

Per SPARQL spec, MINUS only filters when the two arms share at
least one variable. If `MINUS { ?x ex:foo ?y }` shares no variable
with the outer query, it's a no-op — every row of the outer
pattern survives. The translator detects this case at translation
time and emits no SQL at all for that MINUS block.

That's different from how OPTIONAL behaves with disjoint variables
(OPTIONAL does emit a LEFT JOIN regardless). The asymmetry is
inherited from the SPARQL semantics: MINUS without shared vars
is defined to be the identity; OPTIONAL without shared vars is a
cross product.

#### Today's restrictions

- **Each MINUS block holds exactly one triple pattern.** Same
  restriction as OPTIONAL today. Multi-pattern MINUS lands in a
  later slice.
- **Nested MINUS inside MINUS** isn't supported — only flat chains.

### Aggregates and GROUP BY

`pgrdf.sparql` supports the SPARQL set functions `COUNT` (with or
without `DISTINCT`), `SUM`, `AVG`, `MIN`, `MAX`, optionally with
`GROUP BY`. Each aggregate is bound to a SPARQL variable via the
`(EXPR AS ?var)` syntax in the SELECT clause.

```sql
-- Total triples in the database
SELECT * FROM pgrdf.sparql(
  'SELECT (COUNT(*) AS ?n) WHERE { ?s ?p ?o }'
);
--  → {"n": "9"}

-- Distinct subjects
SELECT * FROM pgrdf.sparql(
  'SELECT (COUNT(DISTINCT ?s) AS ?subjects) WHERE { ?s ?p ?o }'
);

-- Sum / Avg over numeric values (non-numeric literals are skipped)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT (SUM(?age) AS ?total) (AVG(?age) AS ?mean)
     WHERE { ?s foaf:age ?age }'
);

-- MIN/MAX lexicographically on the lexical value
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT (MIN(?n) AS ?lo) (MAX(?n) AS ?hi)
     WHERE { ?s foaf:name ?n }'
);

-- GROUP BY: count of triples per predicate
SELECT * FROM pgrdf.sparql(
  'SELECT ?p (COUNT(?o) AS ?n)
     WHERE { ?s ?p ?o }
   GROUP BY ?p'
);
--  → {"p": "http://xmlns.com/foaf/0.1/name", "n": "4"}
--  → {"p": "http://xmlns.com/foaf/0.1/age",  "n": "3"}
--  → {"p": "http://xmlns.com/foaf/0.1/mbox", "n": "2"}

-- GROUP BY + ORDER BY on the aggregate, then LIMIT
SELECT * FROM pgrdf.sparql(
  'SELECT ?p (COUNT(?o) AS ?n)
     WHERE { ?s ?p ?o }
   GROUP BY ?p
   ORDER BY DESC(?n) LIMIT 1'
);
```

#### How values come back

All aggregate values are emitted as JSON **strings** in the row's
JSONB output, consistent with the rest of `pgrdf.sparql`. For
numeric results, parse them on the caller side
(`CAST(j ->> 'total' AS NUMERIC)` in SQL, `int(row.sparql["n"])`
in Python, etc.).

#### SUM / AVG numeric awareness

`SUM(?v)` and `AVG(?v)` cast `?v` to `NUMERIC` if and only if
its dictionary entry's datatype is one of the XSD numeric IRIs
(`xsd:integer`, `xsd:decimal`, `xsd:double`, `xsd:float`, plus
the sized + unsigned + constraint subtypes). Non-numeric values
contribute `NULL` and are ignored by the aggregate per SQL
semantics — no Postgres cast error is raised. This matches the
FILTER ordering semantics.

If your data mixes string-encoded numbers (`"30"^^xsd:string`)
with proper numeric literals, only the latter contribute. Re-load
with explicit XSD datatype annotations to fix this in the
fixture rather than working around it in the query.

#### MIN / MAX caveat

`MIN(?v)` and `MAX(?v)` compare values **lexicographically on the
term's string form** — not type-aware. For string-typed literals
and IRIs this is the intuitive answer; for numeric data
`MAX("10", "2") = "2"` because `"2" > "1"` in lex order. Use
numeric `FILTER(?v >= …)` plus post-SQL `ORDER BY ... LIMIT 1` if
you need numeric extremes today. Type-aware MIN/MAX is queued.

#### Today's restrictions

- **`HAVING`** isn't translated yet. Filter the result with regular
  SQL after `pgrdf.sparql` (`SELECT * FROM pgrdf.sparql(...) j
  WHERE (j->>'n')::int > 5`).
- **`GROUP_CONCAT` and `SAMPLE`** are queued.
- **BIND** (`(EXPR AS ?v)` outside aggregates) isn't supported —
  only the aggregate-aliasing case lands today.
- **Aggregates on top of UNION** aren't supported. Aggregates over
  a UNION result require a derived-table refactor that lands in
  a later slice.

### Solution modifiers — DISTINCT / LIMIT / OFFSET / ORDER BY

The four classic SPARQL modifiers all land in the generated SQL:

```sql
-- DISTINCT — dedup on the projected variables
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT DISTINCT ?n WHERE { ?s foaf:name ?n }'
);

-- REDUCED — treated as DISTINCT (safe over-approximation per spec)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT REDUCED ?n WHERE { ?s foaf:name ?n }'
);

-- LIMIT — cap the number of returned rows
SELECT * FROM pgrdf.sparql(
  'SELECT ?s ?o WHERE { ?s ?p ?o } LIMIT 10'
);

-- OFFSET — skip rows from the start
SELECT * FROM pgrdf.sparql(
  'SELECT ?s ?o WHERE { ?s ?p ?o } OFFSET 10 LIMIT 10'
);

-- ORDER BY ?var — ascending lexicographic on lexical_value
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?n WHERE { ?s foaf:name ?n } ORDER BY ?n'
);

-- ORDER BY DESC(?var)
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?n WHERE { ?s foaf:name ?n } ORDER BY DESC(?n)'
);

-- ORDER BY ASC(?var), DESC(?other) — multiple sort keys
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n
     WHERE { ?s foaf:name ?n }
   ORDER BY ASC(?n) DESC(?s)'
);
```

#### How ORDER BY works under the hood

For each `ORDER BY ?var`, the translator emits

```sql
ORDER BY (SELECT lexical_value FROM pgrdf._pgrdf_dictionary
            WHERE id = qN.<col>) [ASC|DESC] NULLS LAST
```

If `?var` is in the SELECT list, the existing projected column is
reused (no extra subselect). If `?var` is bound in the BGP but
NOT projected, an extra hidden column is appended to the SELECT
list and ORDER BY references it by ordinal position. The
`execute` layer only emits the projected columns into JSONB, so
those hidden columns are invisible to callers.

This is **lexicographic order on the term's string form**, not
SPARQL's full type-aware ordering. For string-typed literals and
IRIs that's the same answer; for numeric literals it sorts as
strings (`"10"` < `"2"`), which is wrong. Use numeric FILTER plus
a Postgres `ORDER BY (sparql->>'n')::numeric` wrapping the
`pgrdf.sparql` call when you need numeric ordering today. Full
type-aware ORDER BY lands in a subsequent Phase 3 slice.

#### DISTINCT + ORDER BY interaction

If `ORDER BY` references a variable that's NOT in the SELECT list,
DISTINCT can't be applied — Postgres requires ORDER BY expressions
to appear in the select list when DISTINCT is used. pgRDF panics
with a clear message in that case rather than silently dropping
DISTINCT or the ORDER BY. Pull the variable into the SELECT clause
or remove DISTINCT.

### Combining with regular SQL

`pgrdf.sparql` is a SETOF function, so you can join its results with
relational tables, filter them with WHERE, aggregate them, anything:

```sql
-- Find FOAF persons whose name matches a regex
SELECT j->>'p' AS person
  FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?p ?n WHERE { ?p foaf:name ?n }'
  ) AS j
 WHERE j->>'n' ~* '^a';
--  → http://example.com/alice

-- Join SPARQL output to your relational data
WITH foaf AS (
  SELECT j->>'p' AS person_iri, j->>'n' AS name
    FROM pgrdf.sparql(
      'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
       SELECT ?p ?n WHERE { ?p foaf:name ?n }'
    ) AS j
)
SELECT customers.email, foaf.name
  FROM customers
  JOIN foaf ON customers.uri = foaf.person_iri;
```

## Inspecting queries before running them

`pgrdf.sparql_parse(q) → JSONB` returns the parsed shape without
executing. Use it when you want to know whether the translator can
handle a query, or to extract structure for code that builds queries:

```sql
SELECT pgrdf.sparql_parse(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n WHERE { ?s foaf:name ?n }'
);
-- {
--   "form": "SELECT",
--   "variables": ["s", "n"],
--   "bgp_pattern_count": 1,
--   "bgp_patterns": [
--     {"s": {"var": "s"},
--      "p": {"iri": "http://xmlns.com/foaf/0.1/name"},
--      "o": {"var": "n"}}
--   ],
--   "unsupported_algebra": []
-- }
```

If your query uses OPTIONAL / aggregates / property paths / etc.,
`unsupported_algebra` lists what the translator can't yet handle.
The query itself parses fine (spargebra is feature-complete) —
`pgrdf.sparql` just won't execute those forms yet:

```sql
SELECT pgrdf.sparql_parse(
  'SELECT ?s ?n WHERE { ?s ?p ?o OPTIONAL { ?s <http://x/n> ?n } }'
);
--  → {…, "unsupported_algebra": ["LeftJoin (OPTIONAL)"]}
```

FILTER is supported as of the Phase 3 first slice — the parser
walks through it without flagging. If the executor encounters a
FILTER expression shape it doesn't yet translate (e.g. numeric
ordering, `regex`), it errors with a clear message rather than
silently dropping the predicate.

## How the translation works

For the curious / debugging — the translator generates one
`_pgrdf_quads` alias per BGP pattern, joins shared variables via
equality predicates, and resolves constants to dictionary ids
*before* building the dynamic SQL. Worked example for

```sparql
PREFIX foaf: <http://xmlns.com/foaf/0.1/>
SELECT ?p ?n ?m
  WHERE { ?p foaf:name ?n .
          ?p foaf:mbox ?m }
```

becomes approximately

```sql
SELECT
  (SELECT lexical_value FROM pgrdf._pgrdf_dictionary WHERE id = q1.subject_id) AS "p",
  (SELECT lexical_value FROM pgrdf._pgrdf_dictionary WHERE id = q1.object_id)  AS "n",
  (SELECT lexical_value FROM pgrdf._pgrdf_dictionary WHERE id = q2.object_id)  AS "m"
FROM pgrdf._pgrdf_quads q1,
     pgrdf._pgrdf_quads q2
WHERE q1.predicate_id = 200    -- foaf:name's dict id
  AND q2.predicate_id = 201    -- foaf:mbox's dict id
  AND q2.subject_id   = q1.subject_id;   -- shared ?p anchor
```

Predicate / subject / object indexes on `_pgrdf_quads` (SPO, POS, OSP
covering indexes per the hexastore design) make those equality
lookups index-only scans. Dict resolution for the projected
variables uses a scalar subquery so any missing term ids come back
as NULL rather than dropping the row.

### Unknown terms are NULL, not error

If a constant in the query (predicate IRI, literal value, etc.) isn't
in the dictionary, the translator inlines `-1` as the dict id, which
matches no quad row → the query returns zero results. This is the
correct SPARQL semantics ("no solutions exist") rather than an
error condition:

```sql
SELECT count(*) FROM pgrdf.sparql(
  'SELECT ?s ?o WHERE { ?s <http://nope.example/never-loaded> ?o }'
);
--  → 0
```

## Performance posture (today)

| Cost | Where it shows up |
|---|---|
| 1× SPI lookup per **constant** in the BGP | At translation time, before the dynamic SQL runs. |
| Dynamic SQL via SPI executes against the partitioned hexastore | One PostgreSQL plan + execute per `pgrdf.sparql` call. |
| Dict round-trip for each projected variable in each output row | Scalar subquery on `_pgrdf_dictionary` (index-only scan on PK). |

For typical "100s of rows out" queries this is sub-millisecond on
local data. For "millions of rows out" the dict round-trips become
the dominant cost — a future optimisation is to hash-join the
dictionary upfront instead of per-row scalar subqueries; Phase 3.

The Postgres prepared-statement cache (LLD §4.2) is a Phase 2.3
delivery — once it lands, repeated `pgrdf.sparql` calls with the
same BGP shape skip the SQL parse + plan.

## Limits / gotchas

- **Blank nodes in queries are rejected.** SPARQL semantics treat
  `?b` and `_:b` as variables of different scoping rules; v0.2
  refuses blank-node terms in patterns to keep semantics unambiguous.
- **RDF-star quoted triples** are out of scope (LLD §2).
- **Cross-graph queries**: today every `pgrdf.sparql` call searches
  ALL graphs. Per-graph scoping (`GRAPH <g> { … }` and the dataset
  clause) arrives in Phase 3.
- **No SPARQL 1.2** anything yet — base SPARQL 1.1 only.

## Next

- [clients/python.md](clients/python.md) — calling `pgrdf.sparql`
  from Python.
- [clients/rust.md](clients/rust.md) — same from Rust.
- The engineering side: [`docs/03-query.md`](../docs/03-query.md)
  for the planner posture + Phase 3 roadmap.
