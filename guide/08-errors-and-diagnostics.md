# 08 — Errors and diagnostics

pgRDF reports problems through standard PostgreSQL mechanisms, so every
driver in every language can handle them without a pgRDF-specific
library.

## Refusals carry a SQLSTATE

When pgRDF declines a request (a locked graph, an unknown profile, a
SPARQL construct it can't run), the error carries a standard SQLSTATE,
plus a message that names what was refused and usually how to fix it.

| SQLSTATE | Meaning | Example |
|---|---|---|
| `55P03` | The graph is locked | writing to a graph under `lock_graph` |
| `55000` | Wrong state for this operation | unlocking an unlocked graph; `move_graph` into a non-empty graph |
| `22023` | Invalid argument or content | unknown `materialize` profile or `validate` mode; negative graph id |
| `0A000` | Unsupported construct | `SERVICE`, `FILTER EXISTS`, `LANGMATCHES` |
| `42704` | Unknown graph | `graph_digest` of a graph that doesn't exist; `drop_graph` of an unknown IRI |
| `42710` | Binding conflict | binding an IRI or id already bound to another graph |
| `2BP01` | Dependent objects | `drop_graph(g, cascade => false)` on a graph with inferred triples |
| `54000` | A configured limit was exceeded | property-path truncation with `pgrdf.on_path_truncation = 'error'` |
| `XX000` | Internal error | a genuine fault |

Match on the code, not the message text. Messages are written for
people and may improve.

```sql
SELECT pgrdf.materialize(1, 'bogus');
-- ERROR:  22023: materialize: unknown profile "bogus" (supported: 'owl-rl', 'rdfs')
```

(In `psql`, `\set VERBOSITY verbose` shows the code next to the message.)

A few refusals, mostly SPARQL expressions the engine can't translate
and the strict-mode checks in `validate`, currently arrive as `XX000`
with a descriptive message. Treat an `XX000` whose message names a
pgRDF function the same way: it tells you what was refused.

### In your driver

```js
// node-postgres
try { await client.query(sql); }
catch (e) {
  if (e.code === '55P03') { /* graph locked; e.message names the unlock */ }
  if (e.code === '0A000') { /* unsupported construct, named in e.message */ }
  if (e.code === '42704') { /* no such graph */ }
}
```

```python
# psycopg 3
try:
    cur.execute(sql)
except psycopg.Error as e:
    if e.sqlstate == "55P03": ...
```

```go
// pgx v5
var pgErr *pgconn.PgError
if errors.As(err, &pgErr) && pgErr.Code == "55P03" { ... }
```

```rust
// tokio-postgres
if let Some(state) = err.code() {
    if state.code() == "55P03" { /* locked */ }
}
```

JDBC: `SQLException.getSQLState()`. libpq: `PQresultErrorField(res, PG_DIAG_SQLSTATE)`.

## Was the answer complete?

Two settings bound the work a query can do, and a query that hits one
returns fewer rows rather than failing. pgRDF tells you when that
happens, in three ways.

**1. A warning on the statement.** A property path that hits
`pgrdf.path_max_depth` raises a `WARNING` alongside its results:

```sql
SET pgrdf.path_max_depth = 1;
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  PREFIX ex:   <http://example.org/>
  SELECT ?who WHERE { ex:alice foaf:knows+ ?who }
$$);
-- WARNING:  sparql: property path truncated at pgrdf.path_max_depth=1 — longer paths are
--           missing from this result; raise pgrdf.path_max_depth, or
--           SET pgrdf.on_path_truncation = 'error' to forbid partial results
--  {"who": "http://example.org/bob"}
```

**2. Per-call figures.** Right after a `sparql`, `construct` or
`describe` call, in the same session:

```sql
SELECT pgrdf.last_call_stats();
-- {"filter_clauses_dropped": 0, "path_depth_truncations": 1}
```

Both zero means the last call's answer was complete. Other sessions
can't affect these numbers.

**3. Refuse partial answers outright:**

```sql
SET pgrdf.on_path_truncation = 'error';
-- the same query now fails:
-- ERROR:  54000: sparql: property path truncated at pgrdf.path_max_depth=1
--         (pgrdf.on_path_truncation=error forbids partial results)
```

## Settings

All settings can be changed per session with `SET`.

| Setting | Default | What it does |
|---|---|---|
| `pgrdf.path_max_depth` | `64` | Maximum depth of property-path (`+`, `*`) walks. |
| `pgrdf.on_path_truncation` | `warn` | `warn` returns partial results with a warning; `error` refuses them (`54000`); `count` returns them silently and only records the truncation in `last_call_stats()` / `stats()`. |
| `pgrdf.auto_analyze` | `on` | Refresh planner statistics after loads and `materialize`. |
| `pgrdf.bulk_defer_index_min` | `100000` | Above this many triples, bulk loads drop and rebuild indexes instead of maintaining them row by row. |
| `pgrdf.dict_batch_size` | `500` | Terms per dictionary batch in the standard loader. |
| `pgrdf.ingest_dict_path` | `combined` | Dictionary resolution strategy used by the standard loader. |
| `pgrdf.shmem_prewarm_on_init` | `off` | Warm the shared term cache before the first load after a restart. |
| `pgrdf.staged_resolve_strategy` | `index` | Join strategy of the staged bulk loader. |
| `pgrdf.staged_temp_tablespaces` | (empty) | Tablespace(s) for the staged loader's temporary files, for example a separate scratch disk. |

List them on your server with:

```sql
SELECT name, setting, short_desc FROM pg_settings WHERE name LIKE 'pgrdf.%';
```

## What is this server running?

```sql
SELECT pgrdf.version(),
       pgrdf.build_id(),
       (SELECT extversion FROM pg_extension WHERE extname = 'pgrdf');
--  version | build_id | extversion
-- ---------+----------+------------
--  0.6.34  | v0.6.34  | 0.6.34
```

- `version()`: the release the loaded library belongs to.
- `build_id()`: which build of it. Official releases show the tag
  (`v0.6.34`). A build from a modified source tree shows a git hash,
  often ending in `-dirty`.
- `extversion`: the SQL objects installed in this database. If it's
  behind `version()`, run `ALTER EXTENSION pgrdf UPDATE`.

## What does this server support?

```sql
SELECT name, identity_args, note
  FROM pgrdf.surface()
 WHERE class = 'stable'
 ORDER BY name;
```

`surface()` lists every function with a stability class:

| Class | Meaning |
|---|---|
| `stable` | Supported API. |
| `internal` | Low-level building blocks; may change without notice. |
| `spike` | Development benchmarks; not for use. |
| `deprecated` | Superseded; the note names the replacement. |

The note on each row says what the function is for. The same list, grouped
by topic, is in the [function reference](09-function-reference.md).

## Looking inside a query

```sql
-- How pgRDF parsed it, and whether anything is unsupported:
SELECT pgrdf.sparql_parse('SELECT * WHERE { SERVICE <http://x> { ?s ?p ?o } }') -> 'unsupported_algebra';
-- ["Service (federation)"]

-- The SQL it would run:
SELECT pgrdf.sparql_sql('SELECT ?s WHERE { ?s a <http://xmlns.com/foaf/0.1/Person> }');
```

Pair `sparql_sql` with PostgreSQL's `EXPLAIN` to investigate a slow
query.

## Instance counters

`pgrdf.stats()` returns cumulative, server-wide counters: shared term
cache hits and misses, plan cache hits and misses, and the running
totals of truncations and dropped filters. Use it for health
dashboards. For "was *my* query complete", use `last_call_stats()`.

## Next

[09 — Function reference](09-function-reference.md)
