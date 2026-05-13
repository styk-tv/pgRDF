# Changelog

All notable changes to pgRDF are tracked here. Format follows
[Keep a Changelog](https://keepachangelog.com/). Versioning is SemVer
once we cut v1.0; pre-1.0 minor bumps may include breaking changes.

## [Unreleased]

### Added
- Project scaffolding: pgrx 0.16 extension targeting PG 14–17.
- SPEC.pgRDF.LLD.v0.2 + SPEC.pgRDF.INSTALL.v0.2 captured under `specs/`.
- Compose-based local runtime: stock `postgres:17.4-bookworm` with
  per-file bind mounts at `$libdir` / `$sharedir/extension`. No init
  script, no entrypoint wrapper.
- Linux builder container (`compose/builder.Containerfile`) that
  produces glibc-bookworm artifacts on macOS hosts.
- CI/release workflow placeholders for the {pg14..pg17}×{amd64, arm64}
  matrix.

### Errata against v0.2 specs
- `shacl-rust` → `shacl_validation` (E-001).
- `reasonable` is OWL 2 RL only, not arbitrary Datalog (E-002).
- PG 18 forward path blocked on pgrx 0.17/0.18 not building on current
  Rust (E-006). Compose targets PG 17 until upstream lands a fix.
- See [`specs/ERRATA.v0.2.md`](specs/ERRATA.v0.2.md) for the full set.
