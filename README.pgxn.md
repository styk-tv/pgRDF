# pgRDF

**A Rust-native PostgreSQL extension for RDF, SPARQL, SHACL and OWL reasoning.**

pgRDF turns a single PostgreSQL instance into a complete semantic-web engine —
dictionary-encoded hexastore storage, a SPARQL 1.1 query **and** update engine,
a W3C-conformant SHACL Core validator, and an OWL 2 RL / RDFS reasoner — with no
sidecar triple store and no second system to operate. Load Turtle, then **reason
over it, validate it, and query it in place**, each step a single function call,
all inside one PostgreSQL session. Every release is CI-built and signed with SLSA
Build Provenance v1.

- **Current version & full docs:** <https://pgrdf.styk.tv/> (authoritative at any time)
- **Source & releases:** <https://github.com/styk-tv/pgRDF>
- **PostgreSQL:** 14 · 15 · 16 · 17
- **License:** MIT

> The PGXN package tracks the GitHub release line; the live version is shown at
> the links above. This README mirrors the project README — see GitHub for badges,
> benchmark tables, and the full guide.

## Capabilities

Everything below runs inside one PostgreSQL instance, addressable from any client
— no sidecar store, no ETL.

### Query — SPARQL 1.1
`SELECT` / `ASK` over N-pattern basic graph patterns, lowered to SQL joins on a
pinned, cross-product-proof plan.

- **Filters** — identity, boolean composition, term-type tests, `REGEX`, numeric & typed comparison
- **Modifiers** — `DISTINCT`, `LIMIT` / `OFFSET`, type-aware `ORDER BY`
- **Patterns** — multi-triple `OPTIONAL`, `UNION`, `MINUS`, `VALUES`, downstream `BIND`
- **Aggregates** — `COUNT` / `SUM` / `AVG` / `MIN` / `MAX` / `GROUP_CONCAT` / `SAMPLE` with `GROUP BY` / `HAVING`
- **CONSTRUCT** and **DESCRIBE** (W3C §16.4 Concise Bounded Description)
- **Property paths** — `^` `+` `*` `?` `|`, with a materialised-closure no-CTE fast path and a depth guard
- **Named graphs** — `GRAPH <iri>` and `GRAPH ?g`, composed across OPTIONAL / UNION / MINUS

### Update — SPARQL 1.1 UPDATE
`INSERT` / `DELETE DATA`, `INSERT` / `DELETE WHERE`, `DELETE`+`INSERT WHERE`,
`WITH <iri>` scoping, and the graph lifecycle algebra (`DROP` / `CLEAR` /
`CREATE GRAPH` × `DEFAULT` / `NAMED` / `ALL`).

### Storage
Dictionary-encoded terms over a LIST-partitioned hexastore (SPO / POS / OSP
covering indexes).

- **Ingest** — Turtle, TriG, N-Quads (`parse_turtle` / `parse_trig` / `parse_nquads`), plus a **parallel bulk loader** (`load_turtle(…, bulk_load => true)`) and a native staged loader for billion-scale `.nt`
- **Per-graph lifecycle** — `drop` / `clear` / `copy` / `move_graph`, with BIGINT and IRI overloads
- **Performance** — cross-backend shared-memory dictionary cache, prepared-plan cache, prepared bulk-INSERT

### Inference — OWL 2 RL + RDFS
`pgrdf.materialize(graph, profile)` forward-chains the closure (`owl-rl` or
`rdfs`), refreshes planner statistics automatically so queries stay fast on the
enlarged graph, and is idempotent across calls.

### Validation — W3C SHACL Core
`pgrdf.validate(data, shapes, mode)` returns a real `sh:ValidationReport` as
JSONB — genuine W3C SHACL Core conformance (25 / 25).

## Install

**From PGXN:**

```bash
pgxn install pgrdf
```

This builds the extension from source (Rust + `cargo-pgrx` toolchain required).

**Pre-built, attested artifacts** (no build): the project also publishes an
SLSA-attested OCI bundle and per-PG×arch tarballs on GitHub — see
[INSTALL.md](https://github.com/styk-tv/pgRDF/blob/main/INSTALL.md). Every
published digest carries a verifiable SLSA Build Provenance v1 attestation.

After installing the extension files into your PostgreSQL:

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version();
```

### Required `postgresql.conf`
pgRDF MUST be in `shared_preload_libraries` so `_PG_init()` runs in the postmaster
context (it registers the shared-memory dictionary cache + plan-cache stats).
Without it, the first pgRDF call panics with `PgAtomic was not initialized`.

```ini
shared_preload_libraries = 'pgrdf'
```

A server **restart** (not a reload) is required after editing this. Verify:

```sql
SHOW shared_preload_libraries;   -- must contain 'pgrdf'
```

## Quickstart

```sql
-- One-time install
CREATE EXTENSION pgrdf;

-- Load a Turtle file from the server-side filesystem into graph 100
SELECT pgrdf.load_turtle('/path/to/foaf.ttl', 100);

-- Query it with SPARQL 1.1
SELECT * FROM pgrdf.sparql($$
  PREFIX foaf: <http://xmlns.com/foaf/0.1/>
  SELECT ?name WHERE { ?p foaf:name ?name } LIMIT 10
$$);

-- Reason over it (OWL 2 RL forward-chaining closure)
SELECT pgrdf.materialize(100, 'owl-rl');
```

## Documentation

- **Guide (using pgRDF):** <https://pgrdf.styk.tv/> and
  [`guide/`](https://github.com/styk-tv/pgRDF/tree/main/guide) — install, loading
  RDF, querying, and client recipes (Python, Rust, Node.js/TypeScript, Go).
- **Engineering docs:**
  [`docs/`](https://github.com/styk-tv/pgRDF/tree/main/docs) — architecture,
  storage, query, inference, validation, release, roadmap.

## Honest scope

A few surfaces are gated on upstream crates, not defects: RDF 1.2 triple terms +
the crates.io publish path are gated on the `reasonable` reasoner's RDF-1.2
support landing upstream (ERRATA E-011 · `gtfierro/reasonable`); SHACL-SPARQL
constraint execution is gated on `rudof`. The shipped surfaces are honest about
what they cover.

## License

MIT — see [LICENSE](https://github.com/styk-tv/pgRDF/blob/main/LICENSE).
