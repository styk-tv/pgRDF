# pgRDF v0.4.6

**The §11 SPARQL surface backlog is complete.** The v0.3-deferred
SPARQL forms — multi-triple OPTIONAL, VALUES inline tables,
downstream BIND, aggregates over UNION, DESCRIBE, and **type-aware
ORDER BY** — all execute end-to-end on the SQL engine. Phase F lands
across a four-group countdown (34 → 22) on top of v0.4.5's full
property-path surface, plus the v0.4.6 release cut.

## Marquee — type-aware ORDER BY (LLD v0.4 §11, SPARQL 1.1 §15.1)

The last §11 item. Before v0.4.6, `ORDER BY` emitted a single
lexical-string compare over `_pgrdf_dictionary.lexical_value`, so
xsd-typed numeric literals sorted as text — `"1","10","100","2"`
instead of the value order `1,2,10,100`. v0.4.6 expands every sort
key into the SPARQL 1.1 §15.1 value-space term list:

- a leading **kind rank** — numeric < `xsd:dateTime` < `xsd:boolean`
  < everything else — groups comparable lexical spaces together so
  value comparison is meaningful within a group and the cross-type
  order is the stable rank;
- then a per-kind comparator: numerics compared **numerically**
  (`2 < 10`), `xsd:dateTime` **chronologically**, `xsd:boolean`
  `false < true`, strings / plain / lang-tagged by **Unicode
  codepoint** (`COLLATE "C"`, locale-independent);
- then a final codepoint tiebreak.

The numeric/dateTime casts are regex-guarded, so a malformed lexical
never raises — it falls through to the codepoint tier (the
§15.1-sanctioned stable fallback). `ORDER BY` is therefore **total
and never raises on incomparable operands** — distinct from `<`
inside FILTER, which can error. `DESC()` reverses; multi-key
(`ORDER BY ?a DESC(?b)`) composes; **expression sort keys**
(`ORDER BY (?a + ?b)`, `ORDER BY STRLEN(?s)`) translate through the
shared BIND/FILTER expression translator.

```sql
-- xsd:integer literals now sort NUMERICALLY: 1, 2, 10, 100
SELECT * FROM pgrdf.sparql(
  'PREFIX ex: <http://example.com/>
   SELECT ?n WHERE { ?s ex:n ?n } ORDER BY ?n');

-- DESC + an expression sort key
SELECT * FROM pgrdf.sparql(
  'PREFIX ex: <http://example.com/>
   SELECT ?s WHERE { ?x ex:s ?s } ORDER BY DESC(STRLEN(?s))');
```

All four SQL builders (single-branch, aggregate, UNION,
aggregate-over-UNION) order over the underlying SQL expression
(group/aggregate expr, dict-lookup, or BIND expr) — never an output
alias buried in an expression (Postgres rejects that). `SELECT
DISTINCT` + ORDER BY wraps the dedup in an outer derived table so
the §15.1 terms run over the deduplicated columns. An expression
sort key combined with an aggregate / UNION / aggregate-over-UNION
query is a documented narrow deferral (bind it with `BIND(... AS
?k)` then `ORDER BY ?k`) — a stable panic, never a wrong answer.

## §11 SPARQL backlog — the full Phase F surface (v0.4.6)

The complete §11 surface, shipped across the Phase F countdown:

| Form | Group | Surface |
|---|---|---|
| Multi-triple `OPTIONAL { BGP }` | F1 | N-triple right side as a LATERAL-style derived table inside the LEFT JOIN; **atomic** (all-or-nothing, W3C §6.1); nested OPTIONAL, OPTIONAL-internal FILTER, optional-var outer FILTER, GRAPH scoping, `+`-path-in-required compose |
| `VALUES` inline tables | F1 | `(VALUES …) AS vN(cols)` derived table joined on shared vars; constants → dict ids ahead of execution; `UNDEF` → NULL no-constraint cell (W3C §10); typed/lang literals datatype-aware |
| Downstream `BIND` | F2 | AST substitution pass: a BIND var is rewritten into a later FILTER / BGP join key / chained BIND **before** the structural walk; unbound-var BIND → NULL not error (W3C §18.2.5) |
| Aggregates over `UNION` | F2 | derived-table refactor; COUNT/SUM/AVG/type-aware MIN-MAX/GROUP_CONCAT/SAMPLE, DISTINCT, GROUP BY, HAVING, GRAPH scoping, property-path branch |
| `DESCRIBE` | F3 | sibling UDF `pgrdf.describe(q TEXT) → SETOF JSONB` (byte-identical to `pgrdf.construct`); W3C §16.4 closure, transitive one-hop blank-node expansion, cycle-safe, dedup'd |
| Type-aware `ORDER BY` | F4 | SPARQL 1.1 §15.1 value-space ordering (this cut's marquee) |

Every form composes with GRAPH scoping and is inherited by
`pgrdf.construct` + SPARQL UPDATE WHERE (shared BGP walker).
`pgrdf.sparql_parse` no longer flags any of OPTIONAL / VALUES /
BIND-downstream / aggregate-over-UNION / DESCRIBE in
`unsupported_algebra` (the LLD §11 acceptance binding; ORDER BY was
already an unflagged SELECT modifier).

## Engine surface delta vs v0.4.5

- **Storage / OWL 2 RL inference / SHACL / SPARQL UPDATE /
  CONSTRUCT / property paths** — unchanged; no breaking changes to
  existing surfaces. No new user-facing UDF this cut (type-aware
  ORDER BY is a translator change behind the existing
  `pgrdf.sparql`; `pgrdf.describe` shipped in F3).
- **SPARQL §11 backlog** — **complete end-to-end across the Phase F
  countdown 34 → 22** (F1 OPTIONAL/VALUES, F2 BIND-downstream +
  aggregates-over-UNION, F3 DESCRIBE, F4 type-aware ORDER BY + the
  Phase F W3C-shape consolidation + this release cut).
- **Compose infra-debt fix** — `compose/compose.yml` collapsed the
  five stale per-version SQL bind-mount lines (`pgrdf--0.4.1.sql`
  … `pgrdf--0.4.5.sql`) to a single per-file mount of the current
  `default_version`'s `pgrdf--<ver>.sql`. A clean
  `cargo pgrx package` emits exactly that one file (the older
  lines only ever resolved from a warm BuildKit cache — the source
  of the recurring hand-create-a-copy + cold-restart workaround).
  Per-file (not a directory mount, which would shadow the stock
  Postgres extension dir and break `initdb` on a fresh cluster);
  a release cut now changes just this one line.

## Test bar

```
pgrx integration  250  (was 248 at v0.4.5 / Phase F3 — +2 type-aware
                        ORDER BY tests)
pg_regress         79  (was 78 — +1 100-sparql-order-by-type-aware;
                        111 expected corrected to §15.1 codepoint
                        order)
w3c-sparql         47  (was 41 — +6 Phase F fixtures
                        42-optional-multi-triple …
                        47-order-by-type-aware)
LUBM-shape          3  (unchanged)
Total: 379 green, plus the pg_dump round-trip gate.
```

All hand-computed; no `ACCEPT=1` autobaselining of new query
coverage. The `111` property-path closure expected output was
corrected to the SPARQL 1.1 §15.1 codepoint order (uppercase IRIs
now sort before lowercase, as the spec mandates — a deliberate,
documented behaviour correction).

## ERRATA

- **E-006** — pgrx 0.17+/0.18 do not build on current rustc;
  pinned to PG 17 + pgrx 0.16 (carried).
- **E-010** — cargo audit informational advisories (carried).
- **E-011** — `reasonable` rdf-12 passthrough patch carried; the
  `publish-crate.yml` workflow stays **disabled** until upstream
  [`gtfierro/reasonable#50`](https://github.com/gtfierro/reasonable/pull/50)
  merges. The v0.4.6 tag fires `release.yml` only (8 platform
  tarballs PG14-17 × amd64/arm64 + SHA256SUMS); **no crates.io
  publish this cut**.

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) and
[`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) for the full text.

## What's deferred from v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](specs/SPEC.pgRDF.LLD.v0.4.md):

- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)

Residual aggregate-over-UNION refinements (a GROUP BY / aggregate
argument on a variable that is ONLY a `GRAPH ?g`-scope var across
the union; a computed BIND as a triple join key; a BIND var in a
CONSTRUCT template output position) are tracked — not lost — in
[`SPEC.pgRDF.LLD.v0.5-FUTURE §8`](specs/SPEC.pgRDF.LLD.v0.5-FUTURE.md):
each is a stable panic, never a wrong answer. The
§7.1-permitted property-path gated remainder (sequence-arm
alternation / sequence-inner recursive) and negated property sets
(`!(...)`) remain out of v0.4 scope. The reasoning-profile selector
and TriG / N-Quads ingest are the next milestone (Phase G →
v0.5.0-rc1).

## Upgrading from v0.4.5

pgRDF v0.x reserves the right to break schema between minor
releases. `ALTER EXTENSION pgrdf UPDATE` is not supported in
v0.x. Drop and recreate:

```sql
-- Dump first if you care about your data
DROP EXTENSION pgrdf CASCADE;
-- Install v0.4.6 artifacts
CREATE EXTENSION pgrdf;
-- Re-ingest
```

The schema is forward-compatible at the table-shape level
(v0.4.5's `_pgrdf_graphs`, `_pgrdf_quads`, `_pgrdf_dictionary`
are unchanged in v0.4.6); no new tables or UDFs land this cut
(type-aware ORDER BY is internal to the query translator). A
`pg_dump` from v0.4.5 will restore against a v0.4.6 install via
the documented `DROP/CREATE EXTENSION; pg_restore` path. See
[`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions).

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
