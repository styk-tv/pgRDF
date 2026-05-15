# pgRDF v0.4.2

Graph-level lifecycle UDFs ship. The §5 LLD surface lands four
partition-level primitives — `pgrdf.drop_graph`, `clear_graph`,
`copy_graph`, `move_graph` — that operate against `_pgrdf_quads`'s
LIST partitioning rather than via N-row DELETE loops. Phase B closes
in five countdown slices (99 → 95) on top of the v0.4.1 named-graph
surface.

## Marquee — Lifecycle UDFs (LLD v0.4 §5)

Partition-level primitives over `_pgrdf_quads`. Constant-time DDL
where possible: `DETACH + DROP` for `drop_graph`, `TRUNCATE ONLY` for
`clear_graph`, `INSERT INTO … SELECT` against the per-graph
partitions for `copy_graph`, and a `copy + drop` compose for
`move_graph`.

- **`pgrdf.drop_graph(id BIGINT, cascade BOOLEAN DEFAULT TRUE) →
  BIGINT`** (slice 99) — removes the LIST partition
  `_pgrdf_quads_g<id>` from `_pgrdf_quads` via `ALTER TABLE … DETACH
  PARTITION` + `DROP TABLE`, deletes the matching `_pgrdf_graphs`
  row, returns the pre-drop triple count. `cascade => FALSE` errors
  with the stable `drop_graph: inferred rows present` prefix if any
  `is_inferred = TRUE` row exists. Default partition (`graph_id =
  0`) is rejected; idempotent on absent graphs (returns 0).
- **`pgrdf.clear_graph(id BIGINT) → BIGINT`** (slice 98) — issues
  `TRUNCATE ONLY pgrdf._pgrdf_quads_g<id>` against the per-graph
  partition and returns the rows-removed count. Partition shell +
  `_pgrdf_graphs` IRI binding survive (contrast with `drop_graph`).
  `clear_graph(0)` is permitted (clears the explicit `g0` partition
  only); negative ids rejected with the stable prefix.
- **`pgrdf.copy_graph(src BIGINT, dst BIGINT) → BIGINT`** (slice
  97) — `INSERT INTO _pgrdf_quads_g<dst> SELECT … FROM
  _pgrdf_quads_g<src>`. Both base and `is_inferred = TRUE` rows
  carry forward. Auto-creates the `dst` partition + IRI binding if
  absent. Idempotent on absent src (returns 0 without erroring); a
  pre-existing IRI binding on `dst` is preserved. The only
  lifecycle UDF that touches every row — cost scales linearly with
  source row count.
- **`pgrdf.move_graph(src BIGINT, dst BIGINT) → BIGINT`** (slice
  96) — `copy_graph(src, dst)` + `drop_graph(src, cascade => TRUE)`
  compose. Both halves run in the calling statement's transaction
  (rollback unwinds both). The LLD §5.2 "metadata-only
  `DETACH/ATTACH` partition rebind" is aspirational for v0.4.2 —
  a true constant-time rebind would need every row's `graph_id`
  column updated to satisfy the post-rebind LIST constraint, itself
  a row scan. Tractable metadata-only `move_graph` is flagged as a
  v0.5 perf optimisation.

### Error-prefix contract (stable for downstream tooling)

Every lifecycle UDF surfaces validation failures with a UDF-name-
prefixed panic message; tooling that parses error strings can
route on the prefix without depending on the trailing detail:

```
drop_graph: graph_id must be >= 0, got <N>
drop_graph: cannot drop default partition (graph_id = 0)
drop_graph: inferred rows present (graph_id = <N>); …
clear_graph: graph_id must be >= 0, got <N>
copy_graph: src and dst must differ (both = <N>)
copy_graph: graph_id must be >= 0, got src=<S>, dst=<D>
move_graph: src and dst must differ (both = <N>)
move_graph: graph_id must be >= 0, got src=<S>, dst=<D>
move_graph: dst graph_id <N> already has data (<M> rows); …
```

### End-to-end integration

Slice 95 wires the four UDFs together against a realistic
load → mutate → verify flow. The per-UDF regression files
(`88-drop-graph.sql` / `89-clear-graph.sql` / `90-copy-graph.sql` /
`91-move-graph.sql`) lock invariants in isolation; the new
[`92-lifecycle-end-to-end.sql`](tests/regression/sql/92-lifecycle-end-to-end.sql)
pins their interactions: load → copy → drop round-trip,
`move_graph` as a faithful `copy + drop` compose, `clear_graph`
isolation under a shared dictionary, SPARQL `GRAPH <iri>`
projection survival across the lifecycle, and the drop-then-rebind
loop.

## SPARQL UPDATE wiring

Deferred to v0.4.3 (Phase C). v0.4.2 ships the §5 lifecycle UDF
surface only; the SPARQL UPDATE algebra (`DROP GRAPH <iri>`, `CLEAR
GRAPH <iri>`, `COPY <src> TO <dst>`, `MOVE <src> TO <dst>`) lands
on top in v0.4.3 by wiring the parser to dispatch into the existing
UDFs.

## Test bar

216 automated tests across four layers plus the pg_dump round-trip
gate:

| Layer | Count | Δ from v0.4.1 |
|---|---|---|
| pgrx integration | 133 | +15 |
| pg_regress golden | 54 | +5 |
| W3C-shape SPARQL conformance | 26 | 0 |
| LUBM-shape correctness | 3 | 0 |
| **Total** | **216** | **+20** |

Plus `tests/regression/scripts/pg-dump-roundtrip.sh` end-to-end
round-trip gate on `_pgrdf_graphs`. The pgrx static `#[pg_test]`
attribute count is 127; pgrx-tests 0.16 generates 6 additional
harness wrappers — the runtime count is what callers see.

## Install — prebuilt tarballs (same layout as v0.4.1)

```bash
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.2/pgrdf-0.4.2-pg17-glibc-amd64.tar.gz
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.2/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf pgrdf-0.4.2-pg17-glibc-amd64.tar.gz
cd pgrdf-0.4.2-pg17-glibc-amd64
sudo cp lib/pgrdf.so $(pg_config --pkglibdir)/
sudo cp share/extension/* $(pg_config --sharedir)/extension/
```

Then in psql:

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version();  -- → 0.4.2
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

## crates.io

v0.4.2 is **not** published to crates.io. The `[patch.crates-io]`
block for `reasonable` (E-011) blocks `cargo publish`; the
`publish-crate.yml` workflow remains disabled until upstream
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
merges and the patch retires. The crate is registered on crates.io
with v0.3.0 (pre-work seed); v0.4.1 + v0.4.2 binaries are available
via the GitHub Release tarballs.

## Known issues — carried from v0.4.1

- **E-011 — `[patch.crates-io]` fork-dep still in place.** Carried.
  v0.4.2 continues to patch `reasonable` against
  [`styk-tv/reasonable@rdf12-passthrough`](https://github.com/styk-tv/reasonable/tree/rdf12-passthrough)
  for `TermRef::Triple(_)` coexistence with `shacl 0.3.x` under
  `oxrdf`'s `rdf-12` feature. The patch retires once
  [gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
  merges.
- **E-006** — pgrx 0.18 / Postgres 18 deferred (carried).
- **E-007** — `extension_control_path` GUC blocked by E-006
  (carried).
- **E-009** — original SHACL upstream-block resolved at the
  validation-engine half; remaining piece is the
  `[patch.crates-io]` route until #50 merges (carried).
- **E-010** — cargo audit informational advisories (carried).

### v0.4.2-introduced

- **pgrx-tests parallelism flake on partition DDL.** Two Phase A
  tests (`pg_add_graph_iri_idempotent`,
  `pg_add_graph_id_iri_synthetic_upgrade`) occasionally race under
  pgrx-tests 0.16's parallel scheduler because both exercise
  partition DDL inside `add_graph(iri)` / `add_graph(id BIGINT)`
  through SPI. Pre-existing on v0.4.1 (verified empirically); the
  v0.4.2 Phase B test annotations were tightened to exact-match the
  panic strings so the four lifecycle-UDF rejection-path tests are
  now deterministic. Pgrx test bar is 133/133 green when this race
  resolves; CI re-runs absorb the noise.

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) and
[`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) for the full text.

## What's deferred from v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](specs/SPEC.pgRDF.LLD.v0.4.md):

- SPARQL UPDATE (§4) — Phase C opens next (v0.4.3)
- CONSTRUCT (§6) — v0.4.4
- Property paths (§7) — v0.4.5
- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11) — v0.4.6
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

These land in subsequent v0.4.x point releases or in a refreshed
v0.5.0 cut.

## Upgrading from v0.4.1

pgRDF v0.x reserves the right to break schema between minor
releases. `ALTER EXTENSION pgrdf UPDATE` is not supported in v0.x.
Drop and recreate:

```sql
-- Dump first if you care about your data
DROP EXTENSION pgrdf CASCADE;
-- Install v0.4.2 artifacts
CREATE EXTENSION pgrdf;
-- Re-ingest
```

The schema is forward-compatible at the table-shape level
(v0.4.1's `_pgrdf_graphs`, `_pgrdf_quads`, `_pgrdf_dictionary` are
unchanged in v0.4.2); only new UDFs land. A `pg_dump` from v0.4.1
will restore against a v0.4.2 install via the documented
`DROP/CREATE EXTENSION; pg_restore` path. See
[`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions).

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
