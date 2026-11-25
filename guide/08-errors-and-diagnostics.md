# 08 — Errors and diagnostics

pgRDF reports problems through standard PostgreSQL mechanisms, so every
driver in every language can handle them without a pgRDF-specific
library.

## Refusals carry a SQLSTATE

When pgRDF declines a request (a locked graph, an unknown profile, a
SPARQL construct it can't run), the error carries a standard SQLSTATE,
plus a message that names what was refused and usually how to fix it.
The code says what kind of refusal it is, and so who has to act.

| SQLSTATE | Meaning | Who acts | Example |
|---|---|---|---|
| `0A000` | Valid request that pgRDF does not support | change the request | `SERVICE`, `FILTER EXISTS`, `LANGMATCHES`; a `FILTER` beside a `UNION` |
| `42601` | The query does not parse | fix the query | an unclosed `{` in a SPARQL query |
| `22023` | Invalid argument | fix the argument | unknown `materialize` profile or `validate` mode; a negative graph id; a control character in a declared-identity setting |
| `42704` | Something named does not exist | fix the name, or create it | `graph_digest` of a graph that doesn't exist; `CLEAR GRAPH` of an unknown IRI, or `COPY` from one; `set_graph_owner` to an unknown role; `recount_graph` of an unknown graph id |
| `42710` | It already exists | choose another name | `create_graph` or `CREATE GRAPH` of a taken IRI; binding an IRI or id already bound to another graph |
| `54000` | A configured limit was exceeded | narrow the request or raise the limit | property-path truncation with `pgrdf.on_path_truncation = 'error'`; a walk past `pgrdf.path_max_pairs`; a diff past `pgrdf.diff_max_rows`; a digest past `pgrdf.digest_max_bnode_quads`; `validate` or `materialize` past `pgrdf.max_inmemory_triples` |
| `22P05` | A term contains a character PostgreSQL cannot store (U+0000) | fix the data | loading or inserting `"a\u0000b"` |
| `22021` | A stored term holds a character pgRDF cannot emit (U+0000) | reload the graph | exporting or digesting a graph that holds a term with U+0000, stored by a version before 0.6.41 |
| `55000` | Wrong state for this operation | change the state first | unlocking an unlocked graph; `move_graph` into a non-empty graph; an export or digest before `ALTER EXTENSION pgrdf UPDATE` |
| `55P03` | The graph is locked | unlock it, or leave it alone | any write to a graph under `lock_graph` |
| `42501` | Missing privilege, or not the graph's owner | grant it, or act as the owner | creating a graph without `INSERT` on the pgRDF tables; a file loader without `pg_read_server_files`; `stats()` without `EXECUTE`; `clear_graph` on another role's graph under `pgrdf.graph_owner_enforce`; `graph_inventory(g)` for a graph that is not listed to the calling role, or `recount_graph(g)` for another role's graph, or `recount_terms()` without the storage owner's privileges (since 0.6.42); a direct write to the act record or to the stored counts' tables, whatever grants the role holds |
| `2BP01` | Dependent objects | pass `cascade`, or remove them | `drop_graph(g, cascade => false)` on a graph with inferred triples |
| `XX000` | A pgRDF defect | report it | see below |

Match on the code, not the message text. Messages are written for
people and may improve.

One example of each kind:

```sql
SELECT * FROM pgrdf.sparql('SELECT * WHERE { SERVICE <http://example.org/q> { ?s ?p ?o } }');
-- ERROR:  0A000: sparql: unsupported algebra in select wrapper: SERVICE (federated query)

SELECT * FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ?o ');
-- ERROR:  42601: sparql: parse error: error at 1:28: expected one of ",", ".", ";", "{", "{|", "}", "~", [_]

SELECT pgrdf.materialize(1, 'bogus');
-- ERROR:  22023: materialize: unknown profile "bogus" (supported: 'owl-rl', 'rdfs')

SELECT pgrdf.graph_digest(99999);
-- ERROR:  42704: graph_digest: graph 99999 does not exist — an absent graph has no digest
--         (an EMPTY graph does: pgrdf.add_graph it first)

SELECT * FROM pgrdf.sparql('CREATE GRAPH <http://example.org/people>');
-- ERROR:  42710: sparql: CREATE GRAPH <http://example.org/people>: graph already exists

SET pgrdf.max_inmemory_triples = 4;
SELECT pgrdf.materialize(pgrdf.graph_id('http://example.org/people'), 'rdfs');
-- ERROR:  54000: materialize: graph 1 holds 5 triples, more than pgrdf.max_inmemory_triples=4;
--         materialize reads them all into backend memory

SELECT * FROM pgrdf.sparql(
  'INSERT DATA { GRAPH <http://example.org/people> { <http://example.org/a> <http://example.org/p> "a\u0000b" } }');
-- ERROR:  22P05: sparql: a term contains U+0000 (written \u0000), which PostgreSQL text cannot hold

SELECT pgrdf.unlock_graph(pgrdf.graph_id('http://example.org/people'), 'audit done');
-- ERROR:  55000: unlock_graph: graph 1 is not locked — nothing to unlock

SELECT pgrdf.stats();     -- as a role without EXECUTE on it
-- ERROR:  42501: permission denied for function stats
```

(In `psql`, `\set VERBOSITY verbose` shows the code next to the message.)

`22021` cannot be shown with a fresh database, because 0.6.41 no longer
stores such a term. Its message has this form:

```
ERROR:  22021: pgrdf: graph 7 holds a term containing U+0000 (dictionary id 4211), which cannot be exported or digested
HINT:  Dictionary id 4211 in graph 7 was stored by a version before 0.6.41; pgrdf.nul_term_report() lists every such term and the graphs using it. Reload graph 7 from a source with the character removed or replaced.
```

### XX000 means a defect, with named exceptions

On the query path (`sparql`, `construct`, `describe`, `sparql_sql`,
`sparql_parse`) every deliberate refusal has one of the codes above
since 0.6.41. Before that, unsupported SPARQL arrived as `XX000`. An
`XX000` from those functions now means pgRDF hit a state it should
never be in: please
[report it](https://github.com/styk-tv/pgRDF/issues) with the query.

A refusal on that path also no longer depends on the data. Some checks
used to run once per result row, so the same unsupported request
succeeded when its pattern matched nothing. It is now refused whatever
the graph holds.

Outside the query path, a few deliberate refusals still arrive as
`XX000` in 0.6.41. Their messages name the function and say what was
refused:

| Refusal | Message starts with |
|---|---|
| A document that is not valid Turtle or N-Triples | `load_turtle: turtle parse error` |
| A document that is not valid TriG or N-Quads | `parse_trig: quad parse error`, `parse_nquads: quad parse error` |
| A file the server cannot open | `load_turtle: failed to open` |
| `validate` under `strict => true` with a shapes graph that selects nothing | `validate: shapes graph … declares no SHACL target` |
| The staged loader on a server without `shared_preload_libraries = 'pgrdf'` | `pgrdf staged loader requires pgrdf in shared_preload_libraries` |
| A blank-node structure past the canonicalization budget | `pgRDF#117: canonicalization budget exceeded` |

Treat those as refusals, and any other `XX000` as a defect.

### In your driver

```js
// node-postgres
try { await client.query(sql); }
catch (e) {
  if (e.code === '55P03') { /* graph locked; e.message names the unlock */ }
  if (e.code === '0A000') { /* unsupported construct, named in e.message */ }
  if (e.code === '42704') { /* no such graph */ }
  if (e.code === 'XX000') { /* a defect, unless it is one of the exceptions listed above */ }
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
-- {"load": null, "filter_clauses_dropped": 0, "path_depth_truncations": 1}
```

Both zero means the last call's answer was complete. Other sessions
can't affect these numbers. Since 0.6.41 the object also has a `load`
key: the [load report](02-loading-rdf.md#load-reports) when the
session's last pgRDF call was a load, `null` otherwise.

**3. Refuse partial answers outright:**

```sql
SET pgrdf.on_path_truncation = 'error';
-- the same query now fails:
-- ERROR:  54000: sparql: property path truncated at pgrdf.path_max_depth=1
--         (pgrdf.on_path_truncation=error forbids partial results)
```

## Settings

Every setting in this table can be changed per session with `SET`, by
any role. The settings for graph ownership, declared identity and
notifications, two of which only a superuser can change, are listed
with these in the
[function reference](09-function-reference.md#settings).

| Setting | Default | What it does |
|---|---|---|
| `pgrdf.path_max_depth` | `64` | Maximum distance of property-path (`+`, `*`) walks from a start node. |
| `pgrdf.on_path_truncation` | `warn` | `warn` returns partial results with a warning; `error` refuses them (`54000`); `count` returns them silently and only records the truncation in `last_call_stats()` / `stats()`. |
| `pgrdf.path_max_pairs` | `1000000` | How many (start, reached) pairs one property-path walk may hold in memory; past it the query refuses (`54000`). Bind an endpoint of the path and the walk starts there instead. |
| `pgrdf.diff_max_rows` | `5000000` | Memory bound of `graph_diff` / `graph_diff_summary`; past it the call refuses (`54000`). |
| `pgrdf.digest_max_bnode_quads` | `5000000` | How many blank-node triples `graph_digest`, `structural_digest` and `graph_manifest` may hold in memory; past it the call refuses (`54000`). Triples without blank nodes are streamed and do not count. |
| `pgrdf.max_inmemory_triples` | `0` (no limit) | Largest graph `materialize` and `validate` will read into memory; a larger one is refused (`54000`) before anything is read. For `validate` the count is the data graph plus the shapes graph. |
| `pgrdf.auto_analyze` | `on` | Refresh planner statistics after loads and `materialize`. |
| `pgrdf.bulk_defer_index_min` | `100000` | Above this many triples, bulk loads drop and rebuild indexes instead of maintaining them row by row. |
| `pgrdf.dict_batch_size` | `500` | Terms per dictionary batch in the standard loader. |
| `pgrdf.ingest_dict_path` | `combined` | Dictionary resolution strategy used by the standard loader. |
| `pgrdf.shmem_prewarm_on_init` | `off` | Warm the shared term cache before the first load after a restart. |
| `pgrdf.staged_resolve_strategy` | `index` | Join strategy of the staged bulk loader. |
| `pgrdf.staged_temp_tablespaces` | (empty) | Tablespace(s) for the staged loader's temporary files, for example a separate scratch disk. |

List them on your server with:

```sql
SELECT name, setting, context, short_desc FROM pg_settings WHERE name LIKE 'pgrdf.%';
```

`context` is `user` for a setting any role may change and `superuser`
for the two only a superuser may.

The `pgrdf.` prefix is reserved (PostgreSQL 15 and later): setting a
name pgRDF does not define is an error, so a typo cannot pass for a
setting.

```sql
SET pgrdf.no_such_setting = 1;
-- ERROR:  42602: invalid configuration parameter name "pgrdf.no_such_setting"
-- DETAIL:  "pgrdf" is a reserved prefix.
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

## Diagnostic functions

Each of these answers one question about the state of a database. For
the first three, no rows is the healthy answer.

| Function | Question | Details |
|---|---|---|
| `pgrdf.duplicate_report()` | Does any graph still hold repeated triples from a version before 0.6.41? | [Managing graphs](05-graphs.md#repeated-triples-from-earlier-versions) |
| `pgrdf.nul_term_report()` | Is any term stored that contains U+0000? | [Identity and export](07-identity-and-export.md#a-stored-u0000) |
| `pgrdf.ownership_drift()` | Does the storage owner lack anything pgRDF needs? | below |
| `pgrdf.orphan_partitions()` | Is there storage that belongs to no graph? | [Managing graphs](05-graphs.md#the-inventory) |
| `pgrdf.graph_activity()` | Who is inside a graph-level act right now? | [Managing graphs](05-graphs.md#who-is-working-on-a-graph-right-now) |
| `pgrdf.graph_acts()` | Who did what to a graph, and when? | [Managing graphs](05-graphs.md#the-act-record) |
| `pgrdf.graph_integrity(g)` | Is every triple of this graph well-formed, and does its stored count agree with a scan? | [Managing graphs](05-graphs.md#checking-a-graphs-health) |
| `pgrdf.graph_totals()` | How many graphs and triples am I listed, out of how many graphs, and is any count unverified? | [Managing graphs](05-graphs.md#totals) |
| `pgrdf.count_terms()` | How many terms does the dictionary hold? | [Managing graphs](05-graphs.md#the-term-count) |
| `pgrdf.graph_act_feed()` | What has been done since I last read the act record? | [Managing graphs](05-graphs.md#following-the-act-record) |

### ownership_drift

pgRDF does a few things as the *storage owner*, the role that owns
`pgrdf._pgrdf_quads`: it creates, empties and removes graph partitions,
allocates graph ids, writes the act record and, since 0.6.42, keeps the
[stored counts](05-graphs.md#stored-counts). When a deployment has
handed the tables to a role of its own, `ownership_drift()` lists every
storage object that role does not own:

| Column | Meaning |
|---|---|
| `relname`, `kind` | The object: a `table`, `partition`, `sequence`, or the `schema`. |
| `owner`, `storage_owner` | Who owns it now, and the storage owner. |
| `blocking` | Whether this stops pgRDF from working. |
| `cure` | The statement that hands the object to the storage owner. |

`blocking` is true exactly when the storage owner lacks what pgRDF does
with that object as that role (since 0.6.41; before, tables that block
nothing were marked too, #160):

| Object | The storage owner needs |
|---|---|
| a graph partition | to own it: pgRDF drops, empties, indexes and puts triggers on it |
| `_pgrdf_graphs` | `SELECT` and `UPDATE` |
| `_pgrdf_graph_acts` | `INSERT` |
| `_pgrdf_graph_counts`, `_pgrdf_graph_count_deltas`, `_pgrdf_graph_count_bypass` | to own them: pgRDF keeps the stored counts there, and their guard admits no other role (since 0.6.42) |
| `_pgrdf_graph_id_seq` | `USAGE`, `SELECT` and `UPDATE` |
| `_pgrdf_graph_act_id_seq` | `USAGE` |
| the `pgrdf` schema | `USAGE` and `CREATE` |
| anything else | nothing: listed, never blocking |

Holding the object owner's privileges always satisfies a row. A worked
example is in the
[install guide](01-install.md#giving-pgrdfs-storage-to-another-role).

The update to 0.6.42 creates the three count tables owned by the
storage owner, so a database that was only updated lists none of them.

## Instance counters

`pgrdf.stats()` returns cumulative, server-wide counters: shared term
cache hits and misses, plan cache hits and misses, and the running
totals of truncations and dropped filters. Use it for health
dashboards. For "was *my* query complete", use `last_call_stats()`.

The counters cover every database on the server, so since 0.6.41
`stats()` is not PUBLIC: superusers and the role that installed the
extension may call it, and a monitoring role needs
`GRANT EXECUTE ON FUNCTION pgrdf.stats() TO <role>`.
`last_call_stats()` stays PUBLIC.

## Next

[09 — Function reference](09-function-reference.md)
