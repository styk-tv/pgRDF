# pgRDF v0.4.0

The first pgRDF release with the full four-engine mission shipping
in earnest: **storage, SPARQL, OWL 2 RL inference, and now W3C
SHACL Core validation**. The validation engine stops being a stub.

## Marquee — SHACL Core validation is real

`pgrdf.validate(data_graph_id, shapes_graph_id) → JSONB` now
executes real W3C SHACL Core validation via `shacl 0.3.1`. The
v0.3.0 stub is gone. The JSONB output is a `sh:ValidationReport`
shape with `conforms`, `results[]`, and per-violation
`focusNode` / `resultPath` / `sourceShape` / `resultMessage` /
`resultSeverity` / `sourceConstraintComponent` / `value`.

Supports `sh:NodeShape` + `sh:property` + `sh:class` /
`sh:datatype` + cardinality, value-type, value-range, node-kind,
pattern, and `sh:in` constraints — whatever `shacl 0.3.1`'s
SHACL Core implementation covers.

## What's in this release

- **Storage Engine** — unchanged from v0.3.0. Dictionary-encoded
  terms, LIST-partitioned quads on `graph_id`, SPO/POS/OSP
  hexastore covering indexes, Turtle ingest with shmem dict cache
  + prepared bulk-INSERT.
- **SPARQL Engine (SELECT / ASK)** — unchanged from v0.3.0.
  N-pattern BGPs, FILTER (identity / boolean / REGEX / IN /
  BOUND / term-type / numeric ordering / string funcs),
  DISTINCT / LIMIT / OFFSET / ORDER BY, OPTIONAL / UNION / MINUS,
  aggregates with type-aware MIN/MAX, HAVING (alias + inline),
  GROUP_CONCAT, SAMPLE, BIND projection.
- **OWL 2 RL Inference** — `pgrdf.materialize`, unchanged.
  Forward-chaining via `reasonable 0.4`, idempotent.
- **SHACL Validation** — **NEW: real impl**. `pgrdf.validate`
  replaces the v0.3.0 stub. Real W3C `sh:ValidationReport`-shape
  JSONB via `shacl 0.3.1`.

## Test bar

160 automated tests across four layers plus the 24-ontology smoke:

| Layer | Count |
|---|---|
| pgrx integration | 94 |
| pg_regress golden | 40 |
| W3C-shape SPARQL conformance | 23 |
| LUBM-shape correctness | 3 |
| **Total** | **160** |

Plus manual smoke: 24 ontologies / 17,134 triples
(W3C / Apache Jena / ValueFlows / ConceptKernel v3.7); totals
locked in `tests/perf/smoke-ontologies.expected.tsv`.

## Install — same as v0.3.0

```bash
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.0/pgrdf-0.4.0-pg17-glibc-amd64.tar.gz
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.0/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf pgrdf-0.4.0-pg17-glibc-amd64.tar.gz
cd pgrdf-0.4.0-pg17-glibc-amd64
sudo cp lib/pgrdf.so $(pg_config --pkglibdir)/
sudo cp share/extension/* $(pg_config --sharedir)/extension/
```

Then in psql:

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version();  -- → 0.4.0
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

## Known issues

- **E-011 — `[patch.crates-io]` fork-dep in place.** v0.4.0 ships
  with `Cargo.toml` containing a `[patch.crates-io]` override
  pointing at the
  [`styk-tv/reasonable@rdf12-passthrough`](https://github.com/styk-tv/reasonable/tree/rdf12-passthrough)
  fork. The patch adds a `TermRef::Triple(_)` arm to `reasonable`
  needed for coexistence with `shacl 0.3.x` under `oxrdf`'s
  `rdf-12` feature. Upstream PR:
  [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50).
  Users `cargo build`ing from source pull the fork transparently.
  Once upstream merges, **v0.4.1** drops the patch and pins the
  released `reasonable` version. Track at
  [`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) E-011.
- **E-006** — pgrx 0.18 / Postgres 18 deferred (carried from v0.3.0).
- **E-007** — `extension_control_path` GUC blocked by E-006
  (carried; per-file bind mounts retain the same observable end-state).
- **E-009** — original SHACL upstream-block; **the
  validation-engine half is resolved by E-011's patch**; the only
  remaining piece is the `[patch.crates-io]` route until #50 merges.
- **E-010** — cargo audit informational advisories (carried).

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) and
[`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) for the full text.

## What's deferred from v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](specs/SPEC.pgRDF.LLD.v0.4.md):

- Named-graph `GRAPH { … }` + `_pgrdf_graphs` IRI mapping (§3)
- SPARQL UPDATE (§4)
- Graph-level lifecycle UDFs (§5)
- CONSTRUCT (§6)
- Property paths (§7)
- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11)
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

These land in subsequent v0.4.x point releases or in a refreshed
v0.5.0 cut.

## Upgrading from v0.3.0

pgRDF v0.x reserves the right to break schema between minor
releases. There is no in-place upgrade path;
`ALTER EXTENSION pgrdf UPDATE` is not supported in v0.x. Drop and
recreate per the v0.x upgrade policy:

```sql
-- Dump first if you care about your data
DROP EXTENSION pgrdf CASCADE;
-- Install v0.4.0 artifacts
CREATE EXTENSION pgrdf;
-- Re-ingest
```

See
[`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions)
for the full procedure.

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
