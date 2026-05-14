# 05 — Validation

> **Status: stub (Phase 5 v0.3).** The SQL surface
> `pgrdf.validate(data BIGINT, shapes BIGINT) → JSONB` exists; the
> body returns `{"status": "stub", …}` because the upstream SHACL
> processor cannot currently coexist with our OWL 2 RL reasoner in
> one workspace. See [`specs/ERRATA.v0.2.md`](../specs/ERRATA.v0.2.md)
> **E-009** for the full dep-resolution analysis.

## Surface

```sql
SELECT pgrdf.validate(data_graph_id, shapes_graph_id);
--   data_graph_id   — graph containing the assertions to validate
--   shapes_graph_id — graph containing the SHACL shapes (sh:NodeShape /
--                     sh:PropertyShape) those assertions must satisfy
-- Returns JSONB:
--   { "status": "stub",
--     "reason": "ERRATA E-009 — …",
--     "data_graph_id":    <i64>,
--     "shapes_graph_id":  <i64>,
--     "data_triples":     <i64>,
--     "shapes_triples":   <i64>,
--     "conforms":         null,        -- placeholder; never set
--     "results":          []           -- placeholder; never populated
--   }
```

`conforms` is `null` (rather than the W3C `sh:conforms` boolean) so
calling code can distinguish "stub" from "validated and clean".

## Why it's a stub today

We need a SHACL processor and an OWL 2 RL reasoner in the same
binary. The two upstream crates have an incompatible dependency
shape:

1. `shacl_validation 0.2.x` (latest 0.2.12, 2026-04-22) ships an
   unfinished `iri_s` → `rudof_iri` migration. `shacl_ast 0.2.9`
   contains references to both crates and fails to compile:
   `expected rudof_iri::IriS, found iri_s::IriS`.
2. `shacl_validation 0.1.149` (last 0.1.x) compiles in isolation,
   but its transitives enable `oxrdf`'s `rdf-12` feature. That
   feature adds the `TermRef::Triple(_)` variant to `oxrdf::TermRef`.
3. `reasonable 0.4.1` (our OWL 2 RL reasoner, see
   [`docs/04-inference.md`](04-inference.md)) has a non-exhaustive
   pattern match in `common.rs::oxrdf_to_rio` that does not cover
   the triple-term variant.

So we can have `shacl_validation OR reasonable` in the workspace
today, but not both. Phase 4 (inference) shipped first and is the
load-bearing user-facing surface; Phase 5 ships as a stub until
upstream catches up.

## What unblocks the real integration

Either of:

- **`shacl_validation` 0.2.x release** that compiles cleanly against
  a single `iri_s` major (i.e. completes the `rudof_iri` migration
  in the AST + IR crates).
- **`reasonable`** publishes a version whose pattern matches handle
  RDF 1.2 triple-term operands (or otherwise tolerates the
  `rdf-12` feature being on).

When that happens the v0.4 ticket is straightforward:
1. Uncomment the `shacl_validation` line in `Cargo.toml`.
2. Replace the stub body of `pgrdf.validate` with serialization of
   both graphs to N-Triples + a `GraphValidation::from_graph(…)
   .validate(&schema_ir)` call.
3. Translate `ValidationReport.results()` into the JSONB
   `sh:ValidationReport` shape.

## Scope when the real validator lands

Per LLD v0.3 §5.3:

- ✅ SHACL Core node + property shapes
- ✅ Cardinality, value-type, value-range constraints
- ⚠️ SHACL-SPARQL constraints — Phase 5 stretch
- ❌ SHACL inheritance via `sh:and` / `sh:or` / `sh:xone` —
      depends on processor support
- ❌ Custom validators via `sh:js` — out of scope

## See also

- Implementation: [`src/validation/shacl.rs`](../src/validation/shacl.rs)
- Regression: [`tests/regression/sql/70-validate-stub.sql`](../tests/regression/sql/70-validate-stub.sql)
- ERRATA: [`E-001`](../specs/ERRATA.v0.2.md) — original
  `shacl-rust` → `shacl_validation` supersession.
- ERRATA: [`E-009`](../specs/ERRATA.v0.2.md) — current dep-block.
