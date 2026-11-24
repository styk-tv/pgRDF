# tests/w3c-shacl — W3C SHACL conformance harness

LLD v0.5 §6 (shipped in
v0.5.0). The third correctness gate, alongside
[`tests/w3c-sparql/`](../w3c-sparql/) (W3C SPARQL-shape) and
`tests/regression/` (pg_regress). Wired into `ci.yml` on every PG
major and runnable locally via `just test-shacl-manifest`.

## Layout

    tests/w3c-shacl/
    ├── README.md                       (this file)
    ├── run.sh                          (the harness)
    └── fixtures/
        ├── NOTICE.md                   (source commit + W3C license notice)
        ├── core/                       (SHACL Core — the `native` gate)
        │   ├── <name>.w3c.ttl          (unmodified W3C test file)
        │   ├── <name>.ttl              (the same file without its manifest)
        │   └── <name>.expected.json    ({conforms}, from the W3C mf:result)
        └── sparql/                     (SHACL-SPARQL — same three files)

The `.ttl` fixtures are a curated, **vendored** subset of the W3C
`data-shapes-test-suite` SHACL Core tests
(<https://github.com/w3c/data-shapes>, `gh-pages`). They are checked
into the repo — the harness never fetches at test time, so CI is
hermetic (same discipline as the w3c-sparql shape fixtures). The
pinned source commit and the W3C license notice are in
[`fixtures/NOTICE.md`](fixtures/NOTICE.md).

Each test ships as THREE files: `<name>.w3c.ttl` (the **unmodified**
W3C source, kept for provenance + to derive the expected from its
`mf:result`), `<name>.ttl` (the data + shapes split — the W3C suite
roots its `mf:Manifest` at the empty relative IRI `<>`, which oxttl
rejects without a base; the SHACL engine never needs the manifest
triples, so the harness loads the `<>`-free split) and
`<name>.expected.json`. The harness skips the `*.w3c.ttl` provenance
copies.

The split removes exactly two top-level statements — the `<>`
`mf:Manifest` statement and the `<test>` `sht:Validate` entry — and
keeps every other statement as it is, including a labelled
blank-node statement that follows the entry (in `node/datatype-001`
that statement is a focus node of the test). A fixture is named after
its W3C path: `node/and-001.ttl` → `node-and-001`,
`property/datatype-001.ttl` → `prop-datatype-001`,
`path/path-inverse-001.ttl` → `path-inverse-001`,
`targets/targetClass-001.ttl` → `target-targetClass-001`,
`misc/severity-001.ttl` → `misc-severity-001`,
`complex/personexample.ttl` → `complex-personexample`.

**Status (pgRDF 0.6.40):** **56 of the 98** W3C SHACL **Core** tests
are vendored in `fixtures/core/`, and all 56 pass in `native` mode.
A pass means pgRDF returns the same `sh:conforms` boolean as the
test's W3C `mf:result` — what the W3C suite calls *partial
compliance*. *Full compliance* (a validation report isomorphic to
the expected one) is not graded; see ERRATA E-013 for why `conforms`,
not violation count, is the gate. No fixture is excluded for
failing: `prop-nodeKind-001` is graded in `fixtures/core/` and passes
with the W3C-authoritative `conforms:false` result. The 42 tests that
are not vendored are listed under [Tried, not vendored](#tried-not-vendored).

Each vendored W3C test is a self-contained `.ttl`: data triples +
`sh:` shapes + an `mf:Manifest` whose `mf:result` carries the spec-authoritative
`sh:ValidationReport`. Per the W3C suite convention
`sht:dataGraph <>` and `sht:shapesGraph <>` both point at the file
itself, so the harness loads the split into ONE pgRDF graph and
validates it against itself (`pgrdf.validate(g, g)`). pgRDF raises a
`WARNING` on every such call (data and shapes are the same graph);
that is expected here.

## Coverage

Vendored fixtures per W3C directory, of the 98 `sht:Validate` tests
under `data-shapes-test-suite/tests/core/` at the pinned commit:

| Area | W3C tests | Vendored |
|---|---|---|
| `node/` | 32 | 17 |
| `property/` | 38 | 23 |
| `path/` | 13 | 7 |
| `targets/` | 7 | 6 |
| `misc/` | 5 | 2 |
| `complex/` | 2 | 1 |
| `validation-reports/` | 1 | 0 |
| **total** | **98** | **56** |

The first 25 fixtures cover value-type, range, string, cardinality
and logical constraints and the `sh:targetClass` / `sh:targetNode`
targets. The 31 added at 0.6.40 take one test for each constraint
component, path form and target form those 25 did not reach, plus
`sh:deactivated`, `sh:severity` and the W3C person example; where
such a component has both a node-shape and a property-shape test,
one of the two was taken.

## Tried, not vendored

All 98 tests were graded against pgRDF 0.6.40 (PostgreSQL 18) when
this selection was made, with `expected` read from the W3C file
first. **None failed on the `sh:conforms` verdict**: no test where
pgRDF's boolean differs from the W3C one, and none that raises an
error. The 42 tests below are left out for the reason in the last
column, not for a result. Their rows are a one-time measurement; only
the 56 vendored fixtures are gated.

- 34 are single-file tests. They were run through this harness
  unchanged (same split, same `expected.json` derivation) from a
  scratch copy of the fixture directory.
- 8 keep data and shapes in separate files
  (`sht:dataGraph <x-data.ttl>`, `sht:shapesGraph <x-shapes.ttl>`).
  The harness loads one file as both graphs and cannot run them; they
  were checked by hand with `pgrdf.validate(data_graph, shapes_graph)`.
  Vendoring them needs a two-file mode in `run.sh`.

| W3C test | W3C `sh:conforms` | pgRDF 0.6.40 | Not vendored because |
|---|---|---|---|
| `node/and-002` | false | false | outside this selection |
| `node/class-002` | false | false | outside this selection |
| `node/class-003` | false | false | outside this selection |
| `node/closed-002` | false | false | outside this selection |
| `node/datatype-002` | false | false | outside this selection |
| `node/disjoint-001` | false | false | outside this selection |
| `node/equals-001` | false | false | outside this selection |
| `node/languageIn-001` | false | false | outside this selection |
| `node/maxExclusive-001` | false | false | outside this selection |
| `node/minInclusive-002` | false | false | outside this selection |
| `node/minInclusive-003` | false | false | outside this selection |
| `node/not-002` | false | false | outside this selection |
| `node/pattern-002` | false | false | outside this selection |
| `node/qualified-001` | false | false | data and shapes are separate files |
| `node/xone-duplicate` | false | false | data and shapes are separate files |
| `property/datatype-003` | false | false | outside this selection |
| `property/datatype-ill-formed` | false | false | data and shapes are separate files |
| `property/lessThan-002` | false | false | outside this selection |
| `property/maxCount-002` | false | false | outside this selection |
| `property/minCount-002` | true | true | outside this selection |
| `property/minExclusive-001` | false | false | outside this selection |
| `property/minExclusive-002` | false | false | outside this selection |
| `property/node-001` | false | false | outside this selection |
| `property/node-002` | false | false | outside this selection |
| `property/or-datatypes-001` | false | false | outside this selection |
| `property/pattern-002` | false | false | outside this selection |
| `property/property-001` | false | false | outside this selection |
| `property/qualifiedMinCountDisjoint-001` | false | false | outside this selection |
| `property/qualifiedValueShapesDisjoint-001` | false | false | outside this selection |
| `property/uniqueLang-002` | true | true | data and shapes are separate files |
| `path/path-complex-002` | false | false | data and shapes are separate files |
| `path/path-sequence-002` | false | false | outside this selection |
| `path/path-sequence-duplicate-001` | false | false | outside this selection |
| `path/path-strange-001` | false | false | outside this selection |
| `path/path-strange-002` | false | false | outside this selection |
| `path/path-unused-001` | false | false | data and shapes are separate files |
| `targets/targetSubjectsOf-002` | false | false | outside this selection |
| `misc/deactivated-002` | false | false | outside this selection |
| `misc/message-001` | false | false | `sh:conforms` is the same with or without `sh:message` |
| `misc/severity-002` | false | false | outside this selection |
| `complex/shacl-shacl` | true | true | data and shapes are separate files |
| `validation-reports/shared` | false | false | data and shapes are separate files |

## Comparison invariant — `{conforms}` (ERRATA.v0.5 E-013)

`run.sh` compares the validator's **`sh:conforms` boolean** against the
committed `expected.json` (`{"conforms":true|false}`), read from each
fixture's W3C `mf:result` `sh:conforms` (the spec-authoritative
answer) — never from pgRDF's output. Never auto-blessed — `ACCEPT=1`
refuses to overwrite an existing `expected.json`.

`conforms` is the W3C suite's *partial compliance* criterion: the
implementation "produces the same boolean value for `sh:conforms` as
required by the expected report". The suite's *full compliance*
criterion — a report isomorphic to the expected one — is not checked
here. 55 of the 56 vendored tests (94 of all 98) expect `false`, so
for those the verdict alone does not show that the *right* results
were reported. The violation **count** is printed for diagnostics but
is **not** a gate criterion: the W3C fixtures use blank-node focus
nodes whose identity does not survive pgRDF's dictionary-encoded
N-Triples rehydrate byte-stable, so a blank-node-focus violation can
be relabelled/coalesced and the count drift by ±1 *without* a
conformance error (the same blank-node-relabel reason focus-node
IRIs are excluded). A genuinely missed or spurious constraint flips
`conforms` (caught); a serialization artifact does not (tolerated).
Full rationale: `specs/ERRATA.v0.5.md` **E-013**.

Measured on pgRDF 0.6.40, for information only: for all 56 vendored
fixtures the printed count equals the number of `sh:result` entries in
the W3C expected report. Five node fixtures (`node-datatype-001`,
`node-maxInclusive-001`, `node-maxLength-001`, `node-minLength-001`,
`node-pattern-001`) used to print one fewer: their split had dropped
the blank-node focus statement that follows the manifest entry. It is
restored; the verdicts did not change.

## Modes

| Invocation | Mode | Fixtures | Gate |
|---|---|---|---|
| `just test-shacl-manifest` | `pgrdf.validate(g,g)` — `'native'` | `fixtures/core/` | **hard gate** — every vendored Core fixture matches its W3C `sh:conforms` |
| `just test-shacl-manifest --pgrdf` | `pgrdf.validate(g,g,'pgrdf')` | `fixtures/sparql/` | **hard gate** — SHACL-SPARQL through the pgRDF-native handler, matched against `expected.json` |
| `just test-shacl-manifest --sparql` | `pgrdf.validate(g,g,'sparql',false)` | `fixtures/sparql/` | contract only — `conforms` is a real Boolean |

`--pgrdf` is the SHACL-SPARQL conformance gate: the fixtures under
`fixtures/sparql/` run through pgRDF's own `sh:sparql` handler and the
`sh:conforms` verdict is compared with the W3C one, exactly as in the
default mode.

`--sparql` runs the same SHACL-SPARQL fixtures through the upstream
(rudof) `SparqlEngine`. ERRATA.v0.5 **E-012** (the engine was an
upstream stub, so `'sparql'` returned `conforms:null`) was resolved by
`shacl 0.3.2`: the mode now dispatches into the engine. Its
`sh:sparql` evaluation is upstream-incomplete (ERRATA.v0.6 **E-014**),
so this sub-run does **not** compare against the W3C verdict. It
asserts only the pgRDF-side contract: the call completes and
`conforms` is `true` or `false`, not JSON `null`. It passes
`strict => false` because the default fail-closed guard refuses a
shapes graph whose constraints the chosen mode does not evaluate.

Vendoring under `fixtures/sparql/` is incremental; each sub-run gates
only on the fixtures present in the directory.

## Runner

```bash
bash tests/w3c-shacl/run.sh                   # Core, native (the gate)
bash tests/w3c-shacl/run.sh --pgrdf           # SHACL-SPARQL, pgRDF-native (gate)
bash tests/w3c-shacl/run.sh --sparql          # SHACL-SPARQL, upstream engine (contract)
bash tests/w3c-shacl/run.sh node-datatype-001 # one fixture
```

Exit 0 iff every selected fixture matches its expected.
