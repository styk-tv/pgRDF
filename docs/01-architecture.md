# 01 — Architecture

pgRDF is a PostgreSQL extension built with `pgrx`. It runs as a `.so`
loaded into the standard Postgres backend (no sidecar, no external
process). It provides four engines, each a Rust module under `src/`:

```
                           ┌───────────────────────────────────┐
                           │     PostgreSQL backend (one        │
                           │     per connection)                │
                           │                                    │
   psql / clients ────────►│   parser ▶ planner ▶ executor      │
                           │                  │                 │
                           │                  ▼                 │
                           │   pgrdf.so (one image per backend):│
                           │     ┌─ query     (SPARQL → SQL)   │
                           │     ├─ storage   (dict + hexastore)│
                           │     ├─ inference (OWL 2 RL)        │
                           │     └─ validation(SHACL)           │
                           │                  │                 │
                           │                  ▼                 │
                           │   shared memory  ◀── shmem dict    │
                           │     cache (LLD §4.1, shipped       │
                           │     Phase 3 step 1) + plan-cache    │
                           │     counters (§4.2)                 │
                           └───────────────────────────────────┘
                                            │
                                            ▼
                              _pgrdf_dictionary (BIGINT IDs)
                              _pgrdf_quads (partitioned by graph_id)
                              hexastore covering indexes (SPO/POS/OSP …)
```

## The four engines (mapping to source modules)

| Engine | Module | Authoritative spec | Status |
|---|---|---|---|
| Storage | `src/storage/{dict,hexastore,loader,shmem_cache,stats}.rs` | LLD §3, §4.1, §4.3 | ✅ schema + CRUD + Turtle/TriG/N-Quads ingest; ✅ shmem dict cache (§4.1); ✅ bulk-INSERT plan reuse (§4.3 phase A); ✅ parallel bulk loader (`bulk_load => true`, v0.6.2–v0.6.6); ⏳ heap_multi_insert / COPY BINARY quad insert (§4.3 phase B) |
| Query | `src/query/{parser,executor,plan_cache}.rs` | LLD §4.2 | ✅ SPARQL 1.1 SELECT/ASK/CONSTRUCT/DESCRIBE + UPDATE; BGP + FILTER + OPTIONAL + UNION + MINUS + VALUES + BIND + property paths + named graphs (GRAPH) + aggregates (incl. type-aware MIN/MAX) + HAVING (alias + inline) + type-aware ORDER BY + solution modifiers; ✅ prepared-plan cache (§4.2) — fully parameterised SQL, per-backend `OwnedPreparedStatement` cache |
| Inference | `src/inference/reasonable.rs` | LLD §2; ERRATA E-002 | ✅ `pgrdf.materialize` via `reasonable` (OWL 2 RL forward chain, idempotent re-derivation) |
| Validation | `src/validation/shacl.rs` | LLD §2; ERRATA E-013 / E-014 | ✅ real SHACL Core (W3C full-pass 25/25) via rudof `shacl`; `mode => 'pgrdf'` adds native SHACL-SPARQL (authoritative, E-014) |

## Key invariants

1. **The dictionary is the source of truth for term identity.** Every
   subject / predicate / object referenced in `_pgrdf_quads` MUST exist
   in `_pgrdf_dictionary` first; foreign keys are not enforced at the
   DB layer for ingestion performance, but the loader enforces this
   before COPY.
2. **The hexastore is partitioned by `graph_id` (LIST partitioning).**
   Dropping a named graph is `DROP TABLE _pgrdf_quads_<n>` — O(seconds),
   not O(rows). The default partition is `_pgrdf_quads_default`.
3. **Inferred triples carry `is_inferred = TRUE`.** They live in the
   same hexastore; the inference engine never writes outside it.
   Truncating inferred state is `DELETE FROM _pgrdf_quads WHERE is_inferred`.
4. **SHACL output is always JSONB.** The validation engine never writes
   into the hexastore.

## Deployment shape

For local dev: `compose/` brings up stock `postgres:17.4-bookworm`
with bind-mounted extension files (PG 18 deferred pending pgrx
upstream — ERRATA E-006). See [`docs/06-installation.md`](06-installation.md)
for K8s.
