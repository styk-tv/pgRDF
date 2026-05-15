# pgRDF v0.4.1

Named-graph SPARQL scoping ships. `GRAPH { … }` lands as a
first-class executor surface — literal IRI, variable form, and full
composition with OPTIONAL / UNION / MINUS — backed by an
`_pgrdf_graphs(graph_id, iri)` mapping table and a five-UDF
binding / lookup API. First pgRDF release on crates.io.

## Marquee — Named-graph SPARQL scoping

LLD v0.4 §3 lands end-to-end across thirteen Phase A countdown
slices (120 → 108). The surface covers the storage layer, the UDF
surface, the SPARQL executor's translator and SQL builder, and the
operational `pg_dump` round-trip discipline.

- **Storage** — new `pgrdf._pgrdf_graphs(graph_id BIGINT PRIMARY
  KEY, iri TEXT NOT NULL UNIQUE)` table establishes the IRI ↔
  `graph_id` mapping. Seed row `(0, 'urn:pgrdf:graph:0')` covers
  the default-partition catch-all. Registered via
  `pg_extension_config_dump('_pgrdf_graphs', '')` so row data
  travels with `pg_dump`.
- **UDF surface — three `add_graph` overloads.**
  - `pgrdf.add_graph(id BIGINT)` (existing) now also inserts the
    synthetic `urn:pgrdf:graph:{id}` binding, idempotently.
  - `pgrdf.add_graph(iri TEXT) → BIGINT` allocates the next
    `graph_id` and binds the user-supplied IRI atomically.
  - `pgrdf.add_graph(id BIGINT, iri TEXT) → BIGINT` explicit
    binding; idempotent on matching pairs, errors on conflicts,
    upgrades a synthetic seed to a user IRI.
- **UDF surface — two symmetric lookups.**
  `pgrdf.graph_id(iri TEXT) → BIGINT` and
  `pgrdf.graph_iri(id BIGINT) → TEXT`; both STRICT, NULL on miss.
- **SPARQL `GRAPH <iri> { … }`** — literal-IRI form translates to
  `q.graph_id = <resolved>` constraints on every BGP alias inside
  the GRAPH block. Unresolved IRIs bind `-1` (spec-correct
  zero-rows semantics).
- **SPARQL `GRAPH ?g { … }`** — variable form emits an INNER JOIN
  to `_pgrdf_graphs g0` and projects `g0.iri` for the graph
  variable. Multi-triple inner BGPs share a single `graph_id`
  alias so cross-graph stitches are impossible.
- **GRAPH composition** — per-triple `GraphScope` plan threads
  scope through OPTIONAL, UNION, and MINUS. OPTIONAL-born scopes
  get a LEFT JOIN (W3C §13.3 semantics preserved); two GRAPH
  blocks binding the same `?g` get a consistency predicate; MINUS
  scopes stay internal to the `NOT EXISTS` subquery.
- **pg_dump round-trip discipline** — LLD v0.4 §3.1 acceptance
  criterion locked by `tests/regression/scripts/pg-dump-roundtrip.sh`
  driving seed → dump → drop → restore → verify.

## Now on crates.io

v0.4.1 is the first pgRDF release published to crates.io. From-source
install:

```bash
cargo add pgrdf  # or in your Cargo.toml
```

See [`docs/06-installation.md`](docs/06-installation.md) for the
full from-source path (`cargo pgrx package` against your local
PG installation).

## Test bar

195 automated tests across four layers plus the pg_dump round-trip
gate:

| Layer | Count |
|---|---|
| pgrx integration | 117 |
| pg_regress golden | 49 |
| W3C-shape SPARQL conformance | 26 |
| LUBM-shape correctness | 3 |
| **Total** | **195** |

Plus `tests/regression/scripts/pg-dump-roundtrip.sh` end-to-end
round-trip gate on `_pgrdf_graphs`.

## Install — prebuilt tarballs (same layout as v0.4.0)

```bash
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.1/pgrdf-0.4.1-pg17-glibc-amd64.tar.gz
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.1/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf pgrdf-0.4.1-pg17-glibc-amd64.tar.gz
cd pgrdf-0.4.1-pg17-glibc-amd64
sudo cp lib/pgrdf.so $(pg_config --pkglibdir)/
sudo cp share/extension/* $(pg_config --sharedir)/extension/
```

Then in psql:

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version();  -- → 0.4.1
```

`shared_preload_libraries = 'pgrdf'` required (see
[INSTALL spec](specs/SPEC.pgRDF.INSTALL.v0.2.md) §6).

### Docker compose

See [`guide/01-install.md`](guide/01-install.md) for the
compose-based local development path.

## Supported Postgres

PG 14, 15, 16, 17 across {amd64, arm64} = 8 prebuilt tarballs.
PG 18 deferred per
[ERRATA E-006](specs/ERRATA.v0.2.md).

## Known issues — carried from v0.4.0

- **E-011 — `[patch.crates-io]` fork-dep still in place.** v0.4.1
  continues to ship with `Cargo.toml` patching `reasonable` against
  [`styk-tv/reasonable@rdf12-passthrough`](https://github.com/styk-tv/reasonable/tree/rdf12-passthrough)
  for `TermRef::Triple(_)` coexistence with `shacl 0.3.x` under
  `oxrdf`'s `rdf-12` feature. Upstream PR
  [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
  remains the gate; the patch retires once it merges.
- **E-006** — pgrx 0.18 / Postgres 18 deferred (carried).
- **E-007** — `extension_control_path` GUC blocked by E-006
  (carried).
- **E-009** — original SHACL upstream-block resolved at the
  validation-engine half; remaining piece is the
  `[patch.crates-io]` route until #50 merges.
- **E-010** — cargo audit informational advisories (carried).

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) and
[`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) for the full text.

## What's deferred from v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](specs/SPEC.pgRDF.LLD.v0.4.md):

- SPARQL UPDATE (§4)
- Graph-level lifecycle UDFs (§5) — Phase B opens next
- CONSTRUCT (§6)
- Property paths (§7)
- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11)
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

These land in subsequent v0.4.x point releases or in a refreshed
v0.5.0 cut.

## Upgrading from v0.4.0

pgRDF v0.x reserves the right to break schema between minor
releases. `ALTER EXTENSION pgrdf UPDATE` is not supported in v0.x.
Drop and recreate:

```sql
-- Dump first if you care about your data
DROP EXTENSION pgrdf CASCADE;
-- Install v0.4.1 artifacts
CREATE EXTENSION pgrdf;
-- Re-ingest
```

See
[`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions)
for the full procedure.

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
