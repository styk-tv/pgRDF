# 03 — Querying with SPARQL

pgRDF runs SPARQL 1.1 inside PostgreSQL. Queries are SQL function
calls, so their results can be filtered, joined and aggregated with
ordinary SQL.

| Function | Query forms | Returns |
|---|---|---|
| `pgrdf.sparql(q)` | `SELECT`, `ASK`, and all `UPDATE` forms | one JSONB row per solution |
| `pgrdf.construct(q)` | `CONSTRUCT` | one JSONB row per triple |
| `pgrdf.describe(q)` | `DESCRIBE` | one JSONB row per triple |

The examples on this page use this data:

```sql
SELECT pgrdf.add_graph('http://example.org/people');
SELECT pgrdf.parse_turtle($$
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
ex:alice a foaf:Person ; foaf:name "Alice" ; foaf:age 34 ; foaf:knows ex:bob ;
         foaf:mbox <mailto:alice@example.org> .
ex:bob   a foaf:Person ; foaf:name "Bob"   ; foaf:age 41 ; foaf:knows ex:carol .
ex:carol a foaf:Person ; foaf:name "Carol" ; foaf:nick "CJ" .
$$, pgrdf.graph_id('http://example.org/people'));
```

## Results

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?name ?age WHERE { ?p foaf:name ?name OPTIONAL { ?p foaf:age ?age } }
$$);
--              sparql
-- --------------------------------
--  {"age": "34", "name": "Alice"}
--  {"age": "41", "name": "Bob"}
--  {"age": null, "name": "Carol"}
```

- Each row is a JSONB object keyed by variable name. The column is
  called `sparql`.
- Values are strings: IRIs, literals and numbers alike. Cast them in SQL
  when you need a number: `(sparql->>'age')::int`.
- A variable with no binding comes back as JSON `null`.
- `ASK` returns one row: `{"_ask": "true"}` or `{"_ask": "false"}`.

### Mixing SPARQL and SQL

```sql
-- filter and cast in SQL
SELECT sparql->>'name' AS name, (sparql->>'age')::int AS age
  FROM pgrdf.sparql($$
    PREFIX foaf: <http://xmlns.com/foaf/0.1/>
    SELECT ?name ?age WHERE { ?p foaf:name ?name ; foaf:age ?age }
  $$)
 WHERE (sparql->>'age')::int > 35;

-- join with a relational table
SELECT c.email, s.sparql->>'name' AS name
  FROM customers c
  JOIN pgrdf.sparql($$
         PREFIX foaf: <http://xmlns.com/foaf/0.1/>
         SELECT ?p ?name WHERE { ?p foaf:name ?name }
       $$) AS s(sparql) ON s.sparql->>'p' = c.person_iri;
```

## Graph patterns

| Feature | Example |
|---|---|
| Basic graph patterns, joins on shared variables | `?p foaf:knows ?q . ?q foaf:name ?n` |
| `OPTIONAL`, including multi-pattern and nested | `OPTIONAL { ?p foaf:mbox ?m }` |
| `UNION` (any number of branches) | `{ ?p foaf:name ?n } UNION { ?p foaf:nick ?n }` |
| `MINUS` | `?p a foaf:Person MINUS { ?p foaf:mbox ?m }` |
| `VALUES` | `VALUES ?p { ex:alice ex:carol }` |
| `BIND` | `BIND(UCASE(?n) AS ?shout)` |
| Expressions in the projection | `SELECT (?age * 12 AS ?months)` |
| Subqueries | `{ SELECT ?p WHERE { … } LIMIT 10 }` |
| Named graphs | `GRAPH <iri> { … }`, `GRAPH ?g { … }` |
| Property paths | `foaf:knows+`, `rdfs:subClassOf*`, `^foaf:knows`, `(ex:a\|ex:b)` |

```sql
-- People without an email address
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?name WHERE { ?p foaf:name ?name MINUS { ?p foaf:mbox ?m } }
$$);
--  {"name": "Bob"}
--  {"name": "Carol"}

-- A name or, failing that, a nickname
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?p ?label WHERE { { ?p foaf:name ?label } UNION { ?p foaf:nick ?label } }
$$);
```

## Filters and functions

`FILTER` and `BIND` accept different sets of functions:

| | In `FILTER` | In `BIND` and `SELECT` expressions |
|---|---|---|
| Comparison | `=` `!=` `<` `>` `<=` `>=`, `IN`, `sameTerm` | only as the condition of `IF` |
| Logic | `&&` `\|\|` `!` | — |
| Term tests | `isIRI`, `isLiteral`, `isBlank`, `BOUND` | — |
| Arithmetic | `+` `-` `*` `/`, `ABS`, `ROUND` | `+` `-` `*` `/`, `ABS`, `ROUND`, `CEIL`, `FLOOR` |
| Terms | `STR`, `LANG`, `DATATYPE` | `STR`, `LANG`, `DATATYPE` |
| Strings | `STRLEN`, `UCASE`, `LCASE`, `CONTAINS`, `STRSTARTS`, `STRENDS`, `REGEX` (with `"i"`) | `STRLEN`, `UCASE`, `LCASE`, `CONCAT` |
| Conditional | — | `IF` |

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?name ?band WHERE {
    ?p foaf:name ?name ; foaf:age ?age
    FILTER(REGEX(?name, "^a", "i") || ?age > 40)
    BIND(IF(?age >= 40, "40+", "under 40") AS ?band)
  }
$$);
--  {"band": "under 40", "name": "Alice"}
--  {"band": "40+", "name": "Bob"}
```

Two semantics to know:

- `=` compares RDF terms exactly: value, datatype and language tag.
  `"1"^^xsd:integer` and `"01"^^xsd:integer` are not `=`. Ordering
  operators (`<`, `>`, …) compare numeric literals by value.
- A comparison that doesn't type-check (a string against a number)
  drops the row instead of raising an error, as SPARQL specifies.

Anything outside this list is refused with a message naming the
expression. See [not supported](#not-supported).

## Aggregates and ordering

`COUNT` (with `DISTINCT` and `*`), `SUM`, `AVG`, `MIN`, `MAX`,
`GROUP_CONCAT` (with `SEPARATOR`) and `SAMPLE`, with `GROUP BY` and
`HAVING`. They also work over `UNION`.

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT (COUNT(?p) AS ?people) (AVG(?age) AS ?avg_age)
  WHERE { ?p foaf:name ?n OPTIONAL { ?p foaf:age ?age } }
$$);
--  {"people": "3", "avg_age": "37.5000000000000000"}
```

`ORDER BY` follows SPARQL's value ordering: numbers sort numerically,
`xsd:dateTime` values chronologically, strings by code point. It
accepts expressions and several keys, for example
`ORDER BY DESC(?age) ?name`. `DISTINCT`, `REDUCED`, `LIMIT` and
`OFFSET` all work.

## Named graphs

A query **without** a `GRAPH` clause matches triples in every graph.
To scope a query, name the graph:

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?name WHERE { GRAPH <http://example.org/people> { ?p foaf:name ?name } }
$$);
```

Or bind the graph to a variable to find out where each match lives:

```sql
SELECT * FROM pgrdf.sparql($$
  SELECT ?g (COUNT(*) AS ?triples) WHERE { GRAPH ?g { ?s ?p ?o } } GROUP BY ?g
$$);
--  {"g": "http://example.org/people", "triples": "16"}
```

- Triples inside one `GRAPH` block must all come from the same graph.
- Separate `GRAPH` blocks can name different graphs, for instance data
  in one graph enriched from another with
  `OPTIONAL { GRAPH <…/extra> { … } }`.
- An IRI that names no graph matches nothing. It is not an error.

> **`FROM` and `FROM NAMED` are not applied.** A dataset clause is
> currently ignored and the query runs against all graphs. Use `GRAPH`
> to scope queries.

## Property paths

```sql
-- everyone Alice reaches through foaf:knows, at any distance
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  PREFIX ex:   <http://example.org/>
  SELECT ?who WHERE { ex:alice foaf:knows+ ?who }
$$);
--  {"who": "http://example.org/carol"}
--  {"who": "http://example.org/bob"}
```

| Path | Meaning |
|---|---|
| `p+` | one or more steps |
| `p*` | zero or more steps (includes the start node) |
| `p?` | zero or one step |
| `^p` | the edge reversed |
| `p1\|p2` | either predicate; combines with the above, e.g. `(ex:a\|ex:b)+` |

Sequence paths (`p1/p2`) are not supported. Write each step as its
own triple pattern: `?a foaf:knows ?b . ?b foaf:name ?n`.

Recursive walks stop at `pgrdf.path_max_depth` (default 64). If a walk
is cut short you get a `WARNING`, and `pgrdf.last_call_stats()` reports
it. See [errors and diagnostics](08-errors-and-diagnostics.md#was-the-answer-complete).
After `materialize`, closure paths such as `rdfs:subClassOf+` are
answered from the materialized triples without walking.

## CONSTRUCT and DESCRIBE

`CONSTRUCT` and `DESCRIBE` produce triples. Each row describes its terms
with their type:

```sql
SELECT * FROM pgrdf.construct($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  PREFIX ex:   <http://example.org/>
  CONSTRUCT { ?a ex:colleagueOf ?b } WHERE { ?a foaf:knows ?b }
$$);
-- {"subject":   {"type": "iri", "value": "http://example.org/alice"},
--  "predicate": {"type": "iri", "value": "http://example.org/colleagueOf"},
--  "object":    {"type": "iri", "value": "http://example.org/bob"}}
-- ...

SELECT * FROM pgrdf.describe('DESCRIBE <http://example.org/carol>');
-- literals carry their datatype or language:
-- {"object": {"type": "literal", "value": "Carol",
--             "datatype": "http://www.w3.org/2001/XMLSchema#string"}, ...}
```

`DESCRIBE` returns the resource's Concise Bounded Description. To
store constructed triples in a graph, use `INSERT { … } WHERE { … }`
(below).

## SPARQL UPDATE

`pgrdf.sparql()` also runs updates. It returns one summary row:

```sql
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  PREFIX ex:   <http://example.org/>
  INSERT DATA { GRAPH <http://example.org/people> { ex:carol foaf:age 29 } }
$$);
-- {"_update": {"form": "INSERT_DATA", "triples_inserted": 1, "triples_deleted": 0,
--              "graphs_touched": ["http://example.org/people"], "elapsed_ms": 0.24}}
```

| Form | Example |
|---|---|
| `INSERT DATA` / `DELETE DATA` | ground triples, optionally inside `GRAPH <iri> { … }` |
| `INSERT { … } WHERE { … }` | `INSERT { GRAPH <g> { ?p ex:senior true } } WHERE { GRAPH <g> { ?p foaf:age ?a FILTER(?a > 40) } }` |
| `DELETE { … } WHERE { … }`, `DELETE WHERE { … }` | `DELETE WHERE { GRAPH <g> { ?p ex:senior ?x } }` |
| `DELETE { … } INSERT { … } WHERE { … }` | change a value in one atomic step |
| `WITH <iri>` | sets the graph for both the template and the `WHERE` |
| `CREATE` / `CLEAR` / `DROP GRAPH`, `DEFAULT` / `NAMED` / `ALL`, `SILENT` | graph management |

- `INSERT DATA` without a `GRAPH` block writes to the default graph
  (`0`).
- `INSERT DATA` of a triple that already exists is a no-op.
- Updates run inside your transaction, so `BEGIN … ROLLBACK` works.
- A write to a locked graph is refused with `55P03`. See
  [locking](05-graphs.md#locking-a-graph).

## Checking a query before running it

```sql
-- parse only: shape of the query, and anything unsupported
SELECT pgrdf.sparql_parse('SELECT * WHERE { SERVICE <http://example.org/q> { ?s ?p ?o } }');
-- {..., "unsupported_algebra": ["Service (federation)"]}

-- the SQL pgRDF would execute (use with EXPLAIN)
SELECT pgrdf.sparql_sql('SELECT ?s WHERE { ?s a <http://xmlns.com/foaf/0.1/Person> }');
```

## Not supported

These are refused with an error that names the construct, unless noted
otherwise:

| Construct | Instead |
|---|---|
| `SERVICE` (federated queries) | run the remote query separately |
| `FILTER EXISTS` / `FILTER NOT EXISTS` | a join, or `MINUS` / `OPTIONAL { … } FILTER(!BOUND(?x))` |
| `LANGMATCHES` | `LANG(?x) = "fr"` |
| `COALESCE`, `SUBSTR` and other functions not listed above | compute in SQL on the result rows |
| Blank nodes in query patterns (`_:x`, `[]`) | use a variable |
| Sequence paths (`foaf:knows/foaf:name`) | one triple pattern per step |
| `UNION` inside `OPTIONAL` | restructure as a top-level `UNION` |
| `VALUES` that binds a `GRAPH` variable, or a `FILTER` on it | list explicit `GRAPH <iri>` blocks, or filter the result rows in SQL |
| `BIND` inside a `UNION` branch; `FILTER` or `UNION` inside `MINUS` | move the `BIND` out of the branch; use several `MINUS` blocks |
| `ORDER BY` an expression together with `DISTINCT` | `BIND` the expression to a variable and order by that |
| `FROM` / `FROM NAMED` | **ignored, not refused**; use `GRAPH` |
| RDF-star quoted triples | not supported in data or queries |

## Performance tips

- Scope queries with `GRAPH <iri>` when you know where the data lives.
- Statistics refresh automatically after loads and `materialize`
  (setting `pgrdf.auto_analyze`).
- Repeated queries with the same shape reuse a cached plan, even when
  their constant IRIs or literals differ.
- `pgrdf.sparql_sql(q)` plus `EXPLAIN` shows what PostgreSQL does with
  a query. Queries also appear in `pg_stat_statements`.

## Next

[04 — Reasoning](04-reasoning.md)
