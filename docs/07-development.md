# 07 — Development

Two parallel dev loops, picked by speed-vs-realism trade-off.

## 7.1 Fast loop — pgrx-managed Postgres

`cargo pgrx run pg17` boots a Postgres instance bundled inside pgrx,
loads the extension, and drops you into a psql shell. Edit Rust →
`\q` → re-run → see changes immediately.

```bash
just dev               # ≡ cargo pgrx run pg{{PG_MAJOR}} (default PG_MAJOR=17)
```

This is the right loop for:
- Iterating on Rust code structure
- Unit + pgrx integration tests (`just test-native` ≡ `cargo pgrx test pg17`)
- Quick "does this UDF return the right thing" checks

It is **not** the right loop for verifying the deployment model —
pgrx's bundled Postgres differs from the production postgres:17.4-bookworm
in several subtle ways (file locations, build flags). Use the slow loop
below to verify that.

PG 18 is deferred pending a stable pgrx 0.17+/0.18 line on current
rustc (ERRATA E-006). The current v0.5.x line pins to pgrx 0.16.1 +
PG 14–17.

## 7.2 Slow loop — stock postgres container

This boots `postgres:17.4-bookworm` and side-loads the locally-built
extension via per-file bind mounts at canonical `$libdir` /
`$sharedir/extension` paths (the PG ≤ 17 incarnation of INSTALL §4.3;
the §7 GUC path activates with PG 18 — see ERRATA E-007).

```bash
just build-ext         # builds linux .so in a builder container (Colima)
just compose-up        # boots compose stack on podman
just psql              # → pgrdf=# CREATE EXTENSION pgrdf;
just test-artifact-parity  # mounted bytes == fresh build == live container
```

Use this loop for:
- Deployment-model verification (CREATE EXTENSION succeeds, version
  matches, shared_preload_libraries works)
- Integration with anything that talks to Postgres over the wire

Builds run on Colima (heavy builder image + cargo cache); the compose
runtime runs on podman. Override with `PGRDF_BUILD_RUNTIME` /
`PGRDF_RUN_RUNTIME`; see `just runtimes` for the active picks.

## 7.3 Test layers

See [08-testing.md](08-testing.md).

## 7.4 Style

- `cargo fmt --all` (`just fmt`) — required, gated in CI.
- `cargo clippy --no-default-features --features pg17 -- -D warnings`
  (`just clippy`) — required.
- Per-module README: not maintained. Module-level docs live in
  `//!` comments inside the corresponding `mod.rs`.

## 7.5 Adding a new UDF

1. Add the `#[pg_extern]` annotation in the right module under `src/`.
2. Run `cargo pgrx schema` to regenerate the install SQL. Pgrx auto-
   appends function bindings to `sql/pgrdf--<ver>.sql`.
3. Write a `#[pg_test]` adjacent to the function definition.
4. Update `docs/`-relevant section.
