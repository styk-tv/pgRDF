# 02 — Loading RDF

pgRDF reads **Turtle**, **N-Triples**, **TriG** and **N-Quads**, from
a string passed in SQL or from a file on the database server.

| You have | Use |
|---|---|
| A snippet or a document in your application | `parse_turtle(content, graph_id)` |
| TriG or N-Quads text (several graphs at once) | `parse_trig(content)` / `parse_nquads(content)` |
| A file on the database server | `load_turtle(path, graph_id)` |
| A very large N-Triples dump | `load_turtle_staged_run(path, graph_id)` or `load_turtle(…, bulk_load => true)` |
| A file bigger than the server's memory | `load_turtle_streaming(path, graph_id)` |

Loaders write into a graph you've created with
`pgrdf.add_graph(iri)`. See [managing graphs](05-graphs.md). Always
create the graph first. Triples loaded into a numeric id that has no
graph are stored, and SPARQL finds them, but they have no IRI and don't
appear in `graph_inventory()`.

## From a string

```sql
SELECT pgrdf.add_graph('http://example.org/people');

SELECT pgrdf.parse_turtle($$
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
ex:alice a foaf:Person ; foaf:name "Alice" ; foaf:knows ex:bob .
ex:bob   a foaf:Person ; foaf:name "Bob" .
$$, pgrdf.graph_id('http://example.org/people'));
--  parse_turtle
-- --------------
--             5
```

It returns the number of triples read. N-Triples is a subset of
Turtle, so `parse_turtle` loads it too. PostgreSQL's `$$ … $$`
quoting avoids escaping quotes inside the data. From an application,
pass the document as a bind parameter:
`SELECT pgrdf.parse_turtle($1, $2)`.

The number is what the loader read, not what the graph gained: a
triple the graph already holds is read and not stored again. See
[a graph is a set](#a-graph-is-a-set).

### Relative IRIs

If a document uses relative IRIs such as `<#me>` or `<../ns>`, supply
a base IRI:

```sql
SELECT pgrdf.parse_turtle('<#me> <http://xmlns.com/foaf/0.1/name> "Me" .',
                          pgrdf.graph_id('http://example.org/people'),
                          'http://example.org/profile');
```

Without one, the load fails with `No scheme found in an absolute IRI`.

## A graph is a set

A graph never stores the same triple twice (since 0.6.41). A repeat is
dropped whether the document states it twice, an earlier load already
stored it, or it arrives another way: SPARQL `INSERT`, `copy_graph`,
`carve_graph`, `move_graph`, `materialize`. PostgreSQL enforces it with
a unique index on each graph.

So loading the same data again changes nothing. Run the load above a
second time:

```sql
SELECT pgrdf.parse_turtle($$
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
ex:alice a foaf:Person ; foaf:name "Alice" ; foaf:knows ex:bob .
ex:bob   a foaf:Person ; foaf:name "Bob" .
$$, pgrdf.graph_id('http://example.org/people'));
--  parse_turtle
-- --------------
--             5          (read)

SELECT pgrdf.count_quads(pgrdf.graph_id('http://example.org/people'));
--  count_quads
-- -------------
--            5          (stored: unchanged)
```

To replace a graph's content instead of adding to it, clear it first
with `pgrdf.clear_graph(…)`. The [load report](#load-reports) says how
many triples a load stored and how many it dropped as repeats.

A graph loaded by a version before 0.6.41 can already hold repeated
triples. It keeps them until you remove them; see
[repeated triples from earlier versions](05-graphs.md#repeated-triples-from-earlier-versions).

### Blank nodes

A labelled blank node (`_:b1`) is stored under its label, and a label
names **one node across the whole database**, not one per document or
per graph. Two consequences:

- Reloading a document that uses labelled blank nodes is idempotent,
  like any other.
- Two documents that use the same label are talking about the same
  node, in whichever graphs they were loaded. Loading one file into two
  graphs gives two graphs that share its blank nodes, and two unrelated
  files that both say `_:b0` are joined on it.

```sql
SELECT pgrdf.add_graph('http://example.org/b1'), pgrdf.add_graph('http://example.org/b2');
SELECT pgrdf.parse_turtle('_:order <http://example.org/sku> "A1" .', pgrdf.graph_id('http://example.org/b1'));
SELECT pgrdf.parse_turtle('_:order <http://example.org/qty> 2 .',    pgrdf.graph_id('http://example.org/b2'));

SELECT * FROM pgrdf.sparql(
  'SELECT ?sku ?qty WHERE { ?o <http://example.org/sku> ?sku . ?o <http://example.org/qty> ?qty }');
--  {"qty": "2", "sku": "A1"}
```

The query has no `GRAPH` clause, so it reads both graphs and finds one
node with a `sku` from the first and a `qty` from the second. Give
blank nodes labels that are unique to their document before loading, or
use the anonymous form, when that is not what you want.

An anonymous blank node (`[ … ]`, and the nodes of a `( … )` list) has
no label in the document. The parser gives it a fresh one on every
load, so it is never joined with another document's nodes, and
**loading such a document again adds its blank-node triples again**, as
new nodes. That is the one case where a reload is not idempotent.

### Asserted beats inferred

`materialize` stores what it derives as *inferred* triples. If you then
load or insert a triple that is already stored as inferred, the stored
triple becomes asserted. It is not stored twice, and the next
`materialize` no longer removes it with the other inferred triples.
The load report counts these under `inferred_made_asserted`.

### Characters that cannot be stored

`"\u0000"` is a valid RDF literal, and PostgreSQL `text` cannot hold
the character U+0000. Every loader and SPARQL `INSERT` refuses a term
that contains it, before anything is written:

```sql
SELECT pgrdf.parse_turtle(
  '<http://example.org/a> <http://example.org/p> "a\u0000b" .',
  pgrdf.graph_id('http://example.org/people'));
-- ERROR:  22P05: pgrdf: a term contains U+0000, which PostgreSQL text cannot store: "a\u0000b"
-- HINT:  U+0000 is valid in RDF but cannot be stored. Remove or replace the character in the
--        source, then load again; nothing from this call was written.
```

## TriG and N-Quads

These formats name their graphs inside the data. Triples outside any
named graph go to `default_graph_id` (graph `0` unless you pass
another).

```sql
SELECT pgrdf.parse_trig($$
@prefix ex: <http://example.org/> .
ex:x ex:p ex:y .
GRAPH <http://example.org/trig-demo> { ex:a ex:b ex:c . }
$$);
-- {"triples": 2, "graphs": [0, 7], ...}

SELECT pgrdf.parse_nquads(
  '<http://example.org/a> <http://example.org/b> "c" <http://example.org/trig-demo> .');
```

- By default, named graphs that don't exist yet are created.
- With `strict => true`, every named graph must already exist, and an
  unknown one is refused (`42704`). This keeps a typo from creating a
  stray graph.

```sql
SELECT pgrdf.parse_trig('GRAPH <http://example.org/typo> { <urn:a> <urn:b> <urn:c> . }', 0, true);
-- ERROR:  42704: parse_trig: unknown graph iri http://example.org/typo
```

Both functions return a JSONB report, including the list of graph ids
they wrote to and the [`load` object](#load-reports).

## From a file

`load_turtle` reads a file from the **database server's** filesystem,
the one the PostgreSQL process can see, not the machine your client
runs on.

```sql
SELECT pgrdf.add_graph('http://example.org/sample');
SELECT pgrdf.load_turtle('/tmp/sample.ttl', pgrdf.graph_id('http://example.org/sample'));
--  load_turtle
-- -------------
--            7
```

With the Docker setup from the [install guide](01-install.md), copy the
file into the container first:

```sh
docker cp sample.ttl pgrdf:/tmp/sample.ttl
```

Arguments: `load_turtle(path, graph_id, base_iri DEFAULT NULL, bulk_load DEFAULT false)`.

The file must be readable by the PostgreSQL server process. If your
client and server are on different machines, read the file in your
application and use `parse_turtle` instead.

Reading a server file needs the privileges of the built-in role
`pg_read_server_files` (a superuser has them), the same rule PostgreSQL
applies to `COPY … FROM 'file'`. Without it every file loader refuses
with SQLSTATE `42501` and the fix in the hint:

```sql
GRANT pg_read_server_files TO loader_role;
```

The string loaders (`parse_turtle`, `parse_nquads`, `parse_trig`) need
no such role: the content arrives as a query argument.

### Load reports

The `_verbose` variants return a JSONB report instead of a count.
`parse_turtle_verbose(content, graph_id)` is the one for strings,
`load_turtle_verbose(path, graph_id)` the one for files. Its `load`
object says what the load did to the graph. Into the `people` graph
from above, which already holds Alice's name:

```sql
SELECT pgrdf.parse_turtle_verbose($$
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
ex:alice foaf:name "Alice" .
ex:alice foaf:name "Alice" .
ex:carol foaf:name "Carol" .
$$, pgrdf.graph_id('http://example.org/people'));
-- {"load": {"triples_read": 3, "parse_skipped": 0, "triples_stored": 1,
--           "inferred_made_asserted": 0, "duplicates_dropped": 2, "graph_was_empty": false},
--  "path": "combined", "triples": 3, "elapsed_ms": 0.70, "quad_batches": 1, ...}
```

| Key of `load` | Meaning |
|---|---|
| `triples_read` | Triples the parser produced, repeats included. The loaders that return a number return this. |
| `parse_skipped` | Malformed statements the parallel N-Triples routes skipped instead of failing. `0` on the standard routes, which fail on the first syntax error. |
| `triples_stored` | Triples the graph did not hold before and holds now. |
| `inferred_made_asserted` | Triples that were stored as inferred and are asserted now. |
| `duplicates_dropped` | The rest: `triples_read` minus the two counts above. It is one total for repeats inside the load and triples the graph already held. |
| `graph_was_empty` | Whether the graph held no triples before the call. When `true`, every dropped duplicate was a repeat inside the load. `null` for a TriG or N-Quads call whose data names a graph. |

Outside `load`, `path` names the loading strategy that ran and the
remaining keys are timings and cache counters.

The same object is available three ways:

- in the return value of every loader that returns JSONB: the
  `_verbose` variants, `parse_trig`, `parse_nquads`,
  `load_turtle_streaming` and `load_turtle_staged_run`;
- in `pgrdf.last_call_stats() -> 'load'`, for the last load of this
  session (`null` when the session's last pgRDF call was not a load);
- in the `detail` of the load's row in the
  [act record](05-graphs.md#the-act-record), together with the function
  called (`entry`) and the strategy (`route`).

One limit in 0.6.41, on the staged loader's side: its act row records
the rows it attached, without `duplicates_dropped`. When the file
repeated triples, a second act named `dedup` beside it says how many
rows were removed, and the function's return value and
`last_call_stats()` carry the final counts.

## Large loads

For hundreds of millions to billions of triples, pgRDF has two
parallel loaders. Both read **N-Triples** (one triple per line). For
other formats, convert first, for example with `riot` (Apache Jena)
or `rapper` (Raptor).

### The staged loader

The fastest path. It splits the file across background workers:

```sql
SELECT pgrdf.add_graph('http://example.org/wikidata');
SELECT pgrdf.load_turtle_staged_run('/data/latest-truthy.nt',
                                    pgrdf.graph_id('http://example.org/wikidata'),
                                    8);          -- workers; 0 = choose automatically
-- {"ok": true, "triples": ..., "quads": ..., "n_workers": 8,
--  "load": {"triples_read": ..., "triples_stored": ..., "duplicates_dropped": ..., ...},
--  "phase_ms": {"stage": ..., "dict": ..., "resolve": ..., "index": ...}}
```

`triples` is what the file stated and `quads` what the graph holds
afterwards; they differ when the file repeated triples.

Or as a procedure: `CALL pgrdf.load_turtle_staged(path, graph_id, n_workers);`

Rules:

- It needs an **empty database**: use it for the first, largest load.
  On a database that already holds data it declines and says so
  (`"ok": false, "fallback": true`). Load the rest with `load_turtle`.
- It commits as it goes, so it **can't run inside a transaction
  block**. Call it as a single statement.
- It needs `pgrdf` in `shared_preload_libraries`.
- Malformed lines are skipped, not fatal. A Turtle file is not
  N-Triples: given one, the staged loader reports `"ok": true` and
  loads **zero** triples. Check `triples` in the report.
- It is an **owner lane**. Its background workers run as the calling
  role, and the staging and partition work they do needs the privileges
  of the owner of the pgRDF tables. Any other role is refused up front
  (`42501`), before a worker starts; `load_turtle` given an N-Triples
  file by such a role takes the standard parser instead of this one.

### `bulk_load => true`

```sql
SELECT pgrdf.load_turtle('/data/dump.nt', pgrdf.add_graph('http://example.org/dump'), NULL, true);
```

A parallel in-process path, also for N-Triples into a fresh database.
It falls back to the standard loader when the database already holds
data, and for a role that doesn't own the pgRDF tables (the fast path
rebuilds indexes, which only the owner may do; a `NOTICE` says so).

> **N-Triples only.** Given a Turtle file with prefixes or multi-line
> statements, `bulk_load => true` currently skips the lines it can't
> read and can load **zero triples without an error**. Check the
> returned count, or `parse_skipped` in the `_verbose` report. For
> Turtle, leave `bulk_load` off.

### Files larger than memory

`load_turtle_streaming(path, graph_id)` reads a file in windows of
`window_triples` (default 20 million) so memory stays bounded. Like
`bulk_load`, its fast path applies to an empty database loaded by the
owner of the pgRDF tables; otherwise it loads through the standard path.

### Server settings for a big import

Set these before a large load into a fresh database. Values assume a
dedicated machine with tens of gigabytes of RAM; scale them to yours.

| Setting | Suggested | Effect |
|---|---|---|
| `shared_buffers` | 25–40 % of RAM | Keeps the term dictionary and hot index pages in memory. |
| `maintenance_work_mem` | 2–8 GB | Faster index builds. |
| `max_wal_size` | 32–64 GB | Fewer checkpoints during the load. |
| `checkpoint_timeout` | 30–60 min | Same. |
| `wal_compression` | `on` | Less WAL written. |
| `effective_io_concurrency` | 200+ on SSD / NVMe | More concurrent I/O. |
| `max_parallel_maintenance_workers` | 4–8 | Parallel index builds. |
| `max_parallel_workers` | about the core count | Headroom for parallel phases. |

For an import you can simply re-run if the machine fails, you can also
trade durability for speed with `synchronous_commit = off`, or with
`fsync = off` together with `full_page_writes = off`. `fsync = off`
can corrupt the cluster on a crash: only use it on a disposable
database, and turn it back on and run `CHECKPOINT` before the data
matters.

pgRDF settings that matter for large loads:

| Setting | Default | Notes |
|---|---|---|
| `pgrdf.bulk_defer_index_min` | `100000` | Above this size, bulk loads drop indexes and rebuild them at the end. |
| `pgrdf.staged_temp_tablespaces` | (empty) | Put the staged loader's temporary files on another disk. |
| `pgrdf.auto_analyze` | `on` | Refresh planner statistics after the load. Leave it on. |

Suggested order:

1. Fresh database, server tuned.
2. The largest N-Triples file with `load_turtle_staged_run`.
3. Smaller or incremental files with `load_turtle` / `parse_turtle`.
4. `pgrdf.materialize(…)` if you need inference.
5. Restore durable settings, `CHECKPOINT`.

## When a load fails

| Error | Meaning |
|---|---|
| `turtle parse error … No scheme found in an absolute IRI` | The document uses relative IRIs. Pass a `base_iri`. |
| `turtle parse error … Invalid character …` | The document isn't valid Turtle. Parsing is strict; fix the source. |
| `… RDF-star …` | Quoted triples are not supported. |
| `failed to open …` | The path isn't readable by the PostgreSQL server process. |
| `22P05 … a term contains U+0000` | The data holds a character PostgreSQL cannot store. Remove or replace it in the source. |
| `55P03 … is locked` | The target graph is locked. See [locking](05-graphs.md#locking-a-graph). |
| `42704 … unknown graph iri` | `strict => true` and the data names a graph that doesn't exist. |

A failed load changes nothing: the statement is rolled back.

## Next

[03 — Querying](03-querying.md)
