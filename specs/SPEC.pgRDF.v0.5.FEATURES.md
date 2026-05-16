# **SPEC.pgRDF.v0.5.FEATURES**

**pgRDF — what you can actually do with it.**

*A capabilities entry point for project managers, data scientists,
ontologists, and anyone who wants to know what pgRDF lets them
build before reading a single line of low-level design.*

---

## 0. Document status

- **Status:** authoritative feature catalogue for the **v0.4 surface
  on `main` today**, plus the **v0.5 forward edge** (clearly marked).
  This document is the canonical "what" — not the "how". For the
  "how", read the authoritative shipped contract
  [`SPEC.pgRDF.LLD.v0.5.md`](SPEC.pgRDF.LLD.v0.5.md) (v0.5.0),
  [`SPEC.pgRDF.LLD.v0.4.md`](SPEC.pgRDF.LLD.v0.4.md) (the v0.4.x
  record), and the forward sibling
  [`SPEC.pgRDF.LLD.v0.6-FUTURE.md`](SPEC.pgRDF.LLD.v0.6-FUTURE.md).
- **Audience:** non-implementers.
  - **Project managers** — to scope what pgRDF can absorb from a
    backlog without bespoke services.
  - **Data scientists** — to know which graph operations are
    in-database (zero-copy SQL) versus external pipelines.
  - **Ontologists** — to know what slice of OWL 2 RL, SHACL Core,
    and SPARQL 1.1 actually runs at the storage layer.
- **Sibling docs:**
  - [`README.md`](../README.md) — the elevator pitch; this file
    expands each line of "What you can do today".
  - [`guide/`](../guide/) — runnable how-to (install, load, query,
    integrate from Python/Rust/Go/TS).
  - [`docs/`](../docs/) — engineering plan and architecture.
  - [`tests/`](../tests/) — the load-bearing evidence. Every
    feature in this catalogue has at least one
    `tests/regression/sql/` and/or
    `tests/w3c-sparql/<NN>-<name>/` fixture pinning the contract.
- **Tense:** **present tense** for everything callable on the v0.4
  cut that ships from `main` today. **Future tense** for the v0.5
  forward edge, marked inline as `🚀 v0.5`.

---

## 1. The four pillars at a glance

pgRDF is a single Postgres extension exposing four pillars under
the `pgrdf.*` schema. Each pillar is a real engine, not a stub.

| Pillar | What it gives you | Entry-point UDFs |
|---|---|---|
| **1 · Semantic storage** | RDF triples land in dictionary-encoded, partitioned Postgres tables you can `SELECT` from with vanilla SQL. Turtle in, quads out. | `pgrdf.load_turtle`, `pgrdf.parse_turtle`, `pgrdf.add_graph`, `pgrdf.count_quads` |
| **2 · Semantic query** | SPARQL 1.1 SELECT/ASK over those triples — multi-pattern joins, FILTER, OPTIONAL, UNION, MINUS, aggregates, BIND, GRAPH. Results come back as JSONB rows you can join with regular SQL. | `pgrdf.sparql`, `pgrdf.sparql_parse` |
| **3 · Semantic materialization** | OWL 2 RL forward-chaining inference. Implicit consequences (subclass, subproperty, equivalence, inverse, transitive) are written back into the same tables as queryable rows. | `pgrdf.materialize` |
| **4 · Semantic validation** | SHACL Core constraint checking. A graph + a shapes graph produce a W3C-shape `sh:ValidationReport` JSONB you can persist, alert on, or gate ingestion with. | `pgrdf.validate` |

The four pillars compose. The same `graph_id` you load Turtle into
is the one you `sparql` against, the one you `materialize`, and
the one you `validate`. Nothing leaves Postgres.

---

## 2. Pillar 1 — Semantic storage

### 2.1 Load any Turtle file from disk

**What:** A single UDF reads a `.ttl` file off the Postgres server
filesystem, parses it, dictionary-encodes every term, and inserts
the resulting quads in batches.

**Why you care:** Ontologists and data engineers stop writing
custom ETL for RDF. The entire FOAF / PROV-O / DCAT / SOSA /
SHACL / OWL ontology set parses with one statement and lives in
Postgres for the rest of the session.

```sql
-- Returns the count of triples loaded.
SELECT pgrdf.load_turtle('/fixtures/ontologies/foaf.ttl', 100);
--  → 631
```

**Evidence:** `tests/regression/sql/20-load-turtle.sql`. The
external ontology smoke suite (`tests/perf/smoke-ontologies.sh`)
parses **24 well-known ontologies → 17,134 triples** with locked
per-ontology counts in `smoke-ontologies.expected.tsv`.

### 2.2 Inline Turtle ingest from a string

**What:** Same parser, no filesystem dependency. Pass the Turtle
source as a SQL text literal.

**Why you care:** Notebooks and orchestration code can build
small graphs inline — fixture data for tests, prompt-driven
synthetic graphs, or seed data for a new tenant — without
landing files on the server.

```sql
SELECT pgrdf.parse_turtle('
@prefix ex: <http://example.com/> .
ex:alice ex:knows ex:bob .
', 1);
--  → 1
```

**Evidence:** `tests/regression/sql/30-sparql-parse.sql` (used as
the setup primitive throughout the regression suite).

### 2.3 Verbose ingest statistics

**What:** A sibling UDF returns a JSONB report from the same
ingest run: triples loaded, dictionary cache hits, dictionary DB
calls, batch count, elapsed milliseconds.

**Why you care:** Data engineers can measure ingest cost
empirically — verify the dictionary cache is hitting, decide
batch sizing, compare cold vs warm runs — without instrumenting
the application.

```sql
SELECT pgrdf.load_turtle_verbose(
  '/fixtures/ontologies/prov.ttl', 200,
  'http://www.w3.org/ns/prov#');
--  → {"triples": 1789, "dict_cache_hits": 4612,
--     "dict_db_calls": 783, "quad_batches": 2,
--     "elapsed_ms": 142.7}
```

**Evidence:** the dictionary cache and bulk-insert pipeline are
real; see §2.6 below. Same JSONB shape returned by
`pgrdf.parse_turtle_verbose` for the inline-string variant.

### 2.4 Per-graph LIST partitions

**What:** Each `graph_id` is its own Postgres LIST partition of
`_pgrdf_quads`. Creating a new partition is one UDF call; dropping
a whole graph is a partition detach, not a row scan.

**Why you care:** Project managers planning multi-tenant or
multi-ontology workloads get cheap, isolated namespaces. Drop a
tenant or roll back a load run by detaching one partition.

```sql
-- Add (or get) a partition for graph_id = 42.
SELECT pgrdf.add_graph(42);
SELECT pgrdf.count_quads(42);
```

**Evidence:** `tests/regression/sql/11-quads-basic.sql`,
`12-graphs.sql`.

### 2.5 Named graphs — IRI ↔ graph_id mapping

**What:** Every graph has a stable IRI and a stable integer id.
The `_pgrdf_graphs` table is the symmetric lookup; helper UDFs
resolve in either direction.

**Why you care:** Ontologists author against IRIs
(`http://example.org/ontology/v3`). Engineers store integers.
pgRDF gives both surfaces and keeps them in sync, so SPARQL
`GRAPH <iri> { … }` clauses route to the correct partition
without bookkeeping in the application.

```sql
-- Allocate a graph and bind it to an IRI in one call.
SELECT pgrdf.add_graph('http://example.org/g1');           --  → 1
SELECT pgrdf.add_graph(7, 'http://example.org/snapshot');  --  → 7

-- Either direction.
SELECT pgrdf.graph_id('http://example.org/g1');            --  → 1
SELECT pgrdf.graph_iri(7);                                 --  → 'http://example.org/snapshot'
```

**Evidence:** `tests/regression/sql/72-graphs-table-shape.sql`
through `77-graph-iri-lookup.sql`.

### 2.6 Hexastore covering indexes + dictionary

**What:** Every quad is stored once, indexed three ways (SPO,
POS, OSP). Every IRI, blank node, and literal is interned in
`_pgrdf_dictionary` with an `i64` id; quads are tuples of four
ids, not strings.

**Why you care:** Data scientists writing ad-hoc joins from the
quad table get the right index for any bound-variable shape
without hinting. Storage cost stays near-optimal because the
text of every IRI lives once, not once per triple.

```sql
-- Inspect the dictionary directly (IRIs == term_type 1).
SELECT * FROM pgrdf._pgrdf_dictionary
 WHERE term_type = 1 LIMIT 5;
```

**Evidence:** `tests/regression/sql/10-dict-roundtrip.sql`. The
covering indexes are pinned by index DDL in
`sql/pgrdf--*.sql` and exercised by every SPARQL test.

### 2.7 Typed literals, language tags, blank nodes, RDF lists

**What:** Turtle ingest preserves the full term-type space:
plain literals, datatyped literals (`"42"^^xsd:integer`),
language-tagged literals (`"colour"@en-GB`), blank nodes, and
RDF collections (`( a b c )` syntax).

**Why you care:** Ontologists ingesting real-world ontologies
(OWL, SHACL, PROV-O) get all four term species round-tripped.
Data scientists doing numeric or language-conditional queries
get type-aware comparison out of the box.

**Evidence:** `tests/regression/sql/21-typed-literals.sql`,
`22-lang-tags.sql`, `23-blank-nodes.sql`, `24-rdf-list.sql`.

### 2.8 Bulk-ingest with prepared `INSERT`

**What:** Ingest binds a single prepared statement once and
re-executes per batch — Phase A of the bulk-insert pipeline.
Combined with the shared-memory dictionary cache (§2.9), large
ontologies load orders of magnitude faster than the row-by-row
baseline.

**Why you care:** Operators loading 100k+ triple graphs into a
cold instance see ingest dominated by parse cost, not insert
cost.

```sql
-- 2 MB Turtle file; one transaction; observable batch count.
SELECT pgrdf.load_turtle_verbose('/fixtures/large.ttl', 9);
-- "quad_batches": 19, "elapsed_ms": ...
```

**Evidence:** `tests/regression/sql/25-bulk-ingest.sql`,
`52-bulk-ingest-perf.sql`.

### 2.9 Shared-memory dictionary cache

**What:** The dictionary `id ↔ term` mapping is mirrored in
Postgres shared memory across backends. Repeated ingest of the
same IRIs (any real ontology — the same `rdf:type`,
`rdfs:subClassOf`, FOAF predicates appear thousands of times)
becomes a memory lookup, not a SQL roundtrip.

**Why you care:** Throughput scales with cache hit rate. The
verbose-ingest report (§2.3) shows the hit rate empirically.
Project managers can plan capacity from observed numbers.

```sql
-- Inspect cache size, hit/miss counters.
SELECT pgrdf.stats() -> 'shmem_dict_cache';
```

**Evidence:** `tests/regression/sql/50-shmem-dict-cache.sql`,
`63-shmem-reset-invalidation.sql`.

---

## 3. Pillar 2 — Semantic query (SPARQL 1.1)

`pgrdf.sparql(q TEXT)` parses SPARQL with the `spargebra` algebra
library, translates the algebra to dynamic SQL against the quad
tables, executes it, and returns one JSONB row per solution.
Solution variables become JSONB keys; unbound variables come
through as `null`.

`pgrdf.sparql_parse(q TEXT)` returns the parsed shape (form,
projection, BGP triple count, modifiers, unsupported-algebra
tags) without executing — useful for tools and validation
upstream of `sparql`.

### 3.1 Multi-pattern BGPs become real SQL joins

**What:** N triple patterns with shared variables compile to N-way
self-joins of `_pgrdf_quads`, with the right hexastore index per
pattern.

**Why you care:** Data scientists write graph-shaped joins as
SPARQL; pgRDF runs them as SQL with Postgres' actual planner.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?p ?n ?m
     WHERE { ?p foaf:name ?n .
             ?p foaf:mbox ?m }');
--  → {"p": "http://example.com/alice", "n": "Alice", "m": "mailto:a@x"}
```

**Evidence:** `tests/w3c-sparql/01-basic-bgp/`,
`tests/regression/sql/32-sparql-multipattern.sql`.

### 3.2 FILTER — boolean composition over solutions

**What:** SPARQL `FILTER` expressions: equality (`=`, `!=`),
ordering (`<`, `>`, `<=`, `>=`), boolean composition (`&&`,
`||`, `!`), term-type tests (`isIRI`, `isLiteral`, `isBlank`),
`bound`, `in`, `regex(?v, "pat", "i")`, numeric comparison on
typed literals, string functions (`STRLEN`, `UCASE`, `LCASE`,
`STR`, `LANG`).

**Why you care:** Ontologists carve subgraphs without writing
SQL `CASE` ladders. Data scientists get text-search and numeric
predicates inside the SPARQL pattern instead of a post-process.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n
     WHERE { ?s foaf:name ?n .
             ?s <http://example.com/age> ?age
             FILTER(?age >= 30 && REGEX(?n, "^A", "i")) }');
```

**Evidence:** `tests/regression/sql/33-sparql-filter.sql`,
`34-sparql-filter-advanced.sql`,
`41-sparql-expressions.sql`,
`tests/w3c-sparql/06-filter-isiri/`, `14-filter-regex/`,
`15-filter-in/`, `21-numeric-filter/`, `17-lang-tag/`,
`16-strlen/`, `18-ucase/`, `20-str-iri/`.

### 3.3 OPTIONAL — left-outer-join semantics

**What:** `OPTIONAL { … }` is a left outer join. Variables only
bound in the optional branch come through as `null` for
solutions where the branch didn't match.

**Why you care:** Real data is sparse. A person may not have a
mailbox; a paper may not have an abstract. OPTIONAL is how
SPARQL says "include them anyway with the field blank" — pgRDF
honours that natively.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?m
     WHERE { ?s foaf:name ?n
             OPTIONAL { ?s foaf:mbox ?m } }');
--  → {"s": "...alice", "n": "Alice", "m": "mailto:a@x"}
--  → {"s": "...bob",   "n": "Bob",   "m": null}
```

**Evidence:** `tests/w3c-sparql/04-optional-chain/`,
`tests/regression/sql/36-sparql-optional.sql`,
`tests/w3c-sparql/19-bound-after-optional/` (the canonical
`!BOUND(?v)` "find unmatched rows" idiom).

### 3.4 UNION — disjoint pattern alternatives

**What:** `{ A } UNION { B }` is a real set union of solution
mappings. Variables only present in one branch are `null` in
solutions from the other.

**Why you care:** Lets ontologists express "either of these
shapes match" without two separate queries — important when
folding alternative property names or class subdivisions.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?n ?m
     WHERE { { ?s foaf:name ?n }
             UNION
             { ?s foaf:mbox ?m } }');
```

**Evidence:** `tests/w3c-sparql/03-union-disjoint/`,
`tests/regression/sql/37-sparql-union.sql`.

### 3.5 MINUS — set difference

**What:** `MINUS { … }` removes solutions that are compatible
with any binding of the MINUS branch. With no shared variables,
MINUS elides (W3C §8.3.2).

**Why you care:** "Everyone who is X but not Y" without a
correlated subquery.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s WHERE { ?s a foaf:Person
                     MINUS { ?s foaf:mbox ?m } }');
```

**Evidence:** `tests/w3c-sparql/05-minus-no-shared/`,
`tests/regression/sql/38-sparql-minus.sql`,
`43-sparql-minus-multi.sql`.

### 3.6 Aggregates + GROUP BY

**What:** `COUNT`, `SUM`, `AVG`, `MIN`, `MAX`, `GROUP_CONCAT`,
`SAMPLE` — all standard SPARQL 1.1 aggregates. `MIN`/`MAX` are
**type-aware**: they pick numeric ordering over numeric literals
and lexical ordering over strings.

**Why you care:** Counts per predicate, sums per category,
distinct value samples — these are the bread-and-butter of any
graph-shape report. They run inside SPARQL, no client-side
post-process.

```sql
SELECT * FROM pgrdf.sparql(
  'SELECT ?p (COUNT(?o) AS ?n)
     WHERE { ?s ?p ?o }
   GROUP BY ?p ORDER BY DESC(?n)');
--  → {"p": "http://xmlns.com/foaf/0.1/name", "n": "4"}
```

**Evidence:** `tests/w3c-sparql/07-aggregates-count/`,
`23-min-max-numeric/`, `tests/regression/sql/39-sparql-aggregates.sql`.

### 3.7 HAVING — post-aggregate FILTER

**What:** `HAVING` filters after grouping. Both shapes work:
referring to the alias (`HAVING(?friends > 1)`) and inline
aggregate (`HAVING(SUM(?v) > 10)`).

**Why you care:** "Predicates with more than N usages",
"properties whose value-sum exceeds a threshold" — single-query
answerable.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s (COUNT(?o) AS ?friends)
     WHERE { ?s foaf:knows ?o }
   GROUP BY ?s HAVING(?friends > 1)');
```

**Evidence:** `tests/w3c-sparql/08-aggregates-having/`,
`22-having-inline-aggregate/`,
`tests/regression/sql/40-sparql-having.sql`.

### 3.8 BIND — project computed values

**What:** `BIND(expr AS ?v)` adds a new variable bound to a
SPARQL expression — string concatenation, arithmetic, IRI
manipulation — visible in subsequent patterns and in the SELECT
projection.

**Why you care:** Compose a display name from given/family name,
build a derived IRI, attach a constant tag — all in-query.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   SELECT ?s ?full
     WHERE { ?s foaf:givenName ?g .
             ?s foaf:familyName ?fam .
             BIND(CONCAT(?g, " ", ?fam) AS ?full) }');
```

**Evidence:** `tests/w3c-sparql/11-bind-concat/`,
`tests/regression/sql/42-sparql-bind.sql`.

### 3.9 Solution modifiers — DISTINCT / ORDER BY / LIMIT / OFFSET

**What:** The canonical SPARQL post-processing: `DISTINCT`,
`ORDER BY` (asc/desc, type-aware), `LIMIT`, `OFFSET`.

**Why you care:** Pagination, deduplication, ranking — without
wrapping the call in an outer SQL `SELECT`.

```sql
SELECT * FROM pgrdf.sparql(
  'SELECT DISTINCT ?p WHERE { ?s ?p ?o }
   ORDER BY ?p LIMIT 20 OFFSET 100');
```

**Evidence:** `tests/w3c-sparql/02-distinct/`,
`09-order-by-desc/`, `10-limit-offset/`,
`tests/regression/sql/35-sparql-modifiers.sql`.

### 3.10 ASK — boolean queries

**What:** `ASK { … }` returns `true` iff the pattern has at
least one solution.

**Why you care:** Existence checks ("does this entity have any
foaf:mbox?") without scanning solutions.

```sql
SELECT * FROM pgrdf.sparql(
  'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
   ASK { ?s a foaf:Person }');
--  → {"ask": true}
```

**Evidence:** `tests/w3c-sparql/12-ask-true/`, `13-ask-false/`,
`tests/regression/sql/44-sparql-ask.sql`.

### 3.11 GRAPH `<iri> { … }` — named-graph scoping

**What:** A SPARQL `GRAPH <iri> { … }` pattern restricts the
inner BGP to the partition bound to that IRI (via §2.5).

**Why you care:** Multi-graph datasets — "what does the
v3-snapshot graph say about subject X" vs "what does the live
graph say" — query each scope explicitly with no global state.

```sql
SELECT * FROM pgrdf.sparql(
  'SELECT ?s ?p ?o
     WHERE { GRAPH <http://example.org/g1> { ?s ?p ?o } }');
```

**Evidence:** `tests/regression/sql/78-sparql-graph-literal-iri.sql`.

**🚀 v0.5 forward edge:** `GRAPH ?g { … }` (variable graph)
joins across graphs and projects the graph IRI as a solution
variable — landing in the v0.4 cycle as slice 113 per
[`SPEC.pgRDF.LLD.v0.4 §3.3`](SPEC.pgRDF.LLD.v0.4.md).

### 3.12 Parse without executing — `pgrdf.sparql_parse`

**What:** Returns the parsed-algebra shape as JSONB —
form (SELECT / ASK / UPDATE / CONSTRUCT / DESCRIBE), projection
variables, BGP pattern count, modifiers in use, list of
unsupported algebra operators encountered.

**Why you care:** Application-level validators and SPARQL
linters can statically reject queries that use unsupported
features before they hit the database, with a stable JSON
contract.

```sql
SELECT pgrdf.sparql_parse(
  'SELECT ?s WHERE { ?s ?p ?o
                     OPTIONAL { ?s <http://x/n> ?n } }');
--  → {"form": "SELECT", "projection": ["s"],
--     "bgp_triples": 1, "modifiers": ["LeftJoin"],
--     "unsupported_algebra": []}
```

**Evidence:** `tests/regression/sql/30-sparql-parse.sql`,
`66-parse-sparql-roundtrip.sql`.

### 3.13 Negative-shape contract — stable error messages

**What:** Unsupported SPARQL shapes emit *stable error prefixes*
locked by regression tests, so client code can match on them.

**Why you care:** Tooling can branch on the failure mode —
"this query uses CONSTRUCT, route to the v0.5 endpoint" — instead
of brittle substring guessing.

**Evidence:** `tests/regression/sql/80-unsupported-shapes.sql`
(7 locked negative signals); `81-error-paths.sql` (UDF error
prefixes, e.g. `load_turtle: failed to open`).

### 3.14 🚀 v0.5 forward edge — SPARQL UPDATE / CONSTRUCT / paths

The next surface increments are tracked in
[`SPEC.pgRDF.LLD.v0.4 §4 / §6 / §7`](SPEC.pgRDF.LLD.v0.4.md):

- **SPARQL UPDATE** — `INSERT DATA`, `DELETE DATA`,
  `INSERT … WHERE`, `DELETE … WHERE`, plus graph-scoped variants.
- **CONSTRUCT** — `pgrdf.construct(q TEXT) → SETOF JSONB`
  returning `{subject, predicate, object}`.
- **Property paths** — `p*`, `p+`, `p?`, `^p`, with `p1|p2`
  alternation.

These are not yet on `main`; they are in the v0.4 cycle. This
spec marks them so PMs scoping near-term work know what is
*about to* land.

---

## 4. Pillar 3 — Semantic materialization (OWL 2 RL inference)

`pgrdf.materialize(graph_id BIGINT) → JSONB` runs OWL 2 RL
forward-chaining inference (via the `reasonable` reasoner) over
the named graph, and writes every entailed triple back to the
same graph with `is_inferred = TRUE`. The call is **idempotent**:
re-running drops previously inferred rows first and replaces
them.

### 4.1 The mental model

You load an ontology + some assertions. Behind every assertion
there is a chain of entailments — subclass closures, subproperty
closures, equivalence cycles, inverse-property completions,
transitive-property propagations, individual-equality unfoldings.
`pgrdf.materialize` writes that chain back into the graph as
real rows you can `SELECT` and `sparql` against.

The output is a JSONB summary with `base_triples`,
`inferred_triples_written`, `total_after`, and per-stage timing.

### 4.2 Worked example — subclass chain

```sql
SELECT pgrdf.add_graph(100);
SELECT pgrdf.parse_turtle('
@prefix ex:   <http://example.com/> .
@prefix rdf:  <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Engineer rdfs:subClassOf ex:Person .
ex:Person   rdfs:subClassOf ex:Agent .
ex:alice    rdf:type        ex:Engineer .
', 100);

SELECT pgrdf.materialize(100);
--  → {"base_triples": 3, "inferred_triples_written": 11, ...}

SELECT * FROM pgrdf.sparql(
  'PREFIX rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#>
   PREFIX ex:  <http://example.com/>
   SELECT ?c WHERE { ex:alice rdf:type ?c }');
--  → {"c": "http://example.com/Engineer"}  ← base
--  → {"c": "http://example.com/Person"}    ← inferred
--  → {"c": "http://example.com/Agent"}     ← inferred
```

The 2-hop subclass entailment is now a queryable row, not a
runtime computation.

### 4.3 What OWL 2 RL gives ontologists

The `reasonable` engine implements the OWL 2 RL/RDF rule set.
The rules that materialize most often in practice (the ones
ontologists rely on when authoring an ontology against a
production graph):

| Vocabulary | Effect on the graph |
|---|---|
| `rdfs:subClassOf` (with closure) | Every instance of a subclass becomes a typed instance of every ancestor class. |
| `rdfs:subPropertyOf` (with closure) | Every triple using a sub-property is replicated using each super-property. |
| `rdfs:domain` / `rdfs:range` | Subjects acquire the domain class; objects acquire the range class. |
| `owl:equivalentClass` | Membership is mirrored both ways. |
| `owl:equivalentProperty` | Triples are mirrored both ways. |
| `owl:inverseOf` | `(?a p ?b)` materialises `(?b p⁻¹ ?a)`. |
| `owl:TransitiveProperty` | `(a p b), (b p c)` materialises `(a p c)`, transitively. |
| `owl:SymmetricProperty` | `(a p b)` materialises `(b p a)`. |
| `owl:sameAs` | Property assertions are replicated across the equivalence class. |
| `owl:FunctionalProperty` / `owl:InverseFunctionalProperty` | Drives `sameAs` consequences when applicable. |

**Why you care:** Ontologists publish models in OWL 2 RL because
the rule set is sound, complete, and decidable in polynomial
time — and pgRDF runs it natively. Application queries written
against the *surface* shape (e.g. `?s rdf:type ex:Agent`) match
implicit instances without the application implementing the
closure itself.

**Evidence:** `tests/regression/sql/60-materialize-owl-rl.sql`,
`61-materialize-then-sparql.sql`, `62-materialize-empty.sql`
(idempotence + empty-graph correctness).

### 4.4 Idempotence and operator safety

**What:** Re-running `pgrdf.materialize(g)` deletes the prior
`is_inferred = TRUE` rows in `g` before running again. The
base graph is untouched.

**Why you care:** Operators can wire materialization into a
cron or trigger without worrying about duplicate inferred rows
or drift between the base and inferred sets.

**Evidence:** `tests/regression/sql/62-materialize-empty.sql`
(asserts double-run yields the same shape;
`base_triples = 0` on an empty graph; non-negative inferred
count).

### 4.5 ✅ v0.5 — profile selector (shipped in v0.5.0)

`pgrdf.materialize(graph_id, profile TEXT DEFAULT 'owl-rl')`
lets consumers pick `'rdfs'` or `'owl-rl'` per call (the
reserved-future `'owl-rl-ext'` is named but not yet wired).
See [`SPEC.pgRDF.LLD.v0.5.md §3`](SPEC.pgRDF.LLD.v0.5.md).

---

## 5. Pillar 4 — Semantic validation (SHACL Core)

`pgrdf.validate(data_graph_id BIGINT, shapes_graph_id BIGINT)
→ JSONB` validates the data graph against the shapes graph and
returns a W3C `sh:ValidationReport`-shape JSONB document.
Implementation: the `shacl` 0.3.x crate from the rudof project,
unblocked via a patched `reasonable` fork — see
[`ERRATA.v0.4 E-011`](ERRATA.v0.4.md).

### 5.1 The mental model

You author a SHACL shapes graph (Turtle) that says "every
foaf:Person must have at least one foaf:name, and the name must
be a string". You load both the data graph and the shapes
graph. `pgrdf.validate` returns a report listing every node
that doesn't satisfy a constraint, with the offending value,
the path, and the violated constraint component.

### 5.2 Worked example — minCount + datatype + nodeKind

```sql
SELECT pgrdf.add_graph(1);  -- data
SELECT pgrdf.add_graph(2);  -- shapes

SELECT pgrdf.parse_turtle('
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix ex:   <http://example.com/> .
ex:alice a foaf:Person ;
    foaf:name "Alice" ;
    foaf:mbox <mailto:alice@example.com> .
ex:bob   a foaf:Person ;
    foaf:name "Bob" .   -- intentionally missing foaf:mbox
', 1);

SELECT pgrdf.parse_turtle('
@prefix sh:   <http://www.w3.org/ns/shacl#> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
@prefix ex:   <http://example.com/> .
@prefix xsd:  <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape a sh:NodeShape ;
    sh:targetClass foaf:Person ;
    sh:property [ sh:path foaf:name ;
                  sh:minCount 1 ;
                  sh:datatype xsd:string ] ;
    sh:property [ sh:path foaf:mbox ;
                  sh:minCount 1 ;
                  sh:nodeKind sh:IRI ] .
', 2);

SELECT pgrdf.validate(1, 2);
--  → {
--    "conforms": false,
--    "results": [
--      { "focusNode": "http://example.com/bob",
--        "resultPath": "http://xmlns.com/foaf/0.1/mbox",
--        "sourceConstraintComponent":
--          "http://www.w3.org/ns/shacl#MinCountConstraintComponent",
--        "resultSeverity": "http://www.w3.org/ns/shacl#Violation",
--        "resultMessage": "Less than 1 value" }
--    ] }
```

### 5.3 What SHACL Core gives ontologists

SHACL Core is the constraint language ontologists pair with
their OWL ontologies. pgRDF supports the core constraint
components consumers actually use against production data:

| Component | What it asserts |
|---|---|
| `sh:targetClass` | Pick every instance of a class as a focus node. |
| `sh:targetNode` / `sh:targetSubjectsOf` / `sh:targetObjectsOf` | Other ways to pick focus nodes. |
| `sh:path` | The property path under inspection (simple paths). |
| `sh:minCount` / `sh:maxCount` | Cardinality bounds. |
| `sh:datatype` | Value must be a literal of the given datatype. |
| `sh:nodeKind` | Value must be `sh:IRI` / `sh:Literal` / `sh:BlankNode` / unions thereof. |
| `sh:class` | Value must be an instance of the given class. |
| `sh:in` | Value must be one of an enumerated list. |
| `sh:minLength` / `sh:maxLength` | String-length bounds on literal lexical form. |
| `sh:pattern` | Regular-expression match against literal lexical form. |
| `sh:minInclusive` / `sh:maxInclusive` / `sh:minExclusive` / `sh:maxExclusive` | Numeric range bounds. |

**Why you care:**
- **Project managers** — a SHACL graph is a *machine-readable
  acceptance criterion*. Either a data load conforms or it
  doesn't; the report is auditable evidence.
- **Data scientists** — a quick, declarative gate on incoming
  data quality without hand-rolled checks.
- **Ontologists** — the canonical pairing for an OWL ontology;
  pgRDF lets you ship both inside the same Postgres instance.

**Evidence:** `tests/regression/sql/71-shacl-real.sql` exercises
the real engine end-to-end with a violating case; the JSONB
report shape is locked. Earlier `70-validate-stub.sql` is kept
as a back-compat signal that the surface signature didn't
break when the v0.3 stub was replaced.

### 5.4 Validation report as data

The report is JSONB, so the violations are queryable:

```sql
WITH r AS (SELECT pgrdf.validate(1, 2) AS rep)
SELECT v ->> 'focusNode' AS who,
       v ->> 'resultMessage' AS why
  FROM r, jsonb_array_elements(r.rep -> 'results') v;
```

**Why you care:** Operators can route violations to a
notification, archive them in an audit table, or aggregate
them across runs — without parsing a non-database artefact.

### 5.5 ✅ v0.5 — SHACL `mode` arg + W3C SHACL Core manifest gate (shipped in v0.5.0)

v0.5 ships the **`pgrdf.validate(data, shapes, mode)`** argument
and wires the **W3C SHACL Core manifest runner** into CI as a real
gate — a genuine 25/25 full-pass. SHACL-SPARQL *constraint
execution* (`mode => 'sparql'`) is a documented upstream-gate:
`shacl 0.3.1` has no SHACL-SPARQL constraint component and its
SPARQL engine is an upstream stub, so the `'sparql'` surface ships
honest + forward-compatible (ERRATA.v0.5 **E-012**), NOT a pgRDF
defect.
See [`SPEC.pgRDF.LLD.v0.5.md §5–§6`](SPEC.pgRDF.LLD.v0.5.md).

---

## 6. Cross-cutting capabilities

These are not a fifth pillar — they're the seams that let the
four pillars compose cleanly inside Postgres.

### 6.1 Operator observability — `pgrdf.stats()`

**What:** A single UDF returns a JSONB snapshot of internal
state: dictionary cache size and hit rate, prepared-plan cache
size, last-run ingest stats, build metadata.

**Why you care:** Operators get a stable health surface they
can scrape into Prometheus / pg_stat_statements pipelines.

```sql
SELECT pgrdf.stats();
```

**Evidence:** `tests/regression/sql/82-stats-shape.sql` locks
the JSONB key contract.

### 6.2 Cache and plan-cache control

**What:** `pgrdf.shmem_reset()` clears the shared-memory
dictionary cache. `pgrdf.plan_cache_clear()` clears the
prepared-plan cache and returns the count cleared.

**Why you care:** Test harnesses, migration scripts, and
operators need explicit cache invalidation around schema or
extension upgrades.

**Evidence:** `tests/regression/sql/63-shmem-reset-invalidation.sql`,
`64-plan-cache-clear.sql`.

### 6.3 Prepared-plan cache for SPARQL

**What:** Translated SPARQL → SQL plans are cached per backend
keyed on the SPARQL text. Repeat invocations of the same query
skip parse + translate.

**Why you care:** Hot queries (dashboards, REST endpoints,
periodic checks) pay the parse cost once per connection, not
per call.

**Evidence:** `tests/regression/sql/51-plan-cache.sql`.

### 6.4 Compose with regular SQL

**What:** `pgrdf.sparql(...)` is a set-returning function. You
can `JOIN` it to a regular Postgres table, use it as a CTE,
funnel its result into `INSERT INTO ... SELECT`, or wrap it in
a Postgres view.

**Why you care:** Data scientists keep their feature pipelines
in SQL. The graph is just another table.

```sql
WITH friends AS (
  SELECT * FROM pgrdf.sparql(
    'PREFIX foaf: <http://xmlns.com/foaf/0.1/>
     SELECT ?a ?b WHERE { ?a foaf:knows ?b }')
)
SELECT a.region, COUNT(*) AS knows_count
  FROM friends f
  JOIN app.users a ON a.iri = f."a"
 GROUP BY a.region;
```

### 6.5 Multi-version Postgres support

pgRDF builds against **PostgreSQL 14, 15, 16, and 17**. The full
test bar runs across all four in CI. PG 18 adoption is deferred —
see [`ERRATA.v0.2 E-006`](ERRATA.v0.2.md).

### 6.6 Drop-in install on stock Postgres images

pgRDF installs by binding three artefacts (`pgrdf.so`,
`pgrdf.control`, `pgrdf--*.sql`) onto a stock `postgres:17.4`
image — no image rebuild. K8s variants land the same artefacts
via an init container. See
[`SPEC.pgRDF.INSTALL.v0.2.md`](SPEC.pgRDF.INSTALL.v0.2.md).

---

## 7. Test evidence at a glance

**This is not a test grid** — it's the load-bearing receipt for
every feature above. Numbers as of the v0.4 cycle on `main`:

| Layer | Count | What it gates |
|---|---|---|
| pgrx integration tests | 94 | UDF correctness inside a managed PG. |
| pg_regress-style | 40 | UDF correctness over the wire to compose Postgres (PG 17). |
| W3C-shape SPARQL | 23 | One-folder-per-test conformance fixtures, lexicographically sorted bag comparison. |
| LUBM-shape | 3 | LUBM-style correctness gates. |
| Ontology smoke | 24 ontologies / 17,134 triples | Real-world Turtle parses cleanly. |
| **Total green** | **160 tests** | Across PG 14-17 and the compose-based runtime. |

Runnable entry points (see [`Justfile`](../Justfile) for the
full set):

- `just test` — pgrx layer.
- `just test-regression` — compose-based pg_regress.
- `just test-w3c` — W3C-shape SPARQL conformance.
- `just test-lubm` — LUBM-shape correctness.
- `just test-conformance` — regression + W3C + LUBM.
- `just test-everything` — the broadest sweep.
- `just smoke-cold` — wipe compose, rebuild, re-up, re-run.

---

## 8. Where to go next

- **First-run walkthrough:** [`guide/00-intro.md`](../guide/00-intro.md)
  → [`guide/01-install.md`](../guide/01-install.md)
  → [`guide/02-loading-rdf.md`](../guide/02-loading-rdf.md)
  → [`guide/03-querying.md`](../guide/03-querying.md).
- **Application integration:** [`guide/clients/python.md`](../guide/clients/python.md),
  [`rust.md`](../guide/clients/rust.md),
  [`typescript.md`](../guide/clients/typescript.md),
  [`go.md`](../guide/clients/go.md).
- **Authoritative contracts:**
  - [`SPEC.pgRDF.LLD.v0.5.md`](SPEC.pgRDF.LLD.v0.5.md) — the
    authoritative shipped contract (v0.5.0): profile selector,
    TriG/N-Quads ingest, SHACL `mode` arg, W3C SHACL Core manifest
    gate, IRI-overload ergonomics, agg-over-UNION residuals.
  - [`SPEC.pgRDF.LLD.v0.4.md`](SPEC.pgRDF.LLD.v0.4.md) — the
    v0.4.x-cut record (named graphs, UPDATE, lifecycle UDFs,
    CONSTRUCT, paths, SHACL real-impl).
  - [`SPEC.pgRDF.LLD.v0.6-FUTURE.md`](SPEC.pgRDF.LLD.v0.6-FUTURE.md)
    — forward edge (v1.0: RDF 1.2 triple terms, incremental
    materialisation, federated SERVICE, the post-v0.5.0
    `executor.rs` carve).
  - [`SPEC.pgRDF.INSTALL.v0.2.md`](SPEC.pgRDF.INSTALL.v0.2.md)
    — drop-in install on stock PG containers.
  - [`ERRATA.v0.2.md`](ERRATA.v0.2.md),
    [`ERRATA.v0.4.md`](ERRATA.v0.4.md) — implementation
    corrections.
- **Engineering plan:** [`docs/01-architecture.md`](../docs/01-architecture.md)
  → [`02-storage.md`](../docs/02-storage.md)
  → [`03-query.md`](../docs/03-query.md)
  → [`04-inference.md`](../docs/04-inference.md)
  → [`05-validation.md`](../docs/05-validation.md)
  → [`10-roadmap.md`](../docs/10-roadmap.md).
- **Evidence:** [`tests/`](../tests/) — every feature in this
  catalogue is pinned by at least one fixture.
