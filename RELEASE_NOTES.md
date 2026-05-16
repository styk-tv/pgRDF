# pgRDF v0.4.5

**Full SPARQL 1.1 property paths.** The LLD v0.4 §7 property-path
column closes: `^` inverse, `+` one-or-more, `*` zero-or-more, `?`
zero-or-one, and `|` alternation all execute end-to-end on the SQL
engine. Phase E lands across a four-group countdown (49 → 35) on
top of v0.4.4's CONSTRUCT surface, plus the v0.4.5 release cut.

## Marquee — SPARQL property paths (LLD v0.4 §7)

A property path lets one triple pattern match a *route* through
the graph instead of a single edge. pgRDF recognises
`GraphPattern::Path` at the single chokepoint every query form
routes through — SELECT, ASK, `pgrdf.construct`, and the UPDATE
WHERE bodies all inherit path support at once (it is not
special-cased per consumer). Paths compose with named-graph
scoping (`GRAPH <iri>` / `GRAPH ?g`), multi-pattern BGP joins, and
OPTIONAL/UNION/MINUS for free.

### Operator surface

| Operator | SPARQL | Semantics |
|---|---|---|
| `^` inverse | `?s ^p ?o` | `?o p ?s` — subject/object swap, no recursion; `^(^p)` folds by parity |
| `+` one-or-more | `?s p+ ?o` | transitive closure (non-reflexive) — recursive CTE, cycle-safe, depth-guarded |
| `*` zero-or-more | `?s p* ?o` | reflexive transitive closure — the `+` walk `UNION` the W3C §9.3 zero-length node-set |
| `?` zero-or-one | `?s p? ?o` | equal-or-linked — non-recursive: the direct edge `UNION` the same zero-length node-set |
| `\|` alternation | `?s (a\|b) ?o` | per-predicate union (non-reflexive single step); n-ary `a\|b\|c`; `(a\|b)+`/`(a\|b)*`/`(a\|b)?`; `^(a\|b)`/`(^a\|^b)` |

### Recursive-path engine

Recursive operators lower to a `WITH RECURSIVE walk(src, dst,
depth)` CTE as a derived FROM relation (it exposes the same
`subject_id` / `object_id` columns a quad alias does, so it joins
through the unchanged BGP machinery). Cycle-safety uses Postgres's
`CYCLE src, dst SET is_cycle USING path` clause (PG14+) — a bare
`UNION` cannot dedup a cycle once the working tuple carries the
`depth` column for the guard. The `pgrdf.path_max_depth` GUC
(Userset, default 64, range 1–1024) caps the walk: a traversal
past the cap is **truncated, not errored**, and
`pgrdf.stats()->>'path_depth_truncations'` accounts a genuine
acyclic cap-hit (a fully-resolved cyclic query correctly reports
no truncation).

### W3C §9.3 zero-length-path semantics

`*` and `?` carry the precise W3C SPARQL 1.1 §9.3 `ZeroLengthPath`
rules (not the LLD §7.2 `SELECT ?s ?s` simplification). A **bound**
endpoint's self-pair `(x,x)` holds unconditionally — even when the
queried IRI is in no graph (pgRDF registers it as a term
reference; **no quad is added**). An **unbound** endpoint's
node-set is the DISTINCT subject∪object of the active scope,
scoped to the active `GRAPH`.

### `|` alternation (the §7.1 stretch — shipped in full)

LLD §7.1 marked alternation a gated stretch goal. The gate is
lifted entirely: every recursive/optional builder already
centralised the single `predicate_id = $P` clause, so generalising
it to a predicate **set** (`predicate_id IN (…)` — the LLD §7.2
"union of per-predicate scans" as one scan; a 1-element set is
byte-identical, semantically and to the planner, to the old
`= $P`) was a uniform one-line change at each site, not a
translator balloon. Consequently the recursion compositions
`(a|b)+` / `(a|b)*` / `(a|b)?` ship too (the alternation becomes
the recursive step's predicate set; the depth guard, the CYCLE
clause, the truncation probe, and the zero-length node-set are all
predicate-match-agnostic and reused verbatim), as does the inverse
`^(a|b)` / `(^a|^b)`.

The §7.1-permitted **gated remainder** (still preview-panics, by
spec allowance — not a regression): an alternation arm that is
itself a sequence/recursive path (`(a/b|c)`, `(a+|b)`), or a
recursive operator whose inner box is a sequence (`(p1/p2)+`).
Folding these would compose a recursive CTE inside an alternation
arm — the genuine translator balloon §7.1 explicitly permits
gating. Negated property sets (`!(...)`) remain out of v0.4 scope.

### Materialised-closure no-CTE fallback (§7.2 / §7.3)

When `pgrdf.materialize(graph_id)` has already entailed the
transitive closure of a path's predicate, a recursive CTE is
wasted work — every transitive pair is already a direct
`is_inferred = TRUE` edge. For a `+`/`*` over a **single**
well-known transitive predicate (`rdfs:subClassOf`,
`rdfs:subPropertyOf`, `owl:sameAs`), the translator probes for a
materialised row and, if present, emits a **direct match instead
of the recursive CTE** — the executed plan carries no `CTE Scan`
(§7.3 acceptance, EXPLAIN-scraped via the new `pgrdf.sparql_sql(q)
→ TEXT` debug hook). The result set is byte-identical to the
non-materialised recursive walk; the optimisation is
semantics-preserving and per-query (not cached). `?`/`^`/`|` are
unaffected (no recursion to elide); a multi-predicate `(a|b)+`
skips the fallback (single-well-known-predicate heuristic).

## Phase E slice attribution (countdown 49 → 35)

- **E1 (49 → 46)** — property-path AST detection + translator
  dispatch; `^` inverse fully supported. New GUC
  `pgrdf.path_max_depth`; `pgrdf.stats().path_depth_truncations`
  scaffold (enforcement lands E2).
- **E2 (45 → 42)** — `+` one-or-more recursive CTE, cycle-safe via
  the `CYCLE` clause, depth guard enforced. All property-path SQL
  generation carved into `src/query/path.rs`.
- **E3 (41 → 38)** — `*` / `?` with full W3C §9.3 zero-length
  semantics; inverse composition (`^(p*)` etc.).
- **E4 (37 → 35)** — `|` alternation (incl. the recursion
  compositions and inverse) via the predicate-set generalisation;
  materialised-closure no-CTE fallback + the `pgrdf.sparql_sql`
  debug hook; Phase E W3C-shape consolidation; the v0.4.5 cut.

## Test bar

```
pgrx integration  230  (was 222 at v0.4.4 / Phase E3)
pg_regress         73  (property-path coverage 108–111)
w3c-sparql         41  (was 35 — +6 property-path fixtures
                        36-path-inverse … 41-path-materialised)
LUBM-shape          3  (unchanged)
Total: 347 green, plus the pg_dump round-trip gate.
```

All hand-computed; no `ACCEPT=1` autobaselining of new query
coverage.

## ERRATA

- **E-006** — pgrx 0.17+/0.18 do not build on current rustc;
  pinned to PG 17 + pgrx 0.16 (carried).
- **E-010** — cargo audit informational advisories (carried).
- **E-011** — `reasonable` rdf-12 passthrough patch carried; the
  `publish-crate.yml` workflow stays **disabled** until upstream
  [`gtfierro/reasonable#50`](https://github.com/gtfierro/reasonable/pull/50)
  merges. The v0.4.5 tag fires `release.yml` only (8 platform
  tarballs PG14-17 × amd64/arm64 + SHA256SUMS); **no crates.io
  publish this cut**.

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) and
[`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) for the full text.

## What's deferred from v0.4 LLD

Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](specs/SPEC.pgRDF.LLD.v0.4.md):

- SPARQL surface backlog — multi-triple OPTIONAL, VALUES,
  BIND-downstream, aggregates over UNION, DESCRIBE (§11)
- `heap_multi_insert` / `COPY BINARY` ingest (§12 phase B)
- W3C SPARQL 1.1 manifest runner (§13)

The §7.1-permitted property-path gated remainder (sequence-arm
alternation / sequence-inner recursive) and negated property sets
(`!(...)`) are intentionally out of v0.4 scope. These land in
subsequent v0.4.x point releases or in a refreshed v0.5.0 cut.

## Upgrading from v0.4.4

pgRDF v0.x reserves the right to break schema between minor
releases. `ALTER EXTENSION pgrdf UPDATE` is not supported in
v0.x. Drop and recreate:

```sql
-- Dump first if you care about your data
DROP EXTENSION pgrdf CASCADE;
-- Install v0.4.5 artifacts
CREATE EXTENSION pgrdf;
-- Re-ingest
```

The schema is forward-compatible at the table-shape level
(v0.4.4's `_pgrdf_graphs`, `_pgrdf_quads`, `_pgrdf_dictionary`
are unchanged in v0.4.5); only one new debug UDF lands
(`pgrdf.sparql_sql`). A `pg_dump` from v0.4.4 will restore
against a v0.4.5 install via the documented `DROP/CREATE
EXTENSION; pg_restore` path. See
[`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions).

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
