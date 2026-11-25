# 05 — Managing graphs

Every triple in pgRDF belongs to a **named graph**. A graph has an IRI,
the name you use in SPARQL, and a numeric id, which the SQL functions
take. This page covers creating graphs, finding them, copying and
carving them, locking them, who owns them and who did what to them,
and checking their health.

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
- A graph is a **set** of triples: it never stores the same triple
  twice. See [loading](02-loading-rdf.md#a-graph-is-a-set).
- A graph records the role that created it, and every graph-level act
  (create, load, clear, lock, …) is written to
  [the act record](#the-act-record).
- pgRDF keeps a count of each graph's triples as it writes (since
  0.6.42), so [listing graphs](#the-inventory) does not read the
  triples.

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

**The IRI is the graph's identity; the id is a handle.** Ids are never
reused: a dropped graph's id is not handed out again, and an IRI that is
dropped and created again gets a new id. Gaps in the numbering are
normal. Keep IRIs in anything that outlives a session (logs, ledgers,
backup names) and look the id up with `graph_id(iri)` when you need it.

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

`pgrdf.graph_inventory()` lists graphs with their size, lock state,
reasoning freshness, what they were loaded from, and who owns them.
Since 0.6.42 the listing follows the role that asks: an admin is listed
every graph, any other role the graphs it owns (see
[which graphs are listed](#which-graphs-are-listed)). As a superuser:

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
| `owner` | The [graph owner](#graph-owner), a role name. `NULL` for a graph created before 0.6.41 and for graph `0`. |
| `created_at` | When the graph was created. `NULL` like `owner`. |
| `created_workspace`, `created_agent`, `created_thread` | What the creating client [declared about itself](#declared-identity), if anything. |
| `locked_by` | The role that took the current lock. `NULL` for an open graph. |
| `change_seq` | A number that moves with every committed change to the graph's triples. Since 0.6.42; see [stored counts](#stored-counts). |
| `counts_verified` | `t`: `asserted` and `inferred` are the count pgRDF keeps. `f`: the graph was counted by scan for this call. The numbers are exact either way. Since 0.6.42. |
| `counts_scanned_at` | When the kept count was last set from the whole graph. `NULL` for a graph that has none yet, which is the state after a restore. Since 0.6.42. |
| `last_act_id` | The `act_id` of the graph's newest row in [the act record](#the-act-record). `NULL` when it has none. Since 0.6.42. |

The four columns added in 0.6.42 come last; the sixteen before them
keep their order.

`asserted` and `inferred` count stored rows. They equal the number of
triples except for a graph that
[still holds repeats](#repeated-triples-from-earlier-versions) from a
version before 0.6.41.

Because it's a normal set-returning function, you can filter, join and
aggregate it like a table:

```sql
SELECT iri, asserted + inferred AS total
  FROM pgrdf.graph_inventory()
 ORDER BY total DESC LIMIT 5;
```

For a single number, `pgrdf.count_quads(graph_id)` returns asserted plus
inferred triples for one graph. It answers for any graph id, to any
role that holds `SELECT` on `pgrdf._pgrdf_quads`; the listing rule does
not apply to it. Since 0.6.42 it reads the
[stored count](#stored-counts) when that count is verified, and counts
the triples otherwise.

`pgrdf.orphan_partitions()` lists storage partitions that no longer
belong to any graph. It should return no rows. If it returns some,
that storage can't be reached through SPARQL.

### One graph

Since 0.6.42 the inventory also answers for one graph, by id or by IRI,
with the row the full listing has for it:

```sql
SELECT graph_id, asserted, inferred, materialization
  FROM pgrdf.graph_inventory('http://example.org/people');
--  graph_id | asserted | inferred | materialization
-- ----------+----------+----------+-----------------
--         1 |       14 |        8 | stale

SELECT asserted, inferred FROM pgrdf.graph_inventory(1);   -- the same graph, by id
```

There are three answers, and they never look alike:

| The graph | Answer |
|---|---|
| is listed to you | its row; an empty graph shows `0` and `0` |
| exists and is not listed to you | refused with `42501` |
| does not exist | no row |

A quoted number is read as an IRI: `graph_inventory('42')` looks for a
graph named `42`. Write `graph_inventory(42)` for the id.

Ask this way when you want one graph. A filter on the full listing
(`FROM pgrdf.graph_inventory() WHERE graph_id = 1`) builds every row
first.

### Stored counts

Since 0.6.42 pgRDF keeps the asserted and inferred count of every graph
and changes it in the transaction that changes the triples. The
inventory reads those counts and no longer counts triples, so a call
takes milliseconds whatever the graphs hold: its cost follows the
number of graphs, not the number of triples. Before 0.6.42 every call
counted every triple of every graph. The count is of the set a graph
holds: a bulk or streaming load of a file that states a triple twice
counts it once.

One thing slows it: a transaction that holds a transaction id and
stays open, most often a session left idle in a transaction. The writes
other sessions commit after it began are added up on every call until
it ends. The counts are exact throughout. The listing calls themselves
(`graph_inventory()` in its three forms, `graph_totals()` and
`count_quads()`) take no transaction id, so a session that only lists
graphs holds nobody up. Other pgRDF read functions still take one
(#177). Measured times are in the [changelog](../CHANGELOG.md).

**`change_seq` works as a cache key.** It moves whenever a committed
write changes the graph's triples, and stays where it is when a write
stores nothing, such as a load of triples the graph already holds. It
never goes back. While `counts_verified` is true, two reads that show
the same `graph_id` and `change_seq` saw the same triples. A client can
keep what it derived from a graph beside that pair, and derive it again
when the number differs.

- A recount and a clear move it even when no triple changed. Read a new
  value as "look again", not as proof of a change.
- An act that touches no triple (a lock, a validation, a change of
  owner) does not move it. `last_act_id` moves with those.
- It belongs to one graph in one database on one server. It starts
  again after a dump and restore, and for a graph that is dropped and
  created again. Do not compare a value read from a standby with one
  read from its primary.

**A count is verified** when a scan set it and every change since came
through pgRDF's own functions. That is the normal state. A write made
in plain SQL on the table that holds the triples leaves a count
unverified:

- on a graph's own partition (`pgrdf._pgrdf_quads_g<id>`): that graph;
- through the parent table `pgrdf._pgrdf_quads`: every graph, because
  such a statement does not say which graphs it touches.

An unverified graph is counted by scan whenever it is asked for, so
`asserted` and `inferred` are exact in every case. What is lost is the
speed, for that graph, until it is counted again. A plain SQL write
that is rolled back changes nothing.

```sql
SELECT pgrdf.add_graph('http://example.org/scratch');   -- 3
SELECT pgrdf.parse_turtle(
  '<http://example.org/a> <http://example.org/p> <http://example.org/b> .', 3);

DELETE FROM pgrdf._pgrdf_quads_g3;   -- plain SQL, on the graph's own partition

SELECT graph_id, asserted, counts_verified FROM pgrdf.graph_inventory();
--  graph_id | asserted | counts_verified
-- ----------+----------+-----------------
--         0 |        0 | t
--         1 |       14 | t
--         2 |       10 | t
--         3 |        0 | f
```

`pgrdf.recount_graph(graph_id)` counts the graph again, stores what it
finds and marks the count verified. It returns the count found beside
the count that was stored:

```sql
SELECT * FROM pgrdf.recount_graph(3);
--  graph_id | asserted | inferred | was_asserted | was_inferred | was_verified
-- ----------+----------+----------+--------------+--------------+--------------
--         3 |        0 |        0 |            1 |            0 | f
```

- It is allowed to the graph's owner and to a role that holds the
  storage owner's privileges. Another role's graph is refused with
  `42501`, an unknown graph id with `42704`.
- Being listed a graph is not enough. A role that sees every graph
  through `pg_read_all_data` is refused too: a recount leaves a row in
  the act record of the graph.
- `pgrdf.recount_graph()`, without an argument, recounts every graph
  you own, one row each, and every graph when you hold the storage
  owner's privileges. That is the cure after a write through the parent
  table.
- It changes no triple, so it runs on a locked graph.
- It can run while other sessions write. A write through pgRDF that
  commits meanwhile is counted. A plain SQL write that commits after
  the scan leaves the count unverified again.
- Each graph recounted gets a `recount` row in
  [the act record](#the-act-record).

**After a restore** every graph is unverified, because the stored
counts are not part of a dump. Run `SELECT * FROM pgrdf.recount_graph();`
once, as a superuser or as the storage owner. See
[dump and restore](01-install.md#dump-and-restore).

**What goes unnoticed.** The check stands on triggers. A privileged
role, the owner of the pgRDF tables or a superuser, can switch them off
or go around them and leave a count that reads verified and is wrong:

- `SET session_replication_role = replica`, which is also how a
  logical replication subscriber applies changes;
- `ALTER TABLE … DISABLE TRIGGER` on a storage table;
- `pg_restore --disable-triggers`;
- attaching, detaching or dropping a partition that holds rows;
- writing the count tables directly.

No grant puts another role among them. A direct write to the count
tables (`pgrdf._pgrdf_graph_counts`, `_pgrdf_graph_count_deltas`,
`_pgrdf_graph_count_bypass`) is refused with `42501` for every role
that does not hold the table owner's privileges, whatever it was
granted.

After any of these, run `recount_graph`.
[`graph_integrity()`](#checking-a-graphs-health) sets the stored count
beside a scan and says whether they agree.

### Totals

`pgrdf.graph_totals()` (since 0.6.42) adds the listing up in one row.
With the three graphs listed at the top of this section, as a
superuser:

```sql
SELECT * FROM pgrdf.graph_totals();
--  scope | graphs_listed | graphs_total | asserted | inferred | unverified_graphs | terms
-- -------+---------------+--------------+----------+----------+-------------------+-------
--  all   |             3 |            3 |       24 |        8 |                 0 |
```

| Column | Meaning |
|---|---|
| `scope` | `all` when the caller is listed every graph, `own` when it is listed the graphs it owns. |
| `graphs_listed` | How many graphs the caller is listed. |
| `graphs_total` | How many graphs the database holds, for every caller. |
| `asserted`, `inferred` | Sums over the listed graphs. |
| `unverified_graphs` | Listed graphs whose count was not verified, and so were counted by scan for this call. |
| `terms` | Reserved for the number of dictionary terms. `NULL` in 0.6.42, for every caller. |

A total is never the database's unless `scope` says `all`. A client
that compares listings over time can tell "no longer listed to me" from
"dropped" by `graphs_total`.

## Clear, copy, move, drop

All four take either graph ids or graph IRIs.

| Function | Effect | Returns |
|---|---|---|
| `clear_graph(g)` | Removes every triple. The graph itself stays. | triples removed |
| `copy_graph(src, dst)` | Adds all of `src` to `dst`, **including inferred triples**. | triples `dst` gained |
| `move_graph(src, dst)` | Moves `src` into an **empty** `dst`, then removes `src`. | triples moved |
| `drop_graph(g, cascade => true)` | Removes the graph and its triples. | triples removed |

```sql
SELECT pgrdf.add_graph('http://example.org/people-backup');
SELECT pgrdf.copy_graph('http://example.org/people', 'http://example.org/people-backup');
-- → 22
SELECT pgrdf.copy_graph('http://example.org/people', 'http://example.org/people-backup');
-- → 0      (the destination already holds every triple)

SELECT pgrdf.graph_digest(pgrdf.graph_id('http://example.org/people')) =
       pgrdf.graph_digest(pgrdf.graph_id('http://example.org/people-backup')) AS same_graph;
-- → true
```

Details that matter:

- **Copy by IRI needs the destination to exist.** Create it with
  `add_graph` first, or you get `42704`. Copy by id creates a missing
  destination for you, named `urn:pgrdf:graph:<id>`.
- **Copy adds, as a set.** A triple the destination already holds is
  not stored again, and the return value is what the destination
  gained (since 0.6.41; it used to be the source's size). For an exact
  copy, start from an empty destination. A copy's inferred triples
  show `materialization = unknown`. Run `materialize` on the copy if
  you rely on them.
- **Clear resets what described the old content.** After `clear_graph`
  the inventory shows `source_sha256` and `source_loads` as `NULL` and
  `materialization` as `never`. What the graph was loaded from stays in
  [the act record](#the-act-record).
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

Like `copy_graph`, both forms return what the destination gained: run
the same carve again and it returns 0.

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
- The inventory's `locked` / `lock_reason` / `locked_by` columns show
  current locks and the role that took each. Both reasons, the lock's
  and the unlock's, are kept in [the act record](#the-act-record).
- Every refusal carries the cure as a hint: `pgrdf.unlock_graph(<id>,
  '<reason>')`.
- A locked graph's `graph_digest` is computed once and then served from
  the inventory's `identity_digest`; unlocking clears it.
- A lock holds against every role except the owner of the pgRDF tables
  and superusers, who can drop its triggers. Use normal PostgreSQL roles
  and privileges to decide who those are.

## Graph owner

A graph records the role that created it, by name. After `SET ROLE` it
is the role acted as, not the login. `graph_inventory()` shows it as
`owner`:

```sql
SET ROLE alice;
SELECT pgrdf.add_graph('http://example.org/catalogue');
RESET ROLE;

SELECT iri, owner FROM pgrdf.graph_inventory() WHERE iri = 'http://example.org/catalogue';
--              iri              | owner
-- ------------------------------+-------
--  http://example.org/catalogue | alice
```

The owner is a pgRDF record, not a PostgreSQL object owner: the
graph's storage stays with the owner of the pgRDF tables (the *storage
owner*). Graphs that existed before 0.6.41 have no recorded owner; the
column reads `NULL`, never a guess.

**By default the owner reserves nothing.** Table privileges alone say
who may do what, as before. The owner decides one thing without any
setting: [which graphs a role is listed](#which-graphs-are-listed)
(since 0.6.42). A superuser can reserve the destructive acts for the
owner, usually per database:

```sql
ALTER DATABASE mydb SET pgrdf.graph_owner_enforce = on;   -- new sessions
```

With `pgrdf.graph_owner_enforce` on, `drop_graph`, `clear_graph`,
`lock_graph`, `unlock_graph`, `dedup_graph` and `set_graph_owner`
refuse with `42501` unless the caller holds the privileges of the
graph's owner or of the storage owner:

```sql
SET ROLE bob;
SELECT pgrdf.clear_graph('http://example.org/catalogue');
-- ERROR:  42501: clear_graph: graph 1 is owned by alice; role bob is neither that owner nor
--         the storage owner (pgrdf.graph_owner_enforce is on)
-- HINT:  Act as alice, or have alice or the storage owner run pgrdf.set_graph_owner(1, '<role>').
```

What the setting does not do:

- It does not reserve writes. Loading into the graph and SPARQL
  `INSERT` / `DELETE` stay governed by table privileges.
- It does not cover a graph with no recorded owner. Such a graph stays
  governed by privileges alone until someone claims it.
- Only a superuser can change it, so a role it restricts cannot switch
  it off.

"Holds the privileges of" means the role is the owner, or a member
that inherits from it. A login that may `SET ROLE` to the owner without
inheriting owns nothing until it does switch.

### Which graphs are listed

Since 0.6.42 `graph_inventory()` and `graph_totals()` answer for the
role that calls them. Before, every role that could call the inventory
was listed every graph.

| Caller | Listed |
|---|---|
| an admin | every graph |
| any other role | the graphs it owns |

An **admin** is a superuser, a role that holds the storage owner's
privileges, or a role that holds `pg_read_all_data`. A role **owns** a
graph when it holds the privileges of the graph's recorded owner: the
test `pgrdf.graph_owner_enforce` uses for the destructive acts, so
there is one notion of owning a graph for both. The listing rule does
not depend on that setting. It is always on.

```sql
SET ROLE alice;
SELECT iri, owner FROM pgrdf.graph_inventory();
--              iri              | owner
-- ------------------------------+-------
--  http://example.org/catalogue | alice

SET ROLE bob;
SELECT iri, owner FROM pgrdf.graph_inventory();
-- (0 rows)

SELECT * FROM pgrdf.graph_inventory('http://example.org/catalogue');
-- ERROR:  42501: pgrdf: graph_inventory: that graph is not listed to this role
-- HINT:  pgrdf.graph_inventory() lists the graphs this role owns; the graph's owner or the
--        storage owner can hand it over with pgrdf.set_graph_owner(graph_id, role).
```

- "Holds" means the privileges are in effect, as above. A role granted
  `pg_read_all_data`, or an owner's role, `WITH INHERIT FALSE` is listed
  nothing more until it switches with `SET ROLE`.
- The listing follows `current_user`: after `SET ROLE`, the role acted
  as; inside a `SECURITY DEFINER` function, that function's owner.
- A graph with no recorded owner is listed to admins only. That is
  every graph created before 0.6.41, and graph `0`.
  [`set_graph_owner`](#transferring-a-graph) gives such a graph an
  owner.
- Being listed needs `USAGE` on the schema and `EXECUTE` on the
  function, and no privilege on the pgRDF tables.
- A role that should be listed everything has to be an admin.
  `GRANT pg_read_all_data TO app` is the grant that adds no right to
  write; it lets the role read every table on the server. It lists
  every graph and makes none of them the role's own:
  [`recount_graph`](#stored-counts) still refuses another role's graph.
- The owner is recorded by role name. A role that is dropped and
  created again under the same name owns, and is listed, the graphs of
  the old one. A renamed role no longer owns its graphs until
  `set_graph_owner` says so.

**This is a listing rule, not secrecy.** It decides what these two
functions show and nothing about what a role can read. A role that
holds `SELECT` on the pgRDF tables can query any graph, count it with
`count_quads()` or a SPARQL query, and look it up with
`graph_id(iri)`, listed or not. The act record names every graph and
is readable by every role with `USAGE` on the schema. Tenants that must
not see each other's graphs belong in separate databases: the database
is the boundary.

### Transferring a graph

`pgrdf.set_graph_owner(graph_id, role)` hands a graph to another role
and returns the new owner's name. It works whether or not
`pgrdf.graph_owner_enforce` is on.

```sql
SELECT pgrdf.set_graph_owner(pgrdf.graph_id('http://example.org/catalogue'), 'bob');
--  set_graph_owner
-- -----------------
--  bob
```

- The current owner may transfer, and so may the storage owner.
- As with `ALTER … OWNER TO`, the caller must be able to act as the new
  owner (`GRANT bob TO alice`), unless the caller is the storage owner.
- A graph with no recorded owner can be claimed by a role that holds
  the grants to drop it.
- An unknown graph or role is refused with `42704`, a caller who may
  not transfer with `42501`.
- The transfer is recorded as a `set_owner` act with the old and the
  new owner.

## The act record

pgRDF keeps one row for every **graph-level act**: a graph was
created, loaded, cleared, locked, validated. Never one per triple.
`pgrdf.graph_acts()` reads it:

```text
pgrdf.graph_acts(graph_id bigint DEFAULT NULL, since timestamptz DEFAULT NULL)
```

Both arguments are optional filters; rows come oldest first. To keep up
with the record as it grows, read it through
[`graph_act_feed()`](#following-the-act-record), not by polling with
`since`.

For the `catalogue` graph from the previous section, after `alice`
loaded it, locked it and unlocked it, and a superuser handed it to
`bob`:

```sql
SELECT act_id, act, acting_role, workspace, detail
  FROM pgrdf.graph_acts(pgrdf.graph_id('http://example.org/catalogue'));
--  act_id |    act    | acting_role | workspace |                 detail
-- --------+-----------+-------------+-----------+----------------------------------------
--       1 | create    | alice       |           | {"iri": "http://example.org/catalogue"}
--       2 | load      | alice       | catalogue | {"load": {"triples_read": 5, "triples_stored": 5, …},
--         |           |             |           |  "entry": "parse_turtle", "route": "combined"}
--       3 | lock      | alice       |           | {"reason": "frozen for audit"}
--       4 | unlock    | alice       |           | {"reason": "audit done"}
--       5 | set_owner | postgres    |           | {"to": "bob", "from": "alice"}
```

The refused `clear_graph` by `bob` is not there: it did not happen.

What you can rely on:

- **Only committed acts are recorded.** The row is written in the act's
  own transaction; an act that fails or is rolled back leaves none.
- **pgRDF writes it.** A role granted every privilege on every pgRDF
  table still cannot insert, change or delete a row: a direct write is
  refused with `42501`. Only a role holding the storage owner's
  privileges, or a superuser, can write it directly.
- **Anyone with `USAGE` on the `pgrdf` schema can read it.** Unlike
  the inventory, it is not narrowed by role: every reader sees the acts
  of every graph.
- **An act made of other acts is one row.** A `move_graph` is recorded
  as `move`, not as a copy and a drop. The exception is `create`: a
  graph created on the way (by a load into a new graph, say) always
  gets its `create` row.
- **Reads are not acts.** Queries, exports and digests record nothing.
  The whole-graph ones are visible while they run, in
  [`graph_activity()`](#who-is-working-on-a-graph-right-now).
- **A read-only transaction records nothing.**

### Who acted: verified and declared

Each row keeps two kinds of identity apart.

| Columns | Kind | Where it comes from |
|---|---|---|
| `session_role`, `acting_role` | **Verified** | PostgreSQL: the role that logged in (`session_user`) and the role in effect when the act started (`current_user`, which `SET ROLE` changes). |
| `application_name` | Declared | The connection's `application_name`. |
| `requester` | Declared | The session setting `ckp.requester`, when a client sets it. |
| `workspace`, `agent`, `thread`, `on_behalf_of`, `harness` | Declared | The five [`pgrdf.*` identity settings](#declared-identity). |

Verified columns are what PostgreSQL authenticated. Declared columns
are what the client says about itself: useful for telling apart many
clients that share one login, and never proof of anything. pgRDF never
uses a declared value to authorise an act.

The other columns: `act_id` (ascending), `graph_id` and `graph_iri`
(the IRI as it was, kept after the graph is dropped), `act`, `at` (when
the act started), `xact` (the transaction id; acts of one transaction
share it) and `detail`.

### The acts

| `act` | Recorded when | `detail` holds |
|---|---|---|
| `create` | a graph is created, by any route | `iri` |
| `rename` | a graph created by id receives its IRI | `from`, `to` |
| `load` | a loader call ends | `entry` (the function called), `route` (the strategy), `load` (the [load report](02-loading-rdf.md#load-reports)); for TriG and N-Quads also `graphs`, and one row per graph written |
| `update` | a SPARQL UPDATE changed triples of the graph | `form`, `inserted`, `deleted` |
| `clear` | `clear_graph`, `CLEAR GRAPH` | `rows` removed |
| `drop` | `drop_graph`, `DROP GRAPH` | `rows` removed, `cascade` |
| `copy` | `copy_graph`, on the destination | `from` (source graph id), `rows_read`, `rows` gained |
| `move` | `move_graph`, one row on each graph | destination: `from`, `rows`; source: `to`, `rows` |
| `carve` | `carve_graph`, on the destination | `from`, `by` (`predicate` or `neighbourhood`), `rows` gained, and `predicate`, `rows_read` or `hops`, `boundary` |
| `lock`, `unlock` | `lock_graph`, `unlock_graph` | `reason` |
| `materialize` | `materialize` | `profile`, `base_triples`, `inferred_written`, `previous_inferred_dropped` |
| `validate` | `validate` answers, on the data graph | `conforms`, `results` (how many), `mode`, `shapes_graph_id`, `shapes_graph_iri`, `shapes_digest`, `materialization` (the data graph's state when it was read). See [validation](06-validation-recipes.md#the-verdict-is-recorded). |
| `dedup` | `dedup_graph`, or a staged load that removed repeats | `removed`, and `asserted_removed` or `during` |
| `set_owner` | `set_graph_owner` | `from`, `to` |
| `recount` | `recount_graph`, one row per graph counted (since 0.6.42) | `asserted`, `inferred` (the count found), `was_asserted`, `was_inferred`, `was_verified` (the count that was stored) |
| `ingest` | the internal JSON ingest functions | `rows`, `stored` |

### Declared identity

Five settings let a client say who it is. Their values are copied into
the act record and `graph_activity()` as they stand when an act
starts.

| Setting | Meant for |
|---|---|
| `pgrdf.workspace` | the workspace the client works in |
| `pgrdf.agent` | the agent or seat acting |
| `pgrdf.thread` | the conversation or session thread |
| `pgrdf.on_behalf_of` | the person the client acts for |
| `pgrdf.harness` | the client software, as `<name>/<version>` |

Any role may set them. A value takes at most 128 bytes and no control
characters; anything else is refused at `SET` (`22001` and `22023`),
never truncated or cleaned, so a reader of the act record sees exactly
what was set. Empty means not declared and is stored as `NULL`.

This is the load recorded as act 2 above:

```sql
BEGIN;
SET LOCAL pgrdf.workspace = 'catalogue';
SET LOCAL pgrdf.agent     = 'nightly-import';
SELECT pgrdf.parse_turtle($$
@prefix ex:   <http://example.org/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .
ex:alice a foaf:Person ; foaf:name "Alice" ; foaf:knows ex:bob .
ex:bob   a foaf:Person ; foaf:name "Bob" .
$$, pgrdf.graph_id('http://example.org/catalogue'));
COMMIT;
```

The lock and unlock that followed were made outside that transaction,
so their rows carry no workspace.

**If your client shares connections, set all five on every
transaction.** A plain `SET` lasts for the session, and with a
connection pool the session outlives the request: the next request on
that connection would record the previous one's values. `SET LOCAL`,
or `set_config(name, value, true)`, lasts for the transaction only.
Set all five each time, with `''` for the ones you do not use, so that
nothing an earlier user of the connection left behind is recorded as
yours.

```sql
BEGIN;
SELECT set_config('pgrdf.workspace',    'catalogue',         true),
       set_config('pgrdf.agent',        'nightly-import',    true),
       set_config('pgrdf.thread',       'run-42',            true),
       set_config('pgrdf.on_behalf_of', 'alice@example.org', true),
       set_config('pgrdf.harness',      'importer/1.4',      true);
SELECT pgrdf.add_graph('http://example.org/pool-1');
COMMIT;

SELECT pgrdf.add_graph('http://example.org/pool-2');   -- next transaction, nothing declared

SELECT graph_iri, act, workspace, agent, thread, on_behalf_of, harness
  FROM pgrdf.graph_acts() WHERE graph_iri LIKE '%pool%';
--          graph_iri         |  act   | workspace |     agent      | thread |   on_behalf_of    |   harness
-- ---------------------------+--------+-----------+----------------+--------+-------------------+--------------
--  http://example.org/pool-1 | create | catalogue | nightly-import | run-42 | alice@example.org | importer/1.4
--  http://example.org/pool-2 | create |           |                |        |                   |
```

### Following the act record

A client that shows acts as they happen, or drops a cache when a graph
changes, reads the record again and again and must not miss a row.
`graph_acts(NULL, since)` cannot promise that. It filters on `at`, the
moment an act started inside its transaction, and a transaction can
commit long after that moment. A load that runs for ten minutes is
recorded with the time it began; a client that has moved its `since`
past that time in the meantime never sees the load. Polling on `act_id`
has the same gap, because an id is taken when the row is written, not
when its transaction commits.

`pgrdf.graph_act_feed()` (since 0.6.42) reads the same record ordered
by transaction, and returns a cursor with it:

```text
pgrdf.graph_act_feed(after xid8 DEFAULT NULL, only_graph bigint DEFAULT NULL,
                     page text DEFAULT NULL, max_acts integer DEFAULT 1000)
```

Each row has the columns of `graph_acts()`, with two in front:

| Column | Meaning |
|---|---|
| `complete_below` | The cursor, a transaction id. Every act of every transaction below it has been delivered. The same on every row of a read. |
| `next_page` | `NULL`, or a token when more acts are waiting than `max_acts` allows in one call. |

The rule for a client:

1. Call with `after` set to the `complete_below` you kept from the
   read before. The first time, leave it out.
2. Show every act you get at once, except those whose `act_id` you have
   shown already.
3. While `next_page` is not `NULL`, call again with `page` set to it
   and the other arguments unchanged.
4. Keep `complete_below` for the next read.

```sql
-- the first read
SELECT complete_below, next_page, act_id, graph_iri, act, xact
  FROM pgrdf.graph_act_feed();

-- every later read: $1 is the complete_below you kept
SELECT complete_below, next_page, act_id, graph_iri, act, xact
  FROM pgrdf.graph_act_feed(after => $1::xid8);

-- a further page of the same read: $2 is the next_page you were given
SELECT complete_below, next_page, act_id, graph_iri, act, xact
  FROM pgrdf.graph_act_feed(after => $1::xid8, page => $2);
```

What you can rely on, and what to expect:

- **An act you are shown is committed and stays.** Nothing in the feed
  is provisional. What is still open is the range from `complete_below`
  upward, where a transaction that has not finished may yet add an act.
  That is why the next read starts at the cursor and returns the acts
  at or above it once more. Drop those by `act_id`. Acts below the
  cursor never come again, so the ids at or above it are all you need
  to remember.
- **A read always returns a row.** When there is no act to deliver, one
  row comes back whose `act_id` is `NULL`. It carries the cursor and is
  not an act.
- **One read, one cursor.** `complete_below` is the same on every page.
  Follow `next_page` to `NULL` before you keep it, and pass the token
  back as you received it.
- **Your own open transaction is left out.** Acts it has recorded can
  still roll back, so the feed shows them only after it commits.
- **The cursor waits for the oldest transaction on the server that
  holds a transaction id and has not ended**, in any database: a long
  load, or a session left idle in a transaction. Acts keep arriving
  meanwhile; each read repeats more of them until that transaction
  ends.
- **`only_graph` narrows the feed to one graph id.** The role does not
  narrow it: like `graph_acts()`, it shows the acts of every graph.
- **`max_acts` is the page size**, 1000 unless you say otherwise.
- **`complete_below` and `xact` are `xid8` values**, PostgreSQL's
  64-bit transaction id. Keep the cursor as text and pass it back with
  a cast, as above.
- **A cursor belongs to the server it was read from.** Acts restored
  from another server are delivered to a reader that starts from the
  beginning; a cursor from before the restore is not valid on the new
  server, so start again with `after` left out.

### Being told when an act commits

With `pgrdf.notify_acts` on, each recorded act is followed by a
notification on the channel `pgrdf_acts`, with the payload
`<act_id>:<graph_id>:<act>`. PostgreSQL delivers it when the act's
transaction commits, and not at all if it rolls back.

```sql
LISTEN pgrdf_acts;
SET pgrdf.notify_acts = on;
SELECT pgrdf.add_graph('http://example.org/notified');
-- Asynchronous notification "pgrdf_acts" with payload "6:2:create" received from
-- server process with PID 82237.
```

The setting is off by default and only a superuser can change it:
PostgreSQL serialises the commits of transactions that notify, across
every database of the server. Turn it on per database or per role with
`ALTER DATABASE … SET` / `ALTER ROLE … SET` where a listener needs it.

A notification is a wake-up, not a cursor. A listener that was
disconnected gets nothing for the time it was away. Let the
notification say when to read, and
[`graph_act_feed()`](#following-the-act-record) what happened.

## Who is working on a graph right now

`pgrdf.graph_activity(graph_id DEFAULT NULL)` lists the connections
that are inside a graph-level act at this moment, in this database: a
load, a clear, a materialize, a validation, or a whole-graph read
(`export`, `manifest`, `digest`, `structural_digest`). Called from a
second session while a load runs:

```sql
SELECT * FROM pgrdf.graph_activity();
-- pid              | 60889
-- graph_id         | 1
-- act              | load
-- started          | 2026-11-24 14:11:34.158185+00
-- part             | backend
-- session_role     | postgres
-- acting_role      | postgres
-- application_name | importer
-- requester        |
-- workspace        | catalogue
-- agent            | nightly-import
-- thread           |
-- on_behalf_of     |
-- harness          |
```

- It reports and never blocks. To stop others writing, use a
  [lock](#locking-a-graph).
- No rows means nobody is inside an act. Without `pgrdf` in
  `shared_preload_libraries` it always returns no rows.
- `part` is `backend`, or `coordinator` and `worker` for the processes
  of a staged load; workers show the identity of the caller they work
  for.
- Visibility follows `pg_stat_activity`: a session of another role
  shows only `pid` and `session_role`, unless you hold that role's
  privileges or `pg_read_all_stats`.
- It lists up to 256 connections at once; beyond that the surplus are
  not listed, and nothing waits or fails.

## Repeated triples from earlier versions

Before 0.6.41 the file loaders stored a triple as many times as the
source stated it, and again on every reload. From 0.6.41 a graph is a
set, but the upgrade rewrites no data: a graph that already holds
repeats keeps them until you remove them. `pgrdf.duplicate_report()`
lists such graphs:

```sql
SELECT * FROM pgrdf.duplicate_report();
-- graph_id             | 9
-- iri                  | http://example.org/old
-- rows                 | 3
-- distinct_rows        | 2
-- surplus              | 1
-- groups               | 1
-- max_multiplicity     | 2
-- set_enforced         | f
-- locked               | f
-- locked_digest_engine |
-- note                 | pgrdf.dedup_graph(9) removes the 1 surplus row(s) and builds the unique index
```

| Column | Meaning |
|---|---|
| `rows`, `distinct_rows` | Stored rows, and distinct triples among them. |
| `surplus` | `rows` minus `distinct_rows`: the rows a set would not have. |
| `groups` | Triples stored more than once. |
| `max_multiplicity` | The most copies any one triple has. |
| `set_enforced` | `f`: the graph has no unique index yet. |
| `locked`, `locked_digest_engine` | Lock state, and the pgRDF version that computed the locked graph's recorded digest; empty when none was computed by 0.6.41 or later. See [identity](07-identity-and-export.md#digests-describe-the-set-of-triples). |
| `note` | What to run. |

No rows is the healthy answer. The function only scans graphs without
the unique index, so it is cheap on a healthy database. One more row,
with `graph_id` `NULL`, appears if the dictionary holds the same term
under two ids, which can happen when two loads introduce the same new
term at the same moment. Exports and digests still treat it as one
term; `dedup_graph` does not remove it.

Until it is cured, such a graph behaves like this:

- It gains no further repeats: every write checks first.
- Queries, exports, digests and the manifest describe the set of
  triples, so a repeat does not change them. The manifest reports the
  surplus as `counts.repeats`.
- `count_quads()` and the inventory's `asserted` / `inferred` count
  stored rows, repeats included.

`pgrdf.dedup_graph(graph_id)` removes the surplus rows and builds the
unique index. It returns the number of rows removed.

```sql
SELECT pgrdf.dedup_graph(pgrdf.graph_id('http://example.org/old'));
--  dedup_graph
-- -------------
--            1
```

- The set of triples does not change. Of each group of repeats one row
  is kept, an asserted one before an inferred one, and a `current`
  materialization stays `current`.
- It needs what `clear_graph` needs: `SELECT` and `DELETE` on
  `pgrdf._pgrdf_quads`.
- It is recorded as a `dedup` act.
- On a graph that is already a set it does nothing and returns 0. An
  unknown graph id is refused with `42704`.
- The `NOTICE` is expected: the index it is about to build is not
  there yet.

**A locked graph** refuses `dedup_graph` with `55P03`, like every other
write. Cure it in three steps:

```sql
SELECT pgrdf.unlock_graph(9, 'remove repeated rows');
SELECT pgrdf.dedup_graph(9);
SELECT pgrdf.lock_graph(9, 'release 7');
```

Locking again lets the graph record a fresh digest, computed over the
set. If you kept the digest of that graph from before, read
[identity and export](07-identity-and-export.md#digests-describe-the-set-of-triples).

## Checking a graph's health

`pgrdf.graph_integrity(graph_id)` checks that every triple is
well-formed: no literal subjects, no blank-node or literal predicates,
no references to missing terms. It also checks the graph's lock and,
since 0.6.42, its stored count.

```sql
SELECT pgrdf.graph_integrity(pgrdf.graph_id('http://example.org/people'));
-- {"clean": true, "graph_id": 1, "illegal_terms": 0, "dangling_refs": 0,
--  "counts": {"quads": 22, "subject_literal": 0, "predicate_bnode": 0, ...},
--  "stored_count": {"asserted": 14, "inferred": 8, "verified": true,
--                   "scanned_asserted": 14, "scanned_inferred": 8, "consistent": true},
--  "lock_custody": {"locked": false, "write_triggers": 0, "consistent": true}}
```

`stored_count` sets the [count pgRDF keeps](#stored-counts) beside a
scan made for this call. `consistent` is false only when a count that
reads verified differs from the scan. That state should never exist,
and it makes the graph not clean. An unverified count reads
`"verified": false` and is still consistent: it claims nothing.
`pgrdf.recount_graph(graph_id)` cures both.

`lock_custody` checks that a locked graph carries its two lock triggers
and an open one carries neither; a mismatch (someone removed a trigger
by hand) makes the graph not clean. Since 0.6.42 every graph's
partition also carries the two triggers that watch for plain SQL
writes (`pgrdf_count_bypass`, `pgrdf_count_bypass_truncate`). They
refuse nothing, and `write_triggers` does not count them.

A graph with illegal terms or dangling references should be reloaded
from source. Validation results on such a graph can't be trusted. An
unknown graph id is refused with `42704`.

## Looking up a term

Query results show terms by their text. If you hold a term id, for
example from a report, `pgrdf.get_term(id)` returns its text.

## Next

[06 — Validation](06-validation-recipes.md)
