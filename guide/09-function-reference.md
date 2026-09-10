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
| `graph_id(iri text)` | `bigint` | Id for an IRI, or `NULL` if there is no such graph. |
| `graph_iri(id bigint)` | `text` | IRI for an id, or `NULL`. |
| `graph_inventory()` | table | Every graph: `graph_id`, `iri`, `asserted`, `inferred`, `locked`, `lock_reason`, `materialization`. |
| `count_quads(g bigint DEFAULT 0)` | `bigint` | Asserted + inferred triples in one graph. |
| `clear_graph(id bigint)` · `clear_graph(iri text)` | `bigint` | Remove all triples; keep the graph. |
| `copy_graph(src bigint, dst bigint)` · `copy_graph(src_iri text, dst_iri text)` | `bigint` | Append `src` into `dst`, inferred triples included. The IRI form needs `dst` to exist. |
| `move_graph(src bigint, dst bigint)` · `move_graph(src_iri text, dst_iri text)` | `bigint` | Move into an empty `dst`, then remove `src`. |
| `drop_graph(id bigint, cascade boolean DEFAULT true)` · `drop_graph(iri text, cascade boolean DEFAULT true)` | `bigint` | Remove a graph. With `cascade => false`, refuses if it holds inferred triples. |
| `carve_graph(src bigint, predicate text, dst bigint)` | `bigint` | Copy every triple using one predicate. |
| `carve_graph(src bigint, seeds text[], dst bigint, max_hops integer DEFAULT 1)` | `bigint` | Copy the neighbourhood of seed IRIs, up to `max_hops` steps. |
| `lock_graph(graph_id bigint, reason text)` | `boolean` | Make a graph read-only for every write path. |
| `unlock_graph(graph_id bigint, reason text)` | `boolean` | Release a lock. |
| `graph_integrity(graph_id bigint)` | `jsonb` | Check every triple is well-formed. |
| `orphan_partitions()` | table | Storage partitions with no graph. Normally empty. |

Guide: [05 — Managing graphs](05-graphs.md)

## Loading

| Function | Returns | Purpose |
|---|---|---|
| `parse_turtle(content text, graph_id bigint, base_iri text DEFAULT NULL)` | `bigint` | Load Turtle or N-Triples from a string. |
| `parse_turtle_verbose(content text, graph_id bigint, base_iri text DEFAULT NULL)` | `jsonb` | Same, with a load report. |
| `parse_trig(content text, default_graph_id bigint DEFAULT 0, strict boolean DEFAULT false)` | `jsonb` | Load TriG. Named graphs in the data are created as needed; with `strict => true` they must already exist. |
| `parse_nquads(content text, default_graph_id bigint DEFAULT 0, strict boolean DEFAULT false)` | `jsonb` | Load N-Quads, same rules. |
| `load_turtle(path text, graph_id bigint, base_iri text DEFAULT NULL, bulk_load boolean DEFAULT false)` | `bigint` | Load a file from the server's filesystem. |
| `load_turtle_verbose(path text, graph_id bigint, base_iri text DEFAULT NULL, bulk_load boolean DEFAULT false)` | `jsonb` | Same, with a load report. |
| `load_turtle_streaming(path text, graph_id bigint, window_triples integer DEFAULT 20000000, id_reserve_block integer DEFAULT 1000000, base_iri text DEFAULT NULL)` | `jsonb` | Windowed load for files larger than memory. |
| `load_turtle_staged_run(path text, graph_id bigint, n_workers integer DEFAULT 0)` | `jsonb` | Parallel multi-worker load of a large N-Triples file into an empty database. |
| `load_turtle_staged(path text, graph_id bigint, n_workers integer DEFAULT 0)` | procedure | Same, as a `CALL`-able procedure. |

Guide: [02 — Loading RDF](02-loading-rdf.md)

## Querying

| Function | Returns | Purpose |
|---|---|---|
| `sparql(query text)` | `SETOF jsonb` | `SELECT`, `ASK` and SPARQL UPDATE. |
| `construct(query text)` | `SETOF jsonb` | `CONSTRUCT`: one row per triple. |
| `describe(query text)` | `SETOF jsonb` | `DESCRIBE`: one row per triple. |
| `sparql_parse(query text)` | `jsonb` | Parse without running; lists unsupported parts. |
| `sparql_sql(query text)` | `text` | The SQL a query would run. |
| `last_call_stats()` | `jsonb` | Completeness figures for this session's last query. |
| `get_term(id bigint)` | `text` | Text of a term id. |

Guide: [03 — Querying](03-querying.md)

## Reasoning and validation

| Function | Returns | Purpose |
|---|---|---|
| `materialize(graph_id bigint, profile text DEFAULT 'owl-rl')` | `jsonb` | Compute and store inferred triples. Profiles: `'owl-rl'`, `'rdfs'`. |
| `validate(data_graph_id bigint, shapes_graph_id bigint, mode text DEFAULT 'native', strict boolean DEFAULT true)` | `jsonb` | SHACL validation report. Modes: `'native'`, `'pgrdf'`, `'sparql'`. |

Guides: [04 — Reasoning](04-reasoning.md) · [06 — Validation](06-validation-recipes.md)

## Identity and export

| Function | Returns | Purpose |
|---|---|---|
| `graph_digest(graph_id bigint)` | `text` | Canonical graph identity (`rdfc-1.0-sha256`). |
| `structural_digest(graph_id bigint)` | `text` | First-degree structural digest (`pgrdf-fd1-sha256`). An unequal result proves the graphs differ. |
| `export_graph(graph_id bigint)` | `SETOF text` | Asserted triples as sorted canonical N-Triples. |
| `graph_manifest(graph_id bigint)` | `jsonb` | Digests, counts, engine version, and what a copy doesn't carry. |

Guide: [07 — Identity and export](07-identity-and-export.md)

## Server and diagnostics

| Function | Returns | Purpose |
|---|---|---|
| `version()` | `text` | Extension version. |
| `build_id()` | `text` | Which build of that version is loaded. |
| `surface()` | table | Every function with its stability class and a usage note. |
| `stats()` | `jsonb` | Cumulative server-wide counters (caches, truncations). |
| `shmem_reset()` | `void` | Reset the shared term cache. Only needed after dropping and re-creating the extension on a running server. |

Guide: [08 — Errors and diagnostics](08-errors-and-diagnostics.md)

## Not listed here

`surface()` also shows functions in the `internal` class (low-level
building blocks) and the `spike` class (benchmarks). They are not part
of the supported API and may change without notice.
