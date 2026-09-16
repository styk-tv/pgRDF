# License register

**Everything in this directory is pgRDF's own work, under pgRDF's MIT license.
No third-party material is vendored here.** Every fixture carries an SPDX MIT
header; `license-guard.sh` fails if one does not, or if any third-party tree
reappears.

This directory is published in a **public repository**. Third-party material
committed here would be visible to everyone, permanently, carrying whatever
obligations its license imposes on every downstream consumer. This file
records everything evaluated and why it is or is not present.

> ## ⛔ A license violation is a CRITICAL STOP
>
> Not a warning, not a TODO, not something to resolve after the commit lands.
> On finding incompatible material — or on any doubt about a file's tier —
> **stop, disclose what was found, and do not push.** Remove it, record the
> verdict below, and only then continue.
>
> - **`.gitignore` is not a license boundary.** An ignored file is still a
>   file in an otherwise-MIT tree.
> - **A container is not a license boundary.** Moving execution elsewhere does
>   not change an artefact's license.
>
> `license-guard.sh` exits non-zero on any violation. A red guard blocks the
> push; it is never overridden.

**Verdict key**

| | |
|---|---|
| **AUTHORED** | pgRDF's own work, MIT. |
| **REFUSED — license** | Incompatible or more restrictive than MIT. Not present, not used as an oracle. |
| **REFUSED — project rule** | Permissively licensed, excluded on a pgRDF rule (no Java, no Python in the shipped path). Usable at development time. |
| **CANDIDATE** | Permissive and compatible. Under consideration for engine work; not yet a dependency. |
| **CITED** | A standard we implement and reference. Never copied. |

---

## What is here

| Path | Origin | License |
|---|---|---|
| `fixtures/` — 57 fixtures over 22 rules | **Authored by pgRDF** | **MIT** |
| `RULES.md`, `COVERAGE.md`, `README.md`, `SPIKE.*`, `*.sh` | **Authored by pgRDF** | **MIT** |

Fixtures were written from the GeoSPARQL 1.1 rule definitions, not copied.
`COVERAGE.md` shows they cover 47 of the 48 scenarios in the OGC corpus, with
one exclusion recorded on scope and ten cases added.

---

## Register

### OGC GeoSPARQL upstream

| Artefact | License | Verdict |
|---|---|---|
| `examples/shacl/*.ttl` (48 fixtures) | Apache-2.0 **by inference only** — see below | **Not vendored.** Read as reference; equivalents authored instead. |
| SHACL shapes graph — `geosparql-next/rdf/validators/geo.ttl` | OGC Document License (self-declared `schema:license`) | **REFUSED — license** |
| SHACL shapes graph — published at `opengis.net/def/geosparql/validator` | OGC Document License (self-declared `sdo:license`) | **REFUSED — license** |
| `examples/demo-dataset.ttl` | CC-BY-4.0 (self-declared `dcterms:license`) | **REFUSED — license** |
| `examples/moreton-island.*` | Undeclared | **REFUSED — license** |
| `spec/sections/*.adoc`, incl. Abstract Test Suite Annex A | OGC Document License | **REFUSED — license** |
| `examples/shacl/test_shapes.py` | Apache-2.0 | **REFUSED — project rule** (Python) |

**Why the fixtures were not vendored despite appearing to be Apache-2.0.**
The claim rests on a single README sentence — *"Software and data ... Apache
Software License 2.0"* — and:

- there is **no `LICENSE` file** on the `geosparql-1.1` branch the fixtures
  live on, nor on `master`; GitHub detects no license for the repository;
- the fixtures **carry no declaration of their own**;
- the OGC policy page that sentence links to lists license *versions*, it does
  not assign them to artefacts;
- `demo-dataset.ttl`, in the same `examples/` tree, declares CC-BY-4.0 —
  proving the tree is not uniformly Apache-2.0.

The `geosparql-1.0` branch *does* carry a LICENSE — the OGC License
Agreement, which is permissive (*"deal in the Intellectual Property without
restriction ... copy, modify, merge, publish, distribute"*, with copyright
notices retained and modifications marked). So OGC's posture, where declared,
is permissive, and the risk was low.

But low risk resting on an inference is not the standard this repository holds
others to, and re-authoring removed the question entirely. **The rule that
follows: a file's own license declaration outranks the directory it sits in,
and an absent declaration is not a permissive one.**

### Test suites and benchmarks

| Artefact | License | Verdict |
|---|---|---|
| `OpenLinkSoftware/GeoSPARQLBenchmark` (206 queries) | **GPL-2.0-only** | **REFUSED — license** |
| `rdf4j-geosparql-testsuite` / `-compliance` | BSD-3-Clause | **REFUSED — project rule** (Java) |

GPL-2.0-only material is not cloned, vendored, adapted, or transcribed — not
into ignored or scratch directories either. Method may be taken from the
published paper (arXiv:2102.06139); artefacts may not.

### Java realm — excluded from the shipped path

Permissively licensed. Excluded on the project rule, not on license.
**Permitted for development-time use** — see below.

| Artefact | License | Verdict |
|---|---|---|
| `apache/jena` (jena-geosparql) | Apache-2.0 | **REFUSED — project rule** |
| `eclipse-rdf4j/rdf4j` | BSD-3-Clause | **REFUSED — project rule** |
| `galbiston/geosparql-jena` | Apache-2.0 | **REFUSED — project rule** |

### Python realm — excluded from the shipped path

Permissively licensed. Excluded on the project rule, not on license.
**Permitted for development-time use** — see below.

| Artefact | License | Verdict |
|---|---|---|
| `pyshacl` | Apache-2.0 | **REFUSED — project rule** |
| `rdflib` | BSD-3-Clause | **REFUSED — project rule** |
| `pytest` | MIT | **REFUSED — project rule** |

### Rust crates — under consideration

Evaluated for the geometry substrate. None is yet a dependency.

| Crate | Version | License | Verdict |
|---|---|---|---|
| `geo` | 0.33.1 | MIT OR Apache-2.0 | **CANDIDATE** |
| `geo-types` | 0.7.20 | MIT OR Apache-2.0 | **CANDIDATE** |
| `wkt` | 0.14.0 | MIT OR Apache-2.0 | **CANDIDATE** |
| `rstar` | 0.13.0 | MIT OR Apache-2.0 | **CANDIDATE** |
| `proj` | 0.31.0 | MIT OR Apache-2.0 (binds C PROJ, MIT-style) | **CANDIDATE**, deferred — C dependency |
| `geos` | 11.1.2 | MIT crate over **LGPL-2.1** GEOS | **REFUSED — license** (transitive) |
| `oxirs-geosparql` | 0.4.1 | Apache-2.0 | Compatible; not adopted — 563 lifetime downloads, first published 2025-10 |

### Standards documents — cited, never redistributed

Citation is not redistribution. Identifiers, conformance-class names and
vocabulary terms are facts and are used freely; specification **prose** is
never copied. This tree quotes zero lines of any specification.

| Document | License | Verdict |
|---|---|---|
| OGC GeoSPARQL 1.1 (22-047r1) | OGC Document License | **CITED** — implement, never copy prose |
| **ISO 19125-1:2004** — Simple feature access Part 1 (GeoSPARQL's normative reference for **WKT**, not GeoSPARQL itself) | ISO copyright — sold per copy, not redistributable | **Not purchased, not vendored, not quoted** |
| **OGC 06-103r4** — Simple Feature Access Part 1, the freely published OGC twin of ISO 19125-1 | OGC Document License, free to download | **CITED** — working source for Simple Features / DE-9IM |

Implementing a standard that cites ISO 19125-1 does not require licensing it.
pgRDF works from the free OGC twin and cites the ISO number as GeoSPARQL does.

### Adjacent systems

| Artefact | License | Verdict |
|---|---|---|
| PostGIS | **GPL-2.0** | **REFUSED — license**. A PostgreSQL extension calling it via SPI runs in the same process; pgRDF does not link, depend on, or require it. |

---

## Development-time use

Artefacts marked **REFUSED — project rule** are permissively licensed. Their
licenses permit local use, and pgRDF's no-Java / no-Python rules exist to keep
a JVM or interpreter out of the **build, test and ship path** — not to forbid
consulting a reference implementation.

**Permitted:** running Jena, RDF4J or pySHACL in an adjacent throwaway
container during development, to cross-check expected values before they are
committed as static fixtures.

**Required, in every case:**

1. Only the **output** enters the repository — a static expected value, never
   a runtime call to another engine.
2. `tests/` must pass on a machine with no JVM and no Python.
3. Never in CI's required path.
4. Every value so derived is stamped `cross-checked`, naming tool and version.
5. **Artefacts marked REFUSED — license may not be run in such a container.**
   A container is not a license boundary.

Full rationale: `SPIKE.pgRDF-GeoSPARQL-1.md` §6.5.

---

## Enforcement

```bash
bash tests/geosparql/license-guard.sh      # run in CI
bash tests/geosparql/compare-upstream.sh   # dev-time coverage check; vendors nothing
```

The guard checks: no vendored third-party tree; an SPDX MIT header on every
fixture; no foreign licensing declaration; no copyleft or non-commercial term;
nothing but `.ttl` under `fixtures/`. Self-tested against planted violations.
