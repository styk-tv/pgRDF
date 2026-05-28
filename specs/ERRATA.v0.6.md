# ERRATA.v0.6

Spec deltas accumulated during the v0.6 cycle. v0.5-era and earlier
entries that remain live are cross-linked to
[`ERRATA.v0.5.md`](ERRATA.v0.5.md) /
[`ERRATA.v0.4.md`](ERRATA.v0.4.md) /
[`ERRATA.v0.2.md`](ERRATA.v0.2.md) rather than duplicated. This file
is the v0.6-era spec-deltas log for the forward-looking
[`SPEC.pgRDF.LLD.v0.6-FUTURE.md`](SPEC.pgRDF.LLD.v0.6-FUTURE.md)
contract; it opens with **E-014** as the first v0.6-era delta
(2026-05-28). Track G task TG-2 (open ERRATA.v0.6.md once the first
v0.6-era delta appears) is closed by this file's creation.

## v0.5 / v0.4 / v0.2 entries still live in v0.6

| Entry | One-line status in v0.6 |
|---|---|
| [E-011 — upstream `reasonable` patch](ERRATA.v0.4.md) | Unchanged. Still gated on <https://github.com/gtfierro/reasonable/pull/50>. `[patch.crates-io]` carries forward through the v0.6 cycle. |
| [E-006 — pgrx 0.18 / PG 18 deferred](ERRATA.v0.2.md) | Unchanged. Largest deferred upstream item carried into v0.6. |
| [E-013 — SHACL `prop-nodeKind-001`](ERRATA.v0.5.md) | Resolved in v0.5.0; full-pass 25 / 25 retained through the v0.6 cycle. |

## v0.6 entries

### E-014 — `shacl 0.3.2` SparqlEngine returns wrong verdict on common SHACL-SPARQL topologies

| Field | Value |
|---|---|
| Filed | 2026-05-28 (Track H task TH-7, v0.5.8 micro-release) |
| Status | **OPEN — upstream-gated.** rudof's `BasicSparqlValidator::validate_sparql` (in `shacl 0.3.2`) returns `conforms=true` with 0 violations on the W3C SHACL-SPARQL fixture `tests/sparql/node/sparql-001.ttl` even though (a) the W3C `mf:result` asserts `conforms=false` with 3 violations, and (b) the compiled `IRSchema` carries the `IRComponent::BasicSparql` constraint correctly (verified via plain-Rust test `validation::pgrdf_sparql::th11_walk_schema_unit_tests`). The bug is in the rudof SparqlEngine's per-focus-node evaluation, not in parsing. |
| Affects | [`SPEC.pgRDF.LLD.v0.6-FUTURE.md`](SPEC.pgRDF.LLD.v0.6-FUTURE.md) §Track H; [`tests/w3c-shacl/run.sh`](../tests/w3c-shacl/run.sh) `--sparql` sub-run |
| Crate | `shacl 0.3.2` (rudof project, 2026-05-26) |
| Disposition | **pgRDF beats rudof.** The W3C conformance gate for SHACL-SPARQL on this fixture passes under `mode => 'pgrdf'` (pgRDF-native handler, TH-9 + TH-8): `pgrdf.validate(g, g, 'pgrdf')` correctly returns `conforms=false` with 3 `sh:Violation` results, each with `sourceConstraintComponent = sh:SPARQLConstraintComponent`. pgRDF-mode is therefore promoted to the authoritative SHACL-SPARQL gate (`tests/w3c-shacl/run.sh --pgrdf`); the rudof `--sparql` sub-run is downgraded to "pgRDF-side contract assertion only" (conforms is a real Boolean, dispatch reaches the engine — no W3C verdict comparison). pgrx integration test `validate_w3c_node_sparql_001_cross_mode` locks both behaviours per PG major. |
| Trigger to re-check | Any subsequent `shacl 0.3.x` / `0.4.x` release; reproduce by running `bash tests/w3c-shacl/run.sh --sparql` and comparing against the (currently downgraded) `expected.json`. The day the rudof verdict matches W3C `mf:result`, this errata closes and the `--sparql` sub-run can re-tighten to the same conformance gate as `--pgrdf`. |

#### Reproduction (pgrx + plain-Rust)

- `validation::pgrdf_sparql::th11_walk_schema_unit_tests` confirms `IRComponent::BasicSparql` is parsed correctly (`walk_schema_for_sparql` returns 1 pair against the fixture's IRSchema).
- `validation::shacl::tests::validate_w3c_node_sparql_001_cross_mode` runs both modes:
  - `mode => 'sparql'` (rudof) → `conforms=true`, `results=[]` (wrong)
  - `mode => 'pgrdf'` (pgRDF-native) → `conforms=false` with 3 violations (right, matches W3C)

#### Why not patch rudof?

Out of scope for this cycle. The pgRDF-native path is the higher-performance backend regardless (avoids `InMemoryGraph` materialisation for the data graph), and now is also the more correct backend on this conformance fixture. A future SPEC pass may file the upstream issue with rudof; for the v0.6 cycle, pgRDF-mode is the de-facto SHACL-SPARQL engine.
