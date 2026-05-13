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
| `DISTINCT`, `REDUCED`, `ORDER BY`, `LIMIT/OFFSET` wrappers | ✅ (pass-through) |
| `FILTER`, `OPTIONAL`, `UNION`, `MINUS`, property paths, aggregates | ⏳ Phase 3 |
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

If your query uses FILTER / OPTIONAL / aggregates / etc.,
`unsupported_algebra` lists what the translator skipped. The query
itself parses fine (spargebra is feature-complete) — `pgrdf.sparql`
just won't execute those forms yet:

```sql
SELECT pgrdf.sparql_parse('SELECT ?s WHERE { ?s ?p ?o FILTER(isIRI(?o)) }');
--  → {…, "unsupported_algebra": ["Filter"]}
```

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
