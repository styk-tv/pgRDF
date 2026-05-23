# tests/w3c-shacl — W3C SHACL conformance harness

[LLD v0.5 §6](../../specs/SPEC.pgRDF.LLD.v0.5.md) (shipped in
v0.5.0). The third correctness gate, alongside
[`tests/w3c-sparql/`](../w3c-sparql/) (W3C SPARQL-shape) and
`tests/regression/` (pg_regress). Wired into `ci.yml` on every PG
major and runnable locally via `just test-shacl-manifest`.

## Layout

    tests/w3c-shacl/
    ├── README.md                       (this file)
    ├── run.sh                          (the harness)
    └── fixtures/core/
        ├── <name>.ttl                  (vendored W3C SHACL Core test)
        └── <name>.expected.json        (hand-derived {conforms})

The `.ttl` fixtures are a curated, **vendored** subset of the W3C
`data-shapes-test-suite` SHACL Core tests
(<https://github.com/w3c/data-shapes>, `gh-pages`). They are checked
into the repo — the harness never fetches at test time, so CI is
hermetic (same discipline as the w3c-sparql shape fixtures).

Each test ships as TWO files: `<name>.w3c.ttl` (the **unmodified**
W3C source, kept for provenance + to hand-derive the expected from
its `mf:result`) and `<name>.ttl` (the data + shapes split — the W3C
suite roots its `mf:Manifest` at the empty relative IRI `<>`, which
oxttl rejects without a base; the SHACL engine never needs the
manifest triples, so the harness loads the `<>`-free split). The
harness skips the `*.w3c.ttl` provenance copies.

**Status:** the vendored W3C SHACL **Core** suite is a genuine
**full-pass — 25 / 25** on the `sh:conforms` invariant (see ERRATA
E-013 for why `conforms`, not violation count, is the gate). There
is no excluded Core fixture: `prop-nodeKind-001` is graded in
`fixtures/core/` and passes with the W3C-authoritative
`conforms:false` result.

Each fixture is a self-contained W3C test `.ttl`: data triples +
`sh:` shapes + an `mf:Manifest` whose `mf:result` carries the
spec-authoritative `sh:ValidationReport`. Per the W3C suite
convention `sht:dataGraph <>` and `sht:shapesGraph <>` both point at
the file itself, so the harness loads the whole `.ttl` into ONE
pgRDF graph and validates it against itself (`pgrdf.validate(g, g)`).
The SHACL engine acts only on the `sh:*` shapes + their targets and
ignores the `mf:` / `sht:` manifest triples (they declare no SHACL
constraint).

## Comparison invariant — `{conforms}` (ERRATA.v0.5 E-013)

`run.sh` compares the validator's **`sh:conforms` boolean** against a
hand-derived `expected.json` (`{"conforms":true|false}`), derived
from each fixture's W3C `mf:result` `sh:conforms` (the
spec-authoritative answer) and committed alongside the fixture.
Never auto-blessed — `ACCEPT=1` refuses to overwrite an existing
`expected.json`.

`conforms` is the headline W3C SHACL conformance signal: a validator
that decides conformance correctly is W3C-conformant at the
report level. The violation **count** is printed for diagnostics but
is **not** a gate criterion: the W3C fixtures use blank-node focus
nodes whose identity does not survive pgRDF's dictionary-encoded
N-Triples rehydrate byte-stable, so a blank-node-focus violation can
be relabelled/coalesced and the count drift by ±1 *without* a
conformance error (the same blank-node-relabel reason focus-node
IRIs are excluded). A genuinely missed or spurious constraint flips
`conforms` (caught); a serialization artifact does not (tolerated).
Full rationale: `specs/ERRATA.v0.5.md` **E-013**.

## Modes

| Invocation | Mode | Gate |
|---|---|---|
| `just test-shacl-manifest` | `pgrdf.validate(g,g)` — `'native'` | **§6.1 #1 — Core full-pass (hard gate)** |
| `just test-shacl-manifest --sparql` | `pgrdf.validate(g,g,'sparql')` | §6.1 #2 — ERRATA.v0.5 E-012 known-state |

`--sparql` runs the same Core fixtures through the upstream SPARQL
mode surface. Per
[`specs/ERRATA.v0.5.md`](../../specs/ERRATA.v0.5.md) **E-012**,
`shacl 0.3.1` has **no SHACL-SPARQL (`sh:select`) constraint
component** and its `SparqlEngine` is an upstream stub
(`unimplemented!()`). pgRDF therefore does **not** invoke it:
`pgrdf.validate(..., 'sparql')` returns a deterministic structured
report (`conforms:null` + an `error` naming the gap) for every
fixture. The `--sparql` sub-run asserts exactly that bounded known
state rather than a raw failure. A true W3C SHACL-SPARQL manifest
cannot pass with the current upstream crate and is NOT vendored here
— it is fully scoped in E-012 and revisited when a future rudof
release ships the engine.

## Runner

```bash
bash tests/w3c-shacl/run.sh                   # Core, native (the gate)
bash tests/w3c-shacl/run.sh --sparql          # sparql sub-run
bash tests/w3c-shacl/run.sh node-datatype-001 # one fixture
```

Exit 0 iff every selected fixture matches its hand-derived expected.
