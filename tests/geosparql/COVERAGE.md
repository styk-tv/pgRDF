# Coverage — pgRDF fixtures vs the OGC corpus

pgRDF authors its own GeoSPARQL fixtures (`fixtures/`). Nothing third-party is
vendored; see `LICENSE.md` for why.

Authoring our own raises one fair question: **did we quietly lose coverage?**
This is the answer, checked against the OGC corpus at
`opengeospatial/ogc-geosparql` `examples/shacl/` @ `f90fff13`, which held 48
fixtures over 20 rules.

**47 of 48 upstream scenarios are covered. One is deliberately excluded. Ten
cases were added.**

| Rule | Upstream | Ours | Status |
|---|---|---|---|
| S01 serialization-cardinality | 4 | 4 | covered |
| S02 geometry-not-nested | 3 | 3 | covered |
| S03 serialization-must-be-literal | 2 | 2 | covered |
| S04 aswkt-datatype-match | 3 | 3 | covered |
| **S05 asgml-datatype-match** | — | 3 | **new** — upstream ships none |
| **S06 asgeojson-datatype-match** | — | 3 | **new** — upstream ships none |
| **S07 askml-datatype-match** | — | 3 | **new** — upstream ships none |
| S09 coordinatedimension-cardinality | 2 | 2 | covered |
| S10 dimension-cardinality | 2 | 2 | covered |
| S11 isempty-cardinality | 2 | 2 | covered |
| S12 issimple-cardinality | 2 | 2 | covered |
| S13 spatialdimension-cardinality | 2 | 2 | covered |
| S14 spatialresolution-cardinality | 3 | 3 | covered |
| S15 spatialaccuracy-cardinality | 3 | 3 | covered |
| S16 wkt-literal-wellformed | 2 | 2 | covered |
| S17 gml-literal-wellformed | 2 | 2 | covered |
| S18 geojson-literal-wellformed | 2 | 2 | covered |
| S19 kml-literal-wellformed | 2 | 2 | covered |
| **S20 dggs-literal-wellformed** | 1 | — | **excluded — DGGS is out of scope** |
| S21 dimension-le-coordinatedimension | 2 | 3 | covered (+1) |
| S22 featurecollection-members | 3 | 3 | covered |
| S23 geometrycollection-members | 3 | 3 | covered |
| S24 spatialobjectcollection-members | 3 | 3 | covered |
| **Total** | **48** | **57** | |

## The one exclusion

**S20 `dggs-literal-wellformed`.** DGGS (Discrete Global Grid System) is out of
scope for the whole series — no Rust implementation of AusPIX exists, and the
spike lists it under permanently-out-of-scope work. Upstream ships only a
positive case for it and no negative one, so even upstream does not test the
rule in the direction that matters.

Excluded on scope, not overlooked. **S08 `asdggs-datatype-match`** is absent
for the same reason; upstream ships no fixture for it either.

## What was added

- **S05, S06, S07** — the GML / GeoJSON / KML analogues of S04's
  datatype-match rule. These rules exist in GeoSPARQL; upstream simply ships
  no fixtures for them. Each gets a valid case plus plain-literal and
  `xsd:string` negatives, mirroring S04.
- **S21 `valid-strictly-less`** — upstream tests only `dimension ==
  coordinateDimension`. The rule is `≤`, so the strict-inequality case is a
  real and untested branch.

## Deliberate differences from upstream

These are improvements, not drift:

- **Every case is named for what it tests.** `invalid-geojson-in-wkt-literal`,
  not `S01-invalid-01`.
- **Every file carries an SPDX MIT header** naming the rule, the expected
  verdict, and why.
- **One directory per rule**, matching the `tests/w3c-sparql/` idiom.
- **Upstream caption defects are not inherited** — S09's empty description and
  S13's caption copy-pasted from S10 (see `RULES.md`).
- **Distinct sample data.** Our own IRIs under `http://example.org/pgrdf/` and
  our own coordinates.

## Re-checking this claim

`compare-upstream.sh` fetches the upstream corpus to a temporary directory
outside the repository, prints this matrix from live data, and deletes it. It
is a development-time tool: it vendors nothing and is never part of a test run.

```bash
bash tests/geosparql/compare-upstream.sh
```

Re-run it when upstream moves. If the corpus has grown, this file is stale.
