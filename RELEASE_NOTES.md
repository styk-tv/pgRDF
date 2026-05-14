# pgRDF v0.3.0

The first official pgRDF release: a Rust-native PostgreSQL extension for RDF, SPARQL, OWL 2 RL inference, and SHACL.

## What's in this release

- **Storage engine** — dictionary-encoded terms, LIST-partitioned quads on `graph_id`, SPO/POS/OSP hexastore covering indexes. UDFs: `pgrdf.{add_graph, count_quads, put_term, get_term, put_quad}`.
- **Turtle ingest** — `pgrdf.{load_turtle, parse_turtle, load_turtle_verbose, parse_turtle_verbose}` with shmem dict cache + prepared bulk-INSERT.
- **SPARQL SELECT/ASK** — N-pattern BGPs; FILTER (identity, boolean, REGEX, IN, BOUND, term-type, numeric ordering, string funcs); DISTINCT/LIMIT/OFFSET/ORDER BY; OPTIONAL/UNION/MINUS; aggregates with type-aware MIN/MAX, HAVING (alias + inline), GROUP_CONCAT, SAMPLE; BIND projection.
- **OWL 2 RL inference** — `pgrdf.materialize(graph_id)` via `reasonable 0.4`. Idempotent.
- **SHACL validation** — `pgrdf.validate(data, shapes)` stub with stable JSONB schema (real integration blocked by ERRATA E-009; see [Known issues](#known-issues)).
- **Performance** — shmem dict cache (< 1 µs hit), prepared-plan cache, prepared bulk-INSERT.

## Test bar

158 automated tests (93 pgrx + 39 pg_regress + 23 W3C-shape + 3 LUBM-shape) plus 24-ontology / 17,134-triple manual smoke. CI green on PG 14–17 × {amd64, arm64}.

## Install

### Drop-in (any Postgres 14–17)

```bash
# Download the matching tarball:
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.3.0/pgrdf-0.3.0-pg17-glibc-amd64.tar.gz
# Verify the checksum:
sha256sum pgrdf-0.3.0-pg17-glibc-amd64.tar.gz
# (compare against the aggregate SHA256SUMS on the release page)
tar -xzf pgrdf-0.3.0-pg17-glibc-amd64.tar.gz
cd pgrdf-0.3.0-pg17-glibc-amd64
sudo cp lib/pgrdf.so $(pg_config --pkglibdir)/
sudo cp share/extension/* $(pg_config --sharedir)/extension/
```

Then in psql:

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version();   -- → 0.3.0
```

Requires `shared_preload_libraries = 'pgrdf'` in `postgresql.conf` (see [INSTALL spec](specs/SPEC.pgRDF.INSTALL.v0.2.md) §6).

### Docker compose

See [`guide/01-install.md`](guide/01-install.md) for the compose-based local development path.

## Supported Postgres

PG 14, 15, 16, 17 across {amd64, arm64} = 8 prebuilt tarballs. PG 18 deferred per ERRATA E-006.

## Known issues

- **E-006** — pgrx 0.18 / Postgres 18 deferred to v0.4.
- **E-007** — INSTALL §7's `extension_control_path` GUC forward
  path is blocked by E-006; v0.3 ships via per-file bind mounts at
  canonical `$libdir`/`$sharedir/extension` paths instead.
- **E-009** — SHACL real integration blocked by upstream dep conflict.
- **E-010** — cargo audit advisories — all informational, no security impact.

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) for the full text.

## Deferred to v0.4

Named-graph `GRAPH { … }`, SPARQL UPDATE, graph lifecycle UDFs, CONSTRUCT, property paths, multi-triple OPTIONAL, VALUES, BIND-downstream, aggregates over UNION, DESCRIBE, `heap_multi_insert` (2× ingest target). See [`specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md`](specs/SPEC.pgRDF.LLD.v0.4-FUTURE.md).

## Upgrading

pgRDF v0.x reserves the right to break schema between minor releases. There is no in-place upgrade path; `ALTER EXTENSION pgrdf UPDATE` is not supported in v0.x. The supported flow is: dump your data via SQL (decode `_pgrdf_quads` against `_pgrdf_dictionary` per graph and serialise to Turtle externally), `DROP EXTENSION pgrdf CASCADE`, install the new version, then `CREATE EXTENSION pgrdf` and re-load. v1.0 will introduce proper `ALTER EXTENSION pgrdf UPDATE` migrations alongside a frozen on-disk schema. See [`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions) for the full procedure.

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
