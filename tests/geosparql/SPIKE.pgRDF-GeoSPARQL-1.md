# SPIKE.pgRDF-GeoSPARQL-1 — Landscape, Licensing, and Test Methodology

**Series:** pgRDF GeoSPARQL, document 1 of n
**Type:** spike. An investigation and a decision record, not a commitment to
build. Nothing here schedules engine work; §8 records a substrate choice and
§9 sizes the tiers so that a later decision is informed rather than improvised.
**Status:** public, tracked in `tests/geosparql/`. Every claim is measured or
marked as judgement.
**Measured:** 2026-09-16. Every external claim in this document was verified
against a live registry on that date; §2 names the method and Appendix A the
commands. Nothing here is quoted from secondary summaries.
**Audience:** pgRDF maintainers and anyone auditing how third-party material
entered this repository. **Public** — it contains only measured facts about
public artefacts, pgRDF behaviour, and licensing.

---

## 0. Why this document exists

A third-party survey of "GeoSPARQL test suites and testing resources" was
brought in as the starting point for adding geospatial RDF to pgRDF. This
document does four things, in order:

1. Audits that survey against measured reality (§3).
2. Establishes what may legally enter an MIT-licensed public repository (§4).
3. Inventories the conformance corpus that actually exists upstream (§5) and
   assesses the TDD methods available against pgRDF's house idiom (§6).
4. Records the substrate decision (§8) and sizes the work as ordered tiers
   (§9).

It deliberately stops short of designing the engine. Document 2 designs
tier 0.

---

## 1. Scope and non-goals

**In scope.** Two questions, treated together because neither is answerable
alone: what a GeoSPARQL capability would cost pgRDF to build, and what
evidence would show that it works. Cost is addressed as ordered tiers (§9)
against a named substrate (§8). Evidence is addressed as a test method (§6)
over a corpus whose licensing and provenance are established first (§4–§5) —
an unusable corpus makes the cost question moot, which is why the licensing
audit precedes the engineering.

**Out of scope for the series, permanently:**

- **Java, in any form.** Not as a dependency, not as a build step, not as a
  test harness, not vendored. Java implementations may be *read* as reference
  material. See §4.1 — a supply-chain rule, not a taste preference.
- **OCI image assembly, bundle composition, attestation linking.** That work
  lives in the downstream repos and does not enter pgRDF.

**Out of scope for tiers T0–T4, revisitable later:** CRS transformation,
the Query Rewrite Extension, and DGGS. See §9.

---

## 2. Method

Claims in this document carry one of three provenance markers:

| Marker | Meaning |
|---|---|
| **[measured]** | Verified 2026-09-16 against a live API — GitHub REST, crates.io, Maven Central, or an HTTP dereference. Command in Appendix A. |
| **[read]** | Read from a file in this working copy at the cited path and line. |
| **[judgement]** | An engineering opinion. Argued, not measured. Disagree with it freely. |

The distinction matters because the source survey (§3) mixes all three without
marking any, which is how several of its errors became load-bearing.

Upstream state is pinned. `opengeospatial/ogc-geosparql` branch `geosparql-1.1`
is at commit `f90fff13235aaffaf69dcba97f3e01888167f080` (committed
2026-07-30) **[measured]**. Every upstream path in this document resolves at
that commit. The repository is *not* archived and its `master` branch moved as
recently as 2026-09-01 **[measured]** — pin, do not track.

**Engine identity.** Working copy is `Cargo.toml version = 0.6.34` and
`pgrdf.control default_version = 0.6.34` **[read]**. This document makes no
bench claim, so the release triple (`version() == build_id() == extversion`)
is not asserted here — but §7.3's SHACL numbers come from an instrument
generated at `0.6.22`, and that drift is called out where it matters.

**Standing rule, learned the hard way in this document's first draft:**
**prefer the instrument to the source.** Where pgRDF has a generated
capability document, it outranks reading the implementation. A grep of
`src/validation/shacl.rs` produced a confidently wrong capability claim that
`tests/shacl-capability/CAPABILITY.json` immediately refuted (§7.3). Any
future statement in this series about what pgRDF enforces cites the
instrument, or is marked **[judgement]**.

---

## 3. Audit of the source survey

The survey is directionally useful and factually unreliable. It correctly
identifies the central fact — that no OGC-endorsed executable test suite
exists — and correctly names the main players. Its specifics do not hold.

### 3.1 Verified correct

| Claim | Status |
|---|---|
| No official OGC executable test suite (ETS) exists | **Correct**, and confirmed by the spec's own framing: the 1.1 SHACL validator is published as "not normative, only informative" **[measured]** |
| RDF4J publishes GeoSPARQL artifacts | **Correct** — `rdf4j-queryalgebra-geosparql`, `rdf4j-geosparql-testsuite`, `rdf4j-geosparql-compliance` all exist on Maven Central **[measured]** |
| `geosparql-1.1` and `geosparql-1.0` branches carry test data | **Correct** **[measured]** |
| The Abstract Test Suite is normative but not executable | **Correct** |

### 3.2 Corrections

| Survey claim | Measured reality |
|---|---|
| RDF4J testsuite "4.3.11", "5.0.0-SNAPSHOT as of April 2024" | Latest are **4.3.16** and **5.1.3** **[measured]** |
| `oyvindlgjesdal/geosparql-jena` is the Jena implementation | That is a **0-star fork, last pushed 2018-12-08** **[measured]**. The real lineage is `galbiston/geosparql-jena` (16 stars, last push 2021), since upstreamed into Apache Jena. The survey cites a dead fork as a live project. |
| OxiRS GeoSPARQL "Production Release (v0.3.1) with comprehensive testing" | **0.4.1**, first published **2025-10-12**, **563 lifetime downloads / 96 in 90 days** **[measured]**. A crate with 563 lifetime downloads is not production-proven. Treat as unvetted. |
| Repo has an "Extended Examples folder" and test data at root | `master` root is `.github, README.md, bblock, charter, docco, ewkb, geosparql-next, scripts` **[measured]**. No examples at root; they are on the `geosparql-1.1` branch. |
| "206 SPARQL queries testing 30 requirements" (benchmark) | Not independently verified. The number originates in the 2021 paper, not from inspecting the repo. Unverifiable without cloning GPL code — see §4.2. |

### 3.3 Omissions that change the conclusion

Two gaps matter more than any individual error.

**The survey never mentions licensing.** Its ranked recommendation #2 — "Use
GeoSPARQL Compliance Benchmark for comprehensive compliance checking" — points
at a **GPL-2.0-only** codebase **[measured]**. Acting on that recommendation
inside pgRDF would be a licensing incident. A survey written to guide adoption
that omits the adoption constraint is not merely incomplete; its ranking is
inverted relative to what pgRDF can actually use.

**The survey misses the one immediately usable asset.** OGC ships
`examples/shacl/` on the `geosparql-1.1` branch: **48 Turtle fixtures — 20 valid, 28 invalid — across 20
numbered shapes (S01–S04, S09–S24)**, each a deliberate valid or invalid case
**[measured]**. It is Apache-2.0 (§4.1), it needs no geometry engine, and
pgRDF already has a SHACL validator. The survey does not mention it once.

**[judgement]** The survey is a reasonable map of the territory and a poor
guide to the route. Use it for orientation; re-derive every specific.

---

## 4. License audit

pgRDF is MIT. Every artefact evaluated for this series has a verdict. The
authoritative register — artefact, license, verdict, grouped by realm — is
`LICENSE.md` in this directory and is enforced by `license-guard.sh`. This
section records the audit's findings; it does not restate the table.

### 4.1 Verdicts by realm

| Realm | Outcome |
|---|---|
| **OGC data** (`examples/shacl/`, 48 fixtures) | Apache-2.0 → **used** |
| **OGC documents** (spec prose, Annex A, both shapes-graph copies) | OGC Document License → **excluded on license** |
| **OGC data with its own declaration** (`demo-dataset.ttl` CC-BY-4.0; `moreton-island.*` undeclared) | **excluded on license** |
| **Benchmarks** (`GeoSPARQLBenchmark`) | GPL-2.0-only → **excluded on license** |
| **Java** (Jena, RDF4J, geosparql-jena) | Apache-2.0 / BSD-3 → **excluded on project rule** |
| **Python** (pySHACL, rdflib, pytest) | Apache-2.0 / BSD-3 / MIT → **excluded on project rule** |
| **Rust crates** (`geo`, `geo-types`, `wkt`, `rstar`, `proj`) | MIT OR Apache-2.0 → **candidates** |
| **Rust with transitive copyleft** (`geos` → LGPL-2.1 GEOS) | **excluded on license** |
| **Adjacent** (PostGIS) | GPL-2.0 → **excluded on license** |

Two distinct grounds, never conflated: **license** means incompatible or more
restrictive than MIT. **Project rule** means permissively licensed but
excluded on pgRDF's no-Java / no-Python-in-the-live-path rules. They are
**not** excluded from development-time use: §6.5 permits them as adjacent
oracles, and their licenses allow it.

### 4.2 The two findings that generalise

Everything else in the register follows from these.

**A file's own license declaration outranks the directory it sits in.**
Directory-level licensing is an assumption; a self-declaration is a fact.
Measured in one tree: `examples/` is Apache-2.0 by the repository README, yet
`demo-dataset.ttl` declares `dcterms:license` CC-BY-4.0, and both shapes-graph
copies declare `schema:license` / `sdo:license`
`<https://www.ogc.org/license>` — which `301`-redirects to the OGC Document
License Agreement **[measured]**. Location does not predict tier. Read every
vendored file's own declaration before trusting the directory's.

**`.gitignore` is not a license boundary.** GPL-2.0-only material is not
cloned, vendored, adapted, or transcribed — including into ignored or scratch
directories. An ignored file is still a file on disk in an otherwise-MIT tree.

### 4.3 The OGC two-tier split

One repository, two licenses **[measured]**:

> OGC documents ... are licensed for use with the OGC's Document License
> Agreement for OGC Resources. Software and data ... are licensed for use
> with the Apache Software License 2.0.

Test data is data → Apache-2.0 → vendorable. Spec prose is document tier →
implement from it, cite it by conformance-class identifier, never copy it.
Rule *names* and *identifiers* are facts and are used freely; `RULES.md`
paraphrases rather than quotes.

### 4.4 Consequence: pgRDF authors its own shapes and fixtures

No OGC shapes graph is usable here, in any version — both copies self-declare
the document tier (§4.2), and the in-repo copy is additionally GeoSPARQL 2.0,
`owl:versionIRI :2.0` **[measured]**, a different standard from the 1.1 whose
fixtures sit beside it.

pgRDF therefore authors its shapes from the GeoSPARQL requirements. This is
§6.3's *cases re-authored, never imported*, applied to shapes.

**The fixtures went the same way, on a weaker but real concern.** They looked
vendorable — Apache-2.0 by the repository README, no contrary declaration. But
that rests on **one README sentence**, with no `LICENSE` file on the branch
they live on, no declaration on the files themselves, and a sibling in the
same `examples/` tree (`demo-dataset.ttl`) declaring CC-BY-4.0 — which proves
the tree is not uniformly one tier **[measured]**.

The risk was low: 412 lines, mean 8.5 per file, minimal factual RDF, and OGC's
posture where declared is permissive (the `geosparql-1.0` branch carries a
permissive OGC License Agreement **[measured]**). **[judgement]** But "low risk
resting on an inference" is exactly the standard this document refuses to let
others rely on (§4.2 on the GPL queries), and applying it selectively to our
own convenience would be the same error in the other direction. Authoring 57
fixtures cost a few hours and removed the question.

**The rule that generalises: an absent license declaration is not a permissive
one.** An earlier revision refused `moreton-island.*` as *undeclared* while
accepting equally-undeclared fixtures from the same directory — a scope
decision recorded as a licensing verdict. Both are now resolved the same way.

§9's T1 carries the shapes cost; `RULES.md` specifies the target and
`COVERAGE.md` records that the authored fixtures cover 47 of the corpus's 48
scenarios, exclude one on scope, and add ten.

### 4.5 Standards: citation is not redistribution

The OGC Document License restricts **redistributing OGC prose**. It does not
restrict reading the standard, implementing it, or referring to it, and it is
**not copyleft** — unlike GPL-2.0, it propagates nothing into work that cites
it. Refusing document-tier material is about not republishing someone else's
text, not about contamination.

Three things are therefore free to use, and this series uses them heavily:

| Free to use | Why |
|---|---|
| **Conformance-class and requirement identifiers** — `conf/core/feature-class`, `conf/geometry-extension/geometry-as-wkt-literal` | Identifiers are facts. Naming a requirement is not reproducing it. |
| **Vocabulary terms** — `geo:asWKT`, `geof:sfWithin`, shape ids `S01`–`S24` | Names are facts, and these are IRIs a conforming implementation must emit. |
| **Implementing the requirements** | Copyright protects expression, not ideas or methods. Building conforming software is what publishing a standard is for. |

**Audit of what this tree actually quotes [measured]:** zero lines of the
GeoSPARQL specification. The only OGC text quoted anywhere is the repository
README's three-line licensing statement (§4.3), reproduced to evidence the
licensing verdict — quoting a license notice to establish provenance is both
standard practice and necessary here. `RULES.md` paraphrases every rule intent
rather than quoting `sh:message`.

### 4.6 ISO 19125-1:2004 — what it is, and why pgRDF does not need it

GeoSPARQL's normative references list it as **`[[[WKT, ISO 19125-1:2004]]]`**
**[measured]** — the anchor is literally `WKT`. **ISO 19125-1 is not
GeoSPARQL.** It is *Geographic information — Simple feature access — Part 1:
Common architecture*: the normative source for **Well-Known Text** and the
**Simple Features** relation family that T2 depends on.

ISO documents are copyrighted, sold per copy, and not redistributable —
**stricter than the OGC Document License**, which is at least free to read.
None of that blocks pgRDF, for two reasons:

1. **A normative reference transfers no license obligation.** Implementing
   GeoSPARQL does not require licensing ISO 19125-1, any more than
   implementing SPARQL requires licensing IETF RFC 3987.
2. **The same standard is published by OGC, free.** OGC **06-103r4**,
   *Simple Feature Access – Part 1: Common Architecture*, is the OGC twin;
   OGC's own SWG page describes maintaining "the common standard that is both
   the OGC Simple Feature Access and ISO 19125" **[measured]**. OGC standards
   are freely downloadable.

**Rule for the series: work from OGC 06-103r4; cite ISO 19125-1 as GeoSPARQL
does.** Never purchase, vendor, or quote ISO text. The DE-9IM model
underlying the relation families is additionally published academic
mathematics (Egenhofer; Clementini), describable independently of either
document.

This matters concretely at T2, whose `spec`-tier oracle is the Simple
Features DE-9IM matrices (§6.4). Those now have a free, citable, quotable-by-
identifier source.

### 4.7 Enforcement

`tests/geosparql/` vendors **nothing**; every file in it is pgRDF's own work
under MIT. `license-guard.sh` fails on: any vendored third-party tree; a
fixture missing its SPDX MIT header; any foreign licensing declaration; any
copyleft or non-commercial term; anything but `.ttl` under `fixtures/`.
Self-tested against planted violations of each kind.

**[judgement]** The guard is the durable part. A licensing rule written only
as prose is re-derived by the next person under time pressure; one that fails
a build is not.

## 5. What actually exists upstream

### 5.1 The Abstract Test Suite

`spec/sections/aa-abstract_test_suite.adoc`, 823 lines, **61 distinct
conformance test identifiers** across 7 conformance classes **[measured]**:

| Conformance class | Test ids |
|---|---|
| Geometry Extension | 60 |
| Core | 17 |
| Geometry Extension — DGGS | 17 |
| Geometry Topology Extension | 9 |
| Topology Vocabulary Extension | 7 |
| RDFS Entailment Extension | 7 |
| Query Rewrite Extension | 7 |

(Counts are occurrences of each `conf/` prefix; the 61 figure is distinct ids.)

`spec/abstract_tests/` contains **exactly one file**,
`TEST_conf_core_sparql-protocol.adoc` **[measured]**. The survey's framing
implies a structured per-test corpus. There is none — the ATS is prose, with
identifiers. Its value is as a **checklist and naming scheme**, not as
importable material.

`ab-functions_summary.adoc` names **69 `geof:` functions** **[measured]** —
the full surface, including the six `geof:agg*` aggregates.

### 5.2 Example geometries

`examples/` on `geosparql-1.1` **[measured]**: `demo-dataset.ttl` plus
Moreton Island in five serializations — `.wkt`, `.gml`, `.geojson`, `.kml`,
`.auspix` (DGGS). One real-world polygon expressed five ways would be a good
cross-serialization fixture.

**Not vendored (§4.1).** `demo-dataset.ttl` self-declares CC-BY-4.0
**[measured]**; the Moreton Island files declare nothing and their underlying
geometry's provenance is not verifiable from the repository. Neither is needed
before T4, so carrying a second and third license tier for years to serve a
tier that may never be reached is a poor trade. Re-evaluate at T4, on the
evidence available then.

### 5.3 The SHACL corpus — the immediately usable asset

`examples/shacl/` **[measured]**: 48 Turtle fixtures (20 valid, 28 invalid),
plus `README.md` and `test_shapes.py`. Named `S<nn>-valid.ttl` and
`S<nn>-invalid[-nn].ttl`, covering S01–S04 and S09–S24. **S05–S08 have no
fixtures.** **Not vendored** (§4.4) — pgRDF authored its own equivalents at
`tests/geosparql/fixtures/`, covering 47 of these 48 scenarios and adding ten
(`COVERAGE.md`). `RULES.md` names all 24 rules and maps every case to what it
constrains, since the upstream `S01-valid` / `S01-invalid-02` filenames carry
no meaning alone.

The fixtures are small and surgical. `S01-valid.ttl` in full **[measured]**:

```turtle
# Valid: testing asWKT predicate & matching datatype
@prefix geo: <http://www.opengis.net/ont/geosparql#> .

<http://example.com/geometry/a>
    geo:asWKT "POINT (153.084230 -27.322738)"^^geo:wktLiteral ;
.
```

and its negative case is a GeoJSON string mislabelled as `geo:wktLiteral`.
That single pair is the whole tier-1 argument: it is a **pure vocabulary and
datatype check** with no geometry mathematics in it at all — and §7.3 shows
pgRDF already enforces both components it needs.

### 5.4 The shapes graph is not where anything says it is

Two dead references **[measured]**:

- The spec annex links
  `github.com/opengeospatial/ogc-geosparql/blob/master/1.1/validator.ttl`.
  No `1.1/` directory exists on `master` — or on `geosparql-1.1`,
  `geosparql-1.1_old`, `geosparql-1.0`, or `13buildingblocks`.
- `test_shapes.py` loads `../../validator.ttl` (repo root). No such file
  exists on the branch. **Upstream's own SHACL test runner cannot run.**

What does exist:

- `geosparql-next/rdf/validators/geo.ttl` on `master`, 37,362 bytes
  **[measured]**. Commit history for the old path includes "put dev version in
  main branch", so this is the descendant.
- The dereferenceable published artefact at
  `http://www.opengis.net/def/geosparql/validator`, `303`-redirecting to the
  OGC definitions server and returning Turtle with
  `owl:versionIRI validator:1.1`, `owl:versionInfo "1.1"` **[measured]**.

**Consequence for us:** the shapes graph is not merely hard to locate — it is
**not usable at all** (§4.4). Both copies self-declare the OGC Document
License, and the repo copy is GeoSPARQL 2.0. pgRDF re-authors its shapes and
keeps only the Apache-2.0 fixtures. The dead links are how the licensing
problem was found; they are not themselves the problem.

Note also a **naming mismatch**: fixtures use zero-padded `S01`, `S02`; shape
IRIs use `:S1-…`, `:S2-…` but `:S09-…`, `:S10-…` **[measured]**. Any runner
mapping fixtures to shapes must normalise, not string-match.

### 5.5 What has no upstream fixture at all

Nothing upstream provides expected *results* for `geof:` function evaluation.
There is no `distance(a,b) == 42` corpus, in any license. The only such corpus
is the GPL benchmark, which is excluded. **Every function-level expectation
pgRDF asserts will be hand-derived.** §6.4 is about how to do that honestly.

---

## 6. TDD methods, assessed

### 6.1 pgRDF's house idiom

From `tests/w3c-sparql/README.md` and a sample case **[read]**:

```
NN-name/
  data.ttl        Turtle loaded into a fresh graph
  query.rq        SPARQL run via pgrdf.sparql
  expected.jsonl  one JSONB result row per line, lexicographically sorted
  description.md  prose citing the spec section exercised
  oracle          provenance marker for the expectation
  setup.sql       optional, pre-data multi-graph fixtures
  kind            optional entry-point selector
```

with a bash runner that drops and recreates the extension per test, sorts both
sides for bag-equivalence, and `diff -u`s. `ACCEPT=1` regenerates expectations,
under an explicit written warning: *"Hand-verify the output against the W3C
spec — never trust `ACCEPT=1` blind."*

**[judgement]** This idiom is well-suited to GeoSPARQL and should be reused
unchanged. It is hermetic, needs no network at test time, is language-free,
and already encodes the oracle-provenance discipline that §6.4 needs. Inventing
a second idiom for geometry would be a net loss.

### 6.2 The four upstream methods

| Method | Mechanism | Disposition |
|---|---|---|
| **HOBBIT benchmark** | 206 queries, containerised platform, ratio + percentage metrics | **Rejected** — GPL-2.0-only (§4.2). Method borrowable from the paper; artefacts not. |
| **RDF4J compliance suite** | Maven/JUnit against RDF4J internals | **Rejected as a runner** — Java (§4.1), and it couples expectations to RDF4J's engine. **Permitted as a development-time oracle** (§6.5). |
| **OGC `test_shapes.py`** | pytest + pySHACL over `examples/shacl/` | **Rejected as a runner** — Python in the shipped path, and it is *broken upstream* (§5.4). **Its corpus is adopted**; pySHACL itself is usable as a development-time oracle (§6.5). |
| **Ad-hoc per-implementation unit tests** (Jena, OxiRS) | Library-level assertions | **Reference only** as a suite; the engines themselves are usable as development-time oracles (§6.5). |

**[judgement]** The disposition is unanimous and not close: every upstream
*runner* is unusable, and exactly one upstream *corpus* is both usable and
well-licensed. That asymmetry — take the data, write the runner — is the
methodological finding of this document.

### 6.3 The adopted method

1. **Corpus vendored, pinned, attributed.** Apache-2.0 material copied under
   `tests/geosparql/`, each vendored tree carrying a `LICENSE.md` naming
   source URL, commit, retrieval date, and license — the
   `tests/fixtures/rdfc10/LICENSE.md` pattern.
2. **Cases re-authored, never imported.** ATS identifiers (`conf/core/...`)
   used as *names* so coverage is traceable to the standard, with case bodies
   written here. Names are not prose; this stays clear of the document tier.
3. **Runner in bash**, mirroring `tests/w3c-sparql/run.sh`.
4. **Every expectation carries an `oracle` marker** (§6.4).
5. **Negative tests are first-class.** GeoSPARQL's own corpus is majority
   negative, and for a validator the negative case is the load-bearing one.

### 6.4 The oracle problem

This is the hard part and the reason this document exists before any code.

For SPARQL, W3C ships expected results. For RDFC-1.0, `rdf-canon` ships
canonical forms. **For GeoSPARQL, neither exists in a license we can use.**
So every expectation is derived here, and the risk is circularity: computing
expectations with the implementation under test, then asserting the
implementation matches itself. `ACCEPT=1` makes that failure mode one
keystroke away.

The mitigation is to make oracle provenance explicit and unequal. Proposed
`oracle` values, strongest first:

| Value | Meaning |
|---|---|
| `spec` | Value stated or unambiguously implied by GeoSPARQL 1.1 or a normative reference — e.g. the Simple Features DE-9IM matrices, taken from the freely published OGC 06-103r4 rather than its paywalled ISO twin (§4.6). Strongest. |
| `rule-derived` | Expectation follows directly from the rule the fixture targets, as catalogued in `RULES.md` — we assert only pass/fail, and the rule fixes which. Strong. |
| `derived` | Hand-computed from geometry by a human, with the working shown in `description.md`. Acceptable — the working is reviewable. |
| `cross-checked` | Computed by an independent reference implementation in a development-time container (§6.5) and *recorded as such*, naming tool and version. Two engines agreeing is stronger than one; a disagreement is a finding, not a tie to break silently. |
| `eligible` | Present in the existing suites; means the case is admissible but the expectation is not independently sourced. **Never valid for a conformance claim in §10.** |

**[judgement]** The rule that follows: **no conformance class may be claimed
publicly (§10) on the strength of `eligible` or `cross-checked` cases alone.**
A claim is only as strong as its weakest oracle, and conformance claims are
public assertions we would have to defend.

---

### 6.5 Development-time oracles — permitted, and bounded

The oracle problem (§6.4) has one good answer that costs nothing to ship:
**run an independent implementation beside the bench during development, and
commit only its output.**

Jena, RDF4J and pySHACL are Apache-2.0 / BSD-3-Clause / MIT. Nothing in those
licenses is triggered by running them locally, and pgRDF's no-Java /
no-Python rules exist to keep a JVM or interpreter out of the **build, test
and ship path** — not to forbid consulting a reference implementation. An
adjacent throwaway container is outside that path entirely.

**Why it is worth doing.** R2 (oracle circularity) is the sharpest
methodological risk in this series: with no usable expected-results corpus,
every `geof:` expectation is hand-derived, and `ACCEPT=1` can bless pgRDF's
own bug as the expectation. An independent engine breaks that loop. Where two
independent engines agree, the evidence is real; where they disagree, the
spec is usually ambiguous at that point and the disagreement is itself worth
recording.

**The boundary.** Five rules, and they are what make this safe rather than a
back door:

1. **Output is the artefact; the tool is scaffolding.** What enters the
   repository is a static expected value in a fixture, never a runtime call
   to another engine.
2. **The suite must pass with the container absent.** If `tests/` cannot run
   on a machine that has never installed a JVM, the rule has been broken.
3. **Never in CI's required path.** Regenerating oracles is a deliberate
   human act, not a pipeline step.
4. **Every such expectation is stamped** `cross-checked`, naming tool and
   version. An unstamped value silently claims a strength it does not have.
5. **A container is not a license boundary.** Exactly as `.gitignore` is not
   (§4.2). GPL-2.0-only material stays refused no matter where it executes —
   moving the benchmark into a container does not launder it. The container
   runs permissively-licensed engines against **pgRDF's own** queries.

**[judgement]** Adopt this at T3, where hand-derivation is heaviest. T1 does
not need it — the OGC fixtures carry their own valid/invalid designation, which
is a stronger oracle (`rule-derived`) than any engine's opinion.

## 7. pgRDF's existing seams

Three measurements, all from this working copy, that determine cost.

### 7.1 Custom-IRI function dispatch already exists

`src/query/executor.rs:6465` **[read]** matches
`Expression::FunctionCall(Function::Custom(iri), args)` guarded on a namespace
prefix, and translates to SQL. The existing tenant is
`XPATH_MATH_NS`, whose comment records the design rationale:

> Chosen over an invented vendor IRI because it is what other SPARQL engines
> expose for the same functions, keeping queries portable.

**Consequence:** `geof:` needs no new extension mechanism, and the precedent
already argues for using the standard namespace. It also establishes the
error-handling contract that tier 3 must honour: domain violations yield
`NULL` ("type error → unbound"), **never a SQL abort**. GeoSPARQL has many
such cases (`geof:distance` on an empty geometry, malformed WKT). The
contract is set; follow it.

### 7.2 Typed literals need no storage change

The dictionary keys on `(term_type, lexical_value, datatype_iri_id,
language_tag)` **[read]**, with `term_type` ∈ `{URI=1, BLANK_NODE=2,
LITERAL=3}`. A `geo:wktLiteral` is a literal with a datatype IRI — already
representable, already interned, already round-trips.

**Consequence:** tier 0 is **vocabulary and validation only**. No storage
migration, no new term type, no dictionary change. This is the single largest
cost saving available and it is why T0 is cheap.

### 7.3 SHACL is strong; the gap is unprobed surface, not missing components

**Corrected 2026-09-16 after a first draft got this backwards.** A grep of
`src/validation/shacl.rs` suggested a thin engine. The authoritative source is
the generated capability document, `tests/shacl-capability/CAPABILITY.json`,
and it says the opposite **[read]**:

```
not_enforced: []
enforced: class, closed, datatype, disjoint, hasValue, in, inversePath,
          maxCount, minCount, minLength, nodeKind, pattern,
          qualifiedMinCount, targetClass, targetNode, targetSubjectsOf
enforced_only_in_mode: { sparqlConstraint: "pgrdf" }
```

Sixteen components enforced, nothing probed and found missing. `sh:pattern`,
`sh:datatype` and `sh:nodeKind` — the three a draft of this document claimed
were absent — are **all enforced**. Reading the source instead of the
instrument is precisely the error the capability harness was built to prevent.

#### What the GeoSPARQL shapes actually require

Full census of the shapes graph **[measured]**:

| SHACL term | Uses | Probed in `CAPABILITY.json`? |
|---|---|---|
| `sh:maxCount` | 15 | ✅ enforced |
| `sh:targetSubjectsOf` | 14 | ✅ enforced |
| **`sh:targetObjectsOf`** | **10** | ❌ **never probed** |
| `sh:targetClass` | 5 | ✅ enforced |
| `sh:pattern` | 5 | ✅ enforced |
| `sh:datatype` | 5 | ✅ enforced |
| `sh:sparql` / `sh:select` | 4 | ⚠️ **only in mode `pgrdf`** |
| **`sh:flags`** | **4** | ❌ **never probed** (regex flags on `sh:pattern`) |
| `sh:minCount` | 3 | ✅ enforced |
| `sh:nodeKind` | 1 | ✅ enforced |
| **`sh:alternativePath`** | **1** | ❌ **never probed** (`inversePath` is) |
| **`sh:deactivated`** | **1** | ❌ **never probed** |

**29 of 33 constraint instances land on enforced components.** The residual
risk is not missing constraints — it is **four unmeasured surfaces**, and one
of them is load-bearing.

#### The four real unknowns

1. **`sh:targetObjectsOf` — 10 uses, never probed.** This is the significant
   one. `targetSubjectsOf` is enforced; that tells us nothing about its
   converse. Ten shapes select their focus nodes this way, including the
   `S1-*` serialization-cardinality family. If unsupported, those ten shapes
   target nothing and report `conforms: true` vacuously.
2. **`sh:sparql` needs mode `pgrdf`.** `pgrdf.validate(data, shapes, mode)`
   defaults to `'native'` **[read]**, and `CAPABILITY.json` records that
   `'native'` and `'sparql'` **silently skip** `sh:sparql` while `'pgrdf'`
   evaluates it. Running the corpus in the default mode drops 4 constraints
   with no error. **The runner must pass `'pgrdf'` explicitly.**
3. **`sh:deactivated`** — if ignored, a shape upstream deliberately switched
   off would fire, producing a false failure rather than a false pass.
4. **`sh:flags`** — `sh:pattern` is enforced, but flag handling is separate,
   and pgRDF translates regex to POSIX where SHACL specifies XPath (R7).

#### The instrument itself is stale

`CAPABILITY.json` records `pgrdf_version: "0.6.22"` **[read]**; the working
copy is at `0.6.34` **[read]**. **Twelve versions of drift.** By the same
argument the harness makes about unenforced components, a capability document
generated twelve versions ago describes an engine that may no longer exist.
Regenerating it is a precondition for trusting any number in this section.

**Consequence — and this replaces the draft's "sharpest finding":** the OGC
corpus is **not** an audit exposing a weak SHACL engine. pgRDF's SHACL engine
is strong, and there is a credible chance it passes most of the corpus today.
The work in T1 is therefore **measurement, not implementation**: regenerate
the capability document at the current version, add four probes, run the
corpus in mode `'pgrdf'`, publish per-shape results. That is a much cheaper
tier than the draft assumed — and its value is a defensible published result
rather than a repaired defect.

#### One entailment caveat that will bite

`CAPABILITY.json` carries this warning **[read]**:

> `sh:targetClass` matches **ASSERTED** `rdf:type` only. A node typed only by
> a subclass is not targeted by a shape on its parent unless
> `pgrdf.materialize` has run.

GeoSPARQL's class hierarchy is exactly this shape: `geo:Feature` and
`geo:Geometry` are both subclasses of `geo:SpatialObject`, and the five
`sh:targetClass` shapes target the specific classes. Fixtures typing a node
only as a subclass will not be targeted without materialization. **The runner
must decide, and record, whether it materializes before validating** — the
two choices give different results, and neither is wrong as long as it is
stated.

---

## 8. Substrate decision

**Decision: pure-Rust `geo` + `wkt`.** Recorded 2026-09-16.

### 8.1 Rationale

- **License is clean.** `geo`, `geo-types`, `wkt`, `rstar` are all
  `MIT OR Apache-2.0` **[measured]** — no asymmetry against pgRDF's MIT.
- **DE-9IM is already there.** `geo` exposes a `Relate` trait **[measured]**.
  This is the decisive technical fact: Simple Features, Egenhofer, and RCC8
  are all *readings of one DE-9IM intersection matrix*. **Three relation
  families — 30-odd `geof:` predicates — reduce to one implementation plus
  three matrix-interpretation tables.** That is what makes T2 tractable.
- **The rest of the non-topological surface is covered:** `BooleanOps`
  (union / intersection / difference / symmetric difference), `Buffer`,
  `ConvexHull`, `ConcaveHull`, `Area`, `Centroid`, `BoundingRect`,
  `EuclideanDistance`, `HaversineDistance`, `Simplify` **[measured]**.
- **In-process, no IPC, no SPI round-trip**, consistent with pgRDF being
  Rust-native.
- **Maturity.** `geo` is at 0.33.1 with 22.4M lifetime downloads
  **[measured]** — a different order of evidence from `oxirs-geosparql`'s 563.

### 8.2 What it does not give

Stated plainly so no tier over-promises:

| Gap | Impact |
|---|---|
| **No CRS transformation** without the `proj` crate, which binds C PROJ | `geof:transform` and the metric `geof:metric*` family are out of T0–T4. Planar-only. |
| **No GML / KML / GeoJSON parsers** | Those serializations need separate crates or hand-written parsers. T4. |
| **No DGGS** | Nothing in Rust implements AusPIX. Permanently out of scope. |
| **DE-9IM completeness unverified** | `Relate` exists **[measured]**; that it yields correct matrices for every SF/Egenhofer/RCC8 case is **not yet verified**. This is T2's first task and its main risk (§11). |

### 8.3 Rejected alternatives

**PostGIS as a runtime dependency.** Rejected. It would give full Simple
Features, DE-9IM and CRS immediately, and the temptation is real. But PostGIS
is **GPL-2.0** **[measured]**, and a PostgreSQL extension calling it via SPI
executes *in the same process*. Whether that constitutes a combined work is a
genuinely unsettled question, not a settled-safe one, and pgRDF is a public
MIT project whose users inherit whatever answer is right. **[judgement]** We
do not need to resolve that question, and a project that can avoid asking it
should. Declining also keeps pgRDF self-contained, which is its stated
character.

**`geos` crate.** Rejected. The crate is MIT but binds GEOS, which is
LGPL-2.1 **[measured]**. Dynamic linking makes LGPL manageable, not free —
and it reintroduces a C dependency for capability `geo` mostly already has.

**`oxirs-geosparql`.** Rejected as a dependency. Apache-2.0 and therefore
legally fine, but 563 lifetime downloads and a first publication in October
2025 **[measured]**. **[judgement]** Worth reading for how it maps `geof:` onto
`geo`; not worth depending on.

---

## 9. Tiered feasibility

Tiers are ordered by *value delivered per unit of risk*, not by spec structure.
Each is independently shippable and independently useful.

### T0 — Vocabulary and `geo:wktLiteral`

Register the `geo:` / `geof:` namespaces; recognise `geo:wktLiteral` as a
datatype; lexical validation of WKT (parse-or-reject) via the `wkt` crate;
`geo:asWKT` round-trip.

- **No geometry mathematics. No storage change (§7.2).**
- Unlocks ATS `conf/core/*` naming and the S01–S04, S16 fixture families.
- **Risk: low.** **Cost: small.**

### T1 — SHACL conformance, by measurement

Author the fixtures (**done** — 57 over 22 rules at `tests/geosparql/`, MIT,
guarded, coverage recorded in `COVERAGE.md`); **author the shapes graph** from
the GeoSPARQL requirements, since no OGC copy is vendorable (§4.4); bash runner
invoking `pgrdf.validate(..., 'pgrdf')`; publish pass/fail per rule.

Authoring 22 shapes is the one cost this tier did not originally carry.
`RULES.md` already specifies what each rule constrains and which SHACL
mechanism it needs, so the work is transcription against a known target rather
than design — and the fixtures are a ready oracle for whether each shape
behaves.

**First, fix the instrument (§7.3):**

1. Regenerate `tests/shacl-capability/CAPABILITY.json` at the current version
   — it is pinned at `0.6.22` against a `0.6.34` tree.
2. Add four probes: `targetObjectsOf`, `alternativePath`, `deactivated`,
   `pattern`-with-`flags`.
3. Only then run the corpus, and only in mode `'pgrdf'`.

- Delivers a **real GeoSPARQL-facing capability with zero geometry code**.
- Anything the four probes find missing is a **general** SHACL improvement,
  not a GeoSPARQL one (see open question 3).
- **Risk: low.** **Cost: small** — likely measurement plus at most one or two
  components, where the draft assumed three.
- **[judgement]** Still the highest value-per-risk in the series, and cheaper
  than first believed. Do this alongside T0.

### T2 — DE-9IM core and the three relation families

Wrap `geo::Relate`; build the SF / Egenhofer / RCC8 interpretation tables on
top of one matrix; expose ~30 `geof:` predicates through the `Function::Custom`
seam (§7.1).

- Unlocks Topology Vocabulary Extension and Geometry Topology Extension.
- **Risk: moderate** — see §11 on `Relate` correctness.
- **Cost: moderate.** One implementation, three tables.

### T3 — Non-topological `geof:` functions

`distance`, `buffer`, `convexHull`, `intersection`, `union`, `difference`,
`symDifference`, `envelope`, `boundary`, `area`, `length`, `isEmpty`,
`isSimple`, `dimension`, `coordinateDimension`, `spatialDimension`,
`geometryType`, `getSRID`, `numGeometries`, `geometryN`, `min/maxX/Y/Z`.

- Must honour §7.1's **NULL-not-abort** contract throughout.
- Excludes the `geof:metric*` family (needs CRS) and `geof:transform`.
- **Risk: moderate** — every expectation is hand-derived (§6.4).
- **Cost: moderate**, and highly parallel — each function is independent.

### T4 — Serializations beyond WKT

`geo:gmlLiteral`, `geo:geoJSONLiteral`, `geo:kmlLiteral` and the matching
`geof:as*`. A cross-serialization fixture is needed and **is not vendored**
(§5.2): either re-evaluate the Moreton Island files' provenance at that point,
or author one polygon in five serializations here — a few hours of work that
avoids the question entirely.

- **Risk: moderate–high** (GML in particular is large and underspecified in
  practice). **Cost: high.** Reassess after T3.

### T5 — Explicitly out of scope

CRS transformation, `geof:metric*`, Query Rewrite Extension, RDFS Entailment
Extension, DGGS. Each needs either a C dependency, a reasoner interaction, or
an ecosystem that does not exist in Rust. Revisit only on demand.

---

## 10. Conformance claims, per tier

Written conservatively on purpose. A conformance claim is a public assertion.

| After | Honestly claimable | Explicitly not claimable |
|---|---|---|
| T0 | "Recognises and validates `geo:wktLiteral`." Not a conformance class. | Core |
| T1 | "Passes N/48 of the OGC GeoSPARQL 1.1 SHACL validator corpus, run in mode `'pgrdf'`, per-shape results published, against a capability document regenerated at the tested version." | Core — SHACL is **non-normative** (§5.4); passing it is not conformance. Also not claimable from a `'native'`-mode run (§7.3). |
| T2 | **Topology Vocabulary Extension**; *partial* Geometry Topology Extension | Geometry Extension |
| T3 | *Partial* **Geometry Extension** — WKT serialization only, planar only | Full Geometry Extension |
| T4 | Geometry Extension across the non-DGGS serializations | DGGS |

**[judgement]** Two rules to carry forward. First, **"passes the SHACL
validator" is not "conforms to GeoSPARQL"** — the validator is informative by
OGC's own declaration, and conflating them would be the exact over-claim §6.4
guards against. Second, publish the **per-shape and per-test result table**
rather than a single percentage; a percentage is the one number that cannot
be checked.

---

## 11. Risks and open questions

| # | Risk | Response |
|---|---|---|
| R1 | **`geo::Relate` correctness is unverified.** T2's whole economy rests on one matrix being right across SF/Egenhofer/RCC8. | First T2 task: a dedicated DE-9IM matrix suite from Simple Features, oracle `spec`. If it fails, T2 is re-costed, not patched. |
| R2 | **Oracle circularity** (§6.4) — `ACCEPT=1` blessing our own output. | Mandatory `oracle` marker; `eligible` barred from §10 claims; independent cross-check via a development-time container at T3 (§6.5). |
| R3 | **The shapes graph has no stable published location** (§5.4). | Pin by content digest + retrieval date in `LICENSE.md`. Never fetch at test time. |
| R4 | **OGC document/software license seam** (§4.3) may be misread by a future contributor. | §4.3 is the standing reference; vendored trees carry their own `LICENSE.md`. |
| R5 | **Upstream `master` is active**; `geosparql-next` may diverge from 1.1. | Pin to `f90fff13…`. Treat `geosparql-next` as a different standard. |
| R6 | **S05–S08 have no fixtures** (§5.3) — coverage gap inherited from upstream. | Record as a known gap. Do not silently renumber. |
| R7 | **`sh:pattern` semantics** — SHACL specifies XPath regex; pgRDF translates to POSIX, and `sh:flags` (4 uses) is unprobed. | Probe during T1. May need the same care `translate_regex` already takes. |
| R8 | **`sh:targetObjectsOf` is unprobed and carries 10 uses** (§7.3). If unsupported, ten shapes target nothing and pass vacuously. | Highest-priority T1 probe. Until measured, **no T1 result is publishable**. |
| R9 | **Default `'native'` mode silently skips `sh:sparql`** (§7.3). A corpus run in the default mode drops 4 constraints with no error. | Runner passes `'pgrdf'` explicitly and records the mode in every result. |
| R10 | **`CAPABILITY.json` is 12 versions stale** (`0.6.22` vs `0.6.34`). | Regenerate before any T1 number is quoted. Treat as the instrument's own expiry. |
| R11 | **`sh:targetClass` matches asserted `rdf:type` only** (§7.3), and GeoSPARQL's classes are a subclass hierarchy. | Runner declares whether it calls `pgrdf.materialize` first, and records it beside every result. |

**Open questions for document 2:**

1. Does `geo::Relate` produce spec-correct DE-9IM matrices? (R1 — measure
   before designing T2.)
2. Vendor the shapes graph from `geosparql-next/rdf/validators/geo.ttl`, or
   from the dereferenced OGC definitions server? They may differ; **[measured]**
   only that both exist.
3. Should T1's SHACL probes and any resulting components be a separate,
   non-GeoSPARQL work item? `targetObjectsOf`, `alternativePath`,
   `deactivated` and `flags` improve the engine for **every** consumer, and
   the board may prefer them tracked as SHACL work, with GeoSPARQL as the
   motivating case rather than the owner.
4. Do we reconcile the `S01`/`S1` naming mismatch in the runner or in a
   mapping file? (§5.4)

---

## 12. Series plan

| Doc | Subject |
|---|---|
| **1** | *This document* — landscape, licensing, method, substrate decision, tiers |
| 2 | T0+T1 design: vocabulary, `geo:wktLiteral`, vendoring plan, SHACL gap closure, runner |
| 3 | T2 design: DE-9IM verification results, relation-family tables, `geof:` dispatch |
| 4 | T3 design: function-by-function, error contract, oracle derivation |
| 5 | Conformance report: measured results, published claims, per-test table |

Documents 3–5 are provisional; R1's outcome may restructure 3 and 4.

### 12.1 What already exists in the tree

`tests/geosparql/` was created alongside this document and is **corpus and
provenance only** — no runner, by design; the runner is document 2's to
specify.

| Path | Contents |
|---|---|
| `SPIKE.pgRDF-GeoSPARQL-1.md` | This document. |
| `RULES.md` | All 24 SHACL rules named and explained, fixture mapping, per-rule pgRDF mechanism and probe status, upstream defects. |
| `LICENSE.md` | The authoritative register: every artefact evaluated, its license, its verdict, grouped by realm. |
| `license-guard.sh` | Five checks; self-tested against planted violations. Run it in CI. |
| `fetch-upstream.sh` | Pinned vendoring at `f90fff13…`, with the refusal list inline. Network here, never at test time. |
| `COVERAGE.md` | Proof the authored fixtures cover the OGC corpus: 47 of 48 scenarios, one scope exclusion, ten additions. |
| `compare-upstream.sh` | Dev-time coverage re-check against upstream; fetches to a temp dir, vendors nothing. |
| `fixtures/` | 57 pgRDF-authored fixtures over 22 rules, MIT, one directory per rule. **No shapes graph yet — T1 authors them (§4.4).** |

---

## Appendix A — Measurement log

All run 2026-09-16.

```bash
# Licenses and repository state
gh api repos/opengeospatial/ogc-geosparql
gh api repos/OpenLinkSoftware/GeoSPARQLBenchmark/contents/LICENSE.md
gh api repos/eclipse-rdf4j/rdf4j --jq .license.spdx_id
gh api repos/apache/jena --jq .license.spdx_id
gh api repos/oyvindlgjesdal/geosparql-jena   # 0 stars, pushed 2018-12-08
gh api repos/galbiston/geosparql-jena
gh api repos/postgis/postgis --jq .license.spdx_id

# Upstream pin
gh api repos/opengeospatial/ogc-geosparql/branches/geosparql-1.1 \
  --jq '.commit.sha'          # f90fff13235aaffaf69dcba97f3e01888167f080

# Corpus inventory
gh api 'repos/opengeospatial/ogc-geosparql/contents/examples/shacl?ref=geosparql-1.1'
gh api 'repos/opengeospatial/ogc-geosparql/contents/spec/sections?ref=geosparql-1.1'
gh api 'repos/opengeospatial/ogc-geosparql/contents/spec/abstract_tests?ref=geosparql-1.1'

# Shapes graph + constraint-component census
gh api repos/opengeospatial/ogc-geosparql/contents/geosparql-next/rdf/validators/geo.ttl
curl -sIL -H 'Accept: text/turtle' http://www.opengis.net/def/geosparql/validator

# Registries
curl -s https://crates.io/api/v1/crates/{geo,wkt,geos,rstar,proj,geo-types}
curl -s https://crates.io/api/v1/crates/oxirs-geosparql
curl -s 'https://search.maven.org/solrsearch/select?q=g:org.eclipse.rdf4j&rows=200&wt=json'
```

Working-copy reads: `src/query/executor.rs:6465`, `src/storage/dict.rs:13-18`,
`src/validation/shacl.rs`, `tests/w3c-sparql/README.md`,
`tests/shacl-capability/README.md`, `tests/fixtures/rdfc10/LICENSE.md`,
`Cargo.toml`.
