# tests/shacl-capability — the SHACL capability document

Generates [`CAPABILITY.json`](CAPABILITY.json): for each SHACL constraint
component and target selector, **does `pgrdf.validate` actually enforce it
on this build?**

## Why

Consumers choose shapes against what the engine enforces, not against what
the SHACL specification defines. Those two sets are not the same, and the
difference is invisible: an unimplemented component contributes no violation
**and no error**, so `conforms:true` does not distinguish *validated clean*
from *never evaluated*.

Before this harness the allowlist lived in prose. A shape chosen against
prose is a shape chosen against a guess.

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
