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

Loading appends. Load the same data again and its triples are stored a
second time, so to reload a graph, clear it first with
`pgrdf.clear_graph(…)`. SPARQL `INSERT DATA` is different: inserting a
triple that is already there changes nothing.

### Relative IRIs

If a document uses relative IRIs such as `<#me>` or `<../ns>`, supply
a base IRI:

```sql
SELECT pgrdf.parse_turtle('<#me> <http://xmlns.com/foaf/0.1/name> "Me" .',
                          pgrdf.graph_id('http://example.org/people'),
                          'http://example.org/profile');
```

Without one, the load fails with `No scheme found in an absolute IRI`.

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
they wrote to.

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

### Load reports

The `_verbose` variants return a JSONB report instead of a count:

```sql
SELECT pgrdf.load_turtle_verbose('/tmp/sample.ttl', pgrdf.graph_id('http://example.org/sample'));
-- {"path": "combined", "triples": 7, "elapsed_ms": 0.41, "parse_skipped": 0,
--  "dict_cache_hits": 11, "shmem_cache_hits": 7, "quad_batches": 1, ...}
```

`parse_turtle_verbose(content, graph_id)` does the same for strings.
`path` names the loading strategy that ran.

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
--  "phase_ms": {"stage": ..., "dict": ..., "resolve": ..., "index": ...}}
```

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

### `bulk_load => true`

```sql
SELECT pgrdf.load_turtle('/data/dump.nt', pgrdf.add_graph('http://example.org/dump'), NULL, true);
```

A parallel in-process path, also for N-Triples into a fresh database.
It falls back to the standard loader when the database already holds
data.

> **N-Triples only.** Given a Turtle file with prefixes or multi-line
> statements, `bulk_load => true` currently skips the lines it can't
> read and can load **zero triples without an error**. Check the
> returned count, or `parse_skipped` in the `_verbose` report. For
> Turtle, leave `bulk_load` off.

### Files larger than memory

`load_turtle_streaming(path, graph_id)` reads a file in windows of
`window_triples` (default 20 million) so memory stays bounded.

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
| `55P03 … is locked` | The target graph is locked. See [locking](05-graphs.md#locking-a-graph). |
| `42704 … unknown graph iri` | `strict => true` and the data names a graph that doesn't exist. |

A failed load changes nothing: the statement is rolled back.

## Next

[03 — Querying](03-querying.md)
