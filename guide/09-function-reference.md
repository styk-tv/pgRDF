# 09 — Function reference

Every supported (`stable`) function, grouped by task. All live in the
`pgrdf` schema. The running server can list them itself:
`SELECT * FROM pgrdf.surface() WHERE class = 'stable'`.

A graph argument (`g`, `graph_id`, `src`, `dst`) is a numeric graph id.
Get one with `pgrdf.graph_id(iri)`. Several functions also accept IRIs
directly, as listed.

## Graphs

| Function | Returns | Purpose |
|---|---|---|
| `add_graph(iri text)` | `bigint` | Create a graph (or find the existing one) and return its id. |
| `create_graph(iri text)` | `bigint` | Create a graph, or refuse (`42710`) if the IRI exists; the hint names the existing graph and its source digest. |
| `can_create_graphs()` · `can_clear_graphs()` · `can_drop_graphs()` | `boolean` | Whether the current role holds the grants `add_graph` / `clear_graph` / `drop_graph` require. |
| `graph_id(iri text)` | `bigint` | Id for an IRI, or `NULL` if there is no such graph. |
| `graph_iri(id bigint)` | `text` | IRI for an id, or `NULL`. |
| `graph_inventory()` | table | Every graph: `graph_id`, `iri`, `asserted`, `inferred`, `locked`, `lock_reason`, `materialization`, `source_sha256`, `source_loads`, `identity_digest`, and since 0.6.41 `owner`, `created_at`, `created_workspace`, `created_agent`, `created_thread`, `locked_by`. |
| `count_quads(g bigint DEFAULT 0)` | `bigint` | Asserted + inferred triples in one graph. |
| `clear_graph(id bigint)` · `clear_graph(iri text)` | `bigint` | Remove all triples; keep the graph. Returns triples removed. Resets the graph's source digest and materialization state. |
| `copy_graph(src bigint, dst bigint)` · `copy_graph(src_iri text, dst_iri text)` | `bigint` | Add `src` to `dst`, inferred triples included. Returns the triples `dst` gained (since 0.6.41; before, the size of `src`). The IRI form needs `dst` to exist. |
| `move_graph(src bigint, dst bigint)` · `move_graph(src_iri text, dst_iri text)` | `bigint` | Move into an empty `dst`, then remove `src`. |
| `drop_graph(id bigint, cascade boolean DEFAULT true)` · `drop_graph(iri text, cascade boolean DEFAULT true)` | `bigint` | Remove a graph. With `cascade => false`, refuses if it holds inferred triples. |
| `carve_graph(src bigint, predicate text, dst bigint)` | `bigint` | Copy every triple using one predicate. Returns the triples `dst` gained (since 0.6.41). |
| `carve_graph(src bigint, seeds text[], dst bigint, max_hops integer DEFAULT 1)` | `bigint` | Copy the neighbourhood of seed IRIs, up to `max_hops` steps. Returns the triples `dst` gained (since 0.6.41). |
| `lock_graph(graph_id bigint, reason text)` | `boolean` | Make a graph read-only for every write path, plain SQL included. |
| `unlock_graph(graph_id bigint, reason text)` | `boolean` | Release a lock. |
| `set_graph_owner(graph_id bigint, role text)` | `text` | Hand a graph to another role; returns the new owner. Allowed for the current owner and the storage owner. Since 0.6.41. |
| `graph_acts(graph_id bigint DEFAULT NULL, since timestamptz DEFAULT NULL)` | table | The act record: one row per committed graph-level act, with `act_id`, `graph_id`, `graph_iri`, `act`, `at`, `xact`, `session_role`, `acting_role`, `application_name`, `requester`, `workspace`, `agent`, `thread`, `on_behalf_of`, `harness`, `detail`. Both arguments filter. Since 0.6.41. |
| `graph_activity(graph_id bigint DEFAULT NULL)` | table | Connections inside a graph-level act right now, in this database: `pid`, `graph_id`, `act`, `started`, `part`, and the identity columns of `graph_acts`. Reports, never blocks. Needs `shared_preload_libraries`. Since 0.6.41. |
| `duplicate_report()` | table | Graphs that still hold repeated triples from a version before 0.6.41: `graph_id`, `iri`, `rows`, `distinct_rows`, `surplus`, `groups`, `max_multiplicity`, `set_enforced`, `locked`, `locked_digest_engine`, `note`. Normally empty. Since 0.6.41. |
| `dedup_graph(graph_id bigint)` | `bigint` | Remove a graph's repeated rows and build its unique index; returns the rows removed. Refuses a locked graph (`55P03`). Since 0.6.41. |
| `graph_integrity(graph_id bigint)` | `jsonb` | Check every triple is well-formed and lock custody is consistent. |
| `orphan_partitions()` | table | Storage partitions with no graph. Normally empty. |
| `ownership_drift()` | table | Storage objects (tables, partitions, sequences, the `pgrdf` schema) not owned by the owner of `_pgrdf_quads`: `relname`, `kind`, `owner`, `storage_owner`, `blocking`, `cure`. `blocking` follows a per-object rule since 0.6.41. Normally empty. |

Guide: [05 — Managing graphs](05-graphs.md)

## Loading

| Function | Returns | Purpose |
|---|---|---|
| `parse_turtle(content text, graph_id bigint, base_iri text DEFAULT NULL)` | `bigint` | Load Turtle or N-Triples from a string. Returns triples read. |
| `parse_turtle_verbose(content text, graph_id bigint, base_iri text DEFAULT NULL)` | `jsonb` | Same, with a load report. |
| `parse_trig(content text, default_graph_id bigint DEFAULT 0, strict boolean DEFAULT false)` | `jsonb` | Load TriG. Named graphs in the data are created as needed; with `strict => true` they must already exist. |
| `parse_nquads(content text, default_graph_id bigint DEFAULT 0, strict boolean DEFAULT false)` | `jsonb` | Load N-Quads, same rules. |
| `load_turtle(path text, graph_id bigint, base_iri text DEFAULT NULL, bulk_load boolean DEFAULT false)` | `bigint` | Load a file from the server's filesystem. Returns triples read. All file loaders need the `pg_read_server_files` role. |
| `load_turtle_verbose(path text, graph_id bigint, base_iri text DEFAULT NULL, bulk_load boolean DEFAULT false)` | `jsonb` | Same, with a load report. |
| `load_turtle_streaming(path text, graph_id bigint, window_triples integer DEFAULT 20000000, id_reserve_block integer DEFAULT 1000000, base_iri text DEFAULT NULL)` | `jsonb` | Windowed load for files larger than memory. |
| `load_turtle_staged_run(path text, graph_id bigint, n_workers integer DEFAULT 0)` | `jsonb` | Parallel multi-worker load of a large N-Triples file into an empty database. Owner of the pgRDF tables only. |
| `load_turtle_staged(path text, graph_id bigint, n_workers integer DEFAULT 0)` | procedure | Same, as a `CALL`-able procedure. |

Every loader stores a graph as a set: a triple the graph already holds
is read and not stored again. The functions that return `jsonb` carry a
`load` object (`triples_read`, `parse_skipped`, `triples_stored`,
`inferred_made_asserted`, `duplicates_dropped`, `graph_was_empty`),
since 0.6.41. A term containing U+0000 is refused with `22P05`.

Guide: [02 — Loading RDF](02-loading-rdf.md)

## Querying

| Function | Returns | Purpose |
|---|---|---|
| `sparql(query text)` | `SETOF jsonb` | `SELECT`, `ASK` and SPARQL UPDATE. An update's `triples_inserted` counts the triples stored (since 0.6.41), not the triples asked for. |
| `construct(query text)` | `SETOF jsonb` | `CONSTRUCT`: one row per triple. |
| `describe(query text)` | `SETOF jsonb` | `DESCRIBE`: one row per triple. |
| `sparql_parse(query text)` | `jsonb` | Parse without running; lists unsupported parts. |
| `sparql_sql(query text)` | `text` | The SQL a query would run. |
| `last_call_stats()` | `jsonb` | Completeness figures for this session's last query, and since 0.6.41 the load report of its last load (`load`). |
| `get_term(id bigint)` | `text` | Text of a term id. |

Guide: [03 — Querying](03-querying.md)

## Reasoning and validation

| Function | Returns | Purpose |
|---|---|---|
| `materialize(graph_id bigint, profile text DEFAULT 'owl-rl')` | `jsonb` | Compute and store inferred triples. Profiles: `'owl-rl'`, `'rdfs'`. |
| `validate(data_graph_id bigint, shapes_graph_id bigint, mode text DEFAULT 'native', strict boolean DEFAULT true)` | `jsonb` | SHACL validation report. Modes: `'native'`, `'pgrdf'`, `'sparql'`. Since 0.6.41 each call that answers is recorded as a `validate` act, and the report adds `act_recorded`, `act_id` and, when nothing was recorded, `act_reason`. |

| `shacl_capability()` | table | Which SHACL Core features `validate` enforces, one row per feature: `feature`, `kind`, `spec_section`, `verdict`, `mode`, `measured_on`. |

`materialize` and `validate` read whole graphs into memory;
`pgrdf.max_inmemory_triples` sets a ceiling (`54000` past it).

Guides: [04 — Reasoning](04-reasoning.md) · [06 — Validation](06-validation-recipes.md)

## Identity and export

| Function | Returns | Purpose |
|---|---|---|
| `graph_digest(graph_id bigint)` | `text` | Canonical graph identity (`rdfc-1.0-sha256`). |
| `structural_digest(graph_id bigint)` | `text` | First-degree structural digest (`pgrdf-fd1-sha256`). An unequal result proves the graphs differ. |
| `export_graph(graph_id bigint)` | `SETOF text` | Asserted triples as sorted canonical N-Triples. |
| `graph_manifest(graph_id bigint)` | `jsonb` | Digests, counts, engine version, and what a copy doesn't carry. Since 0.6.41 also `consistent`, and `counts.repeats` for a graph holding repeats. |
| `nul_term_report()` | table | Stored terms containing U+0000 (possible only in data stored before 0.6.41): `term_id`, `term_type`, `preview`, `graph_ids`. Normally empty. Since 0.6.41. |
| `graph_diff_summary(a bigint, b bigint)` | `jsonb` | What changed from `a` to `b`: counts of ground triples and blank-node components, by predicate and by class. |
| `graph_diff(a bigint, b bigint, side text DEFAULT NULL, predicate text DEFAULT NULL, class text DEFAULT NULL)` | table | The differing triples (`side`, `subject`, `predicate`, `object`, `component`), in a fixed order; blank-node components whole. |

Since 0.6.41 the four whole-graph reads (`graph_digest`,
`structural_digest`, `export_graph`, `graph_manifest`) describe the set
of asserted triples, stream instead of holding the graph in memory,
refuse a graph holding a term with U+0000 (`XX001`), and, the export
aside, refuse past `pgrdf.digest_max_bnode_quads` blank-node triples
(`54000`).

Guide: [07 — Identity and export](07-identity-and-export.md)

## Server and diagnostics

| Function | Returns | Purpose |
|---|---|---|
| `version()` | `text` | Extension version. |
| `build_id()` | `text` | Which build of that version is loaded. |
| `surface()` | table | Every function with its stability class and a usage note. |
| `stats()` | `jsonb` | Cumulative server-wide counters (caches, truncations). Not PUBLIC since 0.6.41: superusers and the installing role, or `GRANT EXECUTE`. |
| `shmem_reset()` | `void` | Reset the shared term cache. Only needed after dropping and re-creating the extension on a running server. Not PUBLIC since 0.6.41. |

Guide: [08 — Errors and diagnostics](08-errors-and-diagnostics.md)

## Settings

Set with `SET`, `SET LOCAL`, `ALTER DATABASE … SET` or `ALTER ROLE …
SET`. "Anyone" means any role may change it for its own session;
"superuser" means only a superuser may. The `pgrdf.` prefix is
reserved: a name not in this table is refused.

Added in 0.6.41:

| Setting | Default | Set by | What it does |
|---|---|---|---|
| `pgrdf.workspace` | (empty) | anyone | Declared identity: the workspace the client works in. Copied into the act record and `graph_activity()`. |
| `pgrdf.agent` | (empty) | anyone | Declared identity: the agent or seat acting. |
| `pgrdf.thread` | (empty) | anyone | Declared identity: the conversation or session thread. |
| `pgrdf.on_behalf_of` | (empty) | anyone | Declared identity: the person the client acts for. |
| `pgrdf.harness` | (empty) | anyone | Declared identity: the client software, as `<name>/<version>`. |
| `pgrdf.graph_owner_enforce` | `off` | superuser | When on, `drop_graph`, `clear_graph`, `lock_graph`, `unlock_graph`, `dedup_graph` and `set_graph_owner` are reserved for the graph's owner and the storage owner (`42501` otherwise). |
| `pgrdf.notify_acts` | `off` | superuser | When on, each recorded act issues `NOTIFY pgrdf_acts '<act_id>:<graph_id>:<act>'`, delivered on commit. |
| `pgrdf.digest_max_bnode_quads` | `5000000` | anyone | Blank-node triples `graph_digest`, `structural_digest` and `graph_manifest` may hold in memory; `54000` past it. |
| `pgrdf.max_inmemory_triples` | `0` (no limit) | anyone | Largest graph `materialize` and `validate` will read into memory; `54000` past it. |

The five declared-identity settings take at most 128 bytes and no
control characters, are never used to authorise anything, and should be
set per transaction by clients that share connections. See
[declared identity](05-graphs.md#declared-identity).

Earlier settings, all set by anyone:

| Setting | Default | What it does |
|---|---|---|
| `pgrdf.path_max_depth` | `64` | Maximum distance of property-path walks. |
| `pgrdf.on_path_truncation` | `warn` | `warn`, `error` or `count` when a walk hits the depth limit. |
| `pgrdf.path_max_pairs` | `1000000` | Memory bound of one property-path walk. |
| `pgrdf.diff_max_rows` | `5000000` | Memory bound of `graph_diff` / `graph_diff_summary`. |
| `pgrdf.auto_analyze` | `on` | Refresh planner statistics after loads and `materialize`. |
| `pgrdf.bulk_defer_index_min` | `100000` | Size above which bulk loads drop and rebuild indexes. |
| `pgrdf.dict_batch_size` | `500` | Terms per dictionary batch in the standard loader. |
| `pgrdf.ingest_dict_path` | `combined` | Dictionary resolution strategy of the standard loader. |
| `pgrdf.shmem_prewarm_on_init` | `off` | Warm the shared term cache before the first load. |
| `pgrdf.staged_resolve_strategy` | `index` | Join strategy of the staged loader. |
| `pgrdf.staged_temp_tablespaces` | (empty) | Tablespace(s) for the staged loader's temporary files. |

Guide: [08 — Errors and diagnostics](08-errors-and-diagnostics.md#settings)

## Not listed here

`surface()` also shows functions in the `internal` class (low-level
building blocks) and the `spike` class (benchmarks). They are not part
of the supported API and may change without notice.
