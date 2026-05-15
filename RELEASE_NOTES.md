# pgRDF v0.4.3

**SPARQL UPDATE surface complete.** The LLD v0.4 §4 UPDATE-form
column closes: every documented variant lands end-to-end on the
SQL engine. Phase C closes in seven countdown slices (84 → 78) on
top of v0.4.2's lifecycle-UDF surface, plus five docs/release
slices (77 → 60).

## Marquee — SPARQL UPDATE (LLD v0.4 §4)

`pgrdf.sparql(q)` accepts UPDATE queries alongside SELECT / ASK.
The function detects the form via `parse_query` first; if that
fails it falls back to `parse_update`. UPDATE forms return a
single summary row of shape `{"_update": …}` carrying `form`,
`triples_inserted`, `triples_deleted`, `graphs_touched`, and
`elapsed_ms` — paralleling the v0.3 `_ask` sentinel for ASK
queries.

### Per-form surface

- **`INSERT DATA { … }`** (slice 84) — static ground-triple
  block. Lands rows in the default graph by default, or in a
  named graph if wrapped with `GRAPH <iri> { … }`. Unknown IRIs
  auto-allocate a fresh `graph_id` via `pgrdf.add_graph(iri)`
  (slice 118). Idempotent: repeat lands no duplicate rows via
  `ON CONFLICT DO NOTHING`. `triples_inserted` reports ATTEMPTED
  inserts (the template size), not net row delta.
- **`DELETE DATA { … }`** (slice 83) — symmetric to INSERT DATA.
  Ground quads only (no variables, no blank nodes — the latter
  forbidden by W3C SPARQL 1.1 §4.1.2). Lookup-only dict path: if
  any term in the triple is absent the delete is a spec-correct
  no-op (never errors). `triples_deleted` counts ACTUAL rows
  removed.
- **`INSERT { template } WHERE { pattern }`** (slice 82) —
  pattern-driven insert. Each solution of the WHERE clause
  produces one concrete triple via the template. The WHERE
  pattern accepts the same shape as a SELECT BGP (joins, FILTER,
  OPTIONAL, UNION, MINUS, GRAPH, …). Template variables MUST be
  bound by the WHERE BGP — unbound template variables panic with
  the stable `INSERT WHERE template feature "unbound template
  variable …"` prefix.
- **`DELETE { template } WHERE { pattern }`** (slice 81) +
  shorthand `DELETE WHERE { pattern }`. Spargebra models the
  template as `Vec<GroundQuadPattern>`, baking the W3C SPARQL
  1.1 §4.1.2 "no blank nodes in DELETE" rule into the AST. The
  per-row delete uses the `WITH d AS (DELETE … RETURNING 1)
  SELECT count(*)` idiom from slice 83's DELETE DATA, so
  `triples_deleted` counts ACTUAL rows removed (not template
  instantiations).
- **`DELETE { … } INSERT { … } WHERE { … }`** (slice 80) — the
  atomic modify form. Both halves resolve against the SAME WHERE
  solutions snapshot: the executor evaluates the pattern exactly
  once, projects every variable referenced by EITHER template,
  and per-row applies DELETE then INSERT. Per W3C SPARQL 1.1
  Update §3.1.3, the DELETE conceptually precedes the INSERT —
  matters for status-flip patterns (`DELETE { ?x ex:status
  "draft" } INSERT { ?x ex:status "published" } WHERE { ?x
  ex:status "draft" }`). Atomicity is naturally provided by
  Postgres — the whole UDF call is one transaction.

### Graph-scoped variants (slice 79)

Every pattern-driven form supports `GRAPH <iri> { … }` inside
the template and/or the WHERE clause. The `WITH <iri>` shortcut
selects `<iri>` as the default graph for BOTH the WHERE
evaluation AND the template's quad routing — per W3C §3.1.3 ¶3,
"If a USING clause is not provided and the WITH clause is
provided, the default graph used to evaluate the WHERE clause
will be the graph from the WITH clause." Spargebra desugars
`WITH <iri>` into a `using: QueryDataset { default: [<iri>],
named: None }` sentinel plus per-quad `graph_name = <iri>` on
every default-graph template triple; the executor lifts the IRI
out, wraps the WHERE pattern in `GraphPattern::Graph(<iri>, …)`
for evaluation, and lets the per-quad routing already carry the
template side.

Cross-graph copy is straightforward — name both graphs
explicitly:

```sparql
INSERT { GRAPH <http://example.org/g2> { ?s ?p ?o } }
WHERE  { GRAPH <http://example.org/g1> { ?s ?p ?o } }
```

### Lifecycle algebra — `DROP / CLEAR / CREATE GRAPH` (slice 78)

The lifecycle operations route to the v0.4.2 §5 graph-management
UDFs (`pgrdf.drop_graph`, `pgrdf.clear_graph`, `pgrdf.add_graph`)
via SPI rather than direct Rust calls. This keeps the SPARQL
front-end and the SQL UDF front-end as two consumers of the same
partition-level primitives — every existence check, partition-
DDL window (`DETACH PARTITION` / `DROP TABLE` / `TRUNCATE ONLY`),
inferred-row cascade guard, and `_pgrdf_graphs` binding update
happens once in the UDFs, not twice.

`spargebra::GraphTarget` enum coverage:

- `GRAPH <iri>` → lookup `_pgrdf_graphs` for the bigint id;
  panic with the stable `DROP GRAPH <iri>: graph not bound` (or
  `CLEAR GRAPH <iri>: graph not bound`) prefix when absent,
  unless `SILENT` was specified (no-op).
- `DEFAULT` → direct partition-wide DELETE for both `CLEAR
  DEFAULT` and `DROP DEFAULT`. Per W3C SPARQL 1.1 Update §3.1.3
  ¶7, `DROP DEFAULT` is an "empty, not destroy"; this also
  avoids the slice-99 `pgrdf.drop_graph(0)` panic guard (the
  default catch-all partition is non-droppable).
- `ALL` → enumerate every `graph_id` in `_pgrdf_graphs`
  (including 0) and dispatch per-id.
- `NAMED` → enumerate every `graph_id <> 0` (default excluded
  per W3C §3.1.3).

`CREATE GRAPH <iri>` errors when the IRI is already bound (W3C
spec requirement) unless `SILENT` was specified. `SILENT`
collapses the "already bound" path to a no-op; the existing
binding survives, and the summary still records the touched
graph_id for operator audit.

**ADD / MOVE / COPY desugar at parse time.** Per spargebra-0.4.6
parser.rs §Add / §Move / §Copy, the SPARQL surface keywords
`ADD`, `MOVE`, `COPY` are NOT separate `GraphUpdateOperation`
variants — they desugar at parse time into compositions of `Drop
+ DeleteInsert` (for COPY) / `Drop + DeleteInsert + Drop` (for
MOVE) / a plain `DeleteInsert` (for ADD). Those compositions
ride the existing per-form dispatcher arms already wired by
slices 80 / 78. No new code path needed.

### Conformance + preview surface (slices 77-74)

Three new W3C-shape fixtures under `tests/w3c-sparql/27-29` lock
the UPDATE surface through the conformance harness. The harness
(`tests/w3c-sparql/run.sh`) gains a sed-based normalisation of
`elapsed_ms: <N>` inside `_update` rows so bag-equivalence diffs
stay stable across runs. Existing 26 fixtures (01-26) are
behaviour-identical.

`pgrdf.sparql_parse(q)` mirrors the executor's runtime
classification on every UPDATE op (slice 74):

```jsonc
{
  "form": "UPDATE",
  "operations": [
    {
      "op": "DeleteInsert",
      "kind": "INSERT_WHERE",           // mirrors _update.form
      "delete_template_size": 0,
      "insert_template_size": 1,
      "where_pattern_size":   1,
      "template_graphs":      ["http://example.org/dst"],
      "with_graph":           "http://example.org/store"
    }
  ],
  "unsupported_algebra": []
}
```

Callers running multi-statement UPDATE preview translatability
per op without executing.

### Error-prefix contract (stable for downstream tooling)

UPDATE-specific surfaces surface validation failures with
form-prefixed panic messages:

```
INSERT WHERE template feature "unbound template variable: <name>"
DELETE WHERE template feature "unbound template variable: <name>"
DELETE/INSERT WHERE template feature "unbound template variable: <name>"
CREATE GRAPH <iri>: graph already exists
DROP GRAPH <iri>: graph not bound
CLEAR GRAPH <iri>: graph not bound
sparql: parse error: <…>           (unchanged from v0.3, covers malformed UPDATE)
```

## Test bar

259 automated tests across four layers plus the pg_dump
round-trip gate:

| Layer | Count | Δ from v0.4.2 |
|---|---|---|
| pgrx integration | 166 | +33 |
| pg_regress golden | 61 | +7 |
| W3C-shape SPARQL conformance | 29 | +3 |
| LUBM-shape correctness | 3 | 0 |
| **Total** | **259** | **+43** |

Plus `tests/regression/scripts/pg-dump-roundtrip.sh` end-to-end
round-trip gate on `_pgrdf_graphs`.

## Install — prebuilt tarballs (same layout as v0.4.2)

```bash
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.3/pgrdf-0.4.3-pg17-glibc-amd64.tar.gz
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.3/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf pgrdf-0.4.3-pg17-glibc-amd64.tar.gz
cd pgrdf-0.4.3-pg17-glibc-amd64
sudo cp lib/pgrdf.so $(pg_config --pkglibdir)/
sudo cp share/extension/* $(pg_config --sharedir)/extension/
```

Then in psql:

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version();  -- → 0.4.3
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

v0.4.3 is **not** published to crates.io. The `[patch.crates-io]`
block for `reasonable` (E-011) blocks `cargo publish`; the
`publish-crate.yml` workflow remains disabled until upstream
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
merges and the patch retires. The crate is registered on
crates.io with v0.3.0 (pre-work seed); v0.4.1 + v0.4.2 + v0.4.3
binaries are available via the GitHub Release tarballs.

## Known issues — carried from v0.4.2

- **E-011 — `[patch.crates-io]` fork-dep still in place.**
  Carried. v0.4.3 continues to patch `reasonable` against
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

### v0.4.2-introduced — carried

- **pgrx-tests parallelism flake on partition DDL.** Two Phase A
  tests (`pg_add_graph_iri_idempotent`,
  `pg_add_graph_id_iri_synthetic_upgrade`) occasionally race
  under pgrx-tests 0.16's parallel scheduler because both
  exercise partition DDL inside `add_graph(iri)` /
  `add_graph(id BIGINT)` through SPI. Pre-existing on v0.4.1
  (verified empirically). CI re-runs absorb the noise.

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) and
[`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) for the full
text.

## What's deferred from v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](specs/SPEC.pgRDF.LLD.v0.4.md):

- CONSTRUCT (§6) — v0.4.4
- Property paths (§7) — v0.4.5
- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11) —
  v0.4.6
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

These land in subsequent v0.4.x point releases or in a refreshed
v0.5.0 cut.

## Upgrading from v0.4.2

pgRDF v0.x reserves the right to break schema between minor
releases. `ALTER EXTENSION pgrdf UPDATE` is not supported in
v0.x. Drop and recreate:

```sql
-- Dump first if you care about your data
DROP EXTENSION pgrdf CASCADE;
-- Install v0.4.3 artifacts
CREATE EXTENSION pgrdf;
-- Re-ingest
```

The schema is forward-compatible at the table-shape level
(v0.4.2's `_pgrdf_graphs`, `_pgrdf_quads`, `_pgrdf_dictionary`
are unchanged in v0.4.3); only new UDF dispatch arms land. A
`pg_dump` from v0.4.2 will restore against a v0.4.3 install via
the documented `DROP/CREATE EXTENSION; pg_restore` path. See
[`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions).

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
