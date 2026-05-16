# pgRDF v0.5.0 — the complete RDF / SPARQL / SHACL / OWL surface

**pgRDF v0.5.0 is the complete in-database RDF toolkit for PostgreSQL.** Storage, SPARQL 1.1 query + update, OWL 2 RL / RDFS reasoning, and W3C SHACL Core validation all run natively inside Postgres — no external triple store, no sidecar service. This is the final v0.5.0 cut: it supersedes the `v0.5.0-rc1` prerelease and becomes the latest release.

## Capability surface (everything that ships)

- **Storage** — dictionary-encoded terms in `_pgrdf_dictionary`; quads in `_pgrdf_quads` LIST-partitioned by `graph_id`; hexastore covering indexes (SPO, POS, OSP). Turtle, **TriG** and **N-Quads** ingest (`pgrdf.load_turtle`, `pgrdf.parse_trig`, `pgrdf.parse_nquads`) — TriG/N-Quads honour inline / 4th-position graph IRIs (auto-allocate, or reject under `strict`).
- **SPARQL SELECT / ASK / CONSTRUCT / DESCRIBE** — `pgrdf.sparql(q) → SETOF JSONB`, `pgrdf.construct(q)`, `pgrdf.describe(q)`; spargebra parser + dynamic-SQL executor with a prepared-plan cache. Full property paths (`^` `+` `*` `?` `|`, recursive `WITH RECURSIVE` lowering, materialised-closure fast path). The §11 SPARQL backlog — multi-triple OPTIONAL, VALUES, downstream BIND, aggregates over UNION (incl. the six v0.5 residuals), DESCRIBE — plus type-aware ORDER BY across the SPARQL 1.1 §15.1 value space.
- **SPARQL UPDATE** — INSERT DATA / DELETE DATA / INSERT…WHERE / DELETE…WHERE / DELETE…INSERT…WHERE, graph-scoped (`WITH <iri>`, inline `GRAPH <iri> { … }`), and the CREATE/DROP/CLEAR/COPY/MOVE lifecycle algebra.
- **Named graphs + lifecycle UDFs** — `GRAPH <iri>` / `GRAPH ?g`, the `_pgrdf_graphs` IRI↔id map (pg_dump round-trip), and `pgrdf.{drop,clear,copy,move}_graph` with both BIGINT and **IRI-keyed overloads** (`drop_graph('http://…')`).
- **OWL 2 RL + RDFS reasoning** — `pgrdf.materialize(graph_id, profile TEXT DEFAULT 'owl-rl')`. The bare 1-arg form is byte-identical to v0.4 (`'owl-rl'`). `'rdfs'` runs a strict, sound, complete RDFS rule subset (a true subset of OWL-RL). Unknown profiles error with no silent fallback.
- **SHACL Core validation** — `pgrdf.validate(data_graph_id, shapes_graph_id, mode TEXT DEFAULT 'native')`. The 2-arg form is byte-identical to v0.4. JSONB gains a `mode` field; unknown modes error before any work. Validation against a `pgrdf.materialize`-d graph reports violations against entailed triples. The vendored **W3C SHACL Core** manifest gate is a genuine **25 / 25 full-pass** on the `sh:conforms` invariant, wired into CI on every PG major.

## Documented limitations (honest, scoped, upstream-gated)

These are documented upstream gates — NOT pgRDF defects — exactly the posture used for E-011 / RDF 1.2.

- **E-011 — crates.io publish is gated.** pgRDF depends on a patched `reasonable` (the `styk-tv/reasonable` `rdf12-passthrough` fork wired via `[patch.crates-io]`) until upstream [`gtfierro/reasonable#50`](https://github.com/gtfierro/reasonable/pull/50) merges. A crate with a `[patch.crates-io]` line cannot be published to crates.io, so the **supported distribution path is the GitHub release tarballs and the OCI bundle below** — `publish-crate.yml` stays disabled.
- **E-012 — SHACL-SPARQL constraint execution is upstream-gated.** `shacl 0.3.1` (rudof) has no SHACL-SPARQL constraint component and its `SparqlEngine` is an `unimplemented!()` stub (rudof issues #21/#94/#1). `pgrdf.validate(…, 'sparql')` therefore does NOT invoke the broken engine — it returns a clean deterministic structured report (`conforms:null` + an `error` naming the upstream gap), never a panic. The `mode` argument, the JSONB shape, and the validation path are exactly what they will be the day rudof ships the engine — one guard is deleted and `'sparql'` routes through with no signature change. This is documented in ERRATA.v0.5 E-012 and is final for v0.5.0.

## Install — GitHub release tarballs (stock postgres:17.4)

Each platform tarball (`pgrdf-0.5.0-pg<PG>-glibc-<arch>.tar.gz`, PG 14-17 × amd64/arm64) unpacks to the `SPEC.pgRDF.INSTALL.v0.2 §3` layout (`lib/pgrdf.so`, `share/extension/pgrdf.control`, `share/extension/pgrdf--0.5.0.sql`, `LICENSE`, `NOTICE`, `SHA256SUMS`). Verify, then bind-mount into a stock Postgres container:

```bash
tar xzf pgrdf-0.5.0-pg17-glibc-amd64.tar.gz
( cd pgrdf-0.5.0-pg17-glibc-amd64 && sha256sum -c SHA256SUMS )

docker run -d --name pg \
  -e POSTGRES_PASSWORD=pw \
  -v "$PWD/pgrdf-0.5.0-pg17-glibc-amd64/lib/pgrdf.so:/usr/lib/postgresql/17/lib/pgrdf.so:ro" \
  -v "$PWD/pgrdf-0.5.0-pg17-glibc-amd64/share/extension/pgrdf.control:/usr/share/postgresql/17/extension/pgrdf.control:ro" \
  -v "$PWD/pgrdf-0.5.0-pg17-glibc-amd64/share/extension/pgrdf--0.5.0.sql:/usr/share/postgresql/17/extension/pgrdf--0.5.0.sql:ro" \
  postgres:17.4
docker exec -it pg psql -U postgres -c 'CREATE EXTENSION pgrdf;'
```

The aggregate `SHA256SUMS` asset on the release verifies every tarball.

## OCI artifacts (ghcr.io/styk-tv/pgrdf-bundle)

```
oras pull ghcr.io/styk-tv/pgrdf-bundle:v0.5.0-pg17-amd64
oras manifest fetch ghcr.io/styk-tv/pgrdf-bundle:v0.5.0   # index of all 8 PG×arch
# digest-pin (recommended): oras pull ghcr.io/styk-tv/pgrdf-bundle@sha256:<digest>
# NOTE: anonymous pull requires the package be public (one-time maintainer setting).
```

The `oci-publish` workflow runs on `release: [published]`, downloads the release tarballs (no rebuild), and pushes one OCI artifact per PG×arch plus the aggregate `:0.5.0` / `:v0.5.0` index manifests. **Anonymous pull requires the GHCR package be set public** — the Actions `GITHUB_TOKEN` lacks `admin:packages`, so the first publish lands the package private and a maintainer flips it public once (GitHub → Package settings → visibility Public, or `gh api`). A **digest pin** (`@sha256:<digest>`) is recommended for reproducible deployments.

## Test bar

All hand-computed / hand-derived — no `ACCEPT=1` autobaselining of new query or SHACL coverage.

```
pgrx integration   274  (PG14-17 matrix in CI)
pg_regress          85
W3C-shape SPARQL    51
W3C SHACL Core      25  (genuine 25/25 full-pass on sh:conforms, no exclusion)
LUBM-shape           3
Total: 438 green across six layers, plus the pg_dump round-trip
gate and the W3C SHACL --sparql E-012 known-state assertion.
```

## Upgrading from v0.4.6

pgRDF v0.x reserves the right to break schema between minor releases; `ALTER EXTENSION pgrdf UPDATE` is not supported. Drop and recreate (dump first if you care about your data):

```sql
DROP EXTENSION pgrdf CASCADE;
-- install the v0.5.0 artifacts
CREATE EXTENSION pgrdf;
-- re-ingest
```

The table shapes (`_pgrdf_graphs`, `_pgrdf_quads`, `_pgrdf_dictionary`) are unchanged from v0.4.6; `pgrdf.validate` gains an optional third argument (the 2-arg form is unchanged) and the `parse_trig`/`parse_nquads` ingest UDFs landed in the v0.5 cycle. A `pg_dump` from v0.4.6 restores against a v0.5.0 install via the documented DROP/CREATE EXTENSION; pg_restore path. See [`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions).

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md). Spec: [`specs/SPEC.pgRDF.LLD.v0.5.md`](specs/SPEC.pgRDF.LLD.v0.5.md) (authoritative, shipped in v0.5.0); errata [`specs/ERRATA.v0.5.md`](specs/ERRATA.v0.5.md).
