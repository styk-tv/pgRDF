# W3C SHACL conformance harness

Mirror of [tests/w3c-sparql/](../w3c-sparql/) for the W3C SHACL test
suite at https://github.com/w3c/data-shapes.

## Layout (target)

    tests/w3c-shacl/
    ├── README.md                  (this file)
    ├── manifest_runner.rs         (Phase 3)
    └── fixtures/                  (submodule: w3c/data-shapes, sparse-checked to data-shapes-test-suite/tests/core/)

## Manifest format

SHACL tests use Turtle manifests pointing at:
- `sht:dataGraph`   → input data graph
- `sht:shapesGraph` → SHACL shapes
- `mf:result`       → expected ValidationReport (Turtle)

The runner calls `pgrdf.validate()` and diffs the returned JSONB
against the expected report (after JSONB→Turtle round-trip via a tiny
deserializer).

## Coverage gates per phase

- Phase 1: scaffolded only.
- Phase 2: runner runs (may all-fail).
- Phase 3: ≥ 50% pass.
- Phase 4: ≥ 90% pass.
