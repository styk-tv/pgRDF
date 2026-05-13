# 07 — Development

Two parallel dev loops, picked by speed-vs-realism trade-off.

## 7.1 Fast loop — pgrx-managed Postgres

`cargo pgrx run pg18` boots a Postgres instance bundled inside pgrx,
loads the extension, and drops you into a psql shell. Edit Rust →
`\q` → re-run → see changes immediately.

```bash
just dev               # ≡ cargo pgrx run pg18
```

This is the right loop for:
- Iterating on Rust code structure
- Unit + pgrx integration tests (`cargo pgrx test pg18`)
- Quick "does this UDF return the right thing" checks

It is **not** the right loop for verifying the deployment model —
pgrx's bundled Postgres differs from the production postgres:18-bookworm
in several subtle ways (file locations, build flags). Use the slow loop
below to verify that.

## 7.2 Slow loop — stock postgres container

This boots `postgres:18-bookworm` and side-loads the locally-built
extension via the INSTALL spec §7 GUC path.

```bash
just build-ext         # builds linux .so in a builder container
just compose-up
just psql              # → pgrdf=# CREATE EXTENSION pgrdf;
```

Use this loop for:
- Deployment-model verification (CREATE EXTENSION succeeds, version
  matches, shared_preload_libraries works)
- Integration with anything that talks to Postgres over the wire

## 7.3 Test layers

See [08-testing.md](08-testing.md).

## 7.4 Style

- `cargo fmt --all` — required, gated in CI.
- `cargo clippy --no-default-features --features pg18 -- -D warnings`
  — required.
- Per-module README: not maintained. Module-level docs live in
  `//!` comments inside the corresponding `mod.rs`.

## 7.5 Adding a new UDF

1. Add the `#[pg_extern]` annotation in the right module under `src/`.
2. Run `cargo pgrx schema` to regenerate the install SQL. Pgrx auto-
   appends function bindings to `sql/pgrdf--<ver>.sql`.
3. Write a `#[pg_test]` adjacent to the function definition.
4. Update `docs/`-relevant section.
