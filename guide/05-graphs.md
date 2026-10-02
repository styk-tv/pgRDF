# 05 — Managing graphs

Every triple in pgRDF belongs to a **named graph**. A graph has an IRI,
the name you use in SPARQL, and a numeric id, which the SQL functions
take. This page covers creating graphs, finding them, copying and
carving them, locking them, and checking their health.

## The model in one paragraph

- Create graphs by IRI with `pgrdf.add_graph(iri)`. It returns the
  graph's id.
- Graph `0` always exists. It is the default graph, named
  `urn:pgrdf:graph:0`.
- A SPARQL query without a `GRAPH` clause searches **all** graphs.
  Scope it with `GRAPH <iri> { … }`. See
  [querying](03-querying.md#named-graphs).
- Each graph is stored in its own partition, so dropping or clearing a
  whole graph is cheap regardless of its size.

## Create and look up

```sql
SELECT pgrdf.add_graph('http://example.org/people');  -- 1
SELECT pgrdf.add_graph('http://example.org/people');  -- 1 (idempotent)

SELECT pgrdf.graph_id('http://example.org/people');   -- 1
SELECT pgrdf.graph_iri(1);  -- http://example.org/people
```

> **Watch for NULL.** `graph_id()` returns `NULL` for an IRI that does
> not exist. Most pgRDF functions are strict: given a `NULL` argument
> they return `NULL` without doing anything or raising an error. When
> a graph might be missing, check first:
>
> ```sql
> SELECT pgrdf.graph_id('http://example.org/typo') IS NOT NULL AS exists;
> ```
>
> The IRI-taking functions (`drop_graph(iri)`, `clear_graph(iri)`,
> `copy_graph(iri, iri)`, `move_graph(iri, iri)`) refuse unknown IRIs
> with SQLSTATE `42704` instead.

### Claiming a new IRI

`add_graph` returns the existing graph when the IRI is already taken,
which is what you want for "make sure this graph exists" and not what
you want when two people load into one database. `create_graph` claims
a **new** IRI or refuses:

```sql
SELECT pgrdf.create_graph('http://example.org/people');
-- ERROR:  42710: create_graph: graph <http://example.org/people> already exists (graph_id 1)
-- HINT:  existing graph_id 1, source_sha256 3f2a… (loads 1): reuse it if that
--        is the same content, or choose another IRI
```

The hint carries the digest of what the existing graph was loaded
from, so you can tell "someone already loaded this file" from "someone
else is using this name".

### Who may create graphs

Creating a graph needs `SELECT` and `INSERT` on `pgrdf._pgrdf_quads` and
`pgrdf._pgrdf_graphs`; dropping one needs `SELECT` and `DELETE` on both;
clearing one needs `SELECT` and `DELETE` on `pgrdf._pgrdf_quads`.
Table ownership is not required: the steps PostgreSQL reserves for the
table owner (creating, emptying or removing the graph's storage
partition) run as the owner inside `add_graph` / `clear_graph` /
`drop_graph`, and nothing else does. A role without the grants is
refused with `42501` and the exact `GRANT` in the hint. Ask the server
directly:

```sql
SELECT pgrdf.can_create_graphs(), pgrdf.can_clear_graphs(), pgrdf.can_drop_graphs();
```

## The inventory

`pgrdf.graph_inventory()` lists every graph with its size, lock state,
reasoning freshness and what it was loaded from:

```sql
SELECT graph_id, iri, asserted, inferred, locked, materialization, source_loads
  FROM pgrdf.graph_inventory();
--  graph_id |            iri            | asserted | inferred | locked | materialization | source_loads
-- ----------+---------------------------+----------+----------+--------+-----------------+--------------
--         0 | urn:pgrdf:graph:0         |        0 |        0 | f      | never           |
--         1 | http://example.org/people |       14 |        8 | f      | stale           |            1
--         2 | http://example.org/shapes |       10 |        0 | f      | never           |            1
```

| Column | Meaning |
|---|---|
| `asserted` | Triples you loaded or inserted. |
| `inferred` | Triples produced by `materialize`. |
| `locked`, `lock_reason` | Whether the graph is write-locked, and why (see below). |
| `materialization` | `never` / `current` / `stale` / `unknown`. See [reasoning](04-reasoning.md#is-the-materialization-current). |
| `source_sha256`, `source_loads` | SHA-256 of the bytes the last load read, and how many loads the graph has had. `NULL` until a load records it. With one load, the digest identifies the source file; with more, the graph is the sum of several. It identifies the *file*, not the graph: never compare it with `graph_digest` or a bytes digest of an export. |
| `identity_digest` | A locked graph's `graph_digest`, once computed. `NULL` for open graphs: their identity is computed on request. |

Because it's a normal set-returning function, you can filter, join and
aggregate it like a table:

```sql
SELECT iri, asserted + inferred AS total
  FROM pgrdf.graph_inventory()
 ORDER BY total DESC LIMIT 5;
```

For a single number, `pgrdf.count_quads(graph_id)` returns asserted plus
inferred triples for one graph.

`pgrdf.orphan_partitions()` lists storage partitions that no longer
belong to any graph. It should return no rows. If it returns some,
that storage can't be reached through SPARQL.

## Clear, copy, move, drop

All four take either graph ids or graph IRIs.

| Function | Effect | Returns |
|---|---|---|
| `clear_graph(g)` | Removes every triple. The graph itself stays. | triples removed |
| `copy_graph(src, dst)` | Appends all of `src` into `dst`, **including inferred triples**. | triples copied |
| `move_graph(src, dst)` | Moves `src` into an **empty** `dst`, then removes `src`. | triples moved |
| `drop_graph(g, cascade => true)` | Removes the graph and its triples. | triples removed |

```sql
SELECT pgrdf.add_graph('http://example.org/people-backup');
SELECT pgrdf.copy_graph('http://example.org/people', 'http://example.org/people-backup');
-- → 22

SELECT pgrdf.graph_digest(pgrdf.graph_id('http://example.org/people')) =
       pgrdf.graph_digest(pgrdf.graph_id('http://example.org/people-backup')) AS same_graph;
-- → true
```

Details that matter:

- **Copy by IRI needs the destination to exist.** Create it with
  `add_graph` first, or you get `42704`. Copy by id creates a missing
  destination for you, named `urn:pgrdf:graph:<id>`.
- **Copy appends.** For an exact copy, clear the destination first. A
  copy's inferred triples show `materialization = unknown`. Run
  `materialize` on the copy if you rely on them.
- **Move refuses a non-empty destination** (`55000`).
- **The default graph (`0`) can't be emptied with `clear_graph`.** It
  returns 0 and leaves the triples in place. Keep data in named
  graphs.
- **Drop refuses over inferred triples when `cascade => false`**
  (`2BP01`). The default is `cascade => true`. The default graph
  (`0`) can't be dropped.

The same operations are available as SPARQL UPDATE:

```sparql
CREATE GRAPH <http://example.org/new>
CLEAR  GRAPH <http://example.org/staging>
DROP   GRAPH <http://example.org/staging>

INSERT { GRAPH <http://example.org/g2> { ?s ?p ?o } }
WHERE  { GRAPH <http://example.org/g1> { ?s ?p ?o } }
```

## Carving out a subgraph

`carve_graph` copies part of a graph into another graph. It has two forms.

**By predicate:** every triple that uses one predicate.

```sql
SELECT pgrdf.add_graph('http://example.org/social');
SELECT pgrdf.carve_graph(
  pgrdf.graph_id('http://example.org/people'),
  'http://xmlns.com/foaf/0.1/knows',
  pgrdf.graph_id('http://example.org/social'));
-- → 2
```

**By neighbourhood:** everything reachable from some seed nodes
within `max_hops` steps (default 1).

```sql
SELECT pgrdf.add_graph('http://example.org/alice-hood');
SELECT pgrdf.carve_graph(
  pgrdf.graph_id('http://example.org/people'),
  ARRAY['http://example.org/alice'],
  pgrdf.graph_id('http://example.org/alice-hood'),
  1);
-- NOTICE:  carve_graph: neighbourhood continues beyond max_hops=1 — 4 adjacent node(s)
--          were not expanded; the slice is the requested 1-hop ball, not a closed
--          component (raise max_hops to widen)
-- → 20
```

The notice tells you when the slice stopped at the hop limit rather
than at the edge of the data.

## Locking a graph

A lock makes a graph read-only until someone unlocks it. While it's
held, **every** write path refuses with SQLSTATE `55P03`: loading,
SPARQL UPDATE, `clear_graph`, `drop_graph`, `materialize`, being the
destination of `copy_graph` / `move_graph` / `carve_graph`, and plain
SQL against the storage tables (`lock_graph` installs triggers on the
graph's partition; `unlock_graph` removes them). Reads keep working.

```sql
SELECT pgrdf.lock_graph(pgrdf.graph_id('http://example.org/people'), 'frozen for audit');

SELECT pgrdf.clear_graph('http://example.org/people');
-- ERROR:  55P03: pgrdf: graph 1 is locked (frozen for audit): clear_graph refused.
--         Unlock with pgrdf.unlock_graph(1, '<reason>').

SELECT pgrdf.unlock_graph(pgrdf.graph_id('http://example.org/people'), 'audit done');
```

- A reason is required for both locking and unlocking.
- Locking an already locked graph refuses (`55P03`), so a standing
  reason is never overwritten. Unlocking an unlocked graph refuses
  (`55000`).
- The inventory's `locked` / `lock_reason` columns show current locks.
- Every refusal carries the cure as a hint: `pgrdf.unlock_graph(<id>,
  '<reason>')`.
- A locked graph's `graph_digest` is computed once and then served from
  the inventory's `identity_digest`; unlocking clears it.
- A lock holds against every role except the owner of the pgRDF tables
  and superusers, who can drop its triggers. Use normal PostgreSQL roles
  and privileges to decide who those are.

## Checking a graph's health

`pgrdf.graph_integrity(graph_id)` checks that every triple is
well-formed: no literal subjects, no blank-node or literal predicates,
no references to missing terms.

```sql
SELECT pgrdf.graph_integrity(pgrdf.graph_id('http://example.org/people'));
-- {"clean": true, "graph_id": 1, "illegal_terms": 0, "dangling_refs": 0,
--  "counts": {"quads": 22, "subject_literal": 0, "predicate_bnode": 0, ...},
--  "lock_custody": {"locked": false, "write_triggers": 0, "consistent": true}}
```

`lock_custody` checks that a locked graph carries its write triggers
and an open one carries none; a mismatch (someone removed a trigger by
hand) makes the graph not clean.

A graph that is not clean should be reloaded from source. Validation
results on such a graph can't be trusted.

## Looking up a term

Query results show terms by their text. If you hold a term id, for
example from a report, `pgrdf.get_term(id)` returns its text.

## Next

[06 — Validation](06-validation-recipes.md)
