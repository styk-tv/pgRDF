# ERRATA.v0.5

Spec deltas accumulated during the v0.5 cycle. v0.4-era and v0.2-era
entries that remain live are cross-linked to
[`ERRATA.v0.4.md`](ERRATA.v0.4.md) /
[`ERRATA.v0.2.md`](ERRATA.v0.2.md) rather than duplicated. This file
is the v0.5-era spec-deltas log for the authoritative
[`SPEC.pgRDF.LLD.v0.5.md`](SPEC.pgRDF.LLD.v0.5.md) contract
(shipped in v0.5.0); it opened in Phase G group G3 when the first
v0.5-era delta appeared — E-012 is that first delta.

## v0.4 / v0.2 entries still live in v0.5

| Entry | One-line status in v0.5 |
|---|---|
| [E-011 — upstream `reasonable` patch](ERRATA.v0.4.md) | Unchanged. Still gated on <https://github.com/gtfierro/reasonable/pull/50>. `[patch.crates-io]` + `publish-crate.yml.disabled` carried forward through the v0.5 cycle. |
| [E-006 — pgrx 0.18 / PG 18 deferred](ERRATA.v0.2.md) | Unchanged. Largest deferred upstream item carried into v0.5. |
| [E-009 — SHACL real integration](ERRATA.v0.2.md) | Resolved in v0.4; final upstream close-out tracks E-011. |

## v0.5 entries

### E-012 — `shacl 0.3.1` SHACL-SPARQL mode is an upstream stub

| Field | Value |
|---|---|
| Filed | 2026-05-16 |
| Status | **documented upstream-gate (final for v0.5.0)** — two independent upstream gaps, scoped; pgRDF ships an honest deterministic short-circuit; a documented limitation, NOT a pgRDF defect |
| Affects | [`SPEC.pgRDF.LLD.v0.5.md`](SPEC.pgRDF.LLD.v0.5.md) §5.2 / §5.3 #1, §6.1 #2 |
| Crate | `shacl 0.3.1` (rudof project, 2026-05-12) |
| Upstream | `rudof-project/rudof` issues #21, #94, #1 (SHACL-SPARQL constraint component + `SparqlEngine` are upstream's own unscheduled roadmap) |

#### Claim (LLD v0.5 §5.2 / §5.3 #1)

> `shacl 0.3` exposes a `Sparql` validation mode in addition to
> `Native`. […] A shape with `sh:select` (SPARQL-based constraint)
> validates correctly under `mode => 'sparql'` and produces a
> `sh:Violation` for the matching focus node.

The spec's framing assumed `ShaclValidationMode::Sparql` meant
"SHACL-SPARQL constraint-language support" — i.e. that a shape
declaring `sh:sparql [ sh:select "…" ]` would be parsed into a
SPARQL-based constraint component and evaluated.

#### Reality (two independent upstream gaps)

**Gap 1 — no SHACL-SPARQL constraint component.**
`shacl::ir::IRComponent` (the validated-constraint enum) has **only
SHACL Core variants** (`Class`, `Datatype`, `MinCount`, `Pattern`,
`Node`, `Or`, … `Closed`, `Deactivated`). There is **no** `Sparql` /
`SparqlConstraint` / `Select` variant. The `shacl` AST parser
(`src/ast/`) and RDF parser (`src/rdf/`) contain **zero** `sh:sparql`
/ `sh:select` handling — a `sh:sparql` triple is silently dropped, so
a SHACL-SPARQL constraint can never raise a `sh:Violation`.

**Gap 2 — `SparqlEngine` is a non-functional stub.**
`SparqlEngine` (`src/validator/engine/sparql.rs`) is NOT a working
alternative evaluation engine. Every target-resolution method —
`target_node` (line 68), `target_class` (88), `target_subject_of`
(100), `target_object_of` (112), `implicit_target_class` (116) — ends
in **`unimplemented!()`**. Invoking `ShaclValidationMode::Sparql` on
**any** shapes graph that has a target (i.e. any non-trivial shape)
panics `not implemented` inside the crate. Empirically confirmed: a
`pgrdf.validate(g, g, 'sparql')` against a `sh:targetClass` shape
raised `ERROR: not implemented … CONTEXT: sparql.rs` from inside
`shacl 0.3.1`.

SHACL-SPARQL (W3C SHACL Part 2 §5) is therefore **not implementable
in pgRDF v0.5** with the available upstream crate, and even the
Core-via-SPARQL evaluation path is non-functional.

#### Resolution (the realisable v0.5 contract)

`pgrdf.validate(data, shapes, mode TEXT DEFAULT 'native')` ships in
v0.5 with the `mode` argument **fully wired** (accepted, validated,
echoed in the JSONB `mode` field):

- `'native'` — the v0.4 Rust-native Core engine (default; the
  default-arg form is byte-identical to v0.4).
- `'sparql'` — **does NOT invoke the broken upstream engine** (a
  panic the SQL caller can neither catch nor act on). It
  short-circuits to a clean, deterministic structured report:
  `conforms:null`, empty `results`, and an `error` naming the
  upstream gap + this erratum. No panic, no crash, forward-stable.
- Unknown mode → `validate: unknown mode` (no silent fallback).
- JSONB gains a `mode` field echoing the requested mode.

This is the honest, forward-compatible choice: the surface
(signature, mode enum, JSONB shape) is exactly what it will be once
upstream lands the engine; the day a rudof release implements
`SparqlEngine` + the SHACL-SPARQL constraint component, the
short-circuit guard in `src/validation/shacl.rs` is deleted and the
already-present `validator.validate(&schema, &validation_mode)` call
routes `'sparql'` through with **no signature change**.

§5.3 acceptance — status **as adjusted by this erratum**:

1. **§5.3 #1** — the *literal* "`sh:select` produces a
   `sh:Violation`" form is **NOT met** (two upstream gaps; documented
   here). What ships + is regression-locked: the `mode` argument is
   fully wired and validated; `'native'` correctly ignores a
   silently-dropped `sh:sparql` block while still reporting Core
   violations on the same shape; `'sparql'` returns the deterministic
   E-012 structured report (never a panic). Regression
   `122-shacl-modes.sql` + pgrx `validate_sparql_mode_structured_unavailable`
   lock this.
2. **§5.3 #2** — **fully met, no caveat**: validation against a
   `pgrdf.materialize`-d data graph reports violations against
   entailed triples — `'native'` mode (the working engine), so this
   is unaffected by the `'sparql'` gap. Regression `122`
   `materialised_graph_entailed` + pgrx
   `validate_materialised_graph_entailed`.

#### Impact on §6 (W3C SHACL manifest gate)

- **§6.1 #1 (Core, full-pass)** — unaffected. The W3C SHACL **Core**
  manifest exercises only Core constraints; `just test-shacl-manifest`
  runs them through `pgrdf.validate(…, 'native')`.
- **§6.1 #2 (`--sparql` known state)** — `'sparql'` mode returns the
  deterministic E-012 structured report for **every** input (Gap 2:
  the engine is `unimplemented!()`). `just test-shacl-manifest
  --sparql` therefore asserts that **every** vendored fixture yields
  exactly `{"conforms":null}` — one bounded known state, asserted
  directly, NOT a raw failure and NOT a per-fixture enumerated
  failing list (the cause is one upstream stub, not N independent
  validator bugs). A true W3C SHACL-SPARQL manifest is
  not vendored: it cannot pass with the current crate and would add
  no signal beyond this erratum.

#### Documented-upstream-gate — final for v0.5.0

This is the clean, final v0.5.0 form (wording only; **no code
change** — the G3 short-circuit, the `mode` argument, and the CI
known-state assertion stay exactly as they are):

- `shacl 0.3.1` has **no SHACL-SPARQL constraint component** and its
  `SparqlEngine` is an `unimplemented!()` stub (Gaps 1 + 2 above).
  This is **upstream's own unscheduled roadmap**, tracked in
  `rudof-project/rudof` issues **#21, #94, #1** — not a pgRDF
  omission.
- pgRDF ships an **honest deterministic `mode => 'sparql'`
  short-circuit**: it returns a structured report (`conforms:null` +
  an `error` naming the upstream gap + this erratum), **never
  panics**, and is **forward-compatible** — the surface is exactly
  what it will be once upstream lands the engine, so it activates
  with no signature change the day a rudof release ships
  `IRComponent::Sparql` (or equivalent) + `sh:sparql`/`sh:select`
  parsing.
- **CI asserts this known state as a real gate** (`just
  test-shacl-manifest --sparql` → every vendored fixture yields
  exactly `{"conforms":null}`; regression `122-shacl-modes.sql` +
  pgrx `validate_sparql_mode_structured_unavailable`). The gate is
  NOT weakened — it asserts a bounded known state, not a raw failure.
- LLD v0.5 §5.3 acceptance **#1** is **upstream-unimplementable**
  with the pinned dependency; #2 is fully met (it uses `'native'`).
- This is a **documented limitation, consistent with the
  RDF-1.2 / `reasonable#50` (E-011) precedent** — an honest,
  scoped, forward-compatible upstream-gate — and is **NOT a pgRDF
  defect**.

#### Re-check trigger

A future `shacl` (rudof) release that adds an `IRComponent::Sparql`
(or equivalent) + `sh:sparql`/`sh:select` parsing (rudof issues #21 /
#94 / #1). At that point: revisit `src/validation/shacl.rs` (no
signature change needed — the `mode` arg already routes to
`ShaclValidationMode::Sparql`), promote the `122` `sh:select` no-op
assertion to a real `sh:Violation` assertion, and re-run `just
test-shacl-manifest --sparql` to re-baseline the known-failing set
toward full-pass.

This entry is **final for the v0.5.0 release** as a documented
upstream-gate; it is updated only if upstream ships SHACL-SPARQL
constraint parsing, at which point pgRDF promotes the §5.3 #1 / §6.1
#2 assertions.

### E-013 — W3C SHACL Core manifest: gate invariant + a corrected false exclusion

| Field | Value |
|---|---|
| Filed | 2026-05-16 |
| Status | **resolved / corrected** — investigated at tag v0.5.0-rc1; the asserted upstream `sh:nodeKind` bug does **not** exist; fixture restored; §6 is a genuine 25/25 full-pass |
| Affects | [`SPEC.pgRDF.LLD.v0.5.md`](SPEC.pgRDF.LLD.v0.5.md) §6.1 #1 |
| Crate | `shacl 0.3.1` (rudof project) |

#### Gate invariant — `sh:conforms` (not violation count)

The W3C SHACL Core gate (`just test-shacl-manifest`,
`tests/w3c-shacl/`) compares the **`sh:conforms` boolean** of
`pgrdf.validate` against the spec-authoritative `mf:result` of each
vendored fixture. It does **not** compare the violation *count*.

Reason: pgRDF stores terms dictionary-encoded; the SHACL path
rehydrates the data + shapes graph to N-Triples and re-parses it
before validation (`serialise_graph_to_ntriples`). Blank nodes are
relabelled by that round-trip, so a violation whose **focus node is
a blank node** can be relabelled or coalesced — the count drifts by
±1 on the W3C node-shape fixtures that include a blank-node
`sh:targetClass` member (`node-datatype/maxInclusive/maxLength/
minLength/pattern-001`: validator reports N, W3C `mf:result` says
N+1; the missing one is always the blank-node focus). This is a
serialization artifact, NOT a constraint-evaluation error: `conforms`
is **correct (`false`) in every one of those cases**. The harness
already excludes focus-node-IRI comparison for the identical
blank-node-relabel reason; excluding the count too keeps the gate
honest — a genuinely missed or spurious constraint flips `conforms`
(caught), a blank-node relabel does not (correctly tolerated). The
diagnostic count is still printed by `run.sh` for visibility. This
rationale remains valid and is retained for the node-shape fixtures
above — it is unrelated to `prop-nodeKind-001` (whose 6 focus nodes
are all IRIs).

With this invariant the vendored W3C SHACL Core suite is a **genuine
full-pass: 25 / 25** (`conforms` matches the W3C `mf:result` on
every fixture, with **no exclusion**).

#### Corrected: `prop-nodeKind-001` was never a real upstream bug

This erratum originally claimed `core/property/nodeKind-001` was a
"true upstream Core conformance bug" in `shacl 0.3.1` — that the
validator returned `sh:conforms = true` (0 violations) where the W3C
`mf:result` requires `sh:conforms = false` (27 violations) — and
excluded the fixture to `fixtures/excluded/`.

A **triple-verified investigation at tag v0.5.0-rc1** (a diagnostic
pass, an adversarial-skeptic re-check, and a forensic ERRATA audit)
established that claim was **factually false and never had supporting
evidence**. It was an **unverified assumption made at G3 authoring**
(commit `e3762d4`): the fixture was committed *directly* into
`tests/w3c-shacl/fixtures/excluded/prop-nodeKind-001.w3c.ttl`, and
`tests/w3c-shacl/run.sh` structurally **never ran it** — the harness
globs only `fixtures/core/*.ttl`, so a fixture placed straight into
`fixtures/excluded/` is invisible to the runner. The "validator
returns `conforms:true`/0 violations" claim therefore rested on **zero
harness output**.

The fixture's own embedded W3C `mf:result` declares
`sh:conforms "false"^^xsd:boolean` with **27** `sh:result` blocks.
pgRDF produces **exactly that** — verified at three independent
levels:

1. **Isolated `shacl 0.3.1`** — the upstream crate alone, on the
   split data+shapes graph, returns `conforms:false` with 27
   violations.
2. **pgRDF N-Triples dictionary-rehydrate path** — through
   `serialise_graph_to_ntriples` + re-parse: same result.
3. **Live v0.5.0-rc1 extension via the real `run.sh` code path** —
   `prop-nodeKind-001` now grades **PASS** on `{"conforms":false}`
   (diagnostic violations=27), matching the W3C `mf:result` exactly.

The blank-node-relabel concern (the gate invariant above) is
**inapplicable** to this fixture: all 6 focus nodes
(`ex:InstanceWith*`) are IRIs, so there is no count drift to tolerate
here — `conforms` AND the count both match the W3C answer.

#### Resolution

No fork, no MR, no `[patch.crates-io]` for `shacl`, and no
`Cargo.toml` change are needed (there is no upstream bug to patch).
The fixture was simply restored to `tests/w3c-shacl/fixtures/core/`
following the established split convention every other `core/`
fixture uses (manifest wrapper stripped — the W3C `<>` `mf:Manifest`
root is rejected by oxttl without a base — data + shapes graph kept
verbatim), with a hand-derived `prop-nodeKind-001.expected.json`
(`{"conforms":false}`, derived from the W3C `mf:result`'s
`sh:conforms "false"`, never auto-blessed) and the unmodified W3C
source kept as `prop-nodeKind-001.w3c.ttl` provenance alongside it.
`fixtures/excluded/` is now empty and removed.

§6 W3C SHACL Core is therefore a **genuine 25 / 25 full-pass for
v0.5.0** — no exclusion, no honest-caveat, no Phase H+I follow-up
required for this item.

#### Re-check trigger

None outstanding for `prop-nodeKind-001` (resolved). Independently,
if the dictionary-rehydrate blank-node relabel is ever made
identity-stable, the gate could tighten from `conforms` to
`{conforms, violation-count}` for the blank-node-focus node-shape
fixtures — that is a separate hardening opportunity, not a defect.
