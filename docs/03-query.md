# 03 — Query

The query engine answers `SELECT pgrdf.sparql($1)` where `$1` is a
SPARQL 1.1 string.

## Pipeline

```
SPARQL string
     │
     ▼  spargebra::Query::parse
algebra AST (Project ▶ Join ▶ Bgp(t1..tn))
     │
     ▼  src/query/executor.rs
parameterized SQL plan over _pgrdf_quads (one CTE per BGP)
     │
     ▼  Spi::prepare + Spi::execute_with_args
result rows
```

## Why prepared plans (LLD §4.2)

Two SPARQL queries with identical structure but different constants
yield identical algebra-up-to-constants. Cache keyed by the canonical
algebra hash. On hit, we bypass the Postgres parser AND planner —
substantial latency reduction for OLTP-style query patterns.

## Plan cache

- Key: SHA-256 of the canonical algebra (constants stripped).
- Value: a `pgrx::Spi`-prepared statement handle + parameter shape.
- Eviction: bounded LRU per backend; shmem-promoted in v0.3.

## v0.2.0 scope

- ✅ Basic Graph Patterns (BGP)
- ✅ `SELECT` with simple projection
- ⏳ `OPTIONAL`, `UNION`, `FILTER` — Phase 2
- ⏳ Property paths — Phase 3
- ⏳ Aggregates (`GROUP BY`, `COUNT`) — Phase 3
- ⏳ Federated `SERVICE` — out of scope for v0 series

## Custom scan hooks (Phase 2)

The Postgres custom scan API lets us bypass the executor entirely for
specific quad-shape access patterns. Wiring this in is a v0.3 target;
v0.2 uses standard SPI prepare+execute which is sufficient for the
LLD's stated performance posture.
