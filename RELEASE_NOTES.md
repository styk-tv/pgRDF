# pgRDF v0.5.0-rc1

**Release candidate — all v0.5-FUTURE v0.5-gate tracks (§3-§8)
complete.** Phase G closes the v0.5 capability scope across three
grouped dispatches (G1 → G3, countdown 21 → 12) on top of the v0.4
cycle. This is a **release candidate**: the final **v0.5.0** follows
after Phase H+I hygiene + the two documented honest follow-ups
(ERRATA.v0.5 E-012, E-013). An rc tag is flagged a GitHub
*prerelease* so it does not supersede v0.4.6 as "latest".

## Headline — the full v0.5 capability surface

Everything the v0.5-FUTURE LLD gates (§3 through §8) ships:

- **Named-graph scoping + IRI map** (v0.4.1) —
  `GRAPH <iri>`/`GRAPH ?g`, `_pgrdf_graphs`, `pg_dump` round-trip.
- **SPARQL UPDATE** (v0.4.3) — INSERT/DELETE DATA, INSERT/DELETE
  WHERE, DELETE-INSERT-WHERE, graph-scoped, the lifecycle algebra.
- **Graph lifecycle UDFs** (v0.4.2) — `drop/clear/copy/move_graph`,
  BIGINT + (v0.5) IRI-keyed overloads.
- **SPARQL CONSTRUCT** (v0.4.4) — constant/variable/blank-node/
  multi-triple templates, `CONSTRUCT WHERE` shorthand, round-trip.
- **Property paths** (v0.4.5) — `^` `+` `*` `?` `|`, recursive
  `WITH RECURSIVE` lowering, materialised-closure fast path.
- **§11 SPARQL backlog** (v0.4.6) — multi-triple OPTIONAL, VALUES,
  downstream BIND, aggregates over UNION, DESCRIBE, type-aware
  ORDER BY (SPARQL 1.1 §15.1 value-space).
- **Reasoning-profile selector** (v0.5, Phase G G1) —
  `pgrdf.materialize(g, profile TEXT DEFAULT 'owl-rl')`; `'rdfs'`
  adds a strict, sound, complete RDFS rule subset; unknown profiles
  error. Closes the last ONTOSYS P1 capability gap.
- **IRI lifecycle overloads** (v0.5, Phase G G1) —
  `pgrdf.{drop,clear,copy,move}_graph(iri TEXT, …)`.
- **TriG / N-Quads ingest** (v0.5, Phase G G2) —
  `pgrdf.parse_trig`, `pgrdf.parse_nquads` honour inline /
  4th-position graph IRIs, reusing the batched-insert path.
- **Aggregates-over-UNION residuals** (v0.5, Phase G G2) — the six
  F2 stable panics are lifted (correct answers).
- **SHACL `mode` argument** (v0.5, Phase G G3) —
  `pgrdf.validate(data, shapes, mode TEXT DEFAULT 'native')`; the
  JSONB gains a `mode` field; unknown modes error; validation
  against a materialised data graph reports violations against
  entailed triples.
- **W3C SHACL Core manifest gate** (v0.5, Phase G G3) — a vendored,
  hermetic W3C SHACL Core subset, genuine 25/25 full-pass (no
  exclusion), wired into CI on every PG major.

## Phase G group G3 — SHACL `mode` + W3C SHACL Core gate

### §5 — `pgrdf.validate(data, shapes, mode TEXT DEFAULT 'native')`

The `mode` argument ships fully: accepted, validated, echoed in a
new JSONB `mode` field. The 2-arg `pgrdf.validate(d, s)` form
defaults `'native'` and is byte-identical to the v0.4 surface — no
regression. An unknown mode raises
`validate: unknown mode "<x>" (supported: 'native', 'sparql')`
**before** any work (no silent fallback — mirrors §3's
`materialize: unknown profile`).

```sql
SELECT pgrdf.validate(data_g, shapes_g);            -- 'native' (default)
SELECT pgrdf.validate(data_g, shapes_g, 'native');  -- explicit
SELECT pgrdf.validate(data_g, shapes_g, 'sparql');  -- see E-012
```

**Validation against a materialised graph (§5.3 #2 — fully met).**
`pgrdf.materialize` then `pgrdf.validate` validates the entailed
closure: a shape requiring chain membership reaches a focus node
bound ONLY by RDFS entailment (regression `122-shacl-modes.sql` §E
+ pgrx `validate_materialised_graph_entailed`).

**`'sparql'` mode — honest scope (ERRATA.v0.5 E-012).** `shacl
0.3.1` has no SHACL-SPARQL constraint component (the parser
silently drops `sh:sparql`) **and** its `SparqlEngine` is an
upstream stub — `unimplemented!()` in every target-resolution
method, so invoking it panics. pgRDF does **not** invoke the broken
engine; `'sparql'` returns a clean, deterministic structured report
(`conforms:null` + an `error` naming the gap + E-012), never a
panic. The surface is forward-compatible: the day a rudof release
ships the engine, one guard is deleted and `'sparql'` routes
through with no signature change.

### §6 — W3C SHACL Core manifest gate

New `just test-shacl-manifest` harness (`tests/w3c-shacl/`,
structured like `tests/w3c-sparql/`). A vendored subset of the W3C
`data-shapes-test-suite` SHACL Core tests — hermetic (checked in,
never fetched at test time). Wired into `ci.yml` on every PG major
(14-17) as a **real gate** (no `continue-on-error`/`if:false`).

The vendored W3C SHACL **Core** suite is a genuine **full-pass —
25 / 25** on the W3C `sh:conforms` invariant, **with no exclusion**.
Per ERRATA.v0.5 **E-013** the gate compares `conforms` (not the
violation *count*, which drifts ±1 from pgRDF's blank-node-relabelling
dictionary rehydrate for blank-node *focus* nodes — a serialization
artifact that does not flip conformance, the same reason focus-node
IRIs are excluded). E-013's earlier "`prop-nodeKind-001`
documented-excluded for a true upstream `sh:nodeKind` bug" claim was
a **G3 unverified assumption** (the fixture was committed straight
into `fixtures/excluded/`, so `run.sh` — which globs only
`fixtures/core/*.ttl` — never ran it); a triple-verified investigation
at v0.5.0-rc1 found **no upstream bug**: pgRDF produces the
W3C-authoritative `conforms:false` / 27 violations, and the fixture
is now graded in `fixtures/core/` and PASSing. No fork, no MR, no
`[patch.crates-io]`. `--sparql` asserts the E-012 known state
(`conforms:null` for every fixture).

## Test bar

```
pgrx integration  274  (was 270 — +4 §5 SHACL-mode tests)
pg_regress         85  (was 84  — +1: 122-shacl-modes)
w3c-sparql         51  (was 47  — +4 Phase G fixtures
                        48-reasoning-profile-rdfs …
                        51-nquads-loaded)
w3c-shacl Core      25  (NEW — vendored W3C SHACL Core gate,
                        genuine 25/25 full-pass on sh:conforms,
                        no exclusion; E-013)
LUBM-shape          3  (unchanged)
Total: 438 green across six layers, plus the pg_dump round-trip
gate and the w3c-shacl --sparql E-012 known-state assertion.
```

All hand-computed / hand-derived; no `ACCEPT=1` autobaselining of
new query or SHACL coverage. The W3C SHACL expecteds are
hand-derived from each fixture's W3C `mf:result` block.

## ERRATA

- **E-006** — pgrx 0.17+/0.18 do not build on current rustc;
  pinned to PG 17 + pgrx 0.16 (carried).
- **E-011** — `reasonable` rdf-12 passthrough patch carried; the
  `publish-crate.yml` workflow stays **disabled** until upstream
  [`gtfierro/reasonable#50`](https://github.com/gtfierro/reasonable/pull/50)
  merges. The v0.5.0-rc1 tag fires `release.yml` only (8 platform
  tarballs PG14-17 × amd64/arm64 + SHA256SUMS); **no crates.io
  publish this cut**.
- **E-012** (new, v0.5) — `shacl 0.3.1` SHACL-SPARQL mode is a
  documented upstream-gate (no constraint component +
  `unimplemented!()` engine; upstream's own roadmap, rudof issues
  #21/#94/#1). The `mode` arg ships forward-compatible; `'sparql'`
  returns a deterministic structured report. Final for v0.5.0 as a
  documented limitation, NOT a pgRDF defect.
- **E-013** (new, v0.5) — **corrected/resolved**. The W3C SHACL Core
  gate uses the `sh:conforms` invariant; its earlier "one W3C Core
  fixture `prop-nodeKind-001` documented-excluded for an upstream
  `sh:nodeKind` bug" claim was a G3 unverified assumption (the
  fixture was committed straight into `fixtures/excluded/` so the
  harness never ran it). A triple-verified investigation at
  v0.5.0-rc1 found no upstream bug; the fixture is restored to
  `fixtures/core/` and W3C SHACL Core is a **genuine 25/25 full-pass,
  no exclusion**. No fork/MR/`[patch.crates-io]`.

See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md),
[`specs/ERRATA.v0.4.md`](specs/ERRATA.v0.4.md) and
[`specs/ERRATA.v0.5.md`](specs/ERRATA.v0.5.md) for the full text.

## Release candidate — what follows

This is a **release candidate**. The v0.5-FUTURE v0.5-gate scope
(§3-§8) is COMPLETE. The final **v0.5.0** follows after **Phase
H+I**: final hygiene, ERRATA close-outs, and the `executor.rs`
core-BGP carve catch-up. E-013 is **resolved** (no upstream bug;
§6 W3C SHACL Core is a genuine 25/25 full-pass). E-012 stays a
documented upstream-gate (the SHACL-SPARQL upstream engine), final
for v0.5.0. Still 🚧 in
[`SPEC.pgRDF.LLD.v0.4.md`](specs/SPEC.pgRDF.LLD.v0.4.md):
`heap_multi_insert` / `COPY BINARY` ingest (§12 phase B).

## Upgrading from v0.4.6

pgRDF v0.x reserves the right to break schema between minor
releases. `ALTER EXTENSION pgrdf UPDATE` is not supported in
v0.x. Drop and recreate:

```sql
-- Dump first if you care about your data
DROP EXTENSION pgrdf CASCADE;
-- Install v0.5.0-rc1 artifacts
CREATE EXTENSION pgrdf;
-- Re-ingest
```

The table shapes (`_pgrdf_graphs`, `_pgrdf_quads`,
`_pgrdf_dictionary`) are unchanged from v0.4.6; `pgrdf.validate`
gains an optional third argument (the 2-arg form is unchanged) and
the `parse_trig`/`parse_nquads` ingest UDFs landed in G2. A
`pg_dump` from v0.4.6 restores against a v0.5.0-rc1 install via the
documented `DROP/CREATE EXTENSION; pg_restore` path. See
[`docs/06-installation.md` § Upgrade between v0.x versions](docs/06-installation.md#upgrade-between-v0x-versions).

## License

Apache 2.0. Copyright 2026 Peter Styk &lt;peter@styk.tv&gt;.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md).
