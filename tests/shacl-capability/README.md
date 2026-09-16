# tests/shacl-capability — the SHACL capability document

Generates [`CAPABILITY.json`](CAPABILITY.json): for each SHACL constraint
component and target selector, **does `pgrdf.validate` actually enforce it
on this build?**

## Why

Consumers choose shapes against what the engine enforces, not against what
the SHACL specification defines. Those two sets are not the same, and the
difference used to be invisible: an unimplemented component contributed no
violation **and no error**, so `conforms:true` did not distinguish *validated
clean* from *never evaluated*.

**That changed for constraint components.** As of `0.6.34` the engine is
fail-closed: an unenforced component **raises**, naming both the component and
the mode that does evaluate it, instead of returning a verdict it cannot stand
behind. The harness records that as `refused-fail-closed` — unsupported, but
impossible to mistake for a clean pass.

**It has not changed for property paths.** The fail-closed check covers
constraint components only. `sh:oneOrMorePath` and `sh:zeroOrMorePath` match
nothing, report zero violations and raise nothing, so a shape using either is
silently unvalidated — measured 0.6.34, and the original hazard exactly. Do
not use those two path types in a shape whose verdict matters.

Before this harness the allowlist lived in prose. A shape chosen against
prose is a shape chosen against a guess.

**Coverage, with its denominator.** 47 of 47 known SHACL Core features are
probed. **46 are enforced and none are unenforced**; `sh:sparql` is enforced in
mode `'pgrdf'` only.

`not_enforced: []` is what this file reported for a year while measuring 17 of
46. It reports the same thing now at 47/47 with `surface_complete: true`. The
words are identical; only the denominator makes one of them true.

The denominator matters more than the count. This harness measured **17 of 46**
features for a year and reported `not_enforced: []` — true of the seventeen,
and read by everything downstream as a clean bill of health for the surface.

> A green suite is not coverage. Coverage is a fraction, and the denominator
> must not be computable from the numerator.

`completeness.sh` enforces that. Its denominator is `SHACL-CORE-SURFACE.tsv`,
enumerated from the W3C SHACL Recommendation with a spec section per row —
**not** from this directory. Adding a probe cannot extend the target it is
measured against; only a change to the specification, or a corrected reading
of it, can, and that is a reviewable edit rather than a side effect of writing
a test.

```bash
bash tests/shacl-capability/completeness.sh   # run in CI, and before any release
```

It fails in both directions: a listed feature with no probe (**UNMEASURED**),
and a probe with no listed feature (**UNLISTED** — either the enumeration is
incomplete or the probe is misnamed). Self-tested against both. It found
`predicatePath` unmeasured on its first run — the basic path every other probe
uses implicitly and nothing had ever measured on its own.

**What it does not do:** it asserts a probe *exists* per feature, never that
the probe is right. A wrong probe passes it — which is what the path matrix
below exists to correct.

## The path matrix — where a per-feature verdict cannot reach

```bash
bash tests/shacl-capability/path-matrix.sh            # writes PATH-MATRIX.json
bash tests/shacl-capability/path-matrix.sh --print    # table only
```

Some faults live in the **interaction** between a path type and the term type
of the value it reaches. One verdict per feature cannot express that, and
before this matrix existed `CAPABILITY.json` was wrong about paths **in both
directions**:

| | Said | Actually |
|---|---|---|
| `oneOrMorePath` / `zeroOrMorePath` | `SILENTLY-SKIPPED` | **overstated** — both were evaluated; only literal values reached through them were lost |
| `sequencePath` | `enforced` | **understated** — a literal partway along `sh:path ( ex:p ex:q )` discarded the whole value set, including a sibling IRI value still reachable |

**That defect is now fixed** — see below — but the structural point stands: a
per-feature verdict cannot describe a per-cell fault, so read `PATH-MATRIX.json`
for path behaviour. It measures every path type
against three term rows — `iri`, `literal` (at the endpoint), and
`literal-intermediate` (partway along, with a sibling IRI value that should
survive) — asking two questions per cell: is the value in the value set at all
(`seen`), and does a value-level constraint fire on it (`checked`).

**17 cells, 0 defective** against the pinned build. It measured **5 defective**
on stock `shacl 0.3.2` — the two recursive path types with a literal endpoint,
and all three multi-hop paths with a literal partway along. `inverse/literal`
is `n/a-by-rdf` rather than unmeasured: the value of `^ex:p` is a subject, and
a literal cannot be one.

Two design notes worth keeping:

- **`zeroOrOnePath` and `zeroOrMorePath` use `maxCount 1`, not `0`.** Both
  include the focus node via the zero-length path, so their value set is never
  empty and `maxCount 0` would fire whether or not the value was seen.
- **The third term row was not in the first version of this matrix**, which
  tested endpoints only and reported `sequence` as clean. The endpoint and
  intermediate cases are genuinely different faults; conflating them hides one.

Root cause was upstream — rudof-project/rudof#818, fixed by
rudof-project/rudof#819. pgRDF pins that commit via `[patch.crates-io]` until
the PR merges and a `shacl` release carries it; see the block at the end of
`Cargo.toml`.

## Method

Each probe is three checked-in `.ttl` files — hermetic, no fetch at test time:

    <component>.shapes.ttl      the shapes graph
    <component>.violating.ttl   data breaking exactly that component
    <component>.control.ttl     data satisfying it

Shapes and data load into **separate** graphs, because that is how a caller
validates, and because a self-validating graph makes the shape node its own
typed subject — which silently breaks any probe using `sh:targetSubjectsOf`.

A component is **enforced** only when `violating => conforms:false` **and**
`control => conforms:true`. The control rules out an engine that reports
`false` for everything; the violating case rules out a silent skip.

## Running

    ./run.sh            # regenerate CAPABILITY.json
    ./run.sh --print    # table only, no file write

Uses standard `PG*` environment variables. Creates and drops its own scratch
graphs; it writes nothing else and is safe against a shared database.

**`CAPABILITY.json` is generated. Never hand-edit it.** Regenerate after any
change to the `shacl` crate pin, the validator, or the PG major.

## Reading the output

`not_enforced` is the load-bearing field. A component listed there is one a
shapes graph may reference and receive no enforcement for, with no diagnostic.

The `caveats` array carries two facts no probe table can express:

1. **Validation does not entail.** `sh:targetClass` matches *asserted*
   `rdf:type` only. A node typed solely by a subclass is not targeted by a
   shape on its parent unless `pgrdf.materialize` has run or the parent type
   was stamped explicitly.
2. **Absence is silent.** See the `not_enforced` note above, and pgRDF#80.
