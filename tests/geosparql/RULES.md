# GeoSPARQL SHACL rules — what each one actually constrains

This is the specification pgRDF's fixtures and shapes are written against:
what each of the 24 GeoSPARQL SHACL rules constrains, which SHACL mechanism it
needs, and where pgRDF's capability is unmeasured.

**Rule intents are paraphrased, not copied.** The GeoSPARQL shapes graph is
under the OGC Document License and is not vendored (`LICENSE.md`); it was read
as reference material only. Rule names and identifiers are facts and are used
freely.

**Fixtures are pgRDF's own**, under `fixtures/<rule>/`, MIT. The *Fixtures*
column describes the cases pgRDF authored for each rule; `COVERAGE.md` maps
them against the OGC corpus.

`pgRDF` column: the SHACL mechanism the rule needs, and whether
`tests/shacl-capability/CAPABILITY.json` has ever probed it.
**⚠ = unprobed**, so a pass on that rule may be vacuous rather than earned.

## Serialization structure

| Rule | Name | Constrains | Mechanism | Fixtures | pgRDF |
|---|---|---|---|---|---|
| S01 | `serialization-cardinality` | A Geometry may carry at most one of each kind of serialization relation (`asWKT`, `asGML`, …) | `maxCount`, `targetObjectsOf` | 1 valid, 3 invalid (wrong content / wrong datatype / repeated `asWKT`) | ⚠ `targetObjectsOf` |
| S02 | `geometry-not-nested` | A Geometry must not point at another Geometry — no `hasGeometry` chains | `maxCount`, `targetObjectsOf` | 1 valid, 2 invalid (in+out `hasGeometry`; `hasDefaultGeometry` in + `hasGeometry` out) | ⚠ `targetObjectsOf` |
| S03 | `serialization-must-be-literal` | The object of `hasSerialization` (or a subproperty) must be an RDF literal, not an IRI or blank node | `nodeKind` | 1 valid, 1 invalid (non-literal serialization) | enforced |

## Datatype agreement — the serialization predicate must match its literal type

| Rule | Name | Constrains | Mechanism | Fixtures | pgRDF |
|---|---|---|---|---|---|
| S04 | `aswkt-datatype-match` | `geo:asWKT` must carry a `geo:wktLiteral` | `datatype` | 1 valid, 2 invalid | enforced |
| S05 | `asgml-datatype-match` | `geo:asGML` must carry a `geo:gmlLiteral` | `datatype` | **none** | enforced |
| S06 | `asgeojson-datatype-match` | `geo:asGeoJSON` must carry a `geo:geoJSONLiteral` | `datatype` | **none** | enforced |
| S07 | `askml-datatype-match` | `geo:asKML` must carry a `geo:kmlLiteral` | `datatype` | **none** | enforced |
| S08 | `asdggs-datatype-match` | `geo:asDGGS` must carry a `geo:dggsLiteral` | `datatype` | **none** | enforced |

**Upstream ships no fixtures for S05–S08** — they are not missing rules, but
the GML / GeoJSON / KML / DGGS analogues of S04, untested upstream. **pgRDF
authors its own for S05–S07** (three cases each, mirroring S04). S08 is DGGS
and is out of scope.

## Metadata cardinality — each property at most once per Geometry

| Rule | Name | Constrains | Mechanism | Fixtures | pgRDF |
|---|---|---|---|---|---|
| S09 | `coordinatedimension-cardinality` | at most one `geo:coordinateDimension` | `maxCount`, `targetSubjectsOf` | 1 valid, 1 invalid | enforced |
| S10 | `dimension-cardinality` | at most one `geo:dimension` | `maxCount`, `targetSubjectsOf` | 1 valid, 1 invalid | enforced |
| S11 | `isempty-cardinality` | at most one `geo:isEmpty` | `maxCount`, `targetSubjectsOf` | 1 valid, 1 invalid | enforced |
| S12 | `issimple-cardinality` | at most one `geo:isSimple` | `maxCount`, `targetSubjectsOf` | 1 valid, 1 invalid | enforced |
| S13 | `spatialdimension-cardinality` | at most one `geo:spatialDimension` | `maxCount`, `targetSubjectsOf` | 1 valid, 1 invalid | enforced |
| S14 | `spatialresolution-cardinality` | at most one `hasSpatialResolution` **and** at most one `hasMetricSpatialResolution`; having both kinds is legal | `maxCount`, `targetSubjectsOf` | 1 valid, 2 invalid (one per property) | enforced |
| S15 | `spatialaccuracy-cardinality` | at most one `hasSpatialAccuracy` **and** at most one `hasMetricSpatialAccuracy` | `maxCount`, `targetSubjectsOf` | 1 valid, 2 invalid | enforced |

## Literal well-formedness — lexical shape of each serialization

These are the only rules that inspect literal *content*, and they are
deliberately shallow: upstream checks the opening token, not full grammar
conformance. A parser is still pgRDF's job.

| Rule | Name | Constrains | Mechanism | Fixtures | pgRDF |
|---|---|---|---|---|---|
| S16 | `wkt-literal-wellformed` | a `wktLiteral` must open like WKT | `pattern` + `flags` | 1 valid, 1 invalid (bad opening character) | ⚠ `flags` |
| S17 | `gml-literal-wellformed` | a `gmlLiteral` must open like GML | `pattern` + `flags` | 1 valid, 1 invalid | ⚠ `flags` |
| S18 | `geojson-literal-wellformed` | a `geoJSONLiteral` must open like JSON | `pattern` + `flags` | 1 valid, 1 invalid | ⚠ `flags` |
| S19 | `kml-literal-wellformed` | a `kmlLiteral` must open like KML | `pattern` + `flags` | 1 valid, 1 invalid | ⚠ `flags` |
| S20 | `dggs-literal-wellformed` | a `dggsLiteral` must open like a DGGS cell list | `pattern` + `flags` | **1 valid only — no invalid case** | ⚠ `flags` |

## Value consistency

| Rule | Name | Constrains | Mechanism | Fixtures | pgRDF |
|---|---|---|---|---|---|
| S21 | `dimension-le-coordinatedimension` | when both are asserted, `geo:dimension` ≤ `geo:coordinateDimension` | `sparql` | 1 valid, 1 invalid | ⚠ **mode `'pgrdf'` only** |

## Collection membership

| Rule | Name | Constrains | Mechanism | Fixtures | pgRDF |
|---|---|---|---|---|---|
| S22 | `featurecollection-members` | a `geo:FeatureCollection` needs ≥1 `rdfs:member`, and every member must be a `geo:Feature` | `minCount`, `class`, `targetClass` | 1 valid, 2 invalid (no members; a Geometry member) | enforced |
| S23 | `geometrycollection-members` | a `geo:GeometryCollection` needs ≥1 member, all `geo:Geometry` | `minCount`, `class`, `targetClass` | 1 valid, 2 invalid | enforced |
| S24 | `spatialobjectcollection-members` | a `geo:SpatialObjectCollection` needs ≥1 member, all `geo:SpatialObject` | `minCount`, `class`, `targetClass` | 1 valid, 2 invalid (no members; a member of an unrelated type) | enforced |

**S22–S24 are where the entailment caveat bites.** `sh:targetClass` matches
*asserted* `rdf:type` only, and `geo:Feature` / `geo:Geometry` are subclasses
of `geo:SpatialObject`. A member typed only as `geo:Feature` will not satisfy
a `class geo:SpatialObject` check unless `pgrdf.materialize` has run. The
runner must record which it did — see `README.md`.

## Upstream defects worth knowing

Recorded so nobody re-derives them:

- **S09's invalid fixture has an empty description** (`# Invalid:` with no
  text). Its content is a duplicated `coordinateDimension`.
- **S13's invalid fixture is captioned "More than one dimension property"**,
  copy-pasted from S10. It actually tests `spatialDimension`.
- **S20 has no invalid fixture**, so the DGGS pattern rule is untested
  upstream in the negative direction. Out of scope for pgRDF anyway.
- **S05–S08 have no fixtures at all** (above).

## Counts

**57 pgRDF-authored fixtures over 22 rules.** S08 and S20 are absent: both are
DGGS, which is out of scope for the series. See `COVERAGE.md` for the mapping
against the 48-fixture OGC corpus.
