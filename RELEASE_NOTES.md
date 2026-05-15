# pgRDF v0.4.4

**SPARQL 1.1 CONSTRUCT surface complete.** The LLD v0.4 §6
CONSTRUCT column closes: the full query form lands end-to-end on
the SQL engine. Phase D closes in nine countdown slices (59 → 51)
on top of v0.4.3's SPARQL UPDATE surface, plus the v0.4.4 release
cut (slice 50).

## Marquee — SPARQL CONSTRUCT (LLD v0.4 §6)

`pgrdf.construct(q TEXT) → SETOF JSONB` is a sibling UDF to
`pgrdf.sparql` — callers signal CONSTRUCT intent at the SQL
boundary rather than overloading the SELECT/ASK/UPDATE entry
point. It evaluates the WHERE pattern through the existing
SELECT-side translator (`parse_select` → `build_bgp_sql` →
`execute`), instantiates the template once per WHERE solution, and
emits one JSONB row per template triple. Each row carries the
structured term shape `{"type": "iri"|"literal"|"bnode", "value":
…, "datatype"?: …, "language"?: …}` documented in LLD v0.4 §6.1.

### Template surface

- **Constant templates** (slice 59) — ground-triple templates per
  W3C SPARQL 1.1 §16.2 emit one row per WHERE solution per
  template triple. DISTINCT / ORDER BY / GROUP BY / aggregate
  wrappings on CONSTRUCT are explicitly rejected at execute time
  (out of scope per LLD §6.2).
- **Variable substitution** (slice 58) — variables in subject,
  predicate, and object positions resolve per solution (RDF
  admits variable predicates in templates). Unbound template
  variables panic with the stable `unbound template variable ?X`
  message. Typed and language-tagged literals carry the full
  structured shape; language-tagged literals carry both the
  `language` field AND the implicit `rdf:langString` datatype IRI
  per RDF 1.1 §3.3.
- **Blank-node templates** (slice 57) — `_:label` in template
  positions mints a fresh per-solution label per W3C SPARQL 1.1
  §16.2; the same template label across positions in one solution
  joins to the same fresh label. Per-call fresh labels carry the
  solution index as a prefix (`b{solution}_{n}`) so the `value`
  column alone distinguishes per-solution bnodes. Predicate-
  position blank nodes are illegal RDF and reject at parse time.
- **Multi-triple templates** (slice 56) — an N-triple template
  emits N rows per solution; blank-node labels are SHARED across
  all N triples within the same solution (and fresh per solution).
  Empty templates `{ }` panic with `empty template`.

### CONSTRUCT WHERE shorthand (slice 54)

`CONSTRUCT WHERE { pattern }` is equivalent to `CONSTRUCT {
pattern } WHERE { pattern }` per W3C SPARQL 1.1 §16.2.4. The
pattern must be a pure BGP (no OPTIONAL / UNION / MINUS / FILTER /
GRAPH / BIND / VALUES) and must contain no blank nodes; both
restrictions panic with explicit W3C-citing messages. Spargebra
populates `template` from the BGP at parse so the shorthand reuses
the multi-triple emission path.

### GRAPH-scoped WHERE (slice 55)

`WHERE { GRAPH <iri> { … } }` (literal) and `WHERE { GRAPH ?g { …
} }` (variable) compose with every template surface. Variable
GRAPH binds `?g` to the source graph IRI per solution; default-
graph quads are excluded per W3C SPARQL 1.1 §13.3 — the JOIN to
`_pgrdf_graphs` now carries `g{S}.graph_id <> 0`, which also
corrected a latent slice-79 / slice-87 SELECT-side bleed (variable
GRAPH previously bound `?g` to `urn:pgrdf:graph:0` when default-
graph quads coexisted with named-graph quads). Empty named graphs
and missing graphs yield zero solutions.

### Round-trip ingest (slice 53)

`pgrdf.put_construct_row(row JSONB, graph_id BIGINT DEFAULT 0) →
BIGINT` and `pgrdf.put_construct_rows(rows JSONB[], graph_id
BIGINT DEFAULT 0) → BIGINT` re-ingest any construct rowset back
into the hexastore, closing LLD v0.4 §6.3's round-trip acceptance
criterion. Typed literals, language tags, plain strings (with the
explicit `xsd:string` datatype the construct emitter writes), and
within-solution blank-node joining are all preserved. The plural
form is the recommended surface: a per-call `HashMap<String, i64>`
of blank-node labels keeps repeated bnode references within one
batch collapsed onto a single stored blank node. Re-ingestion is
idempotent (`WHERE NOT EXISTS`, mirroring `executor::insert_quad`);
a NULL array (from `array_agg` over an empty construct rowset) is
a no-op, so the `(SELECT array_agg(j) FROM pgrdf.construct(…))`
idiom works for empty-result queries too.

### Preview surface — `pgrdf.sparql_parse` (slice 52)

`pgrdf.sparql_parse(q)` mirrors the executor's CONSTRUCT
classification:

```jsonc
{
  "form": "CONSTRUCT",
  "shorthand": false,
  "template": {
    "triple_count":        2,
    "has_variables":       true,
    "has_blank_nodes":     true,
    "has_constants_only":  false,
    "variables":           ["s", "o"]
  },
  "where_shape": {
    "kind":              "Bgp",
    "triple_count":      1,
    "named_graphs_used": [],
    "variables":         ["s", "o"]
  },
  "unsupported_algebra": []
}
```

Callers preview translatability without executing.
`unsupported_algebra` flags `Distinct` / `OrderBy` / `Group` /
`Aggregate` wrappings — `pgrdf.construct` panics on these at
execute time per LLD §6.2.

### Conformance (slice 51)

Six new W3C-shape CONSTRUCT fixtures under `tests/w3c-sparql/30-35`
lock the surface through the conformance harness: basic bnode +
var + multi-triple §16.2.1, WHERE shorthand §16.2.4, constant-
template multiplicity §16.2, variable-GRAPH §13.3, typed/lang-
literal term shaping, and round-trip via `pgrdf.put_construct_rows`.
The harness gained a surgical per-fixture `kind: construct`
selector routing through `pgrdf.construct` instead of
`pgrdf.sparql`. A docs / spec / guide coherence sweep landed
alongside.

### CI-perf hardening (alongside Phase D)

The partition-DDL window in the SPARQL UPDATE / lifecycle paths now
takes a statement-outermost transaction advisory lock, so the
default parallel pgrx-test scheduler no longer flakes on
concurrent partition DDL. Parallel test threads are restored — the
test bar below is verified at default parallelism (no
`--test-threads=1`).

### Error-prefix contract (stable for downstream tooling)

CONSTRUCT-specific surfaces surface validation failures with
form-prefixed panic messages:

```
unbound template variable ?<name>
empty template
pgrdf.construct: parse error: <…>           (covers malformed CONSTRUCT, predicate-position bnodes)
pgrdf.put_construct_row: <…>                (negative graph_id, literal in subject/predicate position)
```

DISTINCT / ORDER BY / GROUP BY / aggregate on CONSTRUCT, and the
shorthand-form BGP / blank-node restrictions, panic with explicit
W3C-citing messages.

## Test bar

301 automated tests across four layers plus the pg_dump
round-trip gate:

| Layer | Count | Δ from v0.4.3 |
|---|---|---|
| pgrx integration | 194 | +28 |
| pg_regress golden | 69 | +8 |
| W3C-shape SPARQL conformance | 35 | +6 |
| LUBM-shape correctness | 3 | 0 |
| **Total** | **301** | **+42** |

Plus `tests/regression/scripts/pg-dump-roundtrip.sh` end-to-end
round-trip gate on `_pgrdf_graphs`.

## Install — prebuilt tarballs (same layout as v0.4.3)

```bash
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.4/pgrdf-0.4.4-pg17-glibc-amd64.tar.gz
curl -L -O https://github.com/styk-tv/pgRDF/releases/download/v0.4.4/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing
tar -xzf pgrdf-0.4.4-pg17-glibc-amd64.tar.gz
cd pgrdf-0.4.4-pg17-glibc-amd64
sudo cp lib/pgrdf.so $(pg_config --pkglibdir)/
sudo cp share/extension/* $(pg_config --sharedir)/extension/
```

Then in psql:

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version();  -- → 0.4.4
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

v0.4.4 is **not** published to crates.io. The `[patch.crates-io]`
block for `reasonable` (E-011) blocks `cargo publish`; the
`publish-crate.yml` workflow remains disabled until upstream
[gtfierro/reasonable#50](https://github.com/gtfierro/reasonable/pull/50)
merges and the patch retires. The crate is registered on
crates.io with v0.3.0 (pre-work seed); v0.4.1 + v0.4.2 + v0.4.3 +
v0.4.4 binaries are available via the GitHub Release tarballs —
use those, not `cargo install`.

## Known issues — carried from v0.4.3

- **E-011 — `[patch.crates-io]` fork-dep still in place.**
  Carried. v0.4.4 continues to patch `reasonable` against
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

### v0.4.2-introduced — resolved in v0.4.4

- **pgrx-tests parallelism flake on partition DDL.** The two Phase
  A tests (`pg_add_graph_iri_idempotent`,
  `pg_add_graph_id_iri_synthetic_upgrade`) that occasionally raced
  under pgrx-tests 0.16's parallel scheduler are now stable: the
  partition-DDL window takes a statement-outermost transaction
  advisory lock (CI-perf hardening, this release), so concurrent
  partition DDL serialises and the default parallel test threads
  are restored.

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) and
[`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) for the full
text.

## What's deferred from v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](specs/SPEC.pgRDF.LLD.v0.4.md):

- Property paths (§7) — v0.4.5
- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11) —
  v0.4.6
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

These land in subsequent v0.4.x point releases or in a refreshed
v0.5.0 cut.

## Upgrading from v0.4.3

pgRDF v0.x reserves the right to break schema between minor
releases. `ALTER EXTENSION pgrdf UPDATE` is not supported in
v0.x. Drop and recreate:

```sql
-- Dump first if you care about your data
DROP EXTENSION pgrdf CASCADE;
-- Install v0.4.4 artifacts
CREATE EXTENSION pgrdf;
-- Re-ingest
```

The schema is forward-compatible at the table-shape level
(v0.4.3's `_pgrdf_graphs`, `_pgrdf_quads`, `_pgrdf_dictionary`
are unchanged in v0.4.4); only new UDFs land
(`pgrdf.construct`, `pgrdf.put_construct_row`,
`pgrdf.put_construct_rows`). A `pg_dump` from v0.4.3 will restore
against a v0.4.4 install via the documented `DROP/CREATE
EXTENSION; pg_restore` path. See
[`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions).

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
