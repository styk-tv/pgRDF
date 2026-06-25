# SPEC.pgRDF.LLD.v0.6.14 — Low-Level Design

| | |
|---|---|
| **Version** | 0.6.14 (`pgrdf.control` `default_version = '0.6.14'`) |
| **Status** | Current — describes the **shipped** release (`origin/main`) |
| **Supersedes** | `SPEC.pgRDF.LLD.v0.5.md` (and the v0.2–v0.4 series) as the authoritative low-level design |
| **Method** | Reconstructed from the shipped source on `origin/main`; where a stale doc-comment contradicts the code, the **code is authoritative** and the discrepancy is flagged inline |

> **Reading note.** This document is the low-level design of pgRDF as it
> actually ships in v0.6.14. It was rebuilt directly from the source
> tree, not edited forward from the older LLD series — several of which
> had drifted from the code. Forward-looking material lives in
> `SPEC.pgRDF.LLD.v0.6-FUTURE.md` and the roadmap; this file is the
> *as-built* contract.

## 1. Overview & architecture

pgRDF is a PostgreSQL extension (Rust, on `pgrx` 0.16, PostgreSQL 14–17)
that turns a stock Postgres instance into a semantic-web engine: load
RDF, query it with SPARQL 1.1, materialize OWL 2 RL / RDFS entailments,
and validate against SHACL — all in-process, addressable from any
Postgres client, with no sidecar store.

### 1.1 The substrate

Everything is built on one idea: **dictionary-encoded quads**. Every RDF
term (IRI, blank node, literal) is interned once into an integer-keyed
dictionary (`_pgrdf_dictionary`), and triples are stored as four
`BIGINT` columns (`subject_id, predicate_id, object_id, graph_id`) in a
single LIST-partitioned table (`_pgrdf_quads`, one partition per
`graph_id`) covered by three hexastore index permutations
(SPO / POS / OSP). An `is_inferred` flag distinguishes asserted base
triples from materialized entailments. A cross-backend shared-memory
dictionary cache accelerates interning; an IRI↔`graph_id` map
(`_pgrdf_graphs`) names every graph.

### 1.2 The four engines

| Engine | UDF entry points | Backed by |
|---|---|---|
| **Storage** | `load_turtle`, `load_turtle_staged_run`, `parse_turtle`/`_trig`/`_nquads`, `add_graph`, `count_quads` | `src/storage/*` — dictionary, hexastore, partitions, loaders, the staged bgworker pool |
| **Query** | `sparql`, `construct`, `describe`, `sparql_parse` | `src/query/*` — `spargebra` parse → parameterised SQL over the hexastore, per-backend plan cache |
| **Inference** | `materialize` | `src/inference/reasonable.rs` — the `reasonable` OWL 2 RL reasoner + a native RDFS fixpoint |
| **Validation** | `validate` | `src/validation/{shacl,pgrdf_sparql}.rs` — rudof SHACL Core + a pgRDF-native SHACL-SPARQL handler |

### 1.3 Module map (shipped source)

```
src/lib.rs                      extension entry, _PG_init, shmem/GUC wiring, version()
src/storage/
  dict.rs                       dictionary interning (single + batch)
  hexastore.rs                  _pgrdf_quads + SPO/POS/OSP, put_quad, count_quads
  partition.rs                  LIST partition DDL + the advisory-lock gate
  graphs.rs                     graph lifecycle (add/drop/clear/copy/move), IRI↔id
  loader.rs                     load_turtle, format-aware dispatch, parse_*, streaming, bulk
  loader_ta11.rs                insert-path measurement spikes
  construct_ingest.rs           CONSTRUCT-result ingest
  shmem_cache.rs                cross-backend shared-memory dictionary cache
  stats.rs                      pgrdf.stats(), shmem_reset, prewarm
  staged/jobctl.rs              shmem job-control segment (coordinator ↔ workers)
  staged/pool.rs                bgworker pool, the coordinator, the worker entry
  staged/phases.rs              STAGE → DICT → RESOLVE → INDEX phase bodies
src/query/
  parser.rs                     spargebra parse, sparql_parse preview
  executor.rs                   algebra → SQL, all operators, UPDATE, CONSTRUCT, DESCRIBE
  path.rs                       property paths (recursive CTE, fast path, depth guard)
  plan_cache.rs                 per-backend prepared-plan cache
  guc.rs                        all pgrdf.* GUCs
src/inference/reasonable.rs     materialize, owl-rl + rdfs
src/validation/
  shacl.rs                      validate, the three-mode dispatch, rudof path
  pgrdf_sparql.rs               the pgRDF-native SHACL-SPARQL handler
```

### 1.4 Deployment shape

- `CREATE EXTENSION pgrdf;` installs the `pgrdf` schema, the tables, the
  hexastore indexes, and the UDF surface. The control file declares
  `relocatable = false`, `superuser = true`, and **no external
  extension dependency**.
- **`shared_preload_libraries = 'pgrdf'`** is required for the
  shared-memory facilities — the dictionary cache, the plan-cache
  counters, and the staged-loader job-control segment — because their
  shmem hooks can only register in the postmaster. The custom GUCs
  register on both the preload and lazy-load paths. Without preload,
  pgRDF still works: the caches degrade to safe no-ops and the staged
  loader is unavailable (the in-process loaders remain).
- The single publishable artifact is the SLSA-attested OCI bundle
  (`ghcr.io/styk-tv/pgrdf-bundle:0.6.14`).

### 1.5 Cross-cutting design principles

1. **Dictionary-encoded, full-identity keyed.** A term is interned once,
   keyed on `(term_type, lexical_value, datatype_id, language)` — never
   lexical value alone — so typed and language-tagged literals never
   collapse.
2. **Parameterised SQL only.** The query translator resolves every
   constant to its dict id at translate time and emits `$N`
   placeholders; generated SQL never carries user strings. This is both
   the injection defence and the plan-cache key.
3. **No silent fallbacks.** Unknown `mode`/`profile` arguments panic with
   a stable prefix before any side effect; a missing constant resolves
   to a `-1` sentinel (zero rows), never an error.
4. **Commit-per-phase at scale.** The staged loader commits each phase in
   its own background-worker transaction, so a failure leaves a resume
   point rather than rolling back a billion-row load.
5. **Shared memory is an accelerator, never a correctness dependency.**
   Every shmem path is a safe no-op when the extension is not preloaded.

The sections that follow document each subsystem as built.


## 2. Storage layer

pgRDF stores RDF as **dictionary-encoded quads**: every term (IRI, blank node, literal) is interned once into an integer-keyed dictionary, and triples/quads are stored as four `BIGINT` columns referencing those ids. The physical layout is a single LIST-partitioned `_pgrdf_quads` table (one partition per `graph_id`) covered by three hexastore index permutations, plus an IRI↔`graph_id` mapping table and a cross-backend shared-memory dictionary cache. Ground truth: `pgrdf.control` `default_version = '0.6.14'`, `schema = 'pgrdf'`, `relocatable = false`, `superuser = true`, no `requires =` line (no external extension deps). Module tree in `src/storage/mod.rs:30-40` (`construct_ingest`, `dict`, `graphs`, `hexastore`, `loader`, `loader_ta11`, `partition`, `shmem_cache`, `staged`, `stats`).

### 2.1 Dictionary encoding

**Purpose.** Map each distinct RDF term to a stable, dense `BIGINT` id (`term → id`) and back (`id → term`), deduplicating at write time so a term is interned exactly once across the whole database. This is the dominant cost of ingest (profiling cited in `src/storage/dict.rs:99-105` put dict resolution at ~73% of total LUBM-1 ingest time), which motivates both the shmem cache (§2.5) and the batch path below.

**Data structures & types.**

The dictionary table (`sql/schema_v0_2_0.sql:4-22`):

```sql
CREATE TABLE IF NOT EXISTS _pgrdf_dictionary (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    term_type       SMALLINT NOT NULL,   -- 1: URI, 2: BlankNode, 3: Literal
    lexical_value   TEXT     NOT NULL,
    datatype_iri_id BIGINT,               -- nullable; FK-by-convention into id of the datatype IRI term
    language_tag    TEXT,                 -- nullable
    lexical_md5     BYTEA GENERATED ALWAYS AS (decode(md5(lexical_value), 'hex')) STORED,
    CONSTRAINT unique_term UNIQUE (term_type, lexical_md5, datatype_iri_id, language_tag)
);
CREATE INDEX IF NOT EXISTS _pgrdf_dict_val_idx
    ON _pgrdf_dictionary USING HASH (lexical_value);
```

Term-type discriminator — Rust constants in `src/storage/dict.rs:15-19`, matching the `term_type SMALLINT` column:

| Constant | Value (`i16`) | Term |
|---|---|---|
| `term_type::URI` | 1 | IRI |
| `term_type::BLANK_NODE` | 2 | Blank node |
| `term_type::LITERAL` | 3 | Literal |

The full dictionary key is the 4-tuple `(term_type, lexical_value, datatype_id, language)`, modeled in Rust as `(i16, String, Option<i64>, Option<String>)` (the batch tuple, `src/storage/dict.rs:132`). `datatype_iri_id` is itself a dictionary id — the datatype IRI is interned as its own URI term, and the literal row references it. Both `datatype_iri_id` and `language_tag` are nullable, and NULL participates in dedup via `IS NOT DISTINCT FROM` (see invariants).

> **CODE-vs-COMMENT discrepancy (load-bearing).** The R1 fix (v0.6.9) re-keyed `unique_term` from `lexical_value` onto a generated **`lexical_md5 BYTEA`** column (`decode(md5(lexical_value),'hex')`, 16 bytes) so the unique btree key is fixed-size — a raw-`lexical_value` key exceeds PostgreSQL's 2704-byte btree limit on long Wikidata literals and aborted the 8.2 B-triple load at final index rebuild (`sql/schema_v0_2_0.sql:10-17`). **However**, the Rust dict code's doc comments and several of its SQL statements were not updated to reflect this. Specifically:
> - `put_term_full`'s pre-INSERT SELECT (`src/storage/dict.rs:39-54`) still filters on `lexical_value = $2`, served by the HASH index `_pgrdf_dict_val_idx`, not by `unique_term`. This is correct behaviorally (the HASH index covers exact-string lookup) but the doc comment at `dict.rs:21-25` describing the key as "`UNIQUE` … on the lookup" is stale.
> - `put_terms_batch`'s anti-join (`dict.rs:189-195`) and join-back (`dict.rs:218-222`) match on `d.lexical_value = t.lv`, not on `lexical_md5`. Correct (md5 is derived from `lexical_value`, so matching the source value is equivalent and avoids hashing in SQL), but the long `ON CONFLICT` vs `WHERE NOT EXISTS` discussion at `dict.rs:144-182` reasons entirely about `lexical_value` and never mentions that the actual constraint is now keyed on `lexical_md5`. **The migration `sql/pgrdf--0.5.1--0.6.14.sql:58-64` is the authority for the shipped constraint shape.** Net: the code works against the v0.6.14 schema, but its comments describe the pre-v0.6.9 key.

**Design & control flow.** Single-term intern, `put_term_full(value, term_type, datatype_id, language) -> i64` (`src/storage/dict.rs:29-82`), is a three-tier lookup:

1. **Shmem cache probe** (`dict.rs:36-38`) — `shmem_cache::lookup`; on hit, return immediately, touching neither index nor table.
2. **SELECT** (`dict.rs:39-63`) — scalar-subquery `SELECT id … WHERE term_type=$1 AND lexical_value=$2 AND datatype_iri_id IS NOT DISTINCT FROM $3 AND language_tag IS NOT DISTINCT FROM $4 LIMIT 1`. The scalar-subquery wrapper yields exactly one row (NULL on miss) rather than tripping SPI's "positioned before start" empty-result error. On hit, `stage_for_commit` warms the shmem cache (commit-deferred — see below) and returns.
3. **INSERT** (`dict.rs:64-81`) — `INSERT … (term_type, lexical_value, datatype_iri_id, language_tag) VALUES … RETURNING id` (the generated `id` and `lexical_md5` are computed by PostgreSQL; the insert lists only the four explicit columns). The new id is staged via `stage_for_commit`.

Why staging rather than immediate publish (`dict.rs:55-62`, `77-80`): a row found-or-inserted inside a still-open write transaction may yet roll back, so the (key→id) mapping is staged in a per-backend pending list and published to shmem only on `XACT_EVENT_COMMIT` (§2.5).

Batch intern, `put_terms_batch(&[(i16, String, Option<i64>, Option<String>)]) -> Vec<i64>` (`dict.rs:132-248`), resolves N terms in **two SPI calls** instead of N round-trips, returning ids in input order. Step 1 bulk-inserts only missing rows via `INSERT … SELECT FROM unnest($1::int2[], $2::text[], $3::int8[], $4::text[]) WHERE NOT EXISTS (… IS NOT DISTINCT FROM …)` — deliberately **not** `ON CONFLICT DO NOTHING`, because PostgreSQL's default `NULLS DISTINCT` unique semantics would let duplicate NULL-datatype/NULL-language rows slip in and make the per-position id non-deterministic (`dict.rs:144-182`; a documented TA-D3 v0.5.27 regression). Step 2 bulk-looks-up all ids with `unnest … WITH ORDINALITY` and re-sorts by the 1-based ordinal back to input order (`dict.rs:205-247`). This batch path bypasses the shmem cache on both read and write sides by design (`dict.rs:121-126`). The remaining concurrent-NULL race (two parallel ingests both passing `NOT EXISTS`) is a documented v0.7+ deferral — the `UNIQUE NULLS NOT DISTINCT` migration was attempted in v0.5.39 but caused pgrx-parallel deadlocks (`dict.rs:170-182`).

Reverse lookup, `get_term(id) -> Option<String>` (`dict.rs:256-262`), is a scalar-subquery `SELECT lexical_value … WHERE id=$1`, NULL on miss.

**UDF / API surface.**

| SQL signature | Rust fn | Notes |
|---|---|---|
| `pgrdf.put_term(value TEXT, term_type SMALLINT) → BIGINT` | `put_term` (`dict.rs:89-93`) | thin wrapper → `put_term_full(value, term_type, None, None)` |
| `pgrdf.get_term(id BIGINT) → TEXT` | `get_term` (`dict.rs:254-262`) | NULL on miss |

`put_term_full` (`dict.rs:29`) and `put_terms_batch` (`dict.rs:132`) are `pub(crate)`, not `#[pg_extern]` — invoked by the Turtle loader; the only SQL surface for typed/language literals is through the loader, not a direct UDF.

**Key invariants.**
- A term is interned exactly once: `(term_type, lexical_md5, datatype_iri_id, language_tag)` is `UNIQUE` (`unique_term`). `put_term` is idempotent (test `put_term_dedups`, `dict.rs:269-284`; distinct values → distinct ids, `put_term_separates`).
- NULL `datatype_iri_id`/`language_tag` participate in dedup (`IS NOT DISTINCT FROM`, `dict.rs:23-25`) — without it, untyped literals would leak duplicates.
- `id` is `GENERATED ALWAYS AS IDENTITY` — ids are dense, never reused, never user-supplied.
- `lexical_md5` is a `STORED` generated column; loader insert paths list explicit columns, so they are unaffected by its presence (`sql/schema_v0_2_0.sql:14-15`).

### 2.2 Hexastore & quad layout

**Purpose.** Store dictionary-encoded quads and serve any triple-pattern access shape (any subset of S/P/O bound) from a covering index without a heap fetch.

**Data structures & types.** Quad table and the three covering indexes (`sql/schema_v0_2_0.sql:24-41`):

```sql
CREATE TABLE IF NOT EXISTS _pgrdf_quads (
    subject_id   BIGINT  NOT NULL,
    predicate_id BIGINT  NOT NULL,
    object_id    BIGINT  NOT NULL,
    graph_id     BIGINT  NOT NULL DEFAULT 0,
    is_inferred  BOOLEAN NOT NULL DEFAULT FALSE
) PARTITION BY LIST (graph_id);

CREATE TABLE IF NOT EXISTS _pgrdf_quads_default PARTITION OF _pgrdf_quads DEFAULT;

CREATE INDEX _pgrdf_idx_spo ON _pgrdf_quads (subject_id, predicate_id, object_id) INCLUDE (is_inferred);
CREATE INDEX _pgrdf_idx_pos ON _pgrdf_quads (predicate_id, object_id, subject_id) INCLUDE (is_inferred);
CREATE INDEX _pgrdf_idx_osp ON _pgrdf_quads (object_id, subject_id, predicate_id) INCLUDE (is_inferred);
```

Each quad is five columns: three dictionary-id references (`subject_id`, `predicate_id`, `object_id`), the partition key `graph_id` (default 0), and `is_inferred` (FALSE = asserted base triple, TRUE = entailment/materialized). There is no separate quad-id or uniqueness constraint on `_pgrdf_quads` in the shipped DDL — duplicate quads are possible at the storage layer (dedup is the loader's/caller's concern).

**Design & control flow.** Three of the six hexastore permutations are materialized as composite btree indexes — **SPO**, **POS**, **OSP** — each with `INCLUDE (is_inferred)` so the planner can satisfy the common access patterns by **Index-Only Scan** (no heap visit). The chosen three cover all eight bound/unbound triple-pattern combinations: SPO serves S-bound and SP-bound; POS serves P-bound and PO-bound; OSP serves O-bound and OS-bound; any prefix of a permutation is an index range scan. The fully-unbound pattern is a scan; the fully-bound pattern hits any of the three. The other three permutations (SOP, PSO, OPS) are not materialized — they would be redundant for prefix coverage of the bound subsets.

`is_inferred` rides in the `INCLUDE` payload rather than the key so queries can filter asserted-only vs. asserted+inferred without a heap fetch and without bloating key comparison.

**UDF / API surface** (`src/storage/hexastore.rs`):

| SQL signature | Rust fn | Notes |
|---|---|---|
| `pgrdf.put_quad(s BIGINT, p BIGINT, o BIGINT, g BIGINT DEFAULT 0)` → void | `put_quad` (`hexastore.rs:16-25`) | single-row `INSERT`; routes to partition `g` (or default) |
| `pgrdf.count_quads(g BIGINT DEFAULT 0) → BIGINT` | `count_quads` (`hexastore.rs:31-42`) | `count(*) WHERE graph_id=$1` |

`put_quad` is the Phase-2.0 SPI single-insert path; bulk ingest goes through the loader (not in scope here).

**Key invariants.**
- Every quad column is `NOT NULL`; `subject_id/predicate_id/object_id` are dictionary ids (referential integrity is by convention, not an enforced FK).
- A quad routes to partition `_pgrdf_quads_g<graph_id>` if it exists, else `_pgrdf_quads_default` (test `put_quad_then_count`, `hexastore.rs:352-381`).
- The three covering indexes are defined on the parent and inherited by every partition.

### 2.3 LIST partitioning

**Purpose.** Physically segregate each named graph into its own partition of `_pgrdf_quads` keyed by `graph_id`, so per-graph operations (`drop_graph`, `clear_graph`) are metadata/partition-DDL-bounded rather than full-table scans, and so the default catch-all bucket absorbs unrouted ids.

**Data structures & types.** `_pgrdf_quads` is `PARTITION BY LIST (graph_id)` (`sql/schema_v0_2_0.sql:30`). Partition naming convention: `_pgrdf_quads_g<graph_id>` for an explicit graph, `_pgrdf_quads_default` for the `DEFAULT` partition (`schema_v0_2_0.sql:32-33`). The serialisation key is a fixed `i64` advisory-lock constant:

```rust
const PARTITION_DDL_LOCK_KEY: i64 = 0x_7067_7264; // "pgrd" ASCII, = 1_886_613_604
```
(`src/storage/partition.rs:44`).

**Design & control flow.** `CREATE TABLE … PARTITION OF _pgrdf_quads` takes an `AccessExclusiveLock` on the parent. Under pgrx's parallel test harness (one backend per worker thread against a shared `$PGDATA`) two sessions each already holding row/relation locks on the parent and both escalating to that parent-level lock deadlock (`partition.rs:1-27`). The fix is a **transaction-scoped advisory lock** (`pg_advisory_xact_lock(PARTITION_DDL_LOCK_KEY)`) gating *all* partition DDL: concurrent creators *queue* on the advisory key instead of *deadlocking* on the catalog lock, and the lock auto-releases at transaction end (pgrx `#[pg_test]` auto-rollback boundary), so no leaks across cases. This replaced the prior `RUST_TEST_THREADS=1` serialisation.

`create_partition_impl(part_name, graph_id)` (`partition.rs:122-150`) is a four-step routine:
1. **Lock-free fast path** — `partition_exists` `pg_class` check (`partition.rs:124-126`); the common idempotent re-call short-circuits without taking the advisory lock.
2. **Slow path** — `acquire_partition_ddl_gate()` (the advisory `pg_advisory_xact_lock`, `partition.rs:87-93`).
3. **Re-check under the lock** (`partition.rs:137-139`) — a concurrent creator may have won the race; the loser observes the partition and skips.
4. `CREATE TABLE IF NOT EXISTS pgrdf.<part_name> PARTITION OF pgrdf._pgrdf_quads FOR VALUES IN (<graph_id>)` (`partition.rs:144-149`) — `IF NOT EXISTS` is belt-and-suspenders.

`part_name` is always built by callers from a validated non-negative `BIGINT`, so there is no user input in the SQL identifier position (`partition.rs:97-101`).

**Global lock-order discipline** (`partition.rs:61-86`): the `add_graph` family has two serialisation points — the parent's `AccessExclusiveLock` and `_pgrdf_graphs`' table/row lock. To avoid inverting lock order, every lifecycle UDF takes the advisory gate **first** (outermost), enforcing a single global order `advisory → _pgrdf_graphs → partition-catalog`. `pg_advisory_xact_lock` is re-entrant within a transaction, so taking the gate up-front and again inside `create_partition_impl` just bumps the hold count.

**API surface** (all `pub(crate)`, no direct `#[pg_extern]`):
- `acquire_partition_ddl_gate()` (`partition.rs:87-93`)
- `create_quads_partition(graph_id: i64)` (`partition.rs:157-160`) — the single production entry point; builds `_pgrdf_quads_g<id>`.
- `create_quads_partition_named(part_name: &str, graph_id: i64)` (`partition.rs:173-176`) — test-only (`#[cfg(any(test, feature = "pg_test"))]`), routes hand-rolled fixture-named partitions through the same gate.

**Key invariants.**
- All partition DDL — production and test — must take `PARTITION_DDL_LOCK_KEY`, or the serialisation guarantee is void (`partition.rs:31-38`).
- The advisory gate is always the outermost lock in any `add_graph`/`drop_graph`/`copy_graph`/`move_graph` path.
- Partition creation is idempotent (test `add_graph_creates_partition_idempotently`, `hexastore.rs:383-401`: exactly one `_pgrdf_quads_g9001` after two `add_graph` calls).
- Unrouted `graph_id`s land in `_pgrdf_quads_default`.

### 2.4 Graph lifecycle

**Purpose.** Manage named graphs: bind a user IRI to an integer `graph_id`, create/destroy/clear/copy/move the backing partition, and resolve IRI↔id both directions. Every graph the user creates gets a queryable IRI (synthetic if none supplied).

**Data structures & types.** The mapping table (`sql/schema_v0_4_0_graphs.sql:20-23`):

```sql
CREATE TABLE IF NOT EXISTS _pgrdf_graphs (
    graph_id BIGINT PRIMARY KEY,
    iri      TEXT   NOT NULL UNIQUE
);
INSERT INTO _pgrdf_graphs (graph_id, iri) VALUES (0, 'urn:pgrdf:graph:0') ON CONFLICT (graph_id) DO NOTHING;
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graphs', '');
```

`graph_id` is `PRIMARY KEY`, `iri` is `NOT NULL UNIQUE` — the mapping is bijective. The seed row `(0, 'urn:pgrdf:graph:0')` names the default partition's catch-all bucket (`schema_v0_4_0_graphs.sql:25-27`). `pg_extension_config_dump('_pgrdf_graphs', '')` (`schema_v0_4_0_graphs.sql:36`) registers it as user data so `pg_dump` includes its rows verbatim (the empty filter = all rows) rather than treating it as extension DDL and wiping user-bound IRIs on restore. Synthetic IRI shape: `urn:pgrdf:graph:{id}`.

**Design & control flow.** All four lifecycle mutators take the partition-DDL gate first. Concurrent allocate-and-insert is serialised by `LOCK TABLE pgrdf._pgrdf_graphs IN SHARE ROW EXCLUSIVE MODE` (blocks writers incl. itself, not readers; releases at txn end) before any `MAX(graph_id)+1` allocation (`hexastore.rs:164-169`).

- **`add_graph(g BIGINT) → BOOLEAN`** (`hexastore.rs:59-111`): the integer-keyed creator. Panics on `g < 0`. Takes the gate, existence-checks `_pgrdf_quads_g<g>`, returns FALSE if present, else `create_quads_partition(g)` then `INSERT … VALUES ($1, 'urn:pgrdf:graph:'||$1::text) ON CONFLICT (graph_id) DO NOTHING` to bind the synthetic IRI (`hexastore.rs:103-109`). Returns TRUE on create.
- **`add_graph(iri TEXT) → BIGINT`** (`add_graph_iri`, `hexastore.rs:148-213`): IRI-keyed. Panics on empty/whitespace IRI (no RFC 3987 validation — no `oxiri` dep). Idempotent on the IRI (returns existing id without a second partition). Allocates the smallest unused positive id via `COALESCE(MAX(graph_id),0)+1` (the seed makes this ≥ 1), binds the IRI *before* re-entering `SELECT pgrdf.add_graph($1::bigint)` so the integer overload's synthetic-IRI insert no-ops via `ON CONFLICT`, leaving the user IRI intact.
- **`add_graph(id BIGINT, iri TEXT) → BIGINT`** (`add_graph_id_iri`, `hexastore.rs:252-345`): binds a specific pair. Panics on `id<0` or empty IRI. The match arms (`hexastore.rs:293-344`): exact `(id,iri)` already bound → idempotent return; `id` bound to its **synthetic** placeholder and `iri` unbound → `UPDATE` in place (the `add_graph(42)` then `add_graph(42,'http://…')` upgrade path); `id` bound to a different non-synthetic IRI → panic; `iri` bound to a different `graph_id` → panic; neither bound → INSERT + re-enter integer overload.
- **`drop_graph(id BIGINT, cascade BOOLEAN DEFAULT TRUE) → BIGINT`** (`drop_graph`, `graphs.rs:210-301`): returns the pre-drop triple count. Panics on `id<0`; **`id==0` panics** (`drop_graph: cannot drop default partition`). Absent partition → returns 0 and prunes any stranded `_pgrdf_graphs` row (idempotent). `cascade=>FALSE` with any `is_inferred=TRUE` row → panic `drop_graph: inferred rows present`. Else `ALTER TABLE … DETACH PARTITION` then `DROP TABLE`, then `DELETE FROM _pgrdf_graphs` (so post-drop `graph_iri`/`graph_id` return NULL).
- **`clear_graph(id BIGINT) → BIGINT`** (`graphs.rs:484-530`): `TRUNCATE ONLY pgrdf._pgrdf_quads_g<id>`, returns pre-truncate count, keeps the partition attached and the IRI binding intact. **`id==0` is permitted** (clears, not destroys, the default bucket). Panics on `id<0`. Absent/empty graph → 0 (idempotent; two calls return `(N, 0)`). `ONLY` is deliberate defence-in-depth against future sub-partitioning.
- **`copy_graph(src BIGINT, dst BIGINT) → BIGINT`** (`graphs.rs:588-671`): the only lifecycle UDF that touches every row. `INSERT INTO _pgrdf_quads_g<dst> (subject_id, predicate_id, object_id, graph_id, is_inferred) SELECT subject_id, predicate_id, object_id, <dst>::bigint, is_inferred FROM _pgrdf_quads_g<src>` — the `graph_id` projection rebinds to `dst` so the router lands rows in the dst partition; both `is_inferred` states carry forward. Auto-creates `dst` via `add_graph(dst)` if absent. Panics on negative ids or `src==dst`. Absent src → 0. **Not** idempotent on re-call — re-copy duplicates rows (caller must `clear_graph(dst)` first).
- **`move_graph(src BIGINT, dst BIGINT) → BIGINT`** (`graphs.rs:357-444`): a **compose** of `copy_graph(src,dst)` then `drop_graph(src, cascade=>TRUE)`; returns the copied count. Panics on negative ids, `src==dst`, or a non-empty `dst` (`dst … already has data`). Absent src → 0. (The LLD §5.2 "metadata-only DETACH/rebind" claim is aspirational; v0.4.2 ships the compose — `graphs.rs:307-315`.)
- **Read-only resolvers:** `graph_id(iri TEXT) → BIGINT` (`graphs.rs:138-146`, `#[pg_extern(strict)]`, NULL on miss) and `graph_iri(id BIGINT) → TEXT` (`graphs.rs:160-168`, `strict`, NULL on miss). Both use the scalar-subquery wrapper to stay on SPI's one-row path.

**IRI-keyed lifecycle overloads** (`graphs.rs:673-804`, the "v0.5-FUTURE §7" block, present and shipping): `drop_graph(iri TEXT, cascade BOOLEAN DEFAULT TRUE)`, `clear_graph(iri TEXT)`, `copy_graph(src_iri TEXT, dst_iri TEXT)`, `move_graph(src_iri TEXT, dst_iri TEXT)`. Each resolves IRI→id via shared `resolve_iri_or_panic` (`graphs.rs:701-711`) then dispatches to the BIGINT overload by SQL re-entry (single-sourced logic). **Intentional difference vs. BIGINT overloads:** an unbound IRI is an **error** (`<fn>: unknown iri`), not a no-op — an IRI is an asserted name, so a miss is a programming error (`graphs.rs:688-693`).

**UDF / API surface (exact signatures).**

| SQL signature | Rust fn |
|---|---|
| `pgrdf.add_graph(g BIGINT) → BOOLEAN` | `add_graph` (`hexastore.rs:61`) |
| `pgrdf.add_graph(iri TEXT) → BIGINT` | `add_graph_iri` (`hexastore.rs:150`) |
| `pgrdf.add_graph(id BIGINT, iri TEXT) → BIGINT` | `add_graph_id_iri` (`hexastore.rs:254`) |
| `pgrdf.graph_id(iri TEXT) → BIGINT` | `graph_id` (`graphs.rs:140`) |
| `pgrdf.graph_iri(id BIGINT) → TEXT` | `graph_iri` (`graphs.rs:162`) |
| `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN DEFAULT TRUE) → BIGINT` | `drop_graph` (`graphs.rs:212`) |
| `pgrdf.drop_graph(iri TEXT, cascade BOOLEAN DEFAULT TRUE) → BIGINT` | `drop_graph_iri` (`graphs.rs:725`) |
| `pgrdf.clear_graph(id BIGINT) → BIGINT` | `clear_graph` (`graphs.rs:486`) |
| `pgrdf.clear_graph(iri TEXT) → BIGINT` | `clear_graph_iri` (`graphs.rs:748`) |
| `pgrdf.copy_graph(src BIGINT, dst BIGINT) → BIGINT` | `copy_graph` (`graphs.rs:590`) |
| `pgrdf.copy_graph(src_iri TEXT, dst_iri TEXT) → BIGINT` | `copy_graph_iri` (`graphs.rs:771`) |
| `pgrdf.move_graph(src BIGINT, dst BIGINT) → BIGINT` | `move_graph` (`graphs.rs:359`) |
| `pgrdf.move_graph(src_iri TEXT, dst_iri TEXT) → BIGINT` | `move_graph_iri` (`graphs.rs:795`) |

(pgrx surfaces the same SQL name for multiple Rust fns via `#[pg_extern(name = "…")]`; Postgres dispatches on argument types.)

**Key invariants.**
- `_pgrdf_graphs` is bijective (`graph_id` PK, `iri` UNIQUE); the seed `(0, 'urn:pgrdf:graph:0')` is the sole row at `CREATE EXTENSION` (test `pgrdf_graphs_seed_row`, `graphs.rs:816-835`).
- Every successful `add_graph(id)` binds an IRI (synthetic if none given); `ON CONFLICT (graph_id) DO NOTHING` preserves a pre-existing user IRI (test `add_graph_populates_synthetic_iri`, `graphs.rs:841-868`).
- `graph_id=0` cannot be dropped but can be cleared.
- Lifecycle UDFs are idempotent on absent graphs (return 0, no error) except `copy_graph` re-call, which duplicates.
- IRI-keyed overloads error on unbound IRIs; BIGINT overloads no-op on absent ids.
- All mutators take `advisory → _pgrdf_graphs(SHARE ROW EXCLUSIVE) → partition-catalog` lock order.

### 2.5 Shared-memory dictionary cache

**Purpose.** A process-shared, fixed-capacity hash table in PostgreSQL shmem caching `(term_type, lexical_value, datatype_id, language) → dict_id` across **all backends** and across calls, so a warmed term resolves with two slot probes under an LWLock and never touches `_pgrdf_dictionary`. It sits below the loader's per-call HashMap: per-call catches "seen in this file"; shmem catches "seen in any backend since postmaster start" (`src/storage/shmem_cache.rs:1-15`).

**Data structures & types** (`shmem_cache.rs:35-90`):

```rust
const SLOTS: usize = 16_384;     // 16 384 slots × 32 B = 512 KiB shmem
const PROBE_DEPTH: usize = 8;    // linear-probe streak before eviction

#[repr(C)] struct DictCacheSlot {
    key_hash1: u64, key_hash2: u64,   // 128-bit fingerprint (two seeded SipHashes)
    generation: u64,                  // cache generation; mismatch ⇒ slot is cold
    dict_id: i64,
    occupied: u8, _pad: [u8; 7],
}
```

`DICT_CACHE: PgLwLock<[DictCacheSlot; SLOTS]>` is the table (`shmem_cache.rs:69-70`). Cross-backend atomic counters: `HITS`, `MISSES`, `INSERTS`, `EVICTIONS`, `GENERATION` (starts at 1 so all-zero initial slots read stale), `PATH_DEPTH_TRUNCATIONS` (`shmem_cache.rs:72-90`). Per-backend thread-locals `PENDING: Vec<(u64,u64,i64)>` and `REGISTERED: bool` stage uncommitted entries (`shmem_cache.rs:230-233`). `SHMEM_READY: AtomicBool` gates the whole module (`shmem_cache.rs:157`). Two hash seeds (`SEED_A = 0x9E37…` golden ratio, `SEED_B = 0xC4F1…`, `shmem_cache.rs:171-172`) produce the 128-bit fingerprint via `DefaultHasher` (SipHash) over `(term_type, value, datatype_id, language)`.

**Design & control flow.**
- **Init** (`init_in_postmaster`, `shmem_cache.rs:99-110`): must run inside `_PG_init` only when `process_shared_preload_libraries_in_progress`. `pg_shmem_init!` registers the slot array (built via `default_const()` Copy-init since `[T;N]: Default` only holds for N≤32) and the atomics, sets `GENERATION=1`, then `mark_ready()`. If the `.so` is lazy-loaded outside `shared_preload_libraries`, `SHMEM_READY` stays false and every entry point short-circuits to `None`/no-op, falling back to the per-call HashMap path.
- **Lookup** (`shmem_cache.rs:198-224`): take `DICT_CACHE.share()`, start at `h1 % SLOTS`, linear-probe up to `PROBE_DEPTH`; a slot matches iff `occupied != 0 && generation == current && key_hash1==h1 && key_hash2==h2`. Hit → `HITS++`, return `dict_id`; exhausted probe → `MISSES++`, `None`.
- **Transactional staging** (`stage_for_commit`, `shmem_cache.rs:236-249`): pushes `(h1, h2, dict_id)` to the per-backend `PENDING` and registers commit/abort callbacks once. On `Commit`, `flush_pending` drains `PENDING` into shmem; on `Abort`, `PENDING` is cleared (`shmem_cache.rs:266-293`). This keeps shmem in lockstep with the table — ids whose rows roll back are never published.
- **Direct publish** (`insert_committed`, `shmem_cache.rs:252-264`): for already-committed (SELECT-found) rows, or as the commit-callback drain target.
- **Insert/eviction** (`insert_slot`, `shmem_cache.rs:295-337`): take `DICT_CACHE.exclusive()`, probe `PROBE_DEPTH`; reuse the first slot that is empty *or stale-generation* (stale = fair game); if the same fingerprint is already present (concurrent insert from another backend), refresh `dict_id` and exit; if the probe streak is full, **evict the canonical slot** (`start`) — cold terms displaced first, hot set stays sticky — and bump `EVICTIONS`.
- **Reset** (`reset`, `shmem_cache.rs:123-134`): `GENERATION.fetch_add(1)` invalidates every slot in one atomic increment (every existing slot's `generation` now mismatches), and directly zeroes `PATH_DEPTH_TRUNCATIONS` (an absolute, not generation-versioned, counter). Use after `DROP EXTENSION pgrdf; CREATE EXTENSION` so the stale id space can't collide.

`PATH_DEPTH_TRUNCATIONS` / `note_path_depth_truncation` (`shmem_cache.rs:89-90, 143-149`) is a Phase-E scaffold (SPARQL property-path depth-guard counter) — initialised, zeroed by reset, surfaced on `stats()`, but `#[allow(dead_code)]` with no incrementing caller yet.

**API surface** (`pub`, called by `dict`/`stats`/`loader`):
`init_in_postmaster()`, `mark_ready()`, `is_ready() -> bool`, `lookup(i16, &str, Option<i64>, Option<&str>) -> Option<i64>`, `stage_for_commit(…, dict_id: i64)`, `insert_committed(…, dict_id: i64)`, `reset()`, `note_path_depth_truncation()`, `snapshot() -> Snapshot`. The `Snapshot` struct (`shmem_cache.rs:340-352`) carries `ready, slots, hits, misses, inserts, evictions, path_depth_truncations`. No direct `#[pg_extern]` here — the SQL surface is in `stats` (§2.6).

**Key invariants.**
- 128-bit fingerprint ⇒ false-hit probability ≈ 2⁻¹²⁸ at fleet scale (`shmem_cache.rs:22-25`).
- `datatype_id` and `language` are part of the key — same lexical value, different datatype → different slot (test `shmem_datatype_in_key`, `shmem_cache.rs:449-455`).
- Freshly-INSERTed mappings are published only on COMMIT (staged); SELECT-found rows publish directly (`shmem_cache.rs:16-19`).
- Slot generation must equal current `GENERATION` to be live; `reset` invalidates all in O(1) (test `shmem_reset_invalidates_slots`, `stats.rs:200-215`).
- When `!is_ready()` every entry point is a safe no-op/`None` — the cache is a pure accelerator, never a correctness dependency.

### 2.6 Statistics

**Purpose.** A cross-backend observability surface (`pgrdf.stats()`) exposing cumulative shmem-cache and plan-cache counters, plus the cache-reset and prewarm controls.

**Data structures & types.** `stats()` composes `shmem_cache::snapshot()` (§2.5) and `plan_cache::snapshot()` (query layer, out of scope) into one `JsonB` (`src/storage/stats.rs:49-69`). Counters are cumulative since postmaster start; tests compare deltas, not absolutes (`stats.rs:7-9`).

**Design & control flow.** `stats()` (`stats.rs:47-69`) returns a JSON object with exactly these keys:

```json
{ "shmem_ready": bool, "shmem_slots": 16384, "shmem_hits", "shmem_misses",
  "shmem_inserts", "shmem_evictions", "plan_cache_hits", "plan_cache_misses",
  "plan_cache_inserts", "plan_cache_local_size", "path_depth_truncations": 0 }
```

`shmem_ready: false` means the `.so` was not loaded via `shared_preload_libraries` — all shmem counters read 0 and `put_term_full` runs without the cross-backend cache (`stats.rs:38-44`). `plan_cache_local_size` is *this* backend's local cache size; the other `plan_cache_*` are cumulative in shmem. `path_depth_truncations` is always 0 in v0.6.14 (the recursive-CTE that would truncate is a later phase; the field ships for shape stability).

`shmem_reset()` (`stats.rs:81-85`) → `shmem_cache::reset()` (atomic generation bump; cheap, idempotent). Call after `DROP/CREATE EXTENSION`; production workloads that never drop the extension never need it.

`shmem_cache_prewarm(limit BIGINT DEFAULT 100000)` (`stats.rs:121-125`) and its `pub(crate)` body `shmem_cache_prewarm_impl` (`stats.rs:157-182`) walk `_pgrdf_dictionary ORDER BY id LIMIT $1` (oldest-first — most-likely-shared core RDF/RDFS/OWL predicates) and **`stage_for_commit`** each row, returning the count warmed. The critical correctness choice (`stats.rs:132-156`): it uses `stage_for_commit`, **not** `insert_committed`, because the `SELECT` can see uncommitted dict rows via MVCC; publishing them immediately would, on an outer-transaction ABORT, leave shmem holding `(fingerprint → id)` entries pointing at rolled-back rows — a future backend would then resolve a stale id and write quads referencing a non-existent dict row. Staging ties the publish to the same commit/abort callbacks, so prewarm is transactional.

**UDF / API surface (exact signatures).**

| SQL signature | Rust fn |
|---|---|
| `pgrdf.stats() → JSONB` | `stats` (`stats.rs:49`) |
| `pgrdf.shmem_reset() → void` | `shmem_reset` (`stats.rs:83`) |
| `pgrdf.shmem_cache_prewarm(limit BIGINT DEFAULT 100000) → BIGINT` | `shmem_cache_prewarm` (`stats.rs:123`) |

**Key invariants.**
- `stats()` always returns a JSON object with the keys above (test `stats_returns_object`, `stats.rs:189-197`); the shape is stable from v0.4.5 onward.
- All counters are cumulative since postmaster start; reset only affects the dict-cache *generation* (and zeroes `path_depth_truncations`), not the cumulative hit/miss/insert/eviction totals.
- `shmem_cache_prewarm` is transactional via `stage_for_commit` — a rolled-back ingest never leaves stale shmem entries.

---



## 3. Ingest & loaders (non-staged)

This section documents the v0.6.14 ingest path that lives in `src/storage/loader.rs` (3659 lines), `src/storage/loader_ta11.rs` (insert-path measurement spikes), and `src/storage/construct_ingest.rs` (CONSTRUCT round-trip ingest). The **staged loader** (`storage::staged::pool::load_turtle_staged_run`) is a separate subsystem; here we document only the **dispatch decision** into it from `load_turtle`.

All paths converge on two shared mechanisms and produce **byte-identical** `_pgrdf_dictionary` + `_pgrdf_quads` rows regardless of which path executes:
- **Term interning** keyed by `DictKey = (i16 term_type, String lexical, Option<i64> datatype_id, Option<String> language)` (`loader.rs:65`). `term_type` constants (from `dict.rs:15`): `URI = 1`, `BLANK_NODE = 2`, `LITERAL = 3`.
- **Quad flush** via one constant prepared statement `QUAD_INSERT_SQL` (`loader.rs:60-63`): `INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id) SELECT s, p, o, $4 FROM unnest($1::bigint[],$2::bigint[],$3::bigint[])`, prepared once per backend and kept in `plan_cache` (`flush_batch`, `loader.rs:242-300`).

The canonical literal rule, enforced identically on every path: a **language-tagged** literal carries `datatype_id = None` (rdf:langString is implicit, never stored); **every other** literal — including plain `xsd:string` — carries an explicit datatype-IRI dict id (`object_to_id` `loader.rs:200-232`; `object_key` `loader.rs:851-881`).

### 3.1 Loader family & when to use each

| UDF | Source | Returns | Path it drives | When to use |
|---|---|---|---|---|
| `load_turtle` | server file | `BIGINT` triples | format-aware dispatch (§3.2): staged / parallel-bulk / `ingest_dispatch` | **Primary front door.** Auto-routes. |
| `load_turtle_verbose` | server file | `JSONB` stats | same as `load_turtle` **except no staged dispatch** | Same, with phase-timing stats. |
| `parse_turtle` | TEXT | `BIGINT` triples | `ingest_dispatch` (combined/baseline GUC route) | In-memory Turtle. |
| `parse_turtle_verbose` | TEXT | `JSONB` | `ingest_dispatch` | In-memory Turtle + stats. |
| `load_turtle_streaming` | server file | `JSONB` | windowed streaming bulk (§3.4) | Billion-scale `.nt` on an **empty** dict, bounded RAM. |
| `parse_trig` | TEXT | `JSONB` (+`graphs[]`) | `ingest_quads_dispatch` (§3.6) | TriG with GRAPH blocks. |
| `parse_nquads` | TEXT | `JSONB` (+`graphs[]`) | `ingest_quads_dispatch` (§3.6) | N-Quads. |
| `parse_turtle_dict_batched` / `load_turtle_dict_batched` | TEXT / file | `JSONB` | `ingest_turtle_dict_batched` 2-pass spike | TA-D3 measurement spike (LUBM-1 scope). |
| `put_construct_row` / `put_construct_rows` | JSONB | `BIGINT` | construct ingest (§3.7) | CONSTRUCT round-trip. |
| `spike_ta11_batch_sweep`, `spike_ta10_*` | — | `JSONB` | insert-path microbenchmarks (§3.5) | Internal benchmarking only; never touch `_pgrdf_quads`. |

**Internal dict-path routing** (`ingest_dispatch`, `loader.rs:1023-1046`): the GUC `pgrdf.ingest_dict_path` selects `baseline` / `batched` / `shmem_warm` / `combined`. Turtle: `Baseline|ShmemWarm → ingest_turtle_with_stats`; `Batched → ingest_turtle_dict_batched`; `Combined → ingest_turtle_combined`. The selected route is recorded in `stats.path` and surfaced as the JSONB `path` field. All four produce identical rows; only the SPI shape differs. `shmem_prewarm_on_init` (or `ShmemWarm`) triggers a once-per-backend prewarm via `maybe_prewarm_once` (`loader.rs:1055-1061`, thread-local latch).

### 3.2 Format-aware dispatch (the sniff classifier, routing, no-data-loss guarantee)

**UDF signature (ground truth):** `pgrdf.load_turtle(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, bulk_load BOOLEAN DEFAULT FALSE) → BIGINT` (`load_turtle`, `loader.rs:2536-2575`).

Control flow of `load_turtle`:
1. `base = base_iri.filter(|s| !s.is_empty())` — empty string is treated as NULL.
2. **If `bulk_load`** → `bulk_load_guarded(path, graph_id, base).triples` and return (§3.3). This explicit opt-in is honoured verbatim and is **independent of** the staged dispatch.
3. **Staged dispatch gate** — route to the native staged loader only when **all three** hold:
   - `crate::storage::staged::jobctl::is_ready()` — pgRDF is in `shared_preload_libraries` so the jobctl shmem segment / worker pool exists, **AND**
   - `base.is_none()` (the N-Triples staged STAGE phase has no relative-IRI base), **AND**
   - `file_sniffs_as_ntriples(path)` returns `Some(true)`.
   On `Some(true)` → `staged_load_default(path, graph_id)` (`loader.rs:2449-2475`), which calls `SELECT pgrdf.load_turtle_staged_run($1,$2,0)` via SPI (`n_workers=0` ⇒ auto); on `ok:false` it raises `load_turtle: staged loader aborted in the {phase} phase: {reason} (staging table left in place as the resume point)`, otherwise returns the `triples` count.
   On `Some(false)` (readable Turtle, preloaded) → emit a `NOTICE` nudging toward N-Triples+preload, then fall through.
   On `None` (missing/unreadable file) → **no notice**, fall through so the standard path's `File::open` surfaces the real `failed to open` error.
4. **Fall-through standard path:** `File::open(path)` then `ingest_dispatch(BufReader::new(file), graph_id, base).triples`.

**The sniff classifier** (`loader.rs:2199-2433`):
- `SNIFF_SAMPLE_BYTES = 64 * 1024`, `SNIFF_MAX_LINES = 200` (`loader.rs:2202,2207`).
- `file_sniffs_as_ntriples(path) -> Option<bool>` (`loader.rs:2418-2433`): opens the file (`None` on any I/O error), fills up to 64 KiB across short reads, calls `sniff_is_ntriples(&buf[..filled])`. Returns `Some(true)` N-Triples, `Some(false)` Turtle, `None` I/O error.
- `sniff_is_ntriples(sample: &[u8]) -> bool` (`loader.rs:2379-2409`): pure over `&[u8]` (unit-testable without pgrx). Decodes lossy UTF-8; **drops a trailing partial line** only when the sample didn't end on `\n` and there's more than one line (so a truncated last line can't cause a false negative). Skips blank/`#`-comment lines; inspects up to `SNIFF_MAX_LINES`; returns `false` on the first line that fails `line_is_bare_ntriples`. Requires **at least one** clean statement (`saw_statement`) — an empty/comment-only sample is **not** confidently N-Triples ⇒ Turtle.
- `line_is_bare_ntriples(line: &str) -> bool` (`loader.rs:2239-2363`): byte-level token walk over one physical line, consuming each term as a unit so a `:`/`;`/`,`/`@` inside an `<IRI>` or `"…"`/`'…'` literal is never mistaken for Turtle. Accepts exactly: `<IRI>` (to first unescaped `>`; `\>` stays inside; unterminated ⇒ false), `_:label` (a bare `_` not followed by `:` ⇒ false), `"…"`/`'…'` literals honouring `\` escapes — a `"""`/`'''` long-string opener ⇒ false — optionally decorated by `@lang` (`[A-Za-z0-9-]+`, set only immediately after a literal closes via the `after_literal` flag) **or** `^^<datatype>` where the datatype **must** be an absolute `<IRI>` (a `^^pfx:type` prefixed datatype ⇒ false), and the terminator `.`. Any other token (bare `:` prefixed name, `;`/`,` lists, `[]`/`()`/`{}`, a stray `@`/`^` not decorating a literal, any bare alphanumeric like the `a` keyword / `true` / numbers / `PREFIX`/`BASE`) ⇒ false. Must end cleanly terminated and not mid-literal (`terminated && !after_literal`). Conservative throughout: any malformation (e.g. a space before `@`) ⇒ false ⇒ routed to the full parser.

**No-data-loss guarantee.** The staged STAGE phase parses with `oxttl::NTriplesParser` (line-oriented bare-term N-Triples only). Routing a real Turtle file there would make it lenient-SKIP every directive/prefixed/multi-line statement = **silent data loss**. The dispatch therefore takes the staged path **only when confident** the input is N-Triples; **Turtle always uses the full `TurtleParser`** via `ingest_dispatch`. The classifier is deliberately conservative — every ambiguity returns `false` (Turtle, safe). This matches the GROUND TRUTH.

> **Doc-vs-code note (minor).** The `load_turtle` docstring (`loader.rs:2507`) references the readiness check as `jobctl::is_ready`; the actual call site (`loader.rs:2555`) is `crate::storage::staged::jobctl::is_ready()` — consistent, just a shortened path in prose. No behavioural discrepancy found.

`load_turtle_verbose` (`loader.rs:2587-2602`) shares the `bulk_load` branch and the `base` filter but **does not** implement the staged/sniff dispatch — on the non-bulk path it always opens the file and runs `ingest_dispatch`, returning `stats_to_jsonb`. **This is a deliberate asymmetry** (`load_turtle_verbose` measures the in-backend path; it cannot report staged stats because the staged coordinator returns its own JSONB). Callers needing staged behaviour must use `load_turtle`.

### 3.3 The inline `parse_*` family (in-backend single-pass ingest)

These share `ingest_dispatch` / `ingest_quads_dispatch` and the `flush_batch` prepared plan.

**Turtle core — `ingest_turtle_with_stats<R: Read>` (`loader.rs:304-385`):** drives `oxttl::TurtleParser` (`.with_base_iri` panics `load_turtle: invalid base IRI …` on bad base). Per triple: `subject_to_id` / `intern_term` (predicate) / `object_to_id`, push to `batch_s/p/o`, `flush_batch` at `BATCH_SIZE = 1000` (`loader.rs:43`). **Invariant:** parse errors are **fatal** here (`r.expect("load_turtle: turtle parse error")`) — the serial in-backend path is **not** lenient (contrast bulk/streaming, §3.3/§3.4). Phase timers (`parse_ms`/`dict_ms`/`insert_ms`) accumulate nanoseconds.

**Combined path — `ingest_turtle_combined` (`loader.rs:686-829`):** the TA-7 production single-pass path. Per term: cache → shmem (`try_resolve_or_defer`, `loader.rs:895-913`) → defer queue; bulk-resolve the queue via `put_terms_batch` (2 SPI calls/flush regardless of size, `flush_defer` `loader.rs:958-985`) when it hits `dict_batch_size` **or** before draining pending triples. **Invariant:** the defer queue MUST be empty before `drain_pending_into_batch` (`loader.rs:991-1015`) so every s/p/o key is resolvable in `cache` (panics `cache miss` otherwise). Newly-resolved terms are `stage_for_commit`-published to shmem for future-ingest hot hits. Datatype IRIs resolve synchronously (`resolve_datatype_iri_sync`, `loader.rs:920-953`).

**Batched spike — `ingest_turtle_dict_batched` (`loader.rs:415-652`):** 2-pass (materialize all triples into `Vec<oxrdf::Triple>`, batch-resolve unique terms via `put_terms_batch`, re-walk + flush). **Memory cost O(triples)** — explicitly **LUBM-1-scope only** per its docstring; not for large loads.

**Counter taxonomy invariant** (all four Turtle paths): `dict_cache_hits + shmem_cache_hits + dict_db_calls = 3 × triples`. `dict_db_calls` is counted **per-term** even on combined/batched (where the physical SPI shape is 2 calls/flush) so the taxonomy stays comparable; the SPI-shape difference shows up only in `elapsed_ms`.

**UDF signatures:**
- `pgrdf.parse_turtle(content TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) → BIGINT` (`loader.rs:2610`).
- `pgrdf.parse_turtle_verbose(content TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) → JSONB` (`loader.rs:2618`).
- `pgrdf.parse_turtle_dict_batched(content TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, dict_batch_size INT DEFAULT 500) → JSONB` (`loader.rs:2642`); overrides `path` to `"dict_batched"` and adds `dict_batch_size` to the JSONB.
- `pgrdf.load_turtle_dict_batched(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, dict_batch_size INT DEFAULT 500) → JSONB` (`loader.rs:2677`).

### 3.4 The streaming loader

**Purpose:** billion-scale `.nt` ingest with **bounded RAM** and no SQL anti-join. Breaks the whole-file-in-RAM ceiling and super-linearity of the parallel-bulk path (a 32 GB whole-file load was killed at ~2 h; doc `loader.rs:1851`).

**UDF signature:** `pgrdf.load_turtle_streaming(path TEXT, graph_id BIGINT, window_triples INT DEFAULT 20000000, id_reserve_block INT DEFAULT 1000000, base_iri TEXT DEFAULT NULL) → JSONB` (`load_turtle_streaming`, `loader.rs:2167-2183`). Args are clamped `.max(1)`; empty `base_iri` → NULL.

**Guard — `streaming_load_guarded` (`loader.rs:2139-2156`):** probes `SELECT NOT EXISTS (SELECT 1 FROM pgrdf._pgrdf_dictionary)`. **Empty dict** → `ingest_turtle_streaming`. **Populated dict** → falls back to the always-correct `ingest_dispatch` (the streaming path dedups via its persistent map, not against existing rows — see invariant). **Note:** the empty-dict fallback passes `base_iri` through to `ingest_dispatch`, but the streaming path itself (`ingest_turtle_streaming`) takes **no `base_iri`** — it is N-Triples-only and applies no base.

**Core — `ingest_turtle_streaming` (`loader.rs:1854-2134`):** streams the file through a 1 MiB `BufReader`, reading `window_triples` non-blank/non-comment lines per window (never `read_to_end`). A **persistent** `HashMap<DictKey,i64>` lives across all windows (the anti-join replacement). Per window: PASS 1 (rayon `par_chunks(4096)`, **lenient** — parse errors are skipped + counted into `parse_skipped`, not fatal), PASS 2 (intern new terms; tier-1 URI/blank/datatype-IRIs first, tier-2 literals; ids from contiguous reserved blocks of `id_reserve_block` via `min(nextval … generate_series)` `OVERRIDING SYSTEM VALUE` insert), PASS 3+4 (pure-lookup resolve + `flush_batch` at `BULK_QUAD_BATCH = 50_000`, `loader.rs:53`). **Defer-index ONCE** across the whole load: `bulk_drop_indexes` before the loop, `bulk_rebuild_indexes` after. Stats: `windows`, `dict_terms` (final map size). **Invariants:** correct **only** on an empty dict; peak RAM ≈ one window + the (sub-linear) persistent unique-term map.

### 3.5 The `bulk_load` fast path

**Purpose:** v0.6.2 parallel in-backend bulk ingest on a **fresh** dictionary; the explicit `bulk_load => TRUE` opt-in. **Independent of and unchanged by v0.6.14's staged dispatch.**

**Guard — `bulk_load_guarded` (`loader.rs:2488-2499`):** probes the empty-dict condition. **Empty** → `ingest_turtle_parallel_bulk`. **Populated** → falls back to `ingest_dispatch` (combined; anti-joins every term — slower, always correct). Recommended order for the win: bulk-load the large file first into a fresh DB, then load smaller files normally.

**Core — `ingest_turtle_parallel_bulk(path, graph_id)` (`loader.rs:1536-1842`):** `read_to_end` the whole file (whole-file-in-RAM — the ceiling the streaming path lifts), split at newline boundaries into `nthreads` chunks. **Four passes:**
- **PASS 1 (rayon):** parallel parse per chunk; **lenient** (syntax errors skipped + summed into `parse_skipped`).
- **Defer-index** (`loader.rs:1645-1651`): when `triples >= pgrdf.bulk_defer_index_min`, `bulk_drop_indexes` now / `bulk_rebuild_indexes` after PASS 4 (recorded in `stats.defer_index`, `index_ms`). Tiny loads skip it (avoid the global ACCESS-EXCLUSIVE DDL).
- **PASS 2 (main):** parallel dedup of unique terms (per-chunk `HashSet` union-reduced), tier-1 URI/blank+datatype-IRIs then tier-2 literals; ids **reserved from the IDENTITY sequence** per chunk (`nextval … generate_series`, chunk_sz = 500_000) — race-free against a concurrent loader (#20), inserted via `OVERRIDING SYSTEM VALUE`. **No per-term anti-join** (the in-Rust dedup guarantees one row per term on a fresh dict).
- **PASS 3 (rayon):** resolve every triple → `(s,p,o)` id tuple against the read-only term→id maps (`resolve_ms`).
- **PASS 4 (main):** `flush_batch` at `BULK_QUAD_BATCH = 50_000`.

`stats.path = "parallel_bulk"`. **Invariants:** correct only on empty dict; rayon regions are pure-CPU (no SPI/palloc inside any closure) so it's safe on one PG backend; `.nt` blank labels are document-scoped (cross-chunk merge by label is correct for N-Triples; multi-line Turtle / anonymous `[]` blanks are out of scope — use the serial path). Dropping `unique_term` is safe because the in-Rust dedup produces no duplicate tuples, and the rebuild **VALIDATES** that as a backstop (`bulk_drop_indexes` `loader.rs:1478-1487`; `bulk_rebuild_indexes` `loader.rs:1496-1507`). `bulk_drop_indexes` is `pub(crate)` because the staged coordinator reuses the exact same defer-index drop.

**Index defer DDL** (`loader.rs:1496-1507`): drops `_pgrdf_idx_spo/pos/osp`, `_pgrdf_dict_val_idx` hash, and the `unique_term` UNIQUE constraint; rebuild mirrors `sql/schema_v0_2_0.sql` — hexastore indexes `INCLUDE (is_inferred)`, dict hash on `lexical_value`, and `unique_term UNIQUE (term_type, lexical_md5, datatype_iri_id, language_tag)`.

**Insert-path spikes (`loader_ta11.rs`):** `spike_ta11_batch_sweep`, `spike_ta10_logged_flat`, `spike_ta10_logged_indexed`, `spike_ta10_logged_partitioned` — each `(triple_count INT DEFAULT 100000, batch_size INT DEFAULT 1000) → JSONB`. They write **synthetic** rows to UNLOGGED/LOGGED flat/partitioned/indexed **temp** targets (`pgrdf_ta11_target`, `pgrdf_ta10_*`), never `_pgrdf_quads`, to decompose the prepared-`unnest` insert cost (SPI roundtrip vs WAL vs partition routing vs index maintenance). **Measurement-only; not part of the production ingest path.**

### 3.6 Quad ingest — `parse_trig` / `parse_nquads` (graph-routed)

**UDF signatures:**
- `pgrdf.parse_trig(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) → JSONB` (`loader.rs:2725`) — drives `oxttl::TriGParser`.
- `pgrdf.parse_nquads(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) → JSONB` (`loader.rs:2751`) — drives `oxttl::NQuadsParser`.

Both call `ingest_quads_dispatch` (`loader.rs:1398-1426`): GUC route → `Baseline|ShmemWarm → ingest_quads_with_stats` (per-term SPI, `loader.rs:1212-1264`) or `Batched|Combined → ingest_quads_combined` (single-pass defer-queue, `loader.rs:1281-1363`). There is no separate 2-pass quad spike — `batched` maps to the combined defer mechanism. JSONB via `quad_stats_to_jsonb` (`loader.rs:1432-1457`): the Turtle stat keys plus a `graphs[]` array (resolved destination ids, first-seen order).

**Graph routing — `resolve_graph_id` (`loader.rs:1158-1204`)** (resolved **before** any term interning/buffering, so a rejection leaves no partial rows): `DefaultGraph → default_graph_id`; a named-node IRI → `pgrdf.graph_id` if bound, else under default (`strict == false`) `pgrdf.add_graph(iri)` auto-allocates id + LIST partition, under `strict == true` panics `{parse_trig|parse_nquads}: unknown graph iri <iri>`; a **blank-node graph label** is illegal and panics. Resolved ids cache per call. Quads partition into `GraphBatches` (`loader.rs:1106-1139`) keyed by `graph_id`, each partition flushed through the same `QUAD_INSERT_SQL` plan (`$4 = graph_id`); `flush_all` flushes in **sorted graph-id order** for deterministic accounting. **Invariant:** parse errors are **fatal** on the quad paths (`panic!("{prefix}: quad parse error: {e}")`) — these are not lenient.

### 3.7 CONSTRUCT-result ingest (`construct_ingest.rs`)

**Purpose:** the inverse of `pgrdf.construct(q)` — decode its structured-term JSONB rows back into the dictionary + hexastore (LLD v0.4 §6.3 round-trip).

**UDF signatures:**
- `pgrdf.put_construct_row(row JSONB, graph_id BIGINT DEFAULT 0) → BIGINT` (`construct_ingest.rs:203`) — independent per-call bnode map; returns 1 if a fresh quad landed, 0 if deduplicated.
- `pgrdf.put_construct_rows(rows JSONB[], graph_id BIGINT DEFAULT 0) → BIGINT` (`construct_ingest.rs:233`) — **recommended surface**; one shared `HashMap<String,i64>` bnode map across the batch; returns the count of newly-inserted quads. **NULL input is a no-op returning 0** (handles `array_agg(j)` returning NULL on empty input). Both panic if `graph_id < 0`.

**Row shape** (`construct_ingest.rs:14-24`): `{subject:{type,value}, predicate:{...}, object:{type:"iri"|"literal"|"bnode", value, datatype?, language?}}`.

**Decode — `decode_term_cell` (`construct_ingest.rs:67-127`):** `iri → put_term_full(value, URI)`; `bnode →` per-call `bnode_map` lookup then `put_term_full(label, BLANK_NODE)`; `literal →` rejects subject/predicate position (RDF), treats `language.is_some()` as the gate for `datatype_id = None`, otherwise resolves the `datatype` IRI (defaulting to `xsd:string` if absent). Unknown type panics `pgrdf.put_construct_row: …: unknown term type`.

**Insert — `insert_quad` (`construct_ingest.rs:135-161`):** auto-creates the partition for non-default `graph_id` via `pgrdf.add_graph`, then inserts through a `WHERE NOT EXISTS` guard (with `is_inferred = false`), returning whether a fresh row landed.

**Invariants:** (1) re-ingesting the same rowset is **idempotent** (set semantics via `WHERE NOT EXISTS`, matching `executor::insert_quad`); (2) within one `put_construct_rows` call, repeated bnode labels collapse to **one** stored blank node (preserving within-solution joining per W3C SPARQL 1.1 §16.2), distinct labels to distinct nodes; (3) cross-call, the shared map is **per call** — a second call re-uses dict ids interned by `put_term_full` but introduces no cross-call bnode joining the captured data doesn't warrant; (4) literals in subject/predicate position are rejected. All panics use the stable prefix `pgrdf.put_construct_row`.

---



## 4. Native staged bulk loader & runtime

> **Source of truth:** `src/storage/staged/{mod,pool,jobctl,phases}.rs` + `src/query/guc.rs` on `origin/main` at v0.6.14. Module-level doc-comments still carry the historical "v0.7 / R2.0 / R2.1" phasing labels; the *shipped code* is the v0.6.14 staged loader and the T1–T5 levers below are all live. Where a comment lags the code this section follows the code and flags it.

### 4.1 Purpose & entry points

The staged loader is the in-database, set-based port of the E32-proven SQL prototype (parse → UNLOGGED staging → parallel set-based dedup → parallel hash-join resolve → concurrent index, **committed per phase**). Its flagship result is 8.2 B truthy Wikidata triples ingested in 4 h 53 m / ~466 K tps, out-of-the-box on stock PostgreSQL.

A single `#[pg_extern]` function cannot COMMIT mid-phase, run several `CREATE INDEX` simultaneously, or own *N* COPY streams — pgrx 0.16 cannot emit a transaction-controlling PROCEDURE (`mod.rs:1-14`). So the loader is a **dynamic background-worker pool**: a thin SQL-callable coordinator spawns *N* workers at runtime; each worker is its own backend with its own committed transaction(s) and owns one phase/shard. Commit-per-phase lives in the *workers* (`BackgroundWorker::transaction(|| …)`), never the coordinator — the load-bearing design decision.

Two SQL entry points (`pool.rs`):
- `pgrdf.load_turtle_staged_run(path TEXT, graph_id BIGINT, n_workers INT DEFAULT 0) → JSONB` — the real loader (`pool.rs:543-706`).
- `pgrdf.load_turtle_staged_ping(n_workers INT) → JSONB` — a standalone regression of the spawn/wait/report machinery, distinct from the real phases (`pool.rs:279-386`).

**Hard prerequisite:** both refuse with a clear `error!` (not a panic) when pgRDF is not in `shared_preload_libraries`, because the pool needs the shmem job-control segment (`pool.rs:282-287, 546-551`; readiness latch `jobctl.rs:170-174`).

Module layout (`mod.rs:16-18`): `pub mod jobctl; pub mod phases; pub mod pool;`.

### 4.2 Architecture: coordinator + bgworker pool + shmem jobctl

```
SQL caller (one open txn, cannot COMMIT)
  └─ coordinator  load_turtle_staged_run  (pool.rs)
       │  per phase: claim slots → spawn N workers → wait_for_startup+shutdown → tally shmem status
       │  holds NO lock on any shared table while waiting  (deadlock-avoidance invariant)
       ▼
   jobctl shmem segment   (PgLwLock<[JobSlot;8]>, PgLwLock<[WorkerSlot;256]>, PgAtomic<u64>)
       ▲                                    ▲
       │ db_oid, path, phase HWM, err       │ phase, shard, [range_lo,range_hi), status
   dynamic bgworker(s)  pgrdf_staged_worker_main  (pool.rs)
       └─ reconnect to db_oid → BackgroundWorker::transaction(|| phases::<phase>(job,w)) → report_worker
```

**Coordinator** (`pool.rs:543-706`): validates prereqs, stats the file, `create_job`s a shmem slot, then drives `run_phase` once per phase (`pool.rs:418-502`), gating A→B→C→D. After each successful phase it records the resumable high-water mark (`advance_phase`). On any phase failure it ABORTS, leaving the staging table as the resume point. It reads final counts **only after** the INDEX phase, when no worker remains (so its `ACCESS SHARE` conflicts with nothing).

**Worker pool** (`pool.rs:66-180`): `spawn_checked` is the single mandated chokepoint for `load_dynamic` — it always sets `set_notify_pid(MyProcPid)` (without it `wait_for_*` return `Err(Untracked)`) and always *matches* the `load_dynamic` `Result` rather than `.unwrap()`ing it (an ignored `Err` on pool exhaustion was the historical null-handle segfault). `set_restart_time(None)` ⇒ fail-fast, no respawn loop; `enable_spi_access()` ⇒ worker may use SPI, starts at `RecoveryFinished`.

**Worker entry** `pgrdf_staged_worker_main(arg: Datum)` (`pool.rs:105-180`):
- `#[no_mangle] #[pg_guard] extern "C-unwind"` — MANDATORY: the postmaster `dlsym`s the worker by the literal string `"pgrdf_staged_worker_main"` (`WORKER_FN`, `pool.rs:45`), so it must be exported unmangled and not dead-code-eliminated (the symbol is referenced only by string).
- `arg` is the worker's `WorkerSlot` index. It reads its slot → follows `job_idx` to the job → attaches `SIGTERM|SIGHUP` handlers → `connect_worker_to_spi_by_oid(Some(db_oid), None)`. A dynamic worker does **not** inherit the spawner's database; the coordinator records `MyDatabaseId` into `JobSlot.db_oid` (a fixed-width `u32`, immune to the path-style inline-array truncation risk) and the worker reconnects by OID.
- The DB-touching body runs in **one** `BackgroundWorker::transaction(|| match w.phase { … })` (the per-phase recovery unit), wrapped in `std::panic::catch_unwind`. The outcome is reported via **shmem, not the exit code** — a worker that `ereport`s ERROR still *stops*, so the parent's `wait_for_shutdown` returns `Ok(())` regardless. The worker writes its true success/failure with `report_worker` before returning.
- `panic_message` (`pool.rs:201-246`) recovers the real cause from the caught panic payload, in priority order: pgrx `CaughtError` → `ErrorReportWithLevel::message()` → `ErrorReport` → `&str`/`String` → a phase+shard+pid fallback pointing at the server log. This is the diagnostic keystone — at 8.2 B rows a SQL ERROR arrives as a `panic_any(ErrorReportWithLevel)`, not a `&str`, so the old downcast lost the message; this recovers the actual `ereport` text into the JSONB `error`.

**Invariants:**
- *No coordinator-held table locks while workers run.* All shared-table mutation (DDL, COPY, CTAS, ATTACH, index builds) is inside workers, which COMMIT and release before the next phase. If the coordinator held an `ACCESS EXCLUSIVE` lock across `wait_for_shutdown`, the workers needing that table would block on it while the coordinator blocks on them — deadlock. This is why STAGE prep is split into its own worker (§4.3) and why partition creation lives in RESOLVE, not coordinator-side prep (`pool.rs:529-532, phases.rs:429-470`).
- *Outcome channel is shmem `WorkerSlot.status`* (1 ok, 2 error, 0 never-reported), tallied per phase (`run_phase` scans only **this** phase's claimed slots; `pool.rs:472-481`).
- *Graceful pool exhaustion:* the ping coordinator records a spawn failure and continues (partial result, never a crash); the real `run_phase` treats any spawn failure / startup failure / never-reported / dead postmaster as a phase failure (the load is only correct if every phase worker ran).

### 4.3 The four phases — STAGE → DICT → RESOLVE → INDEX

Phase ordinals double as the resumable high-water mark (`jobctl::phase`, `jobctl.rs:38-56`): `NONE=0, STAGE=1, DICT=2, RESOLVE=3, INDEX=4, DONE=5`, plus dispatch-only labels `STAGE_PREP=6` and `PING=250` (deliberately outside the 1..5 HWM range so they can never be mistaken for a real mark). All phase bodies live in `phases.rs` and run inside the worker's wrapping transaction (they never COMMIT themselves; returning commits).

**Phase A — STAGE (T3, multi-backend).** Split into two sub-steps so STAGE can go *N*-way without prep DDL racing across the pool (`pool.rs:600-627`):

- **A0 `STAGE_PREP`** — ONE worker (`phases::prepare_for_load`, `phases.rs:453-470`). Runs `apply_session_gucs`, then (1) `bulk_drop_indexes` — drops the 3 hexastore indexes, the dict `lexical_value` hash index, and the `unique_term` constraint so DICT/RESOLVE skip per-row maintenance (Phase D rebuilds the byte-identical set); (2) `CREATE UNLOGGED TABLE IF NOT EXISTS pgrdf._pgrdf_stg_<job_id> (s text, p text, o_type smallint, o_val text, o_dt text, o_lang text) WITH (parallel_workers = nproc)`. UNLOGGED skips WAL (a measured ~141 GB win); the `parallel_workers` reloption lifts PG's per-table worker cap so DICT/RESOLVE scan it on all cores. The destination partition is **not** created here — RESOLVE owns it. Both steps run in this worker's committed txn, releasing their `ACCESS EXCLUSIVE` locks before the next phase.

- **A1 `STAGE`** — *N* workers (`phases::stage`, `phases.rs:680-734`). The coordinator computes *N* newline-snapped byte ranges (`stage_byte_ranges` → `snap_byte_ranges`, `phases.rs:96-170`): the file is cut into *N* roughly-equal pieces, each interior cut pushed FORWARD past the next `\n`. Union is exactly `[0, file_len)`, no gap/overlap — every line staged by exactly one worker (the shared-table correctness invariant). Each worker `seek(lo)` + `.take(hi-lo)` streams its range in bounded **windows** of `STAGE_WINDOW_LINES = 4_000_000` (`phases.rs:63`) — never `read_to_end` (the file is ~1.2 TB; peak RAM is one window). Each window is parsed across all cores via rayon `par_chunks(STAGE_PARSE_CHUNK=4096)`, each chunk by its own `NTriplesParser` (`parse_window_par`, `phases.rs:545-601`), **leniently** — a malformed line is skipped and counted, not fatal (Wikidata control-byte robustness). Parsed rows are COPYed in via a server-side temp TSV at `/tmp/pgrdf_stg_<job>_<shard>_<window>.tsv` (`copy_window`, `phases.rs:616-655`) — pgrx/SPI has no COPY FROM STDIN, and COPY is the measured order-of-magnitude win over `unnest` INSERT (71.5 M rows in 31 s on E160). The TSV is removed on success or failure. Parallelism is therefore BOTH intra-worker (rayon) AND across *N* COPY-issuing backends into the same UNLOGGED heap (PG permits concurrent COPY/INSERT). The coordinator records `phase::STAGE` HWM once prep + all STAGE workers succeed.

  Object→column split mirrors the dict key (`STAGE_COLS`, `phases.rs:53`): `o_type` SMALLINT (`URI=1/BLANK=2/LITERAL=3`); `o_val` lexical value; `o_dt` datatype-IRI **string** (resolved to a dict id in DICT); `o_lang` language tag; NULL `o_dt`/`o_lang` mirror the NULLS-DISTINCT dict key.

**Phase B — DICT (1 worker).** `phases::dict` (`phases.rs:818-943`). Because `_pgrdf_dictionary.id` is `GENERATED ALWAYS AS IDENTITY`, any `INSERT … SELECT` into it is parallel-UNSAFE (serial, `nextval`/row). The fix: do the expensive dedup in **parallel** `CREATE UNLOGGED TABLE … AS SELECT … row_number() OVER () AS id …` materialisations (no IDENTITY target ⇒ PG14+ runs a parallel `Gather → Parallel Append`), pre-assigning contiguous ids, then ONE serial `INSERT … OVERRIDING SYSTEM VALUE` copies the numbered rows in. Steps:
1. `dict_uri` — DISTINCT URIs (non-blank subjects, predicates, object URIs, **and object datatype IRIs**), numbered `base+1…` via `UNION ALL`+`GROUP BY` (parallelisable dedup, not `UNION`). Datatype IRIs live here so a literal's `datatype_iri_id` reads the URI's id.
2. `dict_blank` — DISTINCT blank labels (`s LIKE '\_:%'` + blank objects), ids continue.
3. `dict_lit` — DISTINCT literals by **FULL** identity `(o_val, o_dt, o_lang)` (`dict_lit_dedup_select`, `phases.rs:749-751`), ids continue. The full-key dedup is the v0.6.12 release-blocking fix: grouping by `o_val` alone collapsed `"Berlin"@en`/`"Berlin"@de`/`"1"^^xsd:integer`/plain `"1"` into one impossible row.
4. `dict_all` — the three unioned with final `(term_type, datatype_iri_id, language_tag)`; each literal's `datatype_iri_id = LEFT JOIN dict_uri ON lexical_value = o_dt` (parallel CTAS).
5. ONE `INSERT INTO _pgrdf_dictionary OVERRIDING SYSTEM VALUE (…) SELECT … FROM dict_all` (`lexical_md5` is STORED-generated, not inserted).
6. Re-sync the IDENTITY sequence to `MAX(id)` via `setval(…, is_called=true)` (ids were supplied, so the sequence never advanced); build the transient RESOLVE join index `_pgrdf_dict_resolve_idx (term_type, lexical_md5)` (`dict_resolve_index`, `phases.rs:785-787`); `ALTER TABLE … SET (parallel_workers = nproc)` to widen the dict so RESOLVE runs *N*-wide.
7. Drop the `dict_*` temps; return the dict count.

**Phase C — RESOLVE (1 worker).** `phases::resolve` (`phases.rs:975-1069`). Turns every staged triple into a `(subject_id, predicate_id, object_id, graph_id, is_inferred)` quad via a 3× hash-join to the dict, landed as a **standalone CTAS then ATTACH**, NOT a direct `INSERT INTO _pgrdf_quads` (tuple-routing into a partitioned target is parallel-unsafe ⇒ serial leader). Flow:
1. Apply the join-strategy `SET LOCAL` block from `pgrdf.staged_resolve_strategy` (`resolve_join_strategy_sql`, §4.4 T2); warn + log on fallback.
2. `acquire_partition_ddl_gate()` — the outermost lock, same order as add_graph/drop_graph, since the CREATE + ATTACH escalate to `ACCESS EXCLUSIVE` on `_pgrdf_quads`.
3. Resume-safe `DROP TABLE IF EXISTS pgrdf._pgrdf_quads_g<g>` (DROP detaches+drops an attached partition).
4. `CREATE TABLE pgrdf._pgrdf_quads_g<g> WITH (parallel_workers = nproc) AS SELECT …` — the 3-way join: `ds` subject (term_type 2 if `s LIKE '\_:%'` else 1), `dp` predicate (1), `dobj` object (`o_type`) with a LEFT-JOINed `ddt` supplying the datatype-IRI string. Object matches **full literal identity** (`resolve_object_join_on`, `phases.rs:773-778`): `term_type` + `lexical_md5` + `language_tag IS NOT DISTINCT FROM o_lang` + `ddt.lexical_value IS NOT DISTINCT FROM o_dt`, all NULL-safe so URIs/blanks resolve unchanged while typed/lang literals get their own ids.
5. Make partition-compatible: `SET NOT NULL` on every parent-required column + a redundant `CHECK (graph_id = <g>)` that implies the `FOR VALUES IN (<g>)` bound so `ATTACH` skips its validation scan.
6. `ATTACH PARTITION … FOR VALUES IN (<g>)`, then drop the redundant CHECK (partition is then structurally identical to one made by `PARTITION OF`).

**Phase D — INDEX (5 workers).** `phases::build_index` (`phases.rs:1078-1085`), one worker per `jobctl::index_ddls()` entry (`jobctl.rs:334-343`): the 3 hexastore covering indexes (`_pgrdf_idx_spo/pos/osp` on `_pgrdf_quads`, `INCLUDE (is_inferred)`), the dict `_pgrdf_dict_val_idx` HASH index on `lexical_value`, and the `unique_term` UNIQUE re-add. The worker's `shard` selects the DDL. Plain (non-`CONCURRENTLY`) `CREATE INDEX` is correct — the fresh quads table isn't queried during load; "concurrent" parallelism comes from running the 5 separate builds at once across backends. After the phase the coordinator drops DICT's transient `_pgrdf_dict_resolve_idx` (redundant once `unique_term` exists).

### 4.4 The GUC surface

All custom GUCs are registered once in `_PG_init` via `guc::register` (`guc.rs:209-331`); all are `GucContext::Userset` (per-session `SET` is safe — staged GUCs affect only the setting session's phases). Staged-relevant GUCs:

| GUC | Type | Default | Effect |
|---|---|---|---|
| `pgrdf.staged_temp_tablespaces` (**T1**) | string (tablespace-name list) | empty (`None`) → inherit server `temp_tablespaces` | When non-empty, every staged phase emits `SET LOCAL temp_tablespaces = '<value>'`, routing temp spill — dominated by RESOLVE's hash join (~3 TB at 8.2 B rows) — off the PGDATA disk. `guc.rs:97-118, 281-296`; resolver `staged_temp_tablespaces()` `guc.rs:394-401`. |
| `pgrdf.staged_resolve_strategy` (**T2**) | string `auto`\|`hash`\|`index` | **`index`** | Forces RESOLVE's planner join method. Performance knob only (identical output). `guc.rs:120-145, 297-314`; resolver `staged_resolve_strategy()` `guc.rs:414-421`. |

**T1 details.** The value must be a comma-separated list of plain SQL identifiers, validated by `is_safe_tablespace_list` (`phases.rs:281-294`: each name `[A-Za-z_][A-Za-z0-9_]*`, no quotes/semicolons/dots/inner-spaces) before interpolation, so it can never break out of the single-quoted `SET LOCAL`. `temp_tablespaces_set_fragment` (`phases.rs:259-272`) returns `Ok(None)` for empty (inherit default), `Ok(Some(fragment))` for valid, `Err(reason)` for unsafe — in which case `apply_session_gucs` logs a `warning!` and falls back to the server default (`phases.rs:376-387`).

**T2 details.** `resolve_join_strategy_sql` (`phases.rs:314-341`) maps the strategy to a `SET LOCAL` block run before the RESOLVE CTAS:
- `index` (default) — low-spill index-nested-loop: `enable_nestloop/indexscan/indexonlyscan/bitmapscan = on`, hash/merge off, `hash_mem_multiplier = 2`. The at-scale-validated default: an out-of-the-box 8.2 B-triple E64ads_v7 load completes with no multi-TB hash spill / no ENOSPC.
- `hash` — historical all-hash-join (`enable_hashjoin = on`, all else off); identical output but spills multi-TB at scale. Also the known-safe fallback.
- `auto` — no `enable_*` forcing; planner chooses (still bumps `hash_mem_multiplier = 2`).
- unrecognised → falls back to `hash` and the caller `warning!`s (`phases.rs:993-1001`). Note: `guc.rs:301-310`/`144` document the default as `index`; the `staged_resolve_strategy()` resolver's *blank/cleared* fallback is `auto` (`guc.rs:419`) — only reachable if an operator explicitly RESETs the GUC.

Other staged-touching GUCs (`bulk_defer_index_min`, `auto_analyze`, etc.) are not part of the staged worker path proper. The staged loader has **no** dedicated work_mem/parallelism GUCs — T5 (§4.6) is *computed*, not configured.

### 4.5 Parallel STAGE COPY (T3)

T3 is the multi-backend STAGE described in §4.3-A. `load_turtle_staged_run` chooses the STAGE width (`pool.rs:559-567`): `n_workers > 0` ⇒ that explicit width (clamped to `MAX_SLOTS`); `n_workers = 0` ⇒ AUTO = `std::thread::available_parallelism()`, clamped `[1, STAGE_WORKERS_AUTO_CAP=32]` so a very-high-core box can't exhaust `max_worker_processes` / jobctl `MAX_SLOTS`. Phases are sequential, so STAGE and INDEX never hold slots simultaneously. DICT and RESOLVE stay a single worker each (their parallelism is PG's intra-query parallel hash-agg/hash-join, lifted by the `parallel_workers` reloption + session GUCs); INDEX width is fixed at `index_ddls().len()` = 5. A single STAGE range (tiny file or `requested == 1`) is byte-identical to the prior single-worker behaviour.

### 4.6 Adaptive self-tune (T5)

`apply_session_gucs` (`phases.rs:363-427`) re-applies the per-session parallel levers inside each worker's transaction (a dynamic worker starts with server defaults, and `SET LOCAL` scopes them to that transaction). It is called at the top of every real phase body (`pool.rs:137,147,154,158,162`). It emits one semicolon-joined statement: `max_parallel_workers`, `max_parallel_workers_per_gather` = `nproc`; `max_parallel_maintenance_workers` = `nproc/2`; `enable_parallel_hash = on`; `parallel_setup_cost`/`parallel_tuple_cost` = 0; `min_parallel_table_scan_size`/`min_parallel_index_scan_size` = 0; plus the derived `work_mem`/`maintenance_work_mem` and the optional T1 `temp_tablespaces` fragment.

`work_mem`/`maintenance_work_mem` are **adaptive OOM hardening**, not configured. `host_mem_total_bytes` (`phases.rs:176-186`) reads Linux `/proc/meminfo` `MemTotal`; `derive_work_mem_kb(mem_total, nproc)` (`phases.rs:209-239`) sizes them so RESOLVE's 3-way parallel-hash budget `work_mem × hash_mem_multiplier(2) × nproc × 3 joins` stays ≤ ~50 % of RAM:
- `work_mem = MemTotal / (12 × nproc)`, clamped `[64 MB, 2 GB]`.
- `maintenance_work_mem = MemTotal / 8`, clamped `[256 MB, 16 GB]`.
- `mem_total = None` (e.g. macOS dev box with no `/proc/meminfo`) → the EXACT prior fixed values (2 GB / 16 GB) so behaviour is unchanged where unmeasurable.

So a high-RAM-per-core host pins at the 2 GB cap (identical to the old fixed value), an E32 (251 GiB/32c) lands ~669 MB (the old 2 GB implied a 384 GB hash budget — the OOM this removes), and a tiny host floors at 64 MB and the hash **spills** to temp rather than risking an OOM kill (unit-tested, `phases.rs:1408-1473`). The decision is logged per phase (`staged self-tune: MemTotal=… nproc=… work_mem=… …`, `phases.rs:401-406`) for operator visibility. The comments are explicit that this is *hardening*, not the definitive 8.2 B-scale RESOLVE fix — T2's `index` strategy is what delivers the out-of-the-box at-scale result; T5 lowers OOM risk where RAM is tight.

### 4.7 Resumability

Each phase is committed by its worker, and the coordinator records a high-water mark in shmem after each success (`advance_phase`, `jobctl.rs:319-325`). On ANY phase failure the coordinator ABORTS via the `abort` closure (`pool.rs:587-598`): it returns the error JSONB, **leaves the staging table in place as the resume point**, frees the shmem slots (`release_job`), and does NOT mark the job done (the job row stays `state::FAILED`, set by `report_worker` when the first worker error was recorded). Resume-safety is baked into the phase bodies:
- The staging table name is deterministic (`_pgrdf_stg_<job_id>`), created `IF NOT EXISTS` (`phases.rs:73-75, 466-468`).
- DICT drops any stale per-job `dict_*` temps at entry (re-run from the top, `phases.rs:833-837`).
- RESOLVE drops any stale `_pgrdf_quads_g<g>` — attached or standalone — before rebuilding (`phases.rs:1008-1011`).
- INDEX/DICT DDLs use `IF NOT EXISTS` / idempotent forms.

⚠ The HWM is held in shmem (`JobSlot.phase`), not persisted to a catalog table, and `release_job` clears the slot at the end of every coordinator call (success or abort). So the resume point is the on-disk *staging table + attached state*; a fresh `load_turtle_staged_run` call re-finds those by deterministic name, but the in-memory HWM does not survive the coordinator returning. The committed-per-phase artifacts (staging rows, dict rows, attached partition) are what a re-run reconciles against via the idempotent drops/`IF NOT EXISTS`.

### 4.8 The JSONB report shape

**Success** (`load_turtle_staged_run`, `pool.rs:692-705`):
```json
{ "job_id": <i64>, "ok": true, "triples": <i64>, "dict_terms": <i64>, "quads": <i64>,
  "phase_ms": { "stage": <f64>, "dict": <f64>, "resolve": <f64>, "index": <f64> },
  "n_workers": <requested STAGE width> }
```
`triples` = staging row count, `dict_terms` = `_pgrdf_dictionary` count, `quads` = `_pgrdf_quads WHERE graph_id = $1`, all read after INDEX with no worker running.

**Abort** (`pool.rs:587-598`):
```json
{ "job_id": <i64>, "ok": false, "failed_phase": "stage|dict|resolve|index",
  "error": "<first worker error / spawn / startup cause>", "n_workers": <requested>,
  "note": "staging table left in place as the resume point" }
```

**Ping** (`load_turtle_staged_ping`, `pool.rs:375-385`):
```json
{ "job_id": <i64>, "requested": <n>, "spawned": <n>, "succeeded": <n>, "failed": <n>,
  "spawn_failures": <n>, "ping_rows": <i64>, "postmaster_died": <bool>, "error": <string|null> }
```
`succeeded`/`failed` come from shmem `WorkerSlot.status`; `ping_rows` is an independent SPI `COUNT(*) … WHERE job_id = …` proving each worker committed (on the happy path `succeeded == ping_rows == spawned`).

### 4.9 Shmem job-control structs (`jobctl.rs`)

Registered from `_PG_init` only in the postmaster path (`init_in_postmaster`, `jobctl.rs:160-165`), mirroring `storage::shmem_cache`. `bgw_main_arg` is one `Datum` and `bgw_extra` only ~127 bytes, so a worker receives only its **integer slot index** via `set_argument`; the real payload lives in shmem as fixed inline byte arrays (no Rust `String`/pointers — invalid across backends).

- Capacities: `MAX_JOBS = 8`, `MAX_SLOTS = 256`, `PATH_CAP = 1024`, `TSPACE_CAP = 64`, `GUC_CAP = 512`, `ERR_CAP = 512` (`jobctl.rs:25-35`).
- `JobSlot` (`#[repr(C)] Copy`, `jobctl.rs:68-88`): `in_use, phase` (resumable HWM), `state, n_workers, n_shards, db_oid:u32, graph_id:i64, job_id:i64`, the `*_len` lengths, and inline `path/tspace/guc/err` byte arrays. `tspace`/`guc` are RESERVED (the comment at `jobctl.rs:12-18` notes T-levers are applied in-worker via `apply_session_gucs` computed from `num_cpus`, **not** shipped as a serialised GUC blob through shmem — a code-vs-design discrepancy the comment itself records). Tables: `static JOBS: PgLwLock<[JobSlot; 8]>`, `static WSLOTS: PgLwLock<[WorkerSlot; 256]>`, `static NEXT_JOB_ID: PgAtomic<AtomicU64>` (starts at 1).
- `WorkerSlot` (`jobctl.rs:122-135`): `in_use, phase, status` (0 spawned / 1 ok / 2 error), `job_idx:u16, shard:u16, range_lo:u64, range_hi:u64`.
- Accessors (`jobctl.rs:184-396`), all short-circuiting on `!is_ready()`: `create_job` (claims a JobSlot, assigns monotonic `job_id`, validates `path ≤ PATH_CAP` — never truncates), `claim_worker_slot`, `read_job`/`read_worker` (by-value snapshots, lock released on return), `report_worker` (records first-failure error string + flips job to FAILED), `advance_phase`, `mark_job_done`, `release_job` (clears the JobSlot + every WorkerSlot pointing at it), `tally_job`, `job_err`, `index_ddls`.

**Key invariants:** locking mirrors `shmem_cache` (`.exclusive()` to mutate, `.share()` for a by-value snapshot); only the FIRST worker error is recorded (later workers don't clobber it); a worker that started but never reported (`status == 0`) counts as neither success nor failure; `report_worker`'s outcome — not the worker exit code — is authoritative.

---



## 5. SPARQL query engine

The query engine lowers SPARQL (the full SELECT/ASK/CONSTRUCT/DESCRIBE query surface plus the UPDATE algebra) into **parameterised dynamic SQL over the hexastore** (`pgrdf._pgrdf_quads(subject_id, predicate_id, object_id, graph_id)` joined to `pgrdf._pgrdf_dictionary`), executes it through SPI with a per-backend prepared-plan cache, and returns `SETOF JSONB` rows. The parser is `spargebra`; the algebra is `spargebra`'s `GraphPattern` tree. Code lives in `src/query/{mod.rs, parser.rs, executor.rs, path.rs, plan_cache.rs, guc.rs}`. Ground truth: **v0.6.14**; 51 W3C-SPARQL conformance scenarios pass.

### 5.0 UDF surface (exact signatures)

| UDF | Signature | Forms accepted | Output |
|---|---|---|---|
| `pgrdf.sparql` | `(q TEXT) → SETOF JSONB` | SELECT, ASK, **full UPDATE algebra** | one JSONB object per solution; ASK → `{"_ask":"true"\|"false"}`; UPDATE → one `{"_update":{…}}` summary row |
| `pgrdf.construct` | `(q TEXT) → SETOF JSONB` | CONSTRUCT | one row per `(solution, template-triple)` pair, each `{"subject":…,"predicate":…,"object":…}` |
| `pgrdf.describe` | `(q TEXT) → SETOF JSONB` | DESCRIBE | triple rows (same term shape as CONSTRUCT) |
| `pgrdf.sparql_parse` | `(q TEXT) → JSONB` | any query or UPDATE | parse-time shape preview (form, vars, BGP count, `unsupported_algebra`) — never executes |
| `pgrdf.sparql_sql` | `(q TEXT) → TEXT` | SELECT/ASK | translator-introspection: the lowered SQL with `$N` dict-ids inlined as integer literals (for `EXPLAIN`); not user-facing |
| `pgrdf.plan_cache_clear` | `() → BIGINT` | — | drops this backend's cached plans, returns count |

All are `#[pg_extern]` with `#[search_path(pgrdf, pg_temp)]`. Citations: `src/query/executor.rs:208` (`sparql`), `:375` (`construct`), `:1532` (`describe`), `:283` (`sparql_sql`); `src/query/parser.rs:72` (`sparql_parse`); `src/query/plan_cache.rs` (`plan_cache_clear`).

### 5.1 Parse pipeline (spargebra → algebra)

**Purpose.** Turn query text into a `spargebra` AST, then route by form. **Control flow** (`executor.rs:210` `sparql`): try `SparqlParser::new().parse_query(q)` first (SELECT/ASK/CONSTRUCT/DESCRIBE); on failure retry `parse_update(q)` (spargebra splits the grammar into two entry points); if both fail, panic with the **stable prefix `sparql: parse error: {query_err}`** — the *query*-side error is surfaced (the update-side error is suppressed as noise). `pgrdf.sparql_parse` mirrors this two-try ordering but its panic prefix is `sparql_parse: {query_err}` (`parser.rs:80`).

The parser surface (`parser.rs`) is purely descriptive: it walks the algebra and emits a JSONB shape (`form`, `variables`, `bgp_pattern_count`, `bgp_patterns`, `unsupported_algebra`, plus per-form enrichment for CONSTRUCT/DESCRIBE/UPDATE). It is **infallible on any parsed AST** — shapes the executor will reject still describe cleanly (e.g. `with_iri_from_using` returns `None` rather than panicking on multi-IRI USING). `unsupported_algebra` flags only shapes that cannot execute: `Service` (federation), the §7.1-gated property-path remainder, negated property sets. Note `analysis_triple`/`is_executable` (in `path.rs`) drive whether a `Path` node is flagged, keeping parser preview and executor support in lock-step.

**Invariant:** the algebra shape the parser walks is the same one the translator consumes — `sparql_parse` is a safe dry-run of `sparql`.

### 5.2 Translation strategy (algebra → SQL over the hexastore)

`translate(q: &Query) → ExecPlan` (`executor.rs:2226`) is the entry. It clears the thread-local `PARAM_BUF` (`executor.rs:130`), walks the pattern into a `ParsedSelect` via `parse_select` (`:2658`), builds the SQL string via `build_bgp_sql` (`:3974`), snapshots the params, and returns:

```rust
struct ExecPlan { projected: Vec<String>, sql: String, params: Vec<i64>,
                  truncation_probes: Vec<(String, Vec<i64>)> }   // executor.rs:1909
```

**The parameterisation invariant (anti-injection + plan-cache key).** Every constant — IRI, literal, resolved graph id — is resolved to its **dictionary id at translate time** and emitted as a positional `$N` placeholder via `id_placeholder(id)` (`executor.rs:150`), which pushes the i64 onto `PARAM_BUF` and returns `$len`. The generated SQL **never contains user IRI/literal strings or inlined dict ids**; only the structural shape. Constants absent from the dictionary resolve to the sentinel `-1`, so the predicate reliably matches zero rows ("no solutions") rather than erroring (`lookup_iri_id(...).unwrap_or(-1)`, `executor.rs:5955`). Same structural query with different constants ⇒ identical SQL string ⇒ identical plan-cache key (§5.10).

**Variable binding = INNER JOIN via anchors.** Each BGP triple gets a `_pgrdf_quads` alias `q1,q2,…`. The first occurrence of a variable records `(alias_idx, column)` in `anchors: HashMap<String,(usize,&'static str)>`; later occurrences emit equality predicates against the anchor (`q2.subject_id = q1.subject_id`). That is how a shared variable becomes an INNER JOIN. Binders: `pattern_clauses`/`bind_subject`/`bind_predicate`/`bind_object`/`bind_var` (`executor.rs:6220`–`6332`). Projection reads lexical strings off the dictionary keyed by the anchor id.

**The cross-product-proof pinned plan.** `sparql`/`construct`/`describe` each call `pin_join_order()` first (`executor.rs:263`): `SET LOCAL join_collapse_limit = 1` and `from_collapse_limit = 1`. This pins PostgreSQL to the translator's emitted join order. `build_from_and_where` (`:5059`) reorders the BGP through `connected_order` (`:5268`) so **every pattern after the first shares ≥1 variable with the already-placed set** — guaranteeing each `INNER JOIN … ON (real qN.col = qM.col equality)` and structurally eliminating cross joins. Heuristic: seed with the most-bound pattern (most constant positions = most selective; `boundness = 3 − distinct-var-count`), then greedily append the candidate sharing the most variables (tie-break: boundness, then lowest original index — deterministic). A genuinely disconnected BGP component falls back to a cross join for its first pattern (semantically required). Inner joins are commutative/associative, so the result set is identical — only the plan changes. Rationale (LUBM-100 Q2): without this, standalone type patterns become a Cartesian product (~10¹¹ rows) no statistics can rescue. For `n ≤ 2` patterns `connected_order` is a no-op (byte-identical to pre-M4).

### 5.3 Operator handling

`ParsedSelect` (`executor.rs:2037`) is the central IR: `projected`, `bgp: Vec<ScopedTriple>`, `filters`, `optionals`, `values`, `minuses`, `group_vars`, `aggregates`, `having_filters`, `binds`, `union_branches`, `distinct`, `order_by`, `limit`, `offset`, `graph_scope_counter`. `walk_select_scoped` (`:3487`) populates it; `build_bgp_sql` dispatches to one of three builders: single-branch (`build_single_branch_outer`, `:3993`), union (`build_union_sql`, `:4573`), or aggregate (`build_aggregate_sql`, `:4200`; `build_aggregate_over_union_sql`, `:4700`).

- **BGP joins** — §5.2. `ScopedTriple { triple, scope: Option<GraphScope>, path: Option<PathRelation> }` (`executor.rs`).
- **FILTER** — `translate_filter` (`:5813`) → `Option<String>`; `None` ⇒ panic `sparql: FILTER expression not translatable: {expr:?}`. Handles: `=`/`sameTerm` (dict-id compare, falling back to lexical for `STR(?v)="x"`, `LANG(?v)="en"`); `<`/`>`/`<=`/`>=` via `translate_numeric_cmp` (operand cast to NUMERIC only on XSD-numeric datatype rows, else NULL ⇒ row dropped = "type error → unbound"); `IN(…)` over dict ids; `&&`/`||`/`!`; `BOUND(?v)` → `qN.col IS NOT NULL` (correct for both mandatory INNER and OPTIONAL LEFT-JOIN anchors; unknown var → `FALSE`); `isIRI`/`isBlank`/`isLiteral` via correlated subselect on `_pgrdf_dictionary.term_type`; `REGEX`; `CONTAINS`/`STRSTARTS`/`STRENDS`; arithmetic/string/`LANG`/`DATATYPE` via `expr_to_lexical_sql`/`expr_to_numeric_sql`.
- **OPTIONAL** — `OptionalBlock` (`executor.rs:~1860`) holds an **N-triple** inner BGP (plus nested OPTIONALs, inner VALUES, internal FILTERs, `LeftJoin.expression`). Emitted as `LEFT JOIN LATERAL (SELECT …) qOPT_N ON TRUE` (`emit_optional_lateral`, `:5469`): the lateral subquery binds the whole group **atomically** (all-or-nothing per W3C §6.1) — one row of dict-id `vK` columns when the group matches, zero rows ⇒ LEFT JOIN yields NULLs for every optional var. Optional-only vars are registered in the outer `anchors` as `(qOPT_N, vK)` so projection/FILTER/BOUND work unchanged.
- **UNION** — `distribute_unions` (`:3702`) normalises; each branch is a `UnionBranch` with its own BGP/filters/optionals/minuses/values. `build_union_sql` (`:4573`) emits each as a sub-SELECT combined with `UNION ALL`; branch-bound vars are the projection, vars unbound in a branch emit `NULL::TEXT`; outer DISTINCT/ORDER BY/LIMIT/OFFSET wrap the union via a derived table.
- **MINUS** — `MinusBlock { triples, scope }` → `WHERE NOT EXISTS (SELECT 1 FROM … WHERE shared-var equalities AND inner predicates)` (`translate_minus`, `:5694`). MINUS with **no shared variables is a spec no-op and is elided** (`build_from_and_where` skips a `None` from `translate_minus`).
- **VALUES** — `ValuesBlock { variables, rows: Vec<Vec<Option<i64>>> }`. Constants pre-resolved to dict ids (`None` = `UNDEF` → NULL cell, no constraint). Emitted as `(VALUES (id,…),…) AS vN("?x","?y")` derived table (`emit_values_table`, `:5359`), correlated on shared vars via `anchors`.
- **BIND** — `BindSpec { output_var, expression }` → extra SELECT-list column via `translate_bind_expression` (`:2775`). Computed binds that reference bound vars use `computed_bind_join_clauses` (`:6359`). Filtering directly on a BIND output is not supported.
- **Aggregates / GROUP BY / HAVING** — `AggregateSpec { output_var, synth_aliases, func, distinct, arg_var }` with `AggregateFn ∈ {Count, Sum, Avg, Min, Max, GroupConcat{separator}, Sample}`. Supports `COUNT(*)`/`COUNT(?v)`/`COUNT(DISTINCT ?v)`, `SUM`/`AVG` (numeric-aware: non-numeric → NULL), `MIN`/`MAX` (type-aware: numeric on numeric datatypes, else lexical), `GROUP_CONCAT(?v[; SEPARATOR="…"])` (Postgres `string_agg`, default separator a single space), `SAMPLE` (`MIN` surrogate — spec-conformant "implementation-defined element"). `GROUP BY ?vars` optional (absent ⇒ one aggregate row). HAVING: `parse_select` migrates any filter naming an aggregate output (or its synthetic alias) from WHERE into `having_filters` → SQL `HAVING`. Aggregate values return as strings in the JSONB row. `parse_aggregate` (`:2902`), `build_aggregate_sql` (`:4200`), `translate_filter_with_aggregates` (`:4443`).
- **ORDER BY** — `OrderKey ∈ {Var(String), Expr(Expression)}` with ascending flag. **Type-aware** ordering via `type_aware_order_terms` (`:3914`) so numerics sort numerically and other terms lexically (W3C §15.1). `OrderKey::Expr` (`ORDER BY (?a+?b)`, `STRLEN(?s)`) is translated via the BIND/FILTER translator on the single-branch path; on aggregate/UNION paths an expression sort key is a documented deferral that panics (`require_projected_var`, `:2037`) rather than returning a wrong answer.
- **GRAPH (named graphs)** — `GraphScope ∈ {Literal(i64), Variable{name, scope_id}}` (`executor.rs:~1970`). The graph constraint is **per-pattern**, attached to each `ScopedTriple`, OPTIONAL triple, and MINUS block. `GRAPH <iri>` resolves to `graph_id` at translate time (`_pgrdf_graphs.iri`; unresolved → `-1`, zero rows). `GRAPH ?g` INNER-JOINs `pgrdf._pgrdf_graphs g{scope_id}` and constrains in-scope aliases to that join's `graph_id`, **excluding `graph_id = 0`** (the default graph never binds `?g`, W3C §13.3). Distinct GRAPH blocks get distinct `scope_id`s; multiple blocks binding the same `?g` are tied by `g{a}.graph_id = g{b}.graph_id` (`scope_constraint_clauses_anchor_q`, `:5657`; `make_scope`, `:3316`). GRAPH composes with OPTIONAL/UNION/MINUS by scoping at the triple level; OPTIONAL/MINUS inside a GRAPH inherit the outer scope.

### 5.4 Property paths

All property-path SQL generation lives in `src/query/path.rs`; `executor.rs` only calls into it and threads the resulting `PathRelation` through the FROM/WHERE builder. `GraphPattern::Path{subject,path,object}` is classified by `classify_path` (`path.rs:378`) into a `PathPlan`:

- **`Triple`** (E1, non-recursive): bare predicate `p`, inverse `^p` (`Reverse(NamedNode)` ≡ swap subject/object), nested `^(^…)` (parity fold). Lowered to an ordinary `TriplePattern` and pushed like a BGP triple — `scoped_triple_from_path` (`executor.rs:2964`) returns `ScopedTriple{path:None}`.
- **`OneOrMore` (`+`)**, **`ZeroOrMore` (`*`)**, **`ZeroOrOne` (`?`)**, **`Alternation` (`|`)** — each carries a **predicate set** `Vec<NamedNode>` plus a `swapped` flag. Recursive/optional/alternation paths lower to a parenthesised derived relation exposing `subject_id`/`object_id` (+ `graph_id` for `GRAPH ?g`) — the same columns a quad alias exposes — so `build_from_and_where` substitutes it for the `_pgrdf_quads` alias and the existing var-binder joins it unchanged (`ScopedTriple.path = Some(PathRelation)`).

**Operators / executable surface** (`is_executable`, `path.rs:478`): `p`, `^p`, `^(^…)`; `p+`/`p*`/`p?` and their `^…` inverse compositions over a single predicate; alternation `a|b` (n-ary `a|b|c`), and the recursion-composed `(a|b)+`/`(a|b)*`/`(a|b)?`, and `^(a|b)` over plain uniform-direction predicates. The `|` set is lowered to `predicate_id IN (…)` (the LLD §7.2 "union of per-predicate scans" as one scan); a 1-element set `IN ($1)` is identical to the old `= $1`, so plain `p+` is unchanged.

**Recursive CTE.** `build_one_or_more_relation_sql` (`path.rs:666`) emits a self-contained `(WITH RECURSIVE walk(src,dst[,gid],depth) AS (… UNION ALL …) CYCLE src,dst SET is_cycle USING path SELECT DISTINCT … WHERE NOT is_cycle)`. **Cycle safety** uses Postgres's `CYCLE` clause (PG14+) — `UNION ALL` is required by `CYCLE`, and it stops extending a path the instant a `(src,dst)` pair repeats on that path, so a cycle terminates after one lap regardless of the depth cap (a bare `UNION` cannot, because the working tuple carries `depth`). `swapped` flips edge endpoints. `*` = the `+` walk `UNION` the W3C §9.3 zero-length node-set (`build_zero_or_more_relation_sql`, `:960`); `?` = direct edge `UNION` the same node-set, no recursion (`:1016`); top-level `|` = the non-reflexive single step, no recursion/no identity (`:1083`).

**Zero-length-path semantics (W3C §9.3)** (`zero_length_node_set_sql`, `:899`): a **bound** endpoint's self-pair `(x,x)` holds unconditionally (injected as a constant `SELECT $x,$x`; reflexive `*`/`?` **intern** the endpoint IRI so the projected var can reverse-resolve to its lexical form, `executor.rs:~3050`); an **unbound** endpoint's identity set is the DISTINCT union of `subject_id`∪`object_id` over the active scope, scoped per graph under `GRAPH ?g` (excluding graph 0).

**Depth guard.** GUC `pgrdf.path_max_depth` (`guc.rs:36`, default **64**, range **1..1024**, `Userset` per-session) is read once at translate time and baked into the recursive arm's `WHERE w.depth < $MAX`. It **truncates, never errors** (W3C §7.2). A per-`+`/`*` **truncation probe** (`probe_sql`, run post-execution in `execute`, `executor.rs:6549`) asks whether any non-cycle row sat at the depth cap with a still-continuable edge; if so it bumps `pgrdf.stats().path_depth_truncations`. The detector never under-counts and only benignly over-counts (permitted by §7.2). `?`/`|` carry an empty probe (no recursion); `collect_truncation_probes` (`:1935`) skips empties.

**Materialised-closure no-CTE fast path** (`executor.rs:3112`): for `+`/`*` over a **single well-known transitive predicate** (`rdfs:subClassOf`, `rdfs:subPropertyOf`, `owl:sameAs` — `is_well_known_transitive`, `:6422`), if `_pgrdf_quads` already carries `is_inferred = TRUE` rows for that predicate in the active scope (`inferred_closure_present`, `:6444`), the translator skips the recursive CTE and emits a **direct match** (`+` → the non-reflexive single step = `build_alternation_relation_sql`; `*` → that step ∪ identity = `build_zero_or_one_relation_sql`). The executed plan then has **no `CTE Scan`** (the §7.3 acceptance criterion, scraped via `pgrdf.sparql_sql` + `EXPLAIN`). Per-query detection, not cached. Multi-predicate `(a|b)+`/`(a|b)*` skip the fallback.

**Gated remainder (stable preview panics).** A recursive/alternation path whose inner box/arm is itself a sequence or recursive (`(p*)+`, `(p1/p2)?`, `(a/b|c)`, `(a+|b)`, mixed-direction `a|^b`) panics with `PANIC_ONE_OR_MORE_NESTED` (`path.rs:191`) — the §7.1-permitted gated stretch. Sequence paths `p1/p2` panic with `PANIC_SEQUENCE` (`:201`, "express as a multi-pattern BGP"). Negated property sets `!(…)` panic with `PANIC_NEGATED` (`:195`, out of v0.4 scope). Prefixes are stable so tooling can preview the rollout without depending on the tail.

### 5.5 UPDATE algebra

`execute_update(update: &Update)` (`executor.rs:6650`) is reached after `parse_query` fails and `parse_update` succeeds. It **acquires the partition-DDL gate first** (`acquire_partition_ddl_gate`) as the statement's outermost lock — before any dictionary internment / quad insert / `add_graph` write — to avoid an advisory-vs-data-lock deadlock cycle (a mixed default+named-graph INSERT DATA interns/inserts before reaching `add_graph`). It then walks `update.operations` and dispatches per `GraphUpdateOperation`:

- **`InsertData`** — per ground quad: `resolve_or_allocate_graph`, intern subject/predicate/object, `insert_quad`; counts `triples_inserted`.
- **`DeleteData`** — **set-semantic, lookup-only (no interning)**: a missing dictionary term ⇒ the quad cannot exist ⇒ spec-correct no-op; an unbound named-graph IRI ⇒ skip. Emits `DELETE … RETURNING` counting `triples_deleted`. `graphs_touched` records intent even on a no-op.
- **`DeleteInsert`** — triaged by which template halves are non-empty: `(true,true)` → `execute_delete_insert_where` (`:8083`, atomic DELETE-then-INSERT over one WHERE snapshot, W3C §3.1.3 ordering); `(true,false)` → `execute_delete_where` (`:7778`, lookup-only template, no-op on missing terms); `(false,true)` → `execute_insert_where` (`:7475`). A `WITH <iri>` prefix arrives as `using: Some(QueryDataset)`; `with_iri_from_using` (`:7032`) lifts it and `scope_pattern_to_graph` (`:7052`) wraps the WHERE pattern in `GraphPattern::Graph` so its BGP inherits the scope.
- **`Clear`** — `execute_clear` (`:7283`): `CLEAR DEFAULT/NAMED/ALL/<iri>` TRUNCATEs the partition but preserves the `_pgrdf_graphs` binding (W3C §3.1.3).
- **`Create`** — `execute_create` (`:7334`): allocates a partition+binding; idempotent unless already bound (errors without `SILENT`); touches no row counts.
- **`Drop`** — `execute_drop` (`:7365`); `DROP DEFAULT` routes to `clear_graph(0)` (the default partition is the catch-all). The dispatcher **captures the IRI before the drop** into `captured_graph_iris` because the `_pgrdf_graphs` binding is gone by summary time.
- **`Load`** — panics: out of v0.4 scope (LLD §14).

**Return shape.** A single summary row `{"_update":{form, triples_inserted, triples_deleted, graphs_touched, elapsed_ms}}`. `form` = the shared variant name across all ops, `"MIXED"` if they differ, `"EMPTY"` if none (`update_op_name`, `:6966`). `graphs_touched` is resolved back to IRIs (union of post-op lookup + pre-captured DROP IRIs), sorted.

### 5.6 CONSTRUCT / DESCRIBE

**CONSTRUCT** (`construct`, `executor.rs:375`). The WHERE pattern reuses the full SELECT machinery (`parse_select` + `build_from_and_where`), projecting the dict ids of every template variable. Per `(solution, template-triple)` it instantiates the template (`execute_construct_per_solution_path`, `:567`): constants flow through `encode_constant_*`, variables through `encode_dict_term` after one dict resolve, blank nodes mint fresh per-solution labels via a `BNodeMinter` (same template label within one solution ⇒ same fresh label; shared across the N template triples of that solution). Each output term is `{"type":"iri"|"literal"|"bnode","value":…,"datatype"?:…,"language"?:…}`. Predicate-position blank nodes panic (illegal RDF); empty templates reject with `pgrdf.construct: empty template`. DISTINCT/ORDER BY/GROUP BY/aggregates are rejected by `reject_construct_modifiers` (`:1215`) per W3C §16.2. The shorthand `CONSTRUCT WHERE { … }` is detected by `detect_construct_where_shorthand` (an ASCII probe shared with the parser).

**DESCRIBE** (`describe`, `executor.rs:1532`). spargebra normalises every DESCRIBE into `Project{inner,variables}` where each constant `DESCRIBE <iri>` is a leading `Extend{…, NamedNode(iri)}` layer wrapping the residual WHERE. `describe` peels the constant layers, binds variable-described terms over the residual WHERE (`collect_describe_var_bindings`, `:1711`), and emits the symmetric concise bounded description as triple rows via `describe_closure`/`closure_one_hop` (`:1813`/`:1863`). `pin_join_order()` is called here too. `translate` redirects a DESCRIBE passed to `pgrdf.sparql` with the stable panic `sparql: use pgrdf.describe(q) for DESCRIBE queries`.

### 5.7 Plan cache

`src/query/plan_cache.rs`. A **per-backend** `thread_local! RefCell<HashMap<String, OwnedPreparedStatement>>` keyed on the **parameterised SQL string verbatim** (collision-free by construction — same algebra shape ⇒ same SQL ⇒ same key). `OwnedPreparedStatement` is `SPI_keepplan`-promoted so it survives `SPI_connect`/`SPI_finish` cycles; backends are single-threaded so no locks are needed. `execute` (`executor.rs:6549`) checks `plan_cache::contains`; miss ⇒ `client.prepare(sql, &[INT8;n]).keep()` + `insert` + `record_miss`; hit ⇒ `record_hit`; then runs the plan with the per-call `i64` dict-id Datums (all `INT8OID`). Cumulative `HITS`/`MISSES`/`INSERTS` counters live in **shmem** (`PgAtomic`, registered in `init_in_postmaster`) and surface in `pgrdf.stats()`; `local_size` is per-backend. All counter writes are guarded by `shmem_cache::is_ready()`, so a lazy-loaded (non-preloaded) backend degrades to a no-op stats path instead of panicking. `pgrdf.plan_cache_clear() → BIGINT` drops this backend's plans.

### 5.8 Error / diagnostic contract

- Parse failure → `sparql: parse error: {query_err}` (query-side error preferred); `sparql_parse: {query_err}` for the parser UDF.
- Empty BGP → `sparql: empty BGP` (SELECT) / `sparql: ASK with empty BGP`.
- Untranslatable FILTER → `sparql: FILTER expression not translatable: {expr:?}`.
- DESCRIBE via `pgrdf.sparql` → redirect panic to `pgrdf.describe`; unsupported query form → `sparql: query form not supported yet`.
- Property-path gated/out-of-scope forms → the stable `PANIC_*` prefixes (§5.4).
- UPDATE `LOAD` → out-of-scope panic (LLD §14).
- Diagnostics: `pgrdf.sparql_sql(q)` returns the lowered SQL with `$N` inlined (safe — translate-time integers only) for `EXPLAIN`; `pgrdf.sparql_parse(q)` previews shape + `unsupported_algebra`; `pgrdf.stats()` exposes plan-cache hit/miss/insert/size and `path_depth_truncations`.

**Cross-cutting invariants:** (1) SQL strings never carry user IRI/literal strings — only `$N` placeholders bound to translate-time dict ids; (2) missing-constant → `-1` sentinel → zero rows, never an error; (3) the emitted join order is connected (no structural cross joins) and is pinned via `join_collapse_limit=1`; (4) `sparql_parse` is a faithful, infallible dry-run of `sparql`'s support set.



## 6. Inference engine

**Purpose.** Forward-chaining materialization: derive every entailed RDF triple from a graph's asserted (base) triples under a chosen reasoning profile, persist the derived triples back into the hexastore tagged `is_inferred = TRUE`, and refresh planner statistics so downstream queries over the entailed closure plan correctly. Citations in this section are to `src/inference/reasonable.rs` unless noted; `src/inference/mod.rs` is a one-line module declaration (`pub mod reasonable;`).

### 6.1 UDF signature

```sql
pgrdf.materialize(graph_id BIGINT, profile TEXT DEFAULT 'owl-rl') -> JSONB
```

Declared `#[pg_extern] #[search_path(pgrdf, pg_temp)]` as `fn materialize(graph_id: i64, profile: default!(String, "'owl-rl'")) -> pgrx::JsonB` (`reasonable.rs:111-113`). The bare `pgrdf.materialize(g)` form is byte-for-byte the v0.3/v0.4 surface (`profile => 'owl-rl'`), so it carries no regression.

### 6.2 Profiles (exactly two ship; one reserved)

The profile string is validated **up-front, before any side effect** (`reasonable.rs:120-126`):

- `'owl-rl'` (default) — full OWL 2 RL forward-chain via the `reasonable` crate (`styk-tv/reasonable` fork, branch `rdf12-passthrough`).
- `'rdfs'` — a **strict, sound, complete RDFS entailment-rule subset**, implemented natively in pgRDF (route 2), **not** a lossy post-filter of the OWL-RL output. See §6.5.
- Any other string — including the reserved-but-unimplemented `'owl-rl-ext'` — **panics** with the stable prefix `materialize: unknown profile` (exact: `materialize: unknown profile "<x>" (supported: 'owl-rl', 'rdfs')`). No silent fallback. The pgrx negative test `materialize_unknown_profile_errors` pins the full message (`reasonable.rs:997`).

Up-front validation is an invariant: an unknown profile must not perturb state (it must not reach the idempotency wipe of §6.3).

### 6.3 Control flow

1. **Validate profile** (`reasonable.rs:120-126`) — reject unknown strings before any side effect.
2. **Idempotency wipe** (`reasonable.rs:129-146`) — `DELETE FROM pgrdf._pgrdf_quads WHERE graph_id = $1 AND is_inferred = TRUE`, counting the dropped rows (`previous_inferred_dropped`).
3. **Load base triples** (`reasonable.rs:151`, fn `load_base_triples` 594-660) — one SPI scan of every `is_inferred = FALSE` quad in the graph, joining `_pgrdf_quads` to `_pgrdf_dictionary` four times (s, p, o, plus a LEFT JOIN for the object's datatype IRI), rehydrating each row into an `oxrdf::Triple`. A `HashSet<Triple>` of the base is built for the later set-diff.
4. **Reason under the profile** (`reasonable.rs:160-175`):
   - `'owl-rl'`: `Reasoner::new(); reasoner.load_triples(base.clone()); reasoner.reason();` then collect `reasoner.get_triples()` (base ∪ entailed) and `reasoner.errors()` (stringified into `reasoner_errors`).
   - `'rdfs'`: `rdfs_closure(&base)` — the native fixpoint (§6.5); `reasoner_errors` is always empty.
5. **Set-diff** (`reasonable.rs:179-181`) — keep only `derived` triples not in `base_set`; this is the inferred-only set.
6. **Write back, batched** (`reasonable.rs:198-302`, `WRITE_BATCH = 50_000`):
   - **Phase A** — resolve distinct datatype IRIs of typed-literal objects via `put_terms_batch` first (literals depend on their datatype's dict id).
   - **Phase B** — dedup every term instance to a distinct-term index, then resolve the distinct terms in chunks via `put_terms_batch` (the loader's bulk dict path). This replaced per-instance `put_term_full` calls, which were ~78% of the LUBM-50 materialize wall.
   - **Phase C** — emit quad rows in unnest-array INSERT batches via `flush_inferred` (`reasonable.rs:348-382`): `INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred) SELECT s, p, o, $4, TRUE FROM unnest($1::bigint[], $2::bigint[], $3::bigint[])`. Always `is_inferred = TRUE`.
7. **Auto-ANALYZE** (`reasonable.rs:313-319`) — if `written > 0` **and** the `pgrdf.auto_analyze` GUC (default on, via `crate::query::guc::auto_analyze()`) is set, run `ANALYZE pgrdf._pgrdf_quads`. The closure inflates join cardinalities; without fresh stats the planner mis-plans multi-pattern queries (cited: LUBM Q2 180 s → 1 s with ANALYZE). A no-op materialize (`written == 0`) leaves statistics untouched.

### 6.4 JSONB report shape

Returned `pgrx::JsonB` (`reasonable.rs:327-340`):

```json
{
  "base_triples":              <i64>,
  "inferred_triples_written":  <i64>,
  "previous_inferred_dropped": <i64>,
  "profile":                   "owl-rl" | "rdfs",
  "reasoner_errors":           [ "<string>", ... ],
  "auto_analyzed":             <bool>,
  "load_ms":                   <f64>,
  "reason_ms":                 <f64>,
  "diff_ms":                   <f64>,
  "write_ms":                  <f64>,
  "analyze_ms":                <f64>,
  "elapsed_ms":                <f64>
}
```

The phase timers (`load_ms`/`reason_ms`/`diff_ms`/`write_ms`/`analyze_ms`) are additive fields mirroring the loader's `parse_ms`/`dict_ms`/`insert_ms`; existing consumers key only on the original fields.

### 6.5 The native RDFS profile (route 2)

The `reasonable` fork exposes only a single fused datalog fixpoint (`reason()` / `reason_full()`, both the full OWL-RL rule set) — there is no upstream RDFS-only rule selection, so `'rdfs'` is implemented in-tree as `rdfs_closure` (`reasonable.rs:416-559`), a fixpoint over the base triples emitting the six **productive** RDFS rules (W3C RDF 1.1 Semantics §9.2.1):

- **rdfs11** — `subClassOf` transitivity
- **rdfs5** — `subPropertyOf` transitivity
- **rdfs7** — `subPropertyOf` application (`p ⊑ q ∧ s p o ⇒ s q o`)
- **rdfs9** — `subClassOf` application (`c ⊑ d ∧ s a c ⇒ s a d`)
- **rdfs2** — `rdfs:domain` (`p domain c ∧ s p o ⇒ s a c`)
- **rdfs3** — `rdfs:range` (`p range c ∧ s p o ⇒ o a c`, only when the object can be a type subject — IRI/bnode, never a literal)

Each round re-derives the schema relations (subclass/subprop/domain/range maps) so transitivity feeds the application rules on the next pass; iteration stops at the fixpoint (`closure.insert` returns no growth). The axiomatic reflexive-typing rules (rdfs1/4a/4b/6/8/10/12/13, e.g. universal `… rdf:type rdfs:Resource`) are **deliberately not emitted** — `reasonable`'s OWL-RL output does not emit them either, so emitting them on the `rdfs` side would *violate* the non-strict-subset invariant (`reasonable.rs:407-415`).

**Profile invariants** (locked by pgrx tests):
- `count(rdfs) ≤ count(owl-rl)` — RDFS ⊂ OWL-RL, non-strict subset (`materialize_rdfs_count_is_subset_of_owl_rl`).
- The two profiles agree on the RDFS-axiom entailments (subClassOf/subPropertyOf transitivity, domain/range propagation, type propagation).
- Idempotence: two consecutive calls yield the same `inferred_triples_written`; the second reports `previous_inferred_dropped == first.inferred_triples_written` (`materialize_is_idempotent`).
- Base triples survive every materialize (the wipe only touches `is_inferred = TRUE`).

### 6.6 Scope and current limitations

- **Scope.** `'owl-rl'` covers OWL 2 RL only (class/property hierarchies, inverse/symmetric/transitive properties, sameAs/functional/inverse-functional, domain/range). OWL 2 EL/QL and arbitrary Datalog are out of scope and not emulated. RDF-star object terms are out of scope — `key_obj` panics `materialize: unsupported object term (RDF-star out of scope)` (`reasonable.rs:254-255`).
- **Single-threaded by design (upstream #1).** The `reasonable` reasoner's `reason()` fixpoint is single-thread-bound; materialization does not parallelize across cores. This is the inference-side instance of the reasoner-wall limitation tracked as issue #1. At LUBM-100 scale the closure is ~8.58M inferred triples and the materialize wall is dominated by `reason_ms` (the single-thread fixpoint) plus the now-batched `write_ms`.
- **Malformed terms** are defensively skipped or routed to the `urn:pgrdf:invalid-iri` sentinel during base rehydration (`build_subject`/`build_object`, `reasonable.rs:662-708`).

---

## 7. Validation engine

**Purpose.** `pgrdf.validate` checks a data graph against a SHACL shapes graph and returns a W3C `sh:ValidationReport`-shaped JSONB. v0.6.14 ships **three** validation modes through a single dispatch `match`. Citations are to `src/validation/shacl.rs` and `src/validation/pgrdf_sparql.rs`; `src/validation/mod.rs` declares both submodules (`pub mod pgrdf_sparql; pub mod shacl;`).

> **Stale-comment trap (resolved against code).** The module doc-comment in `pgrdf_sparql.rs` says the `'pgrdf'` path is "unreachable from SQL … until TH-8", and the `shacl.rs` module doc still describes the v0.5 two-mode (`'native'`/`'sparql'`) world with an E-012 short-circuit. **Both are stale.** The authoritative truth is the live `match mode.as_str()` in `shacl.rs:190-208`, which has three reachable arms, and the panic message at `shacl.rs:204-207` which lists all three supported modes. The negative test `validate_unknown_mode_errors` (`shacl.rs:734-736`) pins `(supported: 'native', 'sparql', 'pgrdf')`, and `validate_pgrdf_mode_real_violation` / `validate_w3c_node_sparql_001_cross_mode` exercise the `'pgrdf'` arm end-to-end. CODE beats the comments.

### 7.1 UDF signature

```sql
pgrdf.validate(data_graph_id BIGINT, shapes_graph_id BIGINT, mode TEXT DEFAULT 'native') -> JSONB
```

Declared `#[pg_extern] #[search_path(pgrdf, pg_temp)]` as `fn validate(data_graph_id: i64, shapes_graph_id: i64, mode: default!(String, "'native'")) -> pgrx::JsonB` (`shacl.rs:168-174`). The bare 2-arg form defaults `mode => 'native'`.

### 7.2 The three-mode dispatch (ground truth)

Mode is validated up-front, before any work (`shacl.rs:190-208`). The exact `match mode.as_str()`:

| Mode | Binding / routing | Correctness status |
|---|---|---|
| `'native'` | `ShaclValidationMode::Native` → rudof in-process Rust Core engine | **Authoritative for SHACL Core.** W3C SHACL Core 25/25. The default. |
| `'sparql'` | `ShaclValidationMode::Sparql` → rudof's `SparqlEngine` | Reachable since E-012 closed in `shacl 0.3.2` (the old short-circuit guard is deleted), **but OPEN bug E-014**: returns *wrong* verdicts (`conforms=true` / 0 violations) on common SHACL-SPARQL topologies. |
| `'pgrdf'` | early-returns `run_pgrdf_sparql(data, shapes)` (then layers `elapsed_ms`) | **Authoritative SHACL-SPARQL gate.** Correct `conforms=false` where rudof is wrong. |
| any other | `panic!` | `validate: unknown mode "<x>" (supported: 'native', 'sparql', 'pgrdf')` — no silent fallback. |

The `'pgrdf'` arm **short-circuits before the rudof pipeline** (`shacl.rs:193-203`): it calls `crate::validation::pgrdf_sparql::run_pgrdf_sparql(...)`, inserts `elapsed_ms` into the returned object (so the meta-field shape stays comparable with the other two modes for benchmark-row diffs), and returns immediately — never serialising the data graph to N-Triples. For `'native'` / `'sparql'`, the chosen `ShaclValidationMode` is bound to `validation_mode` and the rudof pipeline (§7.3) runs.

### 7.3 The rudof path (`'native'` / `'sparql'`)

Pipeline (`shacl.rs:229-326`):

1. **Rehydrate** both graphs to N-Triples text via `serialise_graph_to_ntriples` (`shacl.rs:341-439`) — one SPI scan per graph over `_pgrdf_quads` JOIN `_pgrdf_dictionary`, fed through an `oxttl::NTriplesSerializer`. **It includes both `is_inferred = TRUE` and `FALSE` rows** (no `is_inferred` filter in its WHERE clause), so a data graph that has had `pgrdf.materialize` run is validated against its *entailed closure* — locked by `validate_materialised_graph_entailed` (`shacl.rs:876`).
2. **Parse** the data N-Triples into rudof's `InMemoryGraph::from_str(..., RDFFormat::NTriples, ...)`, then `Graph::try_from`.
3. **Compile** the shapes N-Triples to an `IRSchema` via `ShaclDataManager::load`.
4. **Validate**: `GraphValidation::new(data_graph).validate(&schema, &validation_mode)` returns a `ValidationReport`.
5. **Shape** the report into JSONB (`report_result_to_json`, `shacl.rs:444-469`).

Every failure step (data parse, data build, shapes compile, validate error) returns a structured report with `conforms: null`, empty `results`, and an `error` string — no panic reaches the SQL caller. Validation is fully in-process; no SPARQL endpoint or external store is contacted even in `'sparql'` mode.

**E-014 (open).** rudof's `'sparql'` engine ships `SparqlValidator` impls for only a subset of Core constraints and is incomplete on common SHACL-SPARQL shape topologies, returning `conforms=true` / 0 violations where the W3C `mf:result` says `conforms=false`. Documented and gated by `validate_w3c_node_sparql_001_cross_mode` and `lubm_shacl_sparql_dev_gate` (`shacl.rs:1080`, `1191`), which assert only that `'sparql'` returns a real Boolean — never that it is correct — while asserting that `'pgrdf'` on the same fixtures gets the W3C-correct verdict (node-sparql-001 → `conforms=false`; LUBM → exactly 4 violations).

### 7.4 The pgRDF-native path (`'pgrdf'`)

Entry point `run_pgrdf_sparql(data_graph_id, shapes_graph_id) -> serde_json::Value` (`pgrdf_sparql.rs:run_pgrdf_sparql`). It evaluates `sh:sparql [ sh:select … ]` (SHACL Part-2 `IRComponent::BasicSparql`) constraints **directly against the hexastore**, avoiding the N-Triples serialise/rehydrate of the rudof path. Only the data graph's *shapes* are rehydrated to N-Triples (to compile the `IRSchema`); the data graph itself is never serialised — it is hit through the dictionary-indexed hexastore.

Control flow:

1. **Compile shapes** — `serialise_graph_to_ntriples(shapes_graph_id)` (reused, `pub(crate)`) → `ShaclDataManager::load` → `IRSchema`. On compile failure, return a report with `conforms: null` and an `error`.
2. **Walk schema** — `walk_schema_for_sparql(&schema) -> Vec<(IRShape, BasicSparql)>`: iterate every shape via `IRSchema::iter()` (node + property shapes), skip deactivated shapes (`shape.deactivated()`) and deactivated SPARQL constraints (`sparql.deactivated() == Some(true)`), clone out owned `(shape, sparql)` pairs (so no schema borrow is held across the SPI loop). Deterministic insertion order.
3. **Per (shape, sparql)** — resolve focus nodes, then for each focus node substitute `$this`, run the query, and map each binding row to a violation.

**Focus-node resolution** — `resolve_focus_nodes(targets, data_graph_id) -> Vec<String>` handles the five well-formed SHACL §5.1 target forms; output is `sort()` + `dedup()` for determinism:
- `Target::Node(Iri)` — the named term itself.
- `Target::Class(Iri)` / `Target::ImplicitClass(Iri)` — subjects of `?s rdf:type <class>` via `spi_class_targets` (**direct typing only**, no `rdfs:subClassOf` closure — users wanting transitivity run `pgrdf.materialize` first).
- `Target::SubjectsOf(pred)` — distinct subjects of `?s <pred> ?o` via `spi_subjects_of`.
- `Target::ObjectsOf(pred)` — distinct IRI objects of `?s <pred> ?o` via `spi_objects_of` (literals/blanks skipped, SHACL §5.5).
- `Wrong*` (ill-formed, §5.6) — skipped (no focus).

Each `spi_*` helper is a parameterised SPI scan over `_pgrdf_quads` joined to `_pgrdf_dictionary` filtered on `term_type = 1` (URI) and `graph_id = $1`, single-quote-escaped via `esc()` as belt-and-braces SQL hygiene (`pgrdf_sparql.rs`, the SPI-helpers block).

**`$this` substitution** — `substitute_this(sparql, focus_iri) -> String`. Naive `SELECT <iri>` is invalid SPARQL (projection accepts variables only), so the rewrite:
1. replaces every `$this` with the synthetic variable `?_pgrdf_this`;
2. injects `VALUES ?_pgrdf_this { <focus_iri> }` immediately after the first (case-insensitive) `WHERE {` via `inject_values_at_first_where`.
If no `WHERE` is found, the variable-rewritten text passes through and the downstream parser surfaces the real grammar error. (MVP-level lexical replacement; does not yet skip `$this` inside string literals — noted as a future TH-7 refinement.)

**Query dispatch** — `call_pgrdf_sparql_spi(query)` runs `SELECT * FROM pgrdf.sparql('<escaped>')` — the same dictionary-indexed hexastore path that powers `pgrdf.sparql` / `pgrdf.construct` — and collects each binding row as `JsonB`. A SPARQL parse error becomes a single synthetic `{"_error": "..."}` row rather than crashing the whole `validate()` call. **Every returned binding row is a violation** (the constraint's result-set being non-empty is the violation signal).

**Violation mapping** — `build_violation(shape, sparql, focus_iri, row)` emits a `sh:ValidationResult` (`pgrdf_sparql.rs`): `focusNode` (from the substitution context), `resultPath` (the property shape's predicate path, else null), `sourceShape` (the shape IRI or `_:blank`), `resultMessage` (the SPARQL constraint's `sh:message`, else null), `resultSeverity` (from `shape.severity()`, default `sh:Violation`), `value` (the row's `?value` binding lifted opaquely, else null), and a fixed `sourceConstraintComponent = sh:SPARQLConstraintComponent`.

### 7.5 Validation report JSONB shape

`'native'` / `'sparql'` return (`shacl.rs:317-326`):

```json
{
  "conforms":        <bool> | null,
  "results":         [ ValidationResult, ... ],
  "data_graph_id":   <i64>,
  "shapes_graph_id": <i64>,
  "data_triples":    <i64>,
  "shapes_triples":  <i64>,
  "mode":            "native" | "sparql",
  "elapsed_ms":      <f64>,
  "error":           "<present only on a failure branch>"
}
```

`'pgrdf'` returns the same envelope **minus `data_triples`** (the native path never rehydrates the data graph, so the count would be a pointless extra scan) **plus** `mode: "pgrdf"` and the `elapsed_ms` layered on by the dispatcher (`pgrdf_sparql.rs` `run_pgrdf_sparql`; `shacl.rs:196-201`).

Each `ValidationResult` (uniform across all three modes):

```json
{
  "focusNode":                  "<iri | _:bnode | literal-encoded>",
  "resultPath":                 "<iri | null>",
  "sourceShape":                "<iri | _:bnode | null>",
  "resultMessage":              "<string | null>",
  "resultSeverity":             "sh:Violation | sh:Warning | sh:Info | sh:Debug | sh:Trace | <iri>",
  "value":                      "<term-encoded | null>",
  "sourceConstraintComponent":  "<iri>"
}
```

For the rudof path, severity normalises to the canonical `sh:` constants via `encode_severity`, literals render Turtle-ish via `format_literal`, and RDF-star object nesting renders the stable placeholder `<rdf-star-triple>` (`shacl.rs:476-530`).

### 7.6 Why `'pgrdf'` is authoritative, and current limitations

- **Authoritative SHACL-SPARQL gate.** On the W3C `node-sparql-001` fixture and the LUBM "course taught by at most one Professor" dev-gate, `'pgrdf'` returns the W3C-correct `conforms=false` (3 and 4 violations respectively) where `'sparql'` (rudof) returns the wrong answer under **E-014**. The cross-mode tests assert the W3C verdict *only* for `'pgrdf'`.
- **Scalability.** `'sparql'` (rudof) scales with `InMemoryGraph` — it rehydrates the entire data graph as N-Triples text plus a parallel in-memory triple copy (hundreds of MB for a 10⁷-triple graph). `'pgrdf'` runs every constraint through the hexastore directly: O(1) per-focus-node dictionary lookup, planner-usable indexes, prepared-plan cache reuse across the focus iteration.
- **Limitations.** `'pgrdf'` only intercepts `BasicSparql` constraints — Core constraints on a `'pgrdf'`-mode call are not evaluated (a Core-only shape conforms vacuously, per `validate_pgrdf_mode_empty_when_no_sparql_constraint`). `sh:targetClass` resolution is direct-typing only (no subclass closure unless materialized). `$this` substitution is lexical (string-literal `$this` not yet excluded). The pgRDF SPARQL executor does not yet translate `FILTER NOT EXISTS` / `Not(Exists(_))` (Track A), so the natural "must HAVE property" SHACL idiom is expressed inverted in current fixtures. `'sparql'`-mode correctness is gated by upstream rudof (**E-014**, open). `'native'` remains the SHACL Core authority (25/25).

---



## 8. Extension surface

pgRDF installs into the dedicated `pgrdf` schema and exposes its entire user-facing API as `#[pg_extern]` UDFs. Every UDF is enumerated below from `src/lib.rs` (entry/wiring) and the per-subsystem modules; signatures are the SQL-visible shapes (Rust `i64→BIGINT`, `i32→INT`, `&str/String→TEXT`, `pgrx::JsonB→JSONB`, `bool→BOOLEAN`, `SetOfIterator<JsonB>→SETOF JSONB`).

### Install / control model

- **Control file** (`pgrdf.control`): `default_version = '0.6.14'`, `module_pathname = '$libdir/pgrdf'`, `schema = 'pgrdf'`, `relocatable = false`, `superuser = true`, `trusted = false`. **No `requires =` line** — pgRDF has zero external extension dependencies; absence (not an empty list) is the truthful declaration (`pgrdf.control` header comment).
- **Schema DDL** is shipped in-tree via `extension_sql_file!` in `src/lib.rs`, ordered by `requires`: `sql/schema_v0_2_0.sql` (baseline hexastore + dictionary) → `sql/schema_v0_4_0_graphs.sql` (the `_pgrdf_graphs` IRI↔graph_id map, `requires = ["schema_v0_2_0"]`).
- **Staged-loader procedure**: `extension_sql!` defines `CREATE PROCEDURE pgrdf.load_turtle_staged(path TEXT, graph_id BIGINT, n_workers INT DEFAULT 0)`, a thin PL/pgSQL `CALL` wrapper over the coordinator FUNCTION `load_turtle_staged_run`, `requires = [storage::staged::pool::load_turtle_staged_run]` (`src/lib.rs`).
- **Process init** (`_PG_init`, `src/lib.rs`): always calls `query::guc::register()` (custom GUCs registered in BOTH the postmaster shared-preload path AND lazy backend-load path); when `process_shared_preload_libraries_in_progress` (postmaster only) it additionally registers the three shmem hooks: `storage::shmem_cache::init_in_postmaster()`, `query::plan_cache::init_in_postmaster()`, `storage::staged::jobctl::init_in_postmaster()`. Production deployment requires `shared_preload_libraries='pgrdf'`.
- **GUCs** (all `GucContext::Userset`, `src/query/guc.rs`): `pgrdf.path_max_depth`, `pgrdf.ingest_dict_path`, `pgrdf.dict_batch_size`, `pgrdf.shmem_prewarm_on_init`, `pgrdf.auto_analyze`, `pgrdf.staged_temp_tablespaces`, `pgrdf.staged_resolve_strategy`, `pgrdf.bulk_defer_index_min`.

### Meta / version
- `pgrdf.version() -> TEXT` — returns `env!("CARGO_PKG_VERSION")` (i.e. `0.6.14`); the install-verification smoke surface `SELECT pgrdf.version();` (`src/lib.rs`).

### Storage — dictionary (`src/storage/dict.rs`)
- `pgrdf.put_term(value TEXT, term_type SMALLINT) -> BIGINT` — intern a term, returns its dict id (dedups). The SQL surface over the internal `put_term_full`.
- `pgrdf.get_term(id BIGINT) -> TEXT` — reverse lookup; NULL if absent.

### Storage — hexastore (`src/storage/hexastore.rs`)
- `pgrdf.put_quad(s BIGINT, p BIGINT, o BIGINT, g BIGINT DEFAULT 0)` — insert one quad by dict ids.
- `pgrdf.count_quads(g BIGINT DEFAULT 0) -> BIGINT` — row count in a graph.
- `pgrdf.add_graph(g BIGINT) -> BOOLEAN` — register a graph id + LIST partition.
- `pgrdf.add_graph(iri TEXT) -> BIGINT` — auto-allocate a graph id for an IRI (overloaded via `name = "add_graph"`).
- `pgrdf.add_graph(id BIGINT, iri TEXT) -> BIGINT` — bind a specific id to an IRI (overloaded).

### Storage — graphs / lifecycle (`src/storage/graphs.rs`)
- `pgrdf.graph_id(iri TEXT) -> BIGINT` *(strict)* — IRI→id, NULL if unbound.
- `pgrdf.graph_iri(id BIGINT) -> TEXT` *(strict)* — id→IRI.
- `pgrdf.drop_graph(id BIGINT, cascade BOOLEAN DEFAULT true) -> BIGINT` and overload `pgrdf.drop_graph(iri TEXT, cascade BOOLEAN DEFAULT true) -> BIGINT`.
- `pgrdf.clear_graph(id BIGINT) -> BIGINT` and overload `pgrdf.clear_graph(iri TEXT) -> BIGINT`.
- `pgrdf.copy_graph(src BIGINT, dst BIGINT) -> BIGINT` and overload `pgrdf.copy_graph(src_iri TEXT, dst_iri TEXT) -> BIGINT`.
- `pgrdf.move_graph(src BIGINT, dst BIGINT) -> BIGINT` and overload `pgrdf.move_graph(src_iri TEXT, dst_iri TEXT) -> BIGINT`.

### Storage — loaders (`src/storage/loader.rs`)
- `pgrdf.load_turtle(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, bulk_load BOOLEAN DEFAULT FALSE) -> BIGINT` — the default ingest entrypoint; v0.6.14 format-aware dispatch prefers the native staged loader **only** when the pool is ready (preloaded), the file confidently sniffs as N-Triples, and no `base_iri` is set — any miss falls to the full Turtle parser (Turtle input emits a notice, never silent skip).
- `pgrdf.load_turtle_verbose(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, bulk_load BOOLEAN DEFAULT FALSE) -> JSONB` — same, with phase-breakdown stats.
- `pgrdf.load_turtle_streaming(path TEXT, graph_id BIGINT, window_triples INT DEFAULT 20000000, id_reserve_block INT DEFAULT 1000000, base_iri TEXT DEFAULT NULL) -> JSONB` — windowed streaming bulk loader.
- `pgrdf.parse_turtle(content TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) -> BIGINT` — in-memory string source.
- `pgrdf.parse_turtle_verbose(content TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL) -> JSONB`.
- `pgrdf.parse_turtle_dict_batched(content TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, dict_batch_size INT DEFAULT 500) -> JSONB`.
- `pgrdf.load_turtle_dict_batched(path TEXT, graph_id BIGINT, base_iri TEXT DEFAULT NULL, dict_batch_size INT DEFAULT 500) -> JSONB`.
- `pgrdf.parse_trig(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) -> JSONB` — TriG with inline `GRAPH` blocks; `strict => TRUE` rejects unknown graph IRIs (`parse_trig: unknown graph iri <iri>`) with no partial rows.
- `pgrdf.parse_nquads(content TEXT, default_graph_id BIGINT DEFAULT 0, strict BOOLEAN DEFAULT FALSE) -> JSONB` — N-Quads; same `strict` contract (`parse_nquads: unknown graph iri <iri>`).

### Storage — staged loader (`src/storage/staged/pool.rs`)
- `pgrdf.load_turtle_staged_run(path TEXT, graph_id BIGINT, n_workers INT DEFAULT 0) -> JSONB` — the multi-backend bgworker coordinator (spawn/wait/gate; workers own per-phase commits). Wrapped by the `CALL pgrdf.load_turtle_staged(...)` procedure (above).
- `pgrdf.load_turtle_staged_ping(n_workers INT) -> JSONB` — pool-liveness probe (self-deadlock-avoiding ping table).

### Storage — CONSTRUCT ingest (`src/storage/construct_ingest.rs`)
- `pgrdf.put_construct_row(row JSONB, graph_id BIGINT DEFAULT 0) -> BIGINT` — ingest one CONSTRUCT-result row.
- `pgrdf.put_construct_rows(rows JSONB[], graph_id BIGINT DEFAULT 0) -> BIGINT` — batch ingest.

### Storage — stats / shmem (`src/storage/stats.rs`)
- `pgrdf.stats() -> JSONB` — dictionary/cache counters.
- `pgrdf.shmem_reset()` — invalidate shmem dict cache slots.
- `pgrdf.shmem_cache_prewarm(limit BIGINT DEFAULT 100000) -> BIGINT` — preload the shmem dict cache.

### Storage — TA spikes (`src/storage/loader_ta11.rs`)
- `pgrdf.spike_ta11_batch_sweep(...)`, `pgrdf.spike_ta10_logged_flat(...)`, `pgrdf.spike_ta10_logged_indexed(...)`, `pgrdf.spike_ta10_logged_partitioned(...)` — internal throughput-spike harnesses (benchmark instrumentation, not part of the stable query API).

### Query — parser & cache (`src/query/parser.rs`, `src/query/plan_cache.rs`)
- `pgrdf.sparql_parse(query TEXT) -> JSONB` — parse + analyse a SPARQL query/update to structured JSON (flags unsupported algebra; syntax errors panic).
- `pgrdf.plan_cache_clear() -> BIGINT` — flush the prepared-statement plan cache, returns count cleared.

### Query — executor (`src/query/executor.rs`)
- `pgrdf.sparql(query TEXT) -> SETOF JSONB` — execute a SPARQL SELECT/ASK (one JSONB solution per row).
- `pgrdf.sparql_sql(query TEXT) -> TEXT` — return the translated SQL (no execution; introspection).
- `pgrdf.construct(query TEXT) -> SETOF JSONB` — execute CONSTRUCT, returning result triples as JSONB.
- `pgrdf.describe(query TEXT) -> SETOF JSONB` — execute DESCRIBE (bounded-description closure).

### Inference (`src/inference/reasonable.rs`)
- `pgrdf.materialize(graph_id BIGINT, profile TEXT DEFAULT 'owl-rl') -> JSONB` — materialize entailed triples via `reasonable`. Profiles: `'owl-rl'` (full OWL 2 RL) and `'rdfs'` (RDFS subset); idempotent (wipes prior `is_inferred` rows first). Any other profile panics `materialize: unknown profile` before any side effect.

### Validation (`src/validation/shacl.rs`)
- `pgrdf.validate(data_graph_id BIGINT, shapes_graph_id BIGINT, mode TEXT DEFAULT 'native') -> JSONB` — SHACL validation, returning a W3C `sh:ValidationReport`-shaped payload. **Ships three modes** (`src/validation/shacl.rs` dispatch): `'native'` (in-process Rust SHACL Core engine, default), `'sparql'` (rudof `shacl 0.3.x` SparqlEngine), `'pgrdf'` (pgRDF-native SHACL-SPARQL handler, short-circuits before the rudof pipeline). Unknown mode panics `validate: unknown mode` — never a silent fallback. *(The function's older doc-comment still describes `'sparql'` as an E-012-era stub; that comment is stale — the dispatch code routes all three modes through real engines. CODE is authoritative.)*

---

## 9. Invariants & error contract

Cross-cutting invariants, with the stable error-message prefixes the negative tests pin (all from the shipped sources cited):

1. **Unknown enum-arg ⇒ panic, never silent fallback.** Every mode/profile argument is validated up-front, *before* any side effect:
   - `materialize: unknown profile <x> (supported: 'owl-rl', 'rdfs')` — `src/inference/reasonable.rs` (validated before the idempotency wipe, so a bad profile cannot perturb state).
   - `validate: unknown mode <x> (supported: 'native', 'sparql', 'pgrdf')` — `src/validation/shacl.rs` (validated before any rehydration work).
2. **Strict graph resolution leaves no partial rows.** Under `strict => TRUE`, `parse_trig`/`parse_nquads` reject an unbound graph IRI (`parse_trig: unknown graph iri <iri>` / `parse_nquads: unknown graph iri <iri>`) *before* any quad for that IRI is buffered (`src/storage/loader.rs` doc-contracts). Non-strict default auto-allocates via the v0.4 §3.2 `add_graph(iri)` path.
3. **No silent data loss on format mismatch.** `load_turtle`'s v0.6.14 staged-fast-path dispatch (`src/storage/loader.rs`) routes to the native staged loader **only** on a confident N-Triples sniff with the pool ready and no `base_iri`; a Turtle file emits a NOTICE and uses the full parser — it is never fed to the N-Triples-only STAGE phase where it would be silently skipped. A missing/unreadable file emits no notice and falls through so `File::open` surfaces the real `load_turtle: failed to open <path>: <err>`.
4. **`pgrdf.validate` mode is authoritative, not advisory.** Per E-014 (below), `'pgrdf'` is the authoritative SHACL-SPARQL conformance gate; `'sparql'` (rudof) is downgraded to a pgRDF-side contract assertion (returns a real Boolean, dispatch reaches the engine) but is **not** trusted for W3C verdicts on the SPARQL fixtures.
5. **Dictionary full-identity keying.** Terms are keyed on the full term identity (type discriminant + lexical value + datatype id + language tag), not lexical value alone — `put_term_full` / `put_terms_batch` take the `(i16, String, Option<i64>, Option<String>)` key tuple (`src/storage/dict.rs`); this is what makes `put_term` dedup correctly across typed/lang literals (the staged-loader literal-dedup full-key fix, MEMORY v0.6.12).
6. **shmem registration is postmaster-only.** The three shmem caches (`shmem_cache`, `plan_cache`, `staged::jobctl`) register hooks *only* when `_PG_init` runs in the postmaster (`src/lib.rs`); GUCs register on both paths. Production therefore requires `shared_preload_libraries='pgrdf'`.
7. **Version identity invariant.** `pgrdf.version()` returns `CARGO_PKG_VERSION`; `pg_extension.extversion` must equal the git tag. Four sources of truth (`Cargo.toml`, `pgrdf.control` `default_version`, `compose/compose.yml` mount, the tag) must align, enforced by CI Gates 0–3 (`PROVENANCE.md` Rule 7).

*(The "quads == triples gate" referenced in the prompt is the loader/stats consistency check that ingested quad rows equal parsed triples; the loaders return per-phase JSONB stats — triples, quad_batches, dict/resolve/insert breakdown — via `stats_to_jsonb`/`quad_stats_to_jsonb`, `src/storage/loader.rs`, which is the surface that exposes any divergence.)*

---

## 10. Test architecture

The test bar (authoritative v0.6.14 figures; verified against the tree where readable):

| Tier | Location | Count | What it gates |
|---|---|---|---|
| pgrx integration (`#[pg_test]`) | in-source `mod tests` across `src/**` + `tests/pgrx/basic_install.rs` | **294** | In-process correctness per PG major: UDF behaviour, negative-message pins (`materialize: unknown profile`, `validate: unknown mode`, syntax-error panics), cross-mode validate (`validate_w3c_node_sparql_001_cross_mode`), plan-cache hit/miss, dict dedup/roundtrip. |
| pg_regress | `tests/regression/` (`sql/` + `expected/`, `run.sh`) | **93** `.sql` ↔ **93** `.out` *(verified in-tree)* | SQL-surface golden output: install smoke, dict roundtrip, CONSTRUCT/SPARQL feature goldens. Byte-exact diff. |
| W3C SPARQL | `tests/w3c-sparql/` (scenario dirs `01-…`..) | **51** scenarios *(verified: 51 dirs/`.rq`)* | SPARQL conformance shapes: BGP, DISTINCT, UNION, OPTIONAL, MINUS, FILTER, aggregates, ORDER BY, LIMIT/OFFSET, etc. |
| W3C SHACL Core | `tests/w3c-shacl/fixtures/core/` (`run.sh` default / `--sparql` / `--pgrdf`) | **25 / 25** Core fixtures *(verified: 50 `.ttl` = 25 data+shapes pairs; README claims 25/25 full-pass)* | **Hard gate**: `sh:conforms` full-pass via `pgrdf.validate(g,g)` ('native'). `prop-nodeKind-001` is graded in-suite (no excluded Core fixture). `--sparql` sub-run = pgRDF-side contract assertion only (per E-014); `--pgrdf` = the authoritative SHACL-SPARQL gate. |
| Perf / LUBM | `tests/perf/lubm*` | **3** (LUBM-1 / -10 / -100 baselines; LUBM-100 full-pass recorded) | Performance regression baselines (`baseline.lubm-*.json`), TA-spike harnesses, join-order RESULTS; LUBM-100-PASSES.md is the correctness anchor at scale. |

The W3C-SHACL harness is three-mode (`tests/w3c-shacl/run.sh`, TH-7): default native (real W3C Core gate), `--sparql` (rudof SparqlEngine, asserts dispatch only), `--pgrdf` (authoritative SHACL-SPARQL conformance).

**Note on the pgrx count:** the prompt's authoritative bar is **294**; counting `#[pg_test]` over `origin/main` source from this (shared, possibly-AHEAD) working tree yielded **257**, so I could not independently confirm 294 against `origin/main` here. The 93 / 51 / 25 / 3 figures are confirmed against the tree.

---

## 11. Current limitations & errata

Cited from `specs/ERRATA.v0.6.md` and its live cross-links (`ERRATA.v0.4.md` E-011, `ERRATA.v0.2.md` E-006). Resolved this era: **E-012** (SHACL-SPARQL upstream stub — RESOLVED 2026-05-28 by `shacl 0.3.2` + guard deletion, `ERRATA.v0.5.md`) and **E-013** (SHACL `prop-nodeKind-001` — RESOLVED in v0.5.0, 25/25 retained, `ERRATA.v0.6.md`).

### E-014 — `shacl 0.3.2` SparqlEngine returns wrong verdicts (OPEN)
- **What:** rudof's `BasicSparqlValidator::validate_sparql` (`shacl 0.3.2`) returns `conforms=true`/0 violations on W3C fixture `sparql-001.ttl` where `mf:result` asserts `conforms=false`/3 violations. The `IRComponent::BasicSparql` constraint compiles correctly (verified via `pgrdf_sparql::th11_walk_schema_unit_tests`); the bug is in the engine's per-focus-node evaluation, not parsing (`specs/ERRATA.v0.6.md`).
- **Status:** **OPEN — upstream-gated.** Disposition: **pgRDF beats rudof.** `pgrdf.validate(g, g, 'pgrdf')` correctly returns `conforms=false` with 3 `sh:Violation` results (each `sourceConstraintComponent = sh:SPARQLConstraintComponent`); `'pgrdf'` mode is promoted to the **authoritative SHACL-SPARQL gate** (`tests/w3c-shacl/run.sh --pgrdf`), `'sparql'` downgraded to a contract assertion. Locked by `validate_w3c_node_sparql_001_cross_mode`.
- **Trigger to close:** any subsequent `shacl 0.3.x`/`0.4.x` release whose `--sparql` verdict matches W3C `mf:result` — then the `--sparql` sub-run re-tightens to the same conformance gate as `--pgrdf`.

### E-011 — upstream `reasonable` patch for RDF 1.2 coexistence (OPEN)
- **What:** `reasonable 0.4.1` doesn't handle `oxrdf`'s `rdf-12` `TermRef::Triple(_)` variant (hard-enabled by `rudof_rdf 0.3.1`); resolves the remaining `rdf-12 / TermRef::Triple` half of E-009 (`ERRATA.v0.4.md`).
- **Status:** **OPEN — verified locally, upstream PR open.** pgRDF carries the patched fork via `[patch.crates-io]` through the v0.6 cycle (`ERRATA.v0.6.md`: "Unchanged"). Fork: `styk-tv/reasonable@rdf12-passthrough`.
- **Trigger to close:** merge of upstream PR `gtfierro/reasonable#50`; then the `[patch.crates-io]` pin is dropped.

### E-006 — pgrx 0.18 / PG 18 migration deferred (OPEN)
- **What:** pgRDF is pinned to **pgrx 0.16.x** (PG 14–17). pgrx 0.18.0's `impl_table_iter` `E0716` macro errors, the breaking `pgrx_embed`-removal / `crate-type` / manual-`SqlTranslatable` migration, and symbol-leak bug #2281 all block the bump; pgRDF still ships `src/bin/pgrx_embed.rs` + `crate-type = ["cdylib","lib"]` (`ERRATA.v0.2.md`).
- **Status:** **OPEN — largest deferred upstream item carried into v0.6** (`ERRATA.v0.6.md`: "Unchanged"). Compose pins `postgres:17.4-bookworm`; the PG 18 `extension_control_path` GUC drop-in path (E-007) is blocked behind this.
- **Trigger to close:** a pgrx publish > 0.18.0 **or** an `E0716` fix landing in `develop`, with bandwidth for the `pgrx_embed`-removal + `SqlTranslatable` audit. Then compose targets PG 18 + the GUC path.

### #1 — single-threaded reasoning (architectural constraint)
- **What:** OWL 2 RL / RDFS materialization (`pgrdf.materialize`, `src/inference/reasonable.rs`) runs single-threaded through `reasonable` — the reasoner is the load-base-triples → closure → flush path in one backend, not parallelized across workers (the staged multi-worker pool is ingest-only). This is the reasoning "wall" tracked as issue **#1** (reasonable#57; MEMORY v0.6-cycle-state).
- **Status:** OPEN, by-design for the v0.6 line — a v0.7+ concern, not a v0.6.14 deliverable.
- **Trigger to close:** a parallel/partitioned reasoning design (out of scope for the v0.6 bulk-ingest cadence).


## 12. Versioning & provenance

### 12.1 Version identity

`pgrdf.version()` returns `env!("CARGO_PKG_VERSION")`. Four sources of
truth must agree for a release, enforced by CI before a tag is honoured:

1. `Cargo.toml` `version`
2. `pgrdf.control` `default_version` (= `0.6.14`)
3. the annotated git tag (`v0.6.14`)
4. the `compose/compose.yml` per-file mount (`pgrdf--0.6.14.sql`)

`pg_extension.extversion` after `CREATE EXTENSION` must equal the tag.

### 12.2 Release & attestation flow

The single publishable artifact is the **OCI bundle**, built and signed
only by CI — never locally:

```
tag v0.6.14  ──▶ release.yml      per-PG tarballs (pg14–17 × amd64/arm64) + SLSA Build Provenance v1
             ──▶ oci-publish.yml  ghcr.io/styk-tv/pgrdf-bundle:0.6.14 (amd64+arm64), each digest attested
             ──▶ update-latest-md  LATEST.md refreshed after every digest's attestation verifies
```

Consumption is anonymous and verifiable:

```sh
oras pull ghcr.io/styk-tv/pgrdf-bundle:0.6.14-pg17-<arch>
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.6.14 --repo styk-tv/pgRDF   # exit 0
```

The bundle drops `lib/pgrdf.so` + `share/extension/{pgrdf.control,
pgrdf--0.6.14.sql}` next to a stock `postgres:17` install. v0.5.0–v0.5.9
predate the attestation wiring and are not published this way.

### 12.3 Release cadence

The v0.6.x line ships as attested micro-releases (one version = one
commit, never re-cut). The bulk-ingest + post-benchmark threading work
(T1–T6) closed in v0.6.14. **v0.7.0 is the graduation gated on the graph
-carving line (C1–C6)** shipping as further v0.6.n micro-releases — see
the roadmap. This document is re-issued per release that changes the
low-level design.


