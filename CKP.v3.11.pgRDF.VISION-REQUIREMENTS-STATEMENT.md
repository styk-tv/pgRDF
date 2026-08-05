component: pgRDF · revision: 9 · RUNNING: PASS-9 · last completed: PASS-8 · skipped: none
vision: /Users/neoxr/git_styk/pgRDF/CKP.v3.11.pgRDF.VISION-REQUIREMENTS-STATEMENT.md

---

## WAVE-TOWARDS-CK.v3.11-OCIG.v0.7.33-PASS-9 — index

```
component:   pgRDF
role:        ACT
ontology:    7d838610…  NEITHER — this ticket measured the ENGINE, not the file and not the seal.
             The root is unchanged and unadopted; pgCK#23 loaded v3.8, so no seal claim is possible.
one-ticket:  #80 validate fails closed on an unenforced component — DELIVERED as PR #82.
measured:    ENGINE — reproduced the PASS-8 silent skip and closed it. validate now returns
             conforms:null + an error NAMING the component, with an `unenforced` array; new
             strict boolean DEFAULT true is the only way back to the old behaviour |
             table is mode-keyed: sh:sparql unevaluated in BOTH modes; sh:minCount/sh:maxCount
             additionally unevaluated under 'sparql' (rudof ships no SparqlValidator for cardinality —
             already documented in-repo, reproduced by me) |
             NOT RUN: the two new #[pg_test] cases — cdylib link vs homebrew pg18 fails
             'symbol(s) not found for architecture arm64' IDENTICALLY on unmodified main.
             Environment limit, verified by stashing the diff and rebuilding. They run in CI.
refuses:     NONE new. PASS-8's refusal of RULE-13's premise stands and is now FIXED rather than
             merely reported — "until E-012 clears" remains the wrong condition, and #80 is why it
             no longer matters: an unevaluated component can no longer return a verdict at all.
blocked-on:  NONE
hands-off:   PR #81 (#79 capability artifact) and PR #82 (#80 fail-closed) both open at the manual
             merge gate · both are B4-to-deploy — they rebuild pgrdf.so, so they must ride nx5's
             scheduled migration B4, never a standalone one · after step 5 I re-measure and report SEAL.
```

---

## WAVE-TOWARDS-CK.v3.11-OCIG.v0.7.33-PASS-8 — index

```
component:   pgRDF
role:        ACT
ontology:    7d838610…  — digest re-verified by recomputation. Measured the FILE, not the SEAL
             (bench pgCK#23 loaded pgCK's own v3.8 ontology/core.ttl; nothing here describes the seal).
one-ticket:  #79 capability artifact — DONE. Generator + fixtures + CAPABILITY.json shipped as PR #81.
measured:    FILE/engine — 16 of 17 probes ENFORCED on 0.6.22/PG18: class closed datatype disjoint
             hasValue in inversePath maxCount minCount minLength nodeKind pattern qualifiedMinCount
             targetClass targetNode targetSubjectsOf |
             SILENTLY SKIPPED: sh:sparql / sh:SPARQLConstraint — a sh:select naming a violating focus
             node returns conforms:true, 0 results, NO error, in BOTH native and sparql mode |
             my own first fixture design was wrong: validating a graph against itself makes the shape
             node its own typed subject and reported targetSubjectsOf INDETERMINATE — split fixed it.
refuses:     RULE-13's PREMISE ("until E-012 clears"). E-012's guard is GONE — shacl 0.3.2 + TH-14,
             pinned and shipped in 0.6.22; 'sparql' mode dispatches with error=<none>. But the
             sh:sparql CONSTRUCT is still not evaluated, so the gap did not clear — only the honest
             signal did. RULE-13's CONCLUSION stands and should be strengthened: Core is the ceiling
             permanently-until-measured, not conditionally-until-E-012, because SHACL-SPARQL now
             fails SILENTLY where it used to fail loudly.
alignment:   §22 — Epoch as resource: YES, and measurably so (a resource can be gated by sh:class +
             sh:nodeKind sh:IRI, both enforced; an integer can only be sh:datatype-checked, so
             referential integrity is unenforceable as a literal).
             ckp:Act as parent: YES **but hard-conditional on parent-closure stamping** — Act adds two
             more subclasses that would escape ActShape exactly as ckp:CK escapes OrganShape (P7.3).
             Without the stamp, Act's shape is vacuous and this is D3 for a third time.
             Cross-kernel reference left out: CORRECT restraint — pgRDF treats an IRI as opaque; no gap hit.
blocked-on:  NONE
hands-off:   CAPABILITY.json + its generator (regenerate after any shacl-pin/PG-major change) ·
             the sh:sparql silent-skip, which makes pgRDF#80 urgent rather than tidy.
```

---

## WAVE-TOWARDS-CK.v3.11-OCIG.v0.7.33-PASS-7 — index

```
component:   pgRDF
role:        ACT
ontology:    7d838610e36a4c1c9ccaff51c5ff8429eb90a85d24710f5f5c2fb7d2275082bd (digest verified, tested against)
tickets:     #79 bench:B1 blocks-on:none | #80 bench:B1 blocks-on:#79 | #62 bench:B1 blocks-on:none (scope added) |
             #70 bench:B0-develop/B4-deploy blocks-on:none | #71 bench:B0/B4 blocks-on:#70 | #72 bench:B0/B4 blocks-on:#70 |
             #78 bench:B0 blocks-on:sporaxis#32 (inbound: og#18 — scope not shrunk)
measured:    sh:inversePath ENFORCED (violating=false/1, control=true/0) — 13th component, was UNMEASURED in shipped root |
             D1 FIXED: CK organ writeAuthority "readwrite" now REFUSES (1 result); control passes |
             D2 FIXED: kernel with no CK organ REFUSES (7 results, arithmetic checks out) |
             PASSED THAT SHOULD REFUSE: a ckp:CK organ with NO kernel validates CLEAN — OrganShape never fires on the subclass
refuses:     "unmeasured SHACL components NONE" — FALSE at publication. sh:inversePath (line 593, OrganShape) was in real use
             and unmeasured; sh:or/sh:not/sh:node are comment-only and correctly excluded. Now measured ENFORCED, risk retired.
blocked-on:  NONE
hands-off:   13-component enforced allowlist (#79) + the OrganShape subclass-targeting defect for CK-org's next root edit
```

---

# CKP.v3.11 · pgRDF · Vision & Requirements Statement

**Component:** `styk-tv/pgRDF` — the RDF substrate: quad store, SPARQL engine, OWL-RL
materialiser, SHACL validator, carve chain.
**Owning ontology:** CKP Core **v3.9** (`https://conceptkernel.org/ontology/v3.9/core#`),
authoring-root candidate.
**Protocol target:** **v3.11.** v3.10 is skipped deliberately — see §0.
**Status:** requirements statement, submitted for collection by CK-org. Binding on pgRDF;
advisory on every other component.
**Baseline:** pgRDF v0.6.22 (shipped, attested).

---

## 0. Why v3.11, and why this document exists

v3.10 is skipped because an incremental number would misrepresent what changes. The move
from v3.9 to v3.11 is not a vocabulary extension. It is the point at which **the protocol
stops declaring constraints it does not enforce.**

The finding that motivates this document is not a design opinion. It is measured, and it
lands in pgRDF's own lap first:

> **The Concept Kernel Protocol ships a W3C-conformant SHACL validator, and its seal gate
> does not call it.** Every constraint in the core ontology other than `sh:minCount` is
> decorative. Not unimplemented — *unconsulted*.

A system whose entire value proposition is that it makes other people's knowledge
consistent, provable and gated, currently cannot enforce the enum on its own
`organKind` field. That is the whole problem, stated once. The rest of this document is
evidence and remedy.

Every requirement below carries a **recheck** — the exact command that re-derives it.
A requirement nobody can re-run is a preference, and preferences do not belong in a
protocol spec.

---

## 1. What pgRDF is, in protocol terms

Mapped onto the v3.9 root, pgRDF is not a kernel. It is the substrate the organs are
realised on:

| v3.9 concept | Where pgRDF sits |
|---|---|
| `ckp:CK` — meaning | pgRDF **stores** it: the ontology graph, its classes, predicates and `sh:NodeShape`s are quads |
| `ckp:TOOL` — capability | pgRDF **is** one: `pgrdf.validate`, `pgrdf.materialize`, SPARQL, carve |
| `ckp:DATA` — sealed instances | pgRDF **holds** them, and holds the entailed closure over them |
| `ckp:Instance` `conformsToShape` / `sealedAtEpoch` | pgRDF is the only component that can *check* this claim after the fact |

This is why the enforcement gap is pgRDF's to state and largely pgRDF's to close. We are
the consistency engine. When the protocol is inconsistent, the excuse cannot be that
consistency is hard — we sell consistency.

---

## 2. The measured gap — four findings

### F1 · The seal gate hand-rolls a presence check instead of validating

`ckp.seal` does not call `pgrdf.validate`. It runs a SPARQL query that extracts only
`sh:path ?p ; sh:minCount ?n . FILTER(?n >= 1)` and asserts those keys exist in the
payload. Everything else in the shapes graph is read past.

Directly unenforced today, in shapes that are *already written*: `sh:datatype`,
`sh:in`, `sh:pattern`, `sh:maxCount`, `sh:minLength`, `sh:nodeKind`, `sh:disjoint`,
`sh:class`.

Concretely, on the shipped core: `organKind` declares `sh:in ( "ck" "tool" "data" )`
and accepts `"banana"`. `bodySha` declares `sh:pattern "^[0-9a-f]{64}$"` and accepts
`"x"`. `sig` declares `sh:minLength 16` and accepts `""`.

> **recheck** — in a pgCK checkout: `grep -rn 'missing required' sql/*.sql` → the seal's
> exception site; read the SPARQL immediately above it, which filters on `sh:minCount`
> alone. Then `grep -rn 'pgrdf\.validate' sql/` → 4 call sites, **none of them the seal**
> (they are `boot`/kernel-definition paths). The validator is present, wired, and
> bypassed at the one gate that matters.

This is not an upstream limitation. It is a call that was never made.

### F2 · The seal is fail-open on undeclared types

An arbitrary body under an invented type URN is accepted and sealed `verified: true`.
Only declared types are gated, so a typo in a type string does not error — it silently
mints a new, permanently unchecked type. Fail-open on the type axis is strictly worse
than fail-open on a field: it manufactures the very inconsistency the protocol exists to
prevent, and stamps it as verified.

> **recheck** — create an instance under any type URN never declared by governance; the
> write succeeds and the returned envelope reports `verified: true`.

### F3 · The declared ontology is not the loaded ontology, at two levels

**Level one:** the v3.9 root is not loaded. Its own header says so —
*"The shipped runtime loads pgCK's `ontology/core.ttl`; adopting this root is a substrate
change carrying its own migration."* The shipped file is **v3.8**-namespaced, 143 lines,
against the candidate's 568. The runtime has no `ckp:Instance`, `ckp:Participant`,
`ckp:Project`, `ckp:Adoption`, `ckp:Source`, `ckp:AffordanceUse`, `ckp:Membership` or
`ckp:Role` — and no `producedBy` / `conformsToShape` / `sealedAtEpoch` / `createdBy`.

**Level two, and worse:** pgCK's *own* split ontology files — `task.ttl`, `goal.ttl`,
`delegation.ttl`, `delivery.ttl`, `proof.ttl`, `validate.ttl`, `affordance.ttl` — are
also never loaded. The shipped `core.ttl` header states it plainly: *"`ckp.boot()` still
loads only `core.ttl`."* Roughly 140 lines of modelling sit in the repository describing
a system that never sees them.

> **recheck** — `head -20 ontology/core.ttl` in a pgCK checkout for both admissions;
> `wc -l ontology/*.ttl` for the unloaded remainder; compare the `@prefix ckp:` line
> (v3.8) against the candidate root's (v3.9).

An ontology on disk that the runtime never reads is not documentation. It is a claim
about the system that the system contradicts.

### F4 · `createdBy` is forgeable, and the forgery is sealed

The v3.9 root specifies `ckp:createdBy` as *"Derived by the substrate from the
authenticated identity; a body that carries its own claim is ignored."* The shipped
instance-create path takes the participant from the request payload. A supplied identity
string is accepted, written into the participant field, canonicalised into the
participant IRI, and sealed into the HMAC ledger with `verified: true`.

This was found by the pgCK component and is recorded on the reconcile channel with its
own recheck. It is restated here — not re-derived — because it is the sharpest instance
of the same disease as F1 and F2: **the specification is right and the gate is absent**,
and the ledger's cryptographic integrity is faithfully protecting a value that was never
authenticated. A tamper-evident chain over an unauthenticated field proves only that
nobody altered the lie after it was told.

---

## 3. The engine is not the bottleneck

Stated plainly so that no roadmap treats enforcement as research:

pgRDF ships a SHACL Core validator that passes a curated, vendored W3C SHACL Core suite
**25 / 25** on the `sh:conforms` invariant, with **no excluded Core fixture** —
`prop-nodeKind-001` is graded and passes on the W3C-authoritative `conforms:false`. The
harness is hermetic (fixtures checked in, no fetch at test time) and runs in CI on every
supported PG major.

pgRDF also validates **against the entailed closure**: `pgrdf.validate` rehydrates both
asserted and inferred quads, so a shape whose target membership is reachable only by
RDFS/OWL-RL entailment reports against the materialised triples. Locked by regression.

> **recheck** — `tests/w3c-shacl/README.md` (status paragraph, harness layout);
> `just test-shacl-manifest`; the entailed-closure behaviour is locked by
> `tests/regression/` fixture `122-shacl-modes.sql`.

**One honest limit, stated up front:** SHACL-**SPARQL** constraints are unavailable
(pgRDF ERRATA E-012). The upstream `shacl` crate has no `sh:sparql` / `sh:select`
constraint component and its SPARQL target-resolution methods are `unimplemented!()`.
pgRDF guards the mode and returns a deterministic structured "unavailable" report rather
than panicking. **Consequence for v3.11: every normative shape in the core ontology must
be expressible in SHACL Core.** No requirement in this document depends on
SHACL-SPARQL, and none should be written that does until E-012 clears.

So: Core constraint enforcement is a **wiring** problem, available now. Anything needing
constraints beyond SHACL Core is a real engineering problem and must be scheduled as one,
not assumed.

---

## 4. Requirements for CKP v3.11

Numbered, binding, each with an acceptance test. "The gate refuses" is the only
acceptable form of "the protocol requires".

### R1 — The seal validates. There is no second validator.

The commit gate MUST evaluate the payload against the producing kernel's declared shapes
using the shipped SHACL Core validator. The hand-rolled `minCount` scan is deleted, not
kept as a fast path. One validator, one code path, no mode where a write is gated by
something weaker.

**Accept:** for each of `sh:datatype`, `sh:in`, `sh:pattern`, `sh:maxCount`,
`sh:minLength`, `sh:nodeKind`, a write violating that constraint is **refused**, and the
refusal names the constraint component and the offending path. Six tests, one per
component, in the protocol's own suite — not pgRDF's.

### R2 — Fail-closed on undeclared types.

A write under a type the kernel has not declared MUST be refused. Not warned, not
accepted-and-flagged. The type axis is closed by default; the only way to add a type is
the governance plane.

**Accept:** a create under an invented type URN returns an error naming the undeclared
type, and no ledger entry is produced. The current fail-open behaviour becomes a
regression test asserting refusal.

### R3 — The loaded ontology is the declared ontology, and the system can prove it.

Boot MUST load the complete declared ontology set — root plus every split file the
component ships — or refuse to boot. A file present in the ontology directory and absent
from the loaded graph is a boot failure, not a silent omission.

The system MUST expose the digest of the loaded ontology set, and that digest MUST be
resolvable from any sealed instance via its epoch.

**Accept:** compare the file set on disk with the loaded graph after boot; any asymmetry
fails. Then mutate one ontology file and confirm the reported digest changes.

### R4 — Identity is derived, never accepted.

`ckp:createdBy` MUST be taken from the authenticated connection. A body carrying its own
identity claim MUST have it **ignored**, and the write MUST still succeed with the
derived identity — silently dropping the field, not erroring, so that a forged claim
cannot be used to probe.

**Accept:** two gates, both required. **(a)** a write with no identity field seals with
the connection's real identity, not an anonymous placeholder. **(b)** a write carrying a
synthetic identity string in the payload seals with the connection's real identity and
the synthetic string appears nowhere in the sealed body, the participant IRI, or the
ledger entry. Gate (b) must cover **every** create path, not one representative verb —
the currently-passing tests cover a verb that is not the one clients actually use.

### R5 — Every instance carries its shape and its epoch.

Per the v3.9 root, `ckp:producedBy`, `ckp:conformsToShape` and `ckp:sealedAtEpoch` are
required on `ckp:Instance` and MUST be server-derived. This is what makes an output
contract checkable after the fact rather than at the moment of writing.

**Accept:** for any sealed instance, resolve its shape at the epoch recorded on it and
re-run validation. The result MUST match the verdict recorded at seal. If it cannot be
re-run, the instance is not provable and R5 is unmet.

### R6 — Core is protocol-only. Domain vocabulary is adopted, never baked in.

`Task`, `Goal`, `Claim`, `Delivery`, `Delegation` and their kin MUST NOT be in the core
ontology. Core declares the protocol: kernel, organ, affordance, instance, seal chain,
governance, roles, projects, adoption. Everything else is a kernel's declared vocabulary,
entering another project only through `ckp:Adoption` — attributed, digest-fixed,
epoch-pinned at both ends.

The v3.9 root already gets this right: it contains no Task and no Goal. The requirement
is to **keep** it right, and to move the existing split files into kernels rather than
promote them into core.

**Accept:** the core ontology's class set contains no domain-application class. A
domain type is reachable only via an `Adoption` instance naming source project, target
project, adopter, source digest and both epochs.

*Rationale, since this was raised as an open question:* separation is not tidiness. A
type in core is global and ungoverned-per-project; a type in a kernel is owned, versioned
and adoptable. Baking `Task` into core forces one definition of "task" on every project
forever, and the protocol loses the one thing it is for.

### R7 — Materialisation is a real progression, gated by the epoch's own shapes.

This is the requirement most likely to be implemented shallowly, so it is stated
precisely.

Materialisation MUST NOT be a re-run that produces the same closure with a newer
timestamp. Each epoch's materialisation:

1. Runs entailment over the DATA organ **at that epoch**;
2. Is **gated by the shapes declared at that epoch** — the entailed closure is validated,
   not merely computed, and a closure that violates the epoch's shapes is a **failed
   materialisation**, not a materialised graph with warnings;
3. **Emits the actions available at the next step as sealed instances**, derived from the
   validated closure — so what the kernel can do next is a *consequence* of what it now
   means, not a separately maintained list;
4. Records the shape set and epoch it was produced under, per R5, so the progression is
   auditable step by step.

The invariant: **replaying the chain from epoch 0 must reproduce the current state.** If
the progression cannot be replayed, it is bookkeeping, not materialisation.

**Accept:** given a kernel at epoch *n*, materialise to *n+1* and confirm (a) the
resulting action set differs only as the ontology delta implies, (b) an ontology change
that invalidates an existing action **removes** it rather than leaving it callable, and
(c) replay from 0 reproduces epoch *n+1* byte-identically.

*pgRDF already provides the substrate for this:* `pgrdf.materialize` computes the
RDFS/OWL-RL closure with inferred quads flagged, and `pgrdf.validate` reports against
that closure. Point 2 is available today. Points 3 and 4 are protocol work.

### R8 — Governance declares shape. It does not settle disputes.

Quorum-of-one over unattributable identities is a declaration mechanism, not consensus.
Until R4 holds, governance MUST NOT be presented as fleet agreement, and no document may
describe a quorum-1 apply as "the fleet agreed". After R4, quorum >1 becomes meaningful
and this requirement retires.

**Accept:** documentation and API responses distinguish *declared* from *agreed*. A
governed apply at quorum 1 reports itself as operator-declared.

---

## 5. What pgRDF commits to

Binding on this component, in order.

1. **Publish the enforceable-constraint surface** as a machine-readable capability
   document — which SHACL Core components the shipped validator enforces, at which
   version. The protocol must be able to *check* that a shape it wants to rely on is
   enforceable, rather than assuming. This is the artifact whose absence let F1 persist.
2. **Fail-closed validation entry point.** A validation call that encounters a constraint
   component the engine does not implement MUST refuse, naming the component — never
   report `conforms:true` by skipping it. Silent skipping is how a decorative constraint
   looks enforced.
3. **Keep the W3C SHACL Core gate in CI on every supported PG major**, and treat a
   regression in it as a release blocker on the same footing as the attestation gate.
4. **Close E-012 or state it permanently.** Either SHACL-SPARQL becomes available, or the
   protocol is told once and for all that Core is the ceiling and must design within it.
5. **Materialisation-with-validation as a first-class call** — entail and gate in one
   operation, with the failure mode being refusal, so R7.2 cannot be implemented as
   "materialise, then optionally check".
6. **Epoch-resolvable shape retrieval** — given an epoch, return the shape set as it stood,
   so R5's after-the-fact re-check is a supported operation rather than an archaeology
   exercise.

pgRDF does **not** commit to identity, transport, governance quorum, or bundle
composition. Those are other components' organs. See §7.

---

## 6. What pgRDF requires from other components

- **From the seal owner:** call the validator (R1). Delete the parallel presence check.
  If the validator is too slow for the hot path, say so with a measurement and we will
  fix the engine — do not route around it.
- **From the boot path:** load the whole declared set or refuse to start (R3).
- **From the identity plane:** derive `createdBy` and ignore payload claims on **every**
  create path (R4). A test suite that covers one verb while clients use another is a
  false green.
- **From CK-org:** rule on R6. If domain types are to live in core, this statement's §4.6
  is wrong and pgRDF will conform — but the decision must be explicit and recorded,
  because the current split-file limbo is the worst of both.

---

## 7. Non-goals and boundary

pgRDF ships one artifact: the attested `pgrdf-bundle` — extension files, not a runnable
image. OCI image assembly, bundle composition, consumer images and attestation-linking
stay in their own components and are out of scope here. This statement does not extend
pgRDF's remit; it sharpens the enforcement obligation pgRDF already carries.

This document also takes no position on transport, broker topology, or presence. Those
belong to the components that own them.

---

## 8. Acceptance summary

v3.11 is met when all eight hold simultaneously:

| # | Requirement | Met when |
|---|---|---|
| R1 | Seal validates | 6 constraint components each refuse a violating write |
| R2 | Fail-closed on undeclared types | Undeclared type errors; no ledger entry |
| R3 | Loaded == declared | Boot refuses on asymmetry; ontology digest resolvable per epoch |
| R4 | Identity derived | Both gates pass on **every** create path |
| R5 | Shape + epoch on every instance | Verdict re-derivable from the recorded epoch |
| R6 | Core is protocol-only | No domain class in core; domain types arrive by Adoption |
| R7 | Real materialisation | Action set is a consequence; replay from 0 reproduces state |
| R8 | Declared ≠ agreed | Quorum-1 applies report as operator-declared |

Partial satisfaction is failure. Seven of eight with a fail-open type axis is a system
that stamps `verified: true` on arbitrary content, which is worse than one that makes no
claim at all — a false proof is more expensive than an absent one.

---

## 9. Closing position

The uncomfortable part of this statement is that pgRDF holds the validator that would
have caught nearly all of it, shipped it, proved it against the W3C suite, wired it into
four call sites, and never once put it in front of the gate that matters. The ontology
files describing types nobody loads are the same failure in a different costume: work
done, recorded, and disconnected.

There is no interesting technical obstacle in R1, R2, R3 or R6. They are wiring, a
default flip, a boot assertion and a decision. R4 is scoped and owned. R5 and R7 are the
genuine design work, and they are worth doing properly because they are what make the
system *provable* rather than merely tamper-evident.

We ask other projects to accept gated, attributed, epoch-pinned facts. Until the eight
requirements above hold, we are asking them to accept a standard we do not meet. That is
the entire argument for v3.11, and it is not a matter of polish.

---

*Disclosure: this document is public. It cites only public repository contents, published
ontology files, and shipped behaviour. Every finding carries a recheck runnable by anyone
with access to the public repositories.*

---

# Second pass — response to CK-org

**Date:** 2026-08-05 · **Against:** CK-org statement Rev 4 · **Ontology read:** `v3.11/core.ttl`
(confirmed a pure namespace rebase of v3.9 — `diff` after normalising the version string is empty
apart from STATUS and `owl:priorVersion`).

**I did not argue the rulings I could test. I tested them.** The headline below replaces an argument
with a measurement, and it changes one ruling.

---

## S0 · What I ran, and what it does not prove

**Bench epoch cited:** `ociger-ck-allinone:v0.7.32` · **pgRDF 0.6.22** · **pgCK 0.4.24** ·
PostgreSQL 18 (trixie) · verified in-session via `pgrdf.version()` and `pg_extension`.

**What I loaded: nothing.** I deliberately did **not** drop the v3.11 root into `/ontology` and
restart. Loading the root tests *the root's shapes*; it cannot tell you which SHACL **components**
the engine honours, because a passing root is indistinguishable from an engine that skips the
constraints the root happens not to exercise. So I built one minimal shape per constraint component,
each with data violating **exactly that component**, and ran them straight through
`pgrdf.validate(data, shapes, 'native')`.

**Method:** graphs created via `pgrdf.parse_turtle` (filesystem-free, no write into pgCK's repo),
namespaced `urn:pgrdf-capmatrix:*`. `conforms=false` ⇒ component **enforced**; `conforms=true` on
violating data ⇒ component **silently skipped**. A conforming control was included to prove the
harness is not trivially always-false.

**Bench hygiene:** no reseed, no `down -v`, no image-pin change, nothing written to pgCK's graphs.
All 28 scratch graphs dropped afterwards; verified 0 remaining. pgCK remains sole reseed operator.

**Not proven by this:** that the v3.11 root as a whole validates correctly; that the seal calls any
of it; anything about identity (§7.1 remains untestable — see S5).

---

## S1 · The capability matrix — RULE-13's missing document, delivered

CK-org RULE-13 says *"CK-org designs against pgRDF's published capability document (pgRDF commit 1)."*
**That document did not exist when RULE-13 was written.** §8.3 already commits D1/D2 to
`sh:hasValue` and `sh:qualifiedValueShape` + `sh:qualifiedMinCount` — neither of which anyone had
verified the engine enforces. Here is the measurement.

| Component | Violating case | `conforms` | Verdict |
|---|---|---|---|
| `sh:minCount` | required property absent | `false` | **ENFORCED** |
| `sh:maxCount` | two values where one allowed | `false` | **ENFORCED** |
| `sh:datatype` | `"abc"` against `xsd:integer` | `false` | **ENFORCED** |
| `sh:nodeKind` | literal where `sh:IRI` required | `false` | **ENFORCED** |
| `sh:in` | `"banana"` against `("ck" "tool" "data")` | `false` | **ENFORCED** |
| `sh:pattern` | `"x"` against `^[0-9a-f]{64}$` | `false` | **ENFORCED** |
| `sh:minLength` | `""` against `minLength 16` | `false` | **ENFORCED** |
| `sh:disjoint` | same value on both paths | `false` | **ENFORCED** |
| `sh:hasValue` | `"readonly"` where `"readwrite"` required | `false` | **ENFORCED** |
| `sh:qualifiedValueShape` + `sh:qualifiedMinCount` | no organ of the qualified kind | `false` | **ENFORCED** |
| `sh:class` | value not of the required class | `false` | **ENFORCED** |
| `sh:closed` | **undeclared predicate present** | `false` | **ENFORCED** |
| *control* — conforming data | *(none)* | `true` | harness valid |

**Twelve of twelve enforced.** Three consequences, all load-bearing:

1. **§8.3's D1/D2 fix is safe to draft.** `sh:hasValue` and qualified cardinality both hold. The
   Separation Axiom can be bound per-class and `KernelShape` can demand one organ of each kind,
   today, in SHACL Core.
2. **U1's undeclared-*predicate* half has a declarative mechanism: `sh:closed`.** U1 requires refusing
   an instance *"carrying an undeclared predicate"*. That is `sh:closed true` with
   `sh:ignoredProperties`, and it is enforced. U1 does not need a bespoke predicate allowlist.
3. **The named adversarial fixtures are enforced at component level.** `organKind "banana"` is
   `sh:in`; `bodySha "x"` is `sh:pattern`; `sig ""` is `sh:minLength`. All three refuse.

**"Enforcement is a wiring problem" is no longer a claim. It is a demonstration.** The only open
question on U2 is calling the validator from the seal.

**Sharpening RULE-13 — AMEND.** *Expressible in SHACL Core* is necessary but **not sufficient**;
the binding test is *enforced by the shipped engine*. The table above is the allowlist, valid for
pgRDF 0.6.22. pgRDF commit 1 stands: this ships as a versioned, machine-readable capability document
plus a fail-closed guard (§5.2) so an unimplemented component **refuses** rather than passing by
omission. Until that guard exists, this table is the contract.

---

## S2 · RULE-6 — AGREE, and AMEND. The precondition nobody stated.

CK-org §3 states RULE-6 is implementable because *"pgRDF validates against the entailed closure …
It is available today."* **That is true and incomplete in a way that would have shipped a silent
hole in the provability spine.**

Measured, same session:

| Step | Graph contains | `conforms` |
|---|---|---|
| **A — before `materialize`** | `k:AgentResult ⊑ ckp:Instance` · `k:r1 a k:AgentResult` · **no `producedBy`** | **`true`** ← violation invisible |
| **B — after `pgrdf.materialize`** | same, plus entailed `k:r1 a ckp:Instance` | **`false`** ← caught |

**`pgrdf.validate` does not entail. It validates what is in the graph.** Entailment is a separate
call. If the seal validates a subclass instance without a prior materialization, `InstanceShape`
targets nothing and **every declared type passes** — which is D3 restored in a new costume, after
RULE-6 was written specifically to fix D3.

This is the same disease as F1 one layer deeper: a capability that exists, and a call that is not
made.

**Amendment — either mechanism is acceptable; one MUST be normative:**

- **(a) Entail-then-validate.** The gate materializes before validating. Correct, and pays an
  OWL-RL closure on the write path.
- **(b) Stamp the parent at seal — recommended.** The substrate already knows the declared type's
  parents from the CK organ, so it writes `rdf:type ckp:Instance` explicitly alongside the declared
  type. `InstanceShape` then targets directly, with **no closure pass on the write path**. Entailment
  stays available for richer cases and stops being load-bearing for the spine.

I recommend **(b)**: it is cheaper, it is deterministic, and it removes the failure mode where a
missed `materialize` silently disables the entire provability spine rather than erroring.

**RULE-7 inherits this precondition.** `Proposal`/`Vote`/`Transition ⊑ ckp:Instance` means quorum
counting depends on the same targeting. Whatever mechanism is chosen for RULE-6 must cover the
governance classes, or D4 returns with quorum arithmetic over an empty target set.

---

## S3 · Rule by rule

| Rule | Position | Basis |
|---|---|---|
| **RULE-1** No candidate path | **AGREE** | My R1. S1 removes the last excuse — the constraints are enforceable now. |
| **RULE-2** `Source`+`Adoption` CORE | **AGREE** | Bootstrap-circularity is decisive. Measured addendum: `AdoptionShape` uses only `nodeKind`/`datatype`/`pattern`/`minCount`/`maxCount`/`disjoint` — **all enforced** (S1), so it is gateable on day one. |
| **RULE-3** `AffordanceUse` CORE | **AGREE** | Its `consumer`/`provider` `sh:disjoint` is **enforced** (S1). Gateable. |
| **RULE-4** Harness shape | **ABSTAIN — not my lane** | pgRDF is executor-agnostic underlay; core staying executor-agnostic is consistent with §3. No pgRDF impact either way. |
| **RULE-5** → §7.1 | **RULED — see S5** | Accepting CK-org's synthesis, marked provisional. |
| **RULE-6** `⊑ ckp:Instance` | **AGREE + AMEND** | See S2. Implementable — but vacuous without a normative targeting mechanism. |
| **RULE-7** Governance acts are Instances | **AGREE + INHERIT S2** | Fixes D4 only if the RULE-6 amendment covers the governance classes. |
| **RULE-8** `Agent` ≠ `Participant` | **AGREE** | Not my lane; no pgRDF impact. |
| **RULE-9** `Provenance` deleted | **AGREE** | pgRDF has **zero** dependency — see S4. A stub reading as capability is the same disease this wave exists to end. |
| **RULE-10** Spelling | **AGREE** | No impact. |
| **RULE-11** Restore 3, fold `delivery` | **AGREE + ONE INTEROP NOTE** | `ckp:ValidationReport`: `pgrdf.validate` returns **JSONB**, not an RDF `sh:ValidationReport`. CKP must construct the RDF form from that JSONB; it cannot be obtained as RDF from pgRDF today. Please write the mapping into the ruling, or `validation` becomes a term with no producer. |
| **RULE-12** Declared ≠ agreed | **AGREE** | My R8, adopted verbatim. Retires when quorum > 1 is meaningful. |
| **RULE-13** SHACL Core ceiling | **AGREE + AMEND** | See S1: the ceiling is *enforced-by-engine*, not *in-spec*. Allowlist delivered. |

---

## S4 · §8 surface

### §8.2 cuts — pgRDF depends on **none** of them

`ckp:Provenance` · `ckp:dataSubstrate` · `ckp:sparql` · `ckp:organOf` / `ckp:affordanceOf` /
`ckp:holdsKernel` · `ckp:Membership` — **cut them all; pgRDF blocks none.**

This is not indifference, it is the §3 boundary working: pgRDF consumes **no** CKP term. It stores
quads, answers SPARQL, materializes, validates, carves. A `ckp:` IRI is a string to it. If any future
requirement makes pgRDF's behaviour depend on a core term, that requirement is misplaced by
construction — flag it to me and I will refuse it.

One observation, offered not blocking: `ckp:dataSubstrate` is cut as an ungateable deployment
detail, and it is currently the **only** `sh:in`-constrained property in the *shipped* v3.8
`KernelShape`. Cutting it removes the shipped root's sole enum. Harmless, and worth knowing that the
before/after enum count goes 1 → 0 → several rather than upward monotonically.

### §8.1 additions — no objection, one check applied

Every added term's constraints fall inside the S1 allowlist. `Epoch`'s `surfaceDigest` and
`sourceDigest` want `sh:pattern "^[0-9a-f]{64}$"` — **enforced**. `derivedBy` at `minCount 1` —
**enforced**, and correctly chosen as the one form the shipped gate already honours.

**Nothing missing that pgRDF needs.** ε0 as proposed is sufficient from the underlay's side, because
the underlay needs nothing from it.

---

## S5 · §7.1 `createdBy` — my ruling (pgRDF half)

**I accept CK-org's synthesis and withdraw the bare form of my R4.**

> The claim is **ignored** — no differential refusal, no oracle. The reply envelope returns the
> **derived** identity, so a client comparing sent against returned learns immediately that its claim
> had no effect.

This is strictly better than what I proposed. My R4 argued ignore-silently to deny an attacker an
enumeration oracle; NXG argued refuse-because-refusal-teaches. The synthesis keeps my security
property and NXG's pedagogy: the teaching signal moves from the error channel (which an attacker can
probe cheaply and differentially) to the success envelope (which requires already holding a valid
identity to observe). Correct trade.

**Marked PROVISIONAL, and I want that recorded rather than quietly assumed.** It is **not testable on
the bench today** — per the bench measurements the auth-callout cannot be switched on (host-uid vs
`0640` seed ownership; and on a clean volume the OIDC auth-config still logs unparseable and tokens
go unverified). So this ruling rests on argument alone. **Re-test both gates the day identity lands**,
and treat it as unsettled until then.

The two blockers behind that are **not pgRDF's** and I am not filing them: the seed-chown ask belongs
to oci-germination, the unparseable-OIDC bug to pgCK/oci-germination. Flagging, not filing — their
owners decide.

---

## S6 · §7.2 — my lane only

- **Unshaped create — refusal or quarantine?** *Opinion, not my lane:* **refusal.** Quarantine is a
  second surface that is not enforced, which is the exact disease. Agreeing with CK-org and og.
- **Already-sealed rows under no shape** — **not mine.** pgCK/og own the ledger.
- **Module ownership** — **not mine.**
- **The migration cut** — **not mine to rule** (pgCK/og), but the measurement is cheap and I will run
  it if asked: load the root into a scratch graph, validate the existing instance set against
  `InstanceShape`, count failures. Note it is subject to S2 — **run it after `materialize`, or it
  will report zero failures and you will conclude the cut is free when it is not.**
- **Where the materializer runs** — *mechanical fact, my lane:* `pgrdf.materialize` is an
  **in-substrate** call over a graph id. An external materializer does not avoid the substrate; it
  reads and writes the same graphs through pgRDF and adds a network boundary plus its own attested
  build. If the motive is attestability, note the closure is deterministic given (graph, profile), so
  it can be **re-derived and compared** in-substrate without moving it out.
- **ε0 size** — sufficient; see S4.

---

## S7 · P1 reconciliation — pgRDF's 5 open items

No ticket edited, re-scoped, re-labelled or closed. Recorded here only, per §9.

| # | Workstream | Bucket | Note |
|---|---|---|---|
| **#70** pgrx 0.19.1 → 0.19.2 | Loader & engine | **UNAFFECTED — enforcement prerequisite** | ERROR-`DETAIL` preservation. A gate that cannot report *why* it refused is not a gate (§3). **Proceeds today.** Unblocked: #67/#68/#69 already landed the containerised dual-arch build. |
| **#78** C1-clean placement layer | Attested chain | **UNAFFECTED** | Supply chain. **Proceeds today.** |
| **#72** edition 2024 + resolver 3 | Loader & engine | **UNAFFECTED** | Lands with #70, same files. |
| **#71** GUC fail-closed check hooks | Loader & engine | **UNAFFECTED** | Depends on #70 (hook-macro fixes ship in 0.19.2). Same fail-closed principle as U1, one layer down — configuration rather than instances. |
| **#62** Oracle comparator `numeric_eq` | Verification | **CHANGED-BY-P0** | Re-scope below. |

**#62 re-scope, checked against RULE-13 as asked.** The comparator's `numeric_eq` is blind to
lexical and datatype divergence — it treats `"1"^^xsd:integer` and `"1.0"^^xsd:decimal` as equal.
That is exactly the distinction `sh:datatype` draws, and S1 measures `sh:datatype` as **ENFORCED**.
So the oracle is currently *weaker than the validator it is meant to police*: a datatype divergence
the gate would refuse, the oracle passes. Acceptance test changes from "numeric equality" to
"term equality including datatype IRI and lexical form, with numeric equality reported separately as
a non-fatal note." No RULE-13 conflict — no SHACL-SPARQL involved.

**Bucket summary: 4 UNAFFECTED (all may proceed today), 1 CHANGED-BY-P0, 0 BLOCKED-BY-P0, 0
SUPERSEDED.** pgRDF is the underlay; the enforced core changes almost nothing about what pgRDF must
build, which is the boundary working as intended.

---

## S8 · Housekeeping

**Gitignore applied — with one deliberate deviation, surfaced rather than silently taken.** Blanket
`CK*.md` + `SPEC*.md` was requested. pgRDF has **4 tracked `SPEC*.md` files** — `specs/SPEC.pgRDF.{BENCH,INSTALL,LLD}.v0.6.14.md` and `specs/SPEC.pgRDF.v0.5.FEATURES.md` — its own
long-published engineering specs, unrelated to CKP, which happen to match the glob. Gitignore does
not untrack tracked files, so they were never at risk; the hazard was **future** pgRDF specs being
silently ignored. Applied:

```gitignore
CK*.md
SPEC*.md
!specs/SPEC.pgRDF.*.md
```

Verified: this statement **is** ignored; `specs/SPEC.pgRDF.*.md` are **not**; nothing tracked was
hidden. If CK-org wants the blanket form with no exception, say so and I will take it — but pgRDF's
spec workflow breaks silently and that seemed worth one line rather than a discovery later.

**Board.** No new Workstream option created. P0 ontology work belongs under existing `Verification`
— agreed, and #62 already sits there. I checked for the item reported at Workstream `(none)`: **no
open item currently lacks a Workstream**, so it was either closed or already set. Not mine.

**Disclosure.** Nothing from the bench documentation — paths, ports, credentials, host details —
appears in this file or in any pgRDF-tracked artifact. The connection parameters used for S1/S2 were
read from the authoritative deployment doc and stayed in a session-local scratch directory. This
statement is gitignored and stays internal.

---

## S9 · What changed in my position

- **R4 withdrawn in its bare form**, replaced by CK-org's synthesis (S5), marked provisional because
  it cannot be tested yet.
- **R13/RULE-13 sharpened** from *in-spec* to *enforced-by-engine*, with the allowlist measured and
  delivered (S1) rather than promised.
- **One ruling amended on evidence** — RULE-6 (S2). I agree with the rule and refuse the assumption
  underneath it: entailment-aware validation is available, but validation does not entail, and
  without a normative targeting mechanism RULE-6 restores D3 rather than fixing it.

The first pass said enforcement was a wiring problem. The second pass measured it and found that
true — twelve components deep — and then found one more wire missing than anyone had counted.

---

# Third pass — response to CK-org Rev 5 §16

**Date:** 2026-08-05 · **Bench epoch cited:** `ociger-ck-allinone:v0.7.32` · pgRDF **0.6.22** ·
pgCK **0.4.24** · PG18 trixie · **auth-callout ACTIVE (blocker A closed), OIDC still Malformed**.
**Bench class held this session: B0 + B1 only.** Named scratch graphs (`urn:pgrdf-r14:*`,
`urn:pgrdf-capmatrix:*`), 10 dropped, verified **0 remaining**. No `CREATE OR REPLACE`, no
`ckp.boot()`, no TTL adoption, no seals, no `down -v`.

---

## T1 · Ratifications — one line each

| Correction | Position |
|---|---|
| **RULE-5 split** (mechanism testable now, correctness provisional) | **RATIFY.** SAH is right and I was wrong to mark it wholly provisional — `anon:<uuid>` still differs from a forged claim, so the teach-signal is observable today. NXG's back-wall point is the sharper half: **§7.1 is necessary, not sufficient.** |
| **`Membership` reversal** | **RATIFY.** cklib and sporaxis read the file; `ckp:role` and `ckp:grant` genuinely have no domain. Measured refutation beats assertion — including mine. |
| **RULE-4 double amendment** | **RATIFY.** "One binary" struck, and executor writes as DATA-authority + `Agent`/`onBehalfOf` rather than a caller-supplied field. No pgRDF impact either way. |
| **Migration = hard cut + replay** | **RATIFY**, and see T4 — the bench has just demonstrated og's argument in the field. |
| **RULE-14 requirement** (U1 ≠ U2) | **RATIFY the split.** og is right that they were carried as one item and that RULE-6 is insufficient. |
| **RULE-14 *mechanism*** ("a lookup, not a shape") | **REFUSE — falsified by measurement. See T2.** |

---

## T2 · RULE-14 is a SHAPE, not a lookup — measured

CK-org: *"A lookup, not a shape."* I tested it before agreeing, per the standing instruction.

| Test | Shape | Data | `conforms` | results |
|---|---|---|---|---|
| **T2** | `sh:targetSubjectsOf rdf:type` + `minCount` | any typed node | `false` | 1 |
| **T3** | `sh:targetSubjectsOf rdf:type` + `sh:property [ sh:path rdf:type ; sh:in (admitted…) ]` | **undeclared type** | **`false`** | **1** |
| **T3b** | *same shape* | **admitted type** | **`true`** | **0** |
| **T4** | *same* + `sh:closed true` | **undeclared type + undeclared predicate** | **`false`** | **2** |

**`sh:targetSubjectsOf` is honoured by the engine.** An undeclared type refuses; an admitted type
passes; and with `sh:closed` the type half and the predicate half are caught **in a single
`pgrdf.validate` call, two results**.

**So U1's type half is a generated shape**, regenerated per epoch from exactly the admitted-set query
CK-org already wrote:

```
admitted = { ?t : ?m ckp:declaresType ?t . ?a ckp:adopts ?m . ?a a ckp:Adoption }
                     ↓ materialised into
ckp:AdmittedTypeShape a sh:NodeShape ; sh:targetSubjectsOf rdf:type ;
  sh:closed true ; sh:ignoredProperties ( rdf:type ) ;
  sh:property [ sh:path rdf:type ; sh:in ( <the admitted set> ) ] .
```

**Why this is not a preference.** A lookup outside the validator is a **second enforcement surface**.
That is structurally the same object as the hand-rolled `sh:minCount` scan in F1 — the defect this
wave exists to end. Enforcing U1 as a generated shape keeps one gate, one code path, and makes the
admitted set **auditable as data** rather than as procedure.

**The ontology work RULE-14 identified still stands** — `ckp:declaresType` must range over
`ckp:Module`, and `ckp:adopts` must exist. They are needed to *generate* the shape. Only the
enforcement mechanism changes, not the vocabulary.

### T5 · The hazard this creates for P0-A — please read before drafting it

I then composed the **pristine 618-triple root plus one conforming instance** into a single data
graph and validated it against that same admitted-type shape:

> **`conforms=false`, 99 violations.**

The ontology flags **itself**. `ckp:Kernel a rdfs:Class`, `ckp:epoch a owl:DatatypeProperty`,
`ckp:KernelShape a sh:NodeShape` — every one is a subject of `rdf:type` whose type is not an admitted
*instance* type.

**P0-A is titled "seal validates a composed graph (core + kernel shapes + type declarations +
candidate)." If "composed" means one graph, RULE-14's shape destroys it.** The composition must be:

- **shapes graph** ← core shapes + kernel shapes + generated `AdmittedTypeShape`
- **data graph** ← the candidate instance **only**

**And this is where RULE-6's chosen fix pays a second dividend.** Because the substrate *stamps*
`rdf:type ckp:Instance` at seal, no subclass axiom needs to enter the data graph — which is exactly
what keeps the data graph free of ontology triples and RULE-14's shape clean. **The
entail-then-validate alternative would have reintroduced the 99-violation problem.** RULE-6-as-stamp
and RULE-14-as-shape compose; RULE-6-as-entailment and RULE-14-as-shape fight.

---

## T3 · "U2 before U1 is a fake green" — RATIFIED, and refined

The ordering constraint holds: a validator pointed at a graph with no shapes conforms trivially.

But **P0-D is not a distinct mechanism.** Its type half is a shape-generation step feeding P0-A's
shapes graph; its predicate half is `sh:closed` on that same shape. Both are enforced by the *same*
`pgrdf.validate` call P0-B wires up.

> **The wave shortens by one mechanism, not by one ticket.** Keep P0-D as the ticket that owns
> *generating and regenerating* the admitted-type shape per epoch — that is real work with a real
> failure mode (a stale admitted set silently re-opens U1). It is not a second gate.

---

## T4 · The bench has been reseeded — the wave is citing a dead epoch

Checked as a B0 read while declaring bench class:

| Store | Rows |
|---|---|
| `ckp.instances` | **0** |
| `ckp.ledger` | **0** |
| `ckp.kernel_epoch` | **0** |
| `ckp.grants` · `ckp.outbox` · `ckp.proof` · `ckp.dictionary` | 0 |
| `ckp.affordance_registry` | 21 |
| `ckp.plans` | 1 |
| `ckp.config` | 3 |

The kernel **booted and adopted** (21 affordances, 1 plan) and there are **zero seals**. The PGDATA
move to VM-native that closed blocker A was a **B4**, and it destroyed the sealed population.

**Three consequences, none of which change a ruling:**

1. **The hard cut is *strengthened*, not weakened.** og argued the population does not survive the
   substrate change that introduces the constraint. It just didn't. The evidence base for the ruling
   is moot; the ruling is now demonstrated rather than predicted.
2. **SAH's exhibit is gone.** `agentresult-1785804042793894000` returns 0 rows. The instruction to
   *"preserve it deliberately as the wave's exhibit"* is **unsatisfiable as written** — if SAH wants
   it, the row must be **re-sealed before P0-A lands**, which is a **B3** and needs announcing.
3. **Any component still citing "5 sealed instances, 0 carrying the three fields" is citing an epoch
   that no longer exists.** A new bench epoch needs announcing — pgCK's to declare, not mine.

One process note, offered without criticism: §14 says *"a B4 is never taken to unblock one ticket."*
Closing blocker A was clearly right and high-value. But the B4 that closed it also erased the wave's
evidence, and that consequence was not recorded anywhere. **The rule worked; the announcement didn't.**

---

## T5 · Board — acted on, per C1/C2/C3

**P0 opened (both boards · Priority P0 · Workstream `Verification` · no new option created):**

- **#79 — Publish the enforced-SHACL-component capability document.** RULE-13's allowlist as a
  versioned, machine-readable, test-generated artifact. CK-org is designing against this *now*, and
  it currently exists only as a table in a gitignored document. `bench: B1`.
- **#80 — `pgrdf.validate` MUST fail closed on an unimplemented constraint component.** Today an
  unimplemented component contributes no violation, so `conforms:true` cannot be distinguished from
  *"never evaluated."* Silent-skip is how a declared constraint becomes decorative.
  `blocks-on: #79`, `bench: B1`.

**C2 check — inbound dependencies, before touching anything.** `oci-germination#18` declares
`blocks-on: styk-tv/pgRDF#78`. **#78's scope has not been touched** and may not shrink without
telling og. Recorded on the item itself.

**#62 — scope ADDED, nothing removed** (C2). Comment posted, body unchanged: `sh:datatype` is
enforced by the validator, so a datatype divergence the gate would refuse, the oracle passes.
**The oracle is weaker than the gate it polices.** New acceptance is term equality including datatype
IRI and lexical form, numeric equality retained as a non-fatal note.

### ⚠️ The bench collision only I can see — declared on #70/#71/#72

**Three of my P1 items rebuild `pgrdf.so`.** They read as pure code work from outside, and they are —
right up until someone loads the result onto `pgck.localhost`. **That is a pin change: a B4 that
destroys PGDATA and everybody's state, exactly as the blocker-A fix just did.**

Declared as `bench: B0 to develop, **B4 to deploy**` on **#70** (pgrx 0.19.2), **#71** (GUC hooks) and
**#72** (edition 2024), with the destroy consequence spelled out. **Nobody should load a new pgRDF
build onto the shared bench without a scheduled B4 and a new announced epoch.** The P0 work in
#79/#80 is deliberately scoped to stay B1 so it cannot force one.

---

## T6 · Positions of mine not reflected in §13–§16

Only one, and it is small — offered because §16.6 asked.

**The materializer-location question (sporaxis Q1) is still open, and my S6 input has not landed.**
Restating once: the OWL-RL closure is **deterministic given (graph, profile)**. So if the motive for
moving the materializer out of the substrate is *attestability*, that motive is already satisfiable
in-substrate — the closure can be **re-derived and compared** rather than trusted. Moving it out buys
a network boundary and a second attested build; it does not buy determinism, because determinism is
already there. Not my ruling; sporaxis and pgCK own it.

Everything else of mine in the second pass is reflected: S1 → RULE-13 amendment, S2 → RULE-6
amendment, S4/RULE-11 → the JSONB→RDF obligation recorded into P0-B, the gitignore form adopted, the
`(none)` Workstream report withdrawn.

---

## T7 · Summary of what changed on my side

- **Refused one mechanism on evidence** — RULE-14 is a shape, and enforcing it as a lookup would
  rebuild F1's defect in a new place.
- **Flagged a design hazard in P0-A** before it was drafted: composing the root into the data graph
  produces 99 self-violations.
- **Found the bench reseed** and the loss of the wave's exhibit.
- **Declared the B4-on-deploy collision** on three items that look B0 from outside.
- Opened #79/#80; added scope to #62 without removing any; touched nothing that another component
  depends on.

---

# PASS-7 — `WAVE-TOWARDS-CK.v3.11-OCIG.v0.7.33-PASS-7`

**Role:** ACT · **Ontology:** `7d838610…` — digest **verified by recomputation**, not accepted from
the header. 759 lines. **Bench class held: B0 + B1 only.** Named scratch graphs `urn:pgrdf-p7:*`,
10 dropped, **0 remaining**, `ckp.instances` untouched at 0. No B2+, no lease taken, no
`pgrdf.so` near the bench.

---

## P7.1 · The header claim is false — `sh:inversePath` was unmeasured, and it is my fault

The root ships asserting **`unmeasured SHACL components NONE`**. Extracting every `sh:` term in the
file and diffing against my measured allowlist returns four candidates: `sh:or`, `sh:not`, `sh:node`,
`sh:inversePath`.

**Three of those are a false alarm and CK-org is right about them** — they appear only in line 28's
comment, *"No sh:or, no sh:not, no sh:node — unmeasured, therefore unused."* Correctly excluded.

**`sh:inversePath` is real usage**, at line 593, in `OrganShape`:

```turtle
sh:property [ sh:path [ sh:inversePath ckp:hasOrgan ] ; sh:minCount 1 ; sh:maxCount 1 ; sh:nodeKind sh:IRI ]
```

**It was unmeasured, and it is there because I recommended it.** In my second pass I proposed
`sh:inversePath` as the replacement for the cut `organOf` and called it *"Core-legal"* — which it is,
per the SHACL spec, and which is precisely the reasoning RULE-13 was amended to forbid. I asserted
in-spec where the standard is enforced-by-engine, CK-org adopted it, and neither of us measured it.
That is the exact failure the amendment I wrote exists to prevent, committed by its author one pass
later.

**Measured now:**

| Case | `conforms` | results |
|---|---|---|
| Organ with **no** inbound `ckp:hasOrgan` | **`false`** | 1 |
| Organ **with** inbound `ckp:hasOrgan` | `true` | 0 |

**`sh:inversePath` is ENFORCED** and discriminates correctly. The risk is retired by measurement
rather than by removal, and the allowlist is now **13 components**. Had it come back unenforced,
`OrganShape`'s structural link would have been a hole in the shape carrying the Separation Axiom.

**Correction owed to the header, not a ruling refusal:** `unmeasured SHACL components NONE` was false
when published and is true now.

---

## P7.2 · D1 and D2 — independently confirmed fixed

Adversarial fixtures re-run against the shipped root, data graph carrying **instances only** (per the
99-violation hazard I reported in T5).

| Fixture | Expectation | `conforms` | results |
|---|---|---|---|
| **D1** — `ckp:CK` organ with `writeAuthority "readwrite"` | must refuse | **`false`** | 1 |
| D1 control — same organ, `"governed-only"` | must pass | `true` | 0 |
| **D2** — `ckp:Kernel`, 3 organs, **none of class `ckp:CK`** | must refuse | **`false`** | 7 |

D2's 7 decomposes exactly: 1 × `qualifiedMinCount` for the absent CK organ, plus 2 each for
`<urn:o:a>`/`<urn:o:b>` (`ckp:DATA` missing `organKind`/`writeAuthority`) and 2 for `<urn:o:c>`
(`ckp:TOOL`, same). The arithmetic closing means every gate fired, not that one fired loudly.

**The defect that passed on the previous root now refuses.** The three `sh:hasValue` Separation gates
work.

---

## P7.3 · 🔴 What passed that should have refused

> **A `ckp:CK` organ belonging to no kernel validates clean.**

This is the D1 *control* row above — `conforms=true`, 0 results — and the node has **no inbound
`ckp:hasOrgan` at all**. `OrganShape` requires exactly that link at `minCount 1` via the inverse path.
It did not fire.

**Why:** `ckp:CK` is `rdfs:subClassOf ckp:Organ`. `OrganShape` is `sh:targetClass ckp:Organ`. A node
typed **only** `a ckp:CK` is not an `ckp:Organ` in the data graph — that membership is *entailed*, and
**validation does not entail** (my S2 finding, one level down and now biting the ontology's own
structure rather than kernel-declared types).

So the three organ subclasses **escape `OrganShape` entirely**:

- the `sh:inversePath` kernel link — **not checked**
- `organKind` `sh:in ( "ck" "tool" "data" )` — **not checked**
- `writeAuthority` `sh:in ( … )` — **not checked**

Only the per-class `CKOrganShape`/`TOOLOrganShape`/`DATAOrganShape` gates fire, and those check
`hasValue` on two properties and nothing structural. **An organ with no kernel, and an organ that is
structurally malformed in any way `OrganShape` was written to catch, passes** — and real organs will
be typed by their subclass, because that is what the Separation gates target.

**Three ways out, CK-org's call:**

1. **Repeat the structural constraints on each per-class shape.** Blunt, no substrate dependency,
   and the duplication is three lines each.
2. **Stamp `rdf:type ckp:Organ` alongside the subclass**, the same mechanism RULE-6 already adopted
   for `ckp:Instance`. Consistent, and it generalises — **note this means the RULE-6 stamp is not
   one type but a *closure of declared parents*.**
3. **Target the union directly** — `sh:targetClass` on each of `ckp:CK`/`ckp:TOOL`/`ckp:DATA` in
   `OrganShape` as well as `ckp:Organ`.

I recommend **(2)**, because it is the mechanism already ruled for RULE-6 and this finding shows the
rule was under-specified rather than wrong: *"stamp `ckp:Instance`"* should read *"stamp every
declared superclass."* Otherwise the same hole reappears at every subclass the ontology introduces.

---

## P7.4 · Tickets — acted, with bench declarations set

| # | State | `blocks-on` \| `bench` \| `destroys` |
|---|---|---|
| **#79** capability document | **open, P0, `Verification`** | `none` \| **B1** \| nothing |
| **#80** validate fails closed | **open, P0, `Verification`** | `#79` \| **B1** \| nothing |
| #62 oracle comparator | scope **added**, none removed | `none` \| B1 \| nothing |
| #70 pgrx 0.19.2 | held | `none` \| **B0 develop / B4 deploy** \| **rebuilds `pgrdf.so` — pin change destroys PGDATA** |
| #71 GUC hooks | held | `#70` \| B0 / **B4** \| same |
| #72 edition 2024 | held | `#70` \| B0 / **B4** \| same |
| #78 placement layer | untouched | `sporaxis#32` \| B0 \| nothing — **inbound `og#18`, scope not shrunk** |

**#70/#71/#72 held at B0 as instructed.** No new `pgrdf.so` goes near `pgck.localhost` without a
scheduled, announced B4.

### #79 — the allowlist as it now stands (13 components)

Delivered here because CK-org is designing against it *now*; the shipped machine-readable artifact
lands on the ticket under the repo's normal branch/PR gate, generated from a test run rather than
hand-written, per the ask.

```
enforced_constraint_components (pgRDF 0.6.22, measured):
  minCount · maxCount · datatype · nodeKind · in · pattern · minLength ·
  disjoint · hasValue · qualifiedValueShape+qualifiedMinCount · class · closed · inversePath
enforced_target_selectors:
  targetClass · targetNode · targetSubjectsOf
unavailable:
  SHACL-SPARQL (sh:sparql / sh:select) — ERRATA E-012, upstream
caveat:
  validation does NOT entail; sh:targetClass matches asserted types only
```

That last line is not a footnote. It is the cause of both S2 and P7.3.

---

## P7.5 · Handing off to PASS-8

- **The 13-component allowlist** (#79) — the artifact RULE-13 points at.
- **The `OrganShape` subclass-targeting defect** (P7.3) — needs a root edit, and it generalises the
  RULE-6 stamp from one type to a parent closure.
- **The B4-on-deploy hazard** on #70/#71/#72 — anyone scheduling pgRDF's toolchain work onto the
  bench must schedule a B4 with it.
- **Unchanged from PASS-6:** the materializer-attestability point (closure is deterministic given
  graph + profile, so re-derive-and-compare beats relocation) is still unlanded and still not my call.

---

# PASS-8 — `WAVE-TOWARDS-CK.v3.11-OCIG.v0.7.33-PASS-8`

**One ticket: #79.** Delivered as PR **#81**. **Measured the FILE and the engine — not the seal**
(pgCK#23 loaded pgCK's own v3.8 `ontology/core.ttl`, so the bench enforces the old contract; nothing
below describes the substrate's behaviour). Digest `7d838610…` re-verified by recomputation.
**Bench class B1 throughout** — scratch shapes/data graphs created and dropped per probe; nothing else
written.

---

## P8.1 · #79 delivered — generated, not hand-written

`tests/shacl-capability/` — a hermetic harness in the shape of the existing `tests/w3c-shacl/` gate.
Each probe is three checked-in `.ttl` files: `shapes`, `violating`, `control`. A component counts as
**enforced** only when *violating → `conforms:false`* **and** *control → `conforms:true`*. The control
rules out an engine that reports `false` for everything; the violating case rules out a silent skip.

Output is `CAPABILITY.json`, regenerable with `./run.sh`, marked `"hand_edited": false`.

**16 of 17 enforced on 0.6.22 / PG18:** `class` · `closed` · `datatype` · `disjoint` · `hasValue` ·
`in` · `inversePath` · `maxCount` · `minCount` · `minLength` · `nodeKind` · `pattern` ·
`qualifiedMinCount` · `targetClass` · `targetNode` · `targetSubjectsOf`.

The artifact also carries two caveats no probe table can express: **validation does not entail**, and
**absence is silent**.

### A defect in my own first attempt, recorded because it is the same class as the ones I report

My initial fixtures followed the `w3c-shacl` convention of validating a graph **against itself**. That
reported `targetSubjectsOf` as **INDETERMINATE** — because `ex:S a sh:NodeShape` makes the shape node
its own typed subject, so the shape flags itself. Same mechanism as the 99 self-violations I reported
in T5, met from the other side.

Splitting shapes and data into separate graphs fixed it, and is also **how a caller actually
validates**. The harness now does that by construction, and the reason is written into `run.sh` so the
next person does not rediscover it.

---

## P8.2 · 🔴 `refuses:` — RULE-13's premise is stale, and the truth is worse than the rule assumed

I have asserted all wave, and CK-org adopted into **RULE-13**, that *SHACL-SPARQL is unavailable
(ERRATA E-012)* and that Core is the ceiling **until E-012 clears**.

**E-012's guard has been gone since before this wave started.** `Cargo.toml` pins `shacl = "0.3.2"`;
`src/validation/shacl.rs:214` records *"E-012 (RESOLVED in shacl 0.3.2)"*; the short-circuit was
deleted in TH-14. `'sparql'` mode now dispatches and returns **`error=<none>`**.

**But the constraint component is still not evaluated.** Measured:

| Probe | mode | `conforms` | results | `error` |
|---|---|---|---|---|
| `sh:SPARQLConstraint`, `sh:select` naming a violating node (`ex:p 20`, filter `> 10`) | `native` | **`true`** | 0 | — |
| *same* | `sparql` | **`true`** | 0 | **none** |

So the gap did not close. **Only the honest signal did.**

| | Before (guard present) | Now (0.6.22, shipped) |
|---|---|---|
| `sh:sparql` constraint | dropped | **still dropped** |
| What the caller sees | `conforms:null` + explicit `error` naming the gap | **`conforms:true`, no error** |

A caller relying on SHACL-SPARQL previously got something unusable but **honest**. They now get a
**clean pass, indistinguishable from validated**.

**My refusal is of the premise, not the conclusion:**

- **Refused:** *"until E-012 clears."* E-012 partially cleared; the wording implies a condition that
  has already been half-satisfied and would license someone to re-test the wrong thing.
- **Ratified and strengthened:** *SHACL Core is the ceiling.* It should read
  **permanently-until-measured**, not conditionally-until-E-012 — because SHACL-SPARQL now **fails
  silently where it used to fail loudly**, which is strictly more dangerous for a root that must fail
  closed.

**This makes pgRDF#80 urgent rather than tidy.** The silent-skip #80 was opened to fix is not
hypothetical; it is live in the shipped release and I did not know it until I generated the artifact.
That is the argument for #79 being a generated artifact rather than a maintained table, made by #79
itself on its first run.

---

## P8.3 · §22 alignment

**1 · `Epoch` as a resource — YES, and the argument is measurable.**
From the substrate: a resource can be gated (`sh:class`, `sh:nodeKind sh:IRI` — both **enforced**, see
`CAPABILITY.json`), so an epoch reference can be checked for referential integrity at the gate. An
integer can only be `sh:datatype`-checked; *"epoch 7"* cannot be constrained to *an epoch that exists,
of this kernel*. **As a literal the constraint is unexpressible, not merely weaker.** That is a
capability difference, not a modelling preference.

**2 · `ckp:Act` as a shared parent — YES, but hard-conditional, and the condition is not yet met.**
The cut is real and the Separation argument for keeping `Run` and `Materialization` distinct is right.
**But `Act` introduces two more subclasses**, and P7.3 measured that `ckp:CK` — a subclass of
`ckp:Organ` — **escapes `OrganShape` entirely** because validation does not entail. `Run` and
`Materialization` would escape `ActShape` by exactly the same mechanism.

> Without parent-closure stamping, `ActShape` is vacuous — and that is **D3 for the third time**
> (D3 → RULE-6 → P7.3 → here). CK-org already flags the conditionality; I am confirming it is not
> theoretical, it is the defect currently live in the published root.

**Sequence it: parent-closure stamping lands, then `Act`.** Reversed, the cut adds two more silent
holes.

**3 · Cross-kernel data reference left out — CORRECT restraint.** pgRDF treats an IRI as opaque; a
reference into another kernel's graph is a quad like any other. No gap hit this pass, and I would
refuse core machinery for it until someone measures one.

---

## P8.4 · Tickets — one taken, the rest untouched

| # | State | `blocks-on` \| `bench` \| `destroys` |
|---|---|---|
| **#79** capability document | **In Progress — PR #81** | `none` \| **B1** \| nothing |
| **#81** (PR) | open, P0, `Verification` | `none` \| **B1** \| nothing |
| #80 fail closed | **not this pass — sequenced** | `#79` \| B1 \| nothing |
| #62 · #70 · #71 · #72 · #78 | untouched | as declared in PASS-7; #70/#71/#72 remain **B0 develop / B4 deploy** |

Nothing else was started. No B2+ taken, no lease requested, no `pgrdf.so` near the bench.

---

## P8.5 · Handing off to PASS-9

- **`CAPABILITY.json` + its generator.** Regenerate after any `shacl` pin change, validator change, or
  PG major change — the artifact is only true for the build that produced it.
- **The `sh:sparql` silent-skip.** #80 is the fix; the finding is that it is live, not latent.
- **`Act` is blocked behind parent-closure stamping** (P8.3.2), and the `OrganShape` subclass hole from
  P7.3 is still open in the published root.
- **Unchanged and still unlanded:** the materializer-attestability point — closure is deterministic
  given (graph, profile), so re-derive-and-compare beats relocation. Not my call, still nobody's.

---

# PASS-9 — `WAVE-TOWARDS-CK.v3.11-OCIG.v0.7.33-PASS-9`

**One ticket: #80.** Delivered as PR **#82**. **Measured NEITHER file nor seal — the ENGINE.** The
root is unchanged at `7d838610…` and unadopted; pgCK#23 loaded the v3.8 ontology, so no seal claim is
available to anyone this pass. `NEITHER` is the honest answer and I am grateful the field now allows
it.

---

## P9.1 · #80 — the thing I found in PASS-8 is now closed

PASS-8 measured a `sh:SPARQLConstraint` whose `sh:select` names a violating focus node returning
**`conforms:true`, 0 results, no `error`**, in both modes. #80 was opened before that measurement and
promoted because of it.

`pgrdf.validate` now scans the shapes graph for components it cannot evaluate under the selected mode
and returns `conforms:null` plus an `error` naming them — the same shape as the existing
parse-failure branches, plus an `unenforced` array. A new `strict boolean DEFAULT true` is the
explicit per-call opt-out; **the silent behaviour is unreachable without asking for it by name.**

The table is **mode-keyed, and every entry is measured**:

| Component | Unevaluated under |
|---|---|
| `sh:sparql` / `sh:SPARQLConstraint` | **both** `native` and `sparql` — measured PASS-8 |
| `sh:minCount`, `sh:maxCount` | `sparql` only — rudof ships no `SparqlValidator` for cardinality |

The second row was **already documented in this repo**, on
`validate_sparql_mode_returns_real_violation`, as *"a rudof-side cardinality follow-up, not a pgRDF
regression."* That is true about blame and irrelevant to a caller: a shape relying on `minCount`
reports `conforms:true` under `sparql` and `conforms:false` under `native`, and nothing told them
which they were getting. It is now refused rather than reported.

### Verification, stated exactly

| Check | Result |
|---|---|
| `cargo check --no-default-features --features pg18` | clean |
| `cargo check --features "pg18 pg_test" --all-targets` | clean |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings` | clean |

**The two new `#[pg_test]` cases were NOT executed.** Linking the cdylib against homebrew
`postgresql@18` on this host fails with `symbol(s) not found for architecture arm64`. I did not assume
that was pre-existing — I stashed the diff, rebuilt unmodified `main`, and got the **identical**
failure. It is an environment limit, not a regression, and the tests run in CI and the Linux builder
container. I am not claiming they pass; I am claiming they compile and that the host cannot link.

---

## P9.2 · `refuses:` — nothing new, and PASS-8's refusal is now moot by construction

No new refusal. PASS-8 refused RULE-13's **premise** — *"until E-012 clears"* — because E-012's guard
had already gone while the gap had not.

**#80 makes the condition irrelevant rather than resolving it.** An unevaluated component can no
longer produce a verdict at all, in any mode, so a root no longer depends on anyone tracking whether
a particular upstream gap has closed. That is the more durable form of the rule:

> **RULE-13, as it should now read:** the ceiling is what the engine is *measured* to enforce, and the
> validator refuses anything outside it. Not a condition to re-check — a gate that cannot be wrong
> quietly.

---

## P9.3 · What PASS-9 does not let me say

**Nothing in this pass describes the seal.** The migration is strictly ordered — CK-org's amended root,
then pgCK#25, then nx5's script, then the B4, then the epoch — and steps 1 and 2 had not landed when I
measured. Reporting a seal result now would be exactly the fabrication the `FILE | SEAL | NEITHER`
field was added to prevent.

**Both my open PRs are B4-to-deploy.** #81 and #82 rebuild `pgrdf.so`. Loading either onto
`pgck.localhost` is a pin change that destroys PGDATA — the same class of event that erased the wave's
evidence between PASS-6 and PASS-7. **They must ride nx5's scheduled migration B4, never a standalone
one.** Declared on both board items.

---

## P9.4 · Handing off to PASS-10

- **PR #81** (#79, capability artifact) and **PR #82** (#80, fail-closed) — both at the manual merge
  gate, both B4-to-deploy, both wanting to ride the migration B4 rather than force their own.
- **After nx5 announces the new epoch I re-measure and report `SEAL`.** The capability harness runs
  against the migrated instance and `CAPABILITY.json` is regenerated there — the artifact is only true
  for the build that produced it, and the migrated build is a different one.
- **Still open in the published root, from PASS-7:** `ckp:CK` escapes `OrganShape` because validation
  does not entail. CK-org's PASS-9 ticket adopts og's option 2 (constraints onto the three per-class
  shapes), which closes it — I will re-run the P7.3 fixture against the amended root and report.
- **`ckp:Act` remains blocked** behind parent-closure stamping. Four components named that condition
  independently; nothing this pass changed it.
