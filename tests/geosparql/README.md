# tests/geosparql

GeoSPARQL 1.1 conformance material for pgRDF. **Fixtures, analysis and
provenance only** — the runner is designed in `SPIKE.pgRDF-GeoSPARQL-1.md`
§12, not built here.

```
SPIKE.pgRDF-GeoSPARQL-1.md   the analysis: landscape, licensing, TDD method, substrate, tiers
RULES.md                     what each of the 24 GeoSPARQL SHACL rules constrains
COVERAGE.md                  proof that our fixtures cover the OGC corpus, and what differs
LICENSE.md                   the register: everything evaluated, its license, its verdict
license-guard.sh             fails if anything non-MIT ever lands here
compare-upstream.sh          dev-time coverage re-check against upstream; vendors nothing
fixtures/                    57 pgRDF-authored fixtures over 22 rules, MIT
```

## Everything here is pgRDF's own work

**Nothing third-party is vendored.** Every fixture is MIT, carries an SPDX
header, and was authored from the GeoSPARQL rule definitions rather than
copied. `LICENSE.md` records everything evaluated and why.

That includes the OGC's own fixture corpus, which looked vendorable and was
not: its Apache-2.0 status rests on a single README sentence, with no LICENSE
file on the branch and no declaration on the files themselves. Low risk — but
an inference, and re-authoring removed the question. The full reasoning is in
`LICENSE.md`.

`COVERAGE.md` answers the fair follow-up: our 57 fixtures cover **47 of the 48
upstream scenarios**, exclude one on scope (DGGS), and add ten.

## Layout

One directory per rule, named for what the rule constrains, matching the
`tests/w3c-sparql/` idiom:

```
fixtures/S01-serialization-cardinality/
    valid.ttl
    invalid-repeated-aswkt.ttl
    invalid-geojson-in-wkt-literal.ttl
    invalid-aswkt-carries-gml-datatype.ttl
```

Every file names its rule, its expected verdict and why, in the header. No
case is called merely "invalid-02".

## What this corpus is not

**It is not a conformance test suite, and passing it is not GeoSPARQL
conformance.** The OGC publishes its SHACL validator as *informative, not
normative*. Any published result must say so.

There is no OGC-endorsed executable test suite for GeoSPARQL. The only
comprehensive community suite is GPL-2.0-only and excluded on license; Java
and Python suites are excluded on project rule. See `LICENSE.md`.

## ⚠ No shapes graph yet

The fixtures need shapes to validate against. None is vendored: every copy of
the OGC shapes graph self-declares the OGC **Document** License, so none may
enter an MIT repository (`LICENSE.md`).

**pgRDF authors its own shapes**, and `RULES.md` specifies the target for each
— what it constrains, which SHACL mechanism it needs, which fixtures exercise
it. That is T1 work; the shapes do not exist yet.

## Before any result from this corpus is published

Three measurements first, in order. Each can silently invalidate every number
the corpus produces.

1. **Regenerate `tests/shacl-capability/CAPABILITY.json`.** It is pinned at
   pgRDF `0.6.22` against a `0.6.34` tree. A capability document generated
   twelve versions before the engine it describes is not an instrument.

2. **Probe four unmeasured SHACL surfaces.** The rules use them; the
   capability harness has never tested them:

   | Term | Rules affected | Why it matters |
   |---|---|---|
   | `sh:targetObjectsOf` | S01, S02 | `targetSubjectsOf` is enforced; that says nothing about its converse. If unsupported, those rules select no focus nodes and pass **vacuously**. |
   | `sh:flags` | S16–S19 | `sh:pattern` is enforced, but flag handling is separate — and SHACL specifies XPath regex where pgRDF translates to POSIX. |
   | `sh:alternativePath` | S01 family | `inversePath` is probed; this is not. |
   | `sh:deactivated` | any | If ignored, a deliberately-disabled shape fires — a false *failure*. |

3. **Run in mode `'pgrdf'`, explicitly.** `pgrdf.validate(data, shapes, mode)`
   defaults to `'native'`, and `'native'` **silently skips `sh:sparql`** —
   which S21 needs. A default-mode run drops that rule with no error and no
   log line. Record the mode in every result.

Until 1–3 are done, a green run means "nothing objected", not "the data was
judged". That distinction is why `tests/shacl-capability` exists.

## One entailment caveat

`sh:targetClass` matches **asserted** `rdf:type` only. GeoSPARQL's classes are
a hierarchy — `geo:Feature` and `geo:Geometry` are both subclasses of
`geo:SpatialObject` — so a node typed only by a subclass is not targeted by a
shape on its parent unless `pgrdf.materialize` has run. This hits **S22–S24**
directly. The runner must **decide and record** whether it materializes first.

## Reference implementations during development

Jena, RDF4J and pySHACL are excluded from what pgRDF **ships and tests** — not
from what it may consult. Running one in an adjacent throwaway container to
cross-check an expected value is permitted and encouraged where
hand-derivation is heavy.

Only the output enters the repository, `tests/` must pass with no JVM and no
Python present, and every value so derived is stamped `cross-checked` with
tool and version. Artefacts refused **on license** may not be run this way.
See `LICENSE.md` and `SPIKE.pgRDF-GeoSPARQL-1.md` §6.5.

## Verifying

A guard failure is a **critical stop** — this tree is public. Do not commit,
do not push, do not override; see `LICENSE.md`.

```bash
bash tests/geosparql/license-guard.sh      # run this in CI
bash tests/geosparql/compare-upstream.sh   # re-check coverage when upstream moves
```
