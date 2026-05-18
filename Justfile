set shell := ["bash", "-uc"]

PG_MAJOR := env_var_or_default("PG_MAJOR", "17")

# Two container runtimes, scoped by purpose:
#   BUILD = docker (Colima)  — heavy builder images (~5 GB) + cargo cache live
#                              in the Colima VM (100 GB) rather than the
#                              podman VM (30 GB, used for the user's other
#                              container setups).
#   RUN   = podman           — boots the compose stack (postgres:17.4 + the
#                              extension files bind-mounted from the host).
# Override either via env: `PGRDF_BUILD_RUNTIME=podman just build-ext` etc.
BUILD := env_var_or_default("PGRDF_BUILD_RUNTIME", "docker")
RUN   := env_var_or_default("PGRDF_RUN_RUNTIME",   "podman")

# Linux build cache lives in a docker named volume inside Colima
# (which has 100 GB). A bind-mount to the macOS host was the cleaner
# idea, but Postgres's data-directory ownership check trips on Colima's
# rootless user-namespace mapping (host UID 501 ↔ container root,
# container postgres UID 100 has no host-side counterpart). The named
# volume side-steps that without using the podman VM's disk.
TARGET_VOLUME := "pgrdf-target-pg" + PG_MAJOR

# List recipes.
default:
    @just --list

# Print which runtime each recipe uses (sanity check).
runtimes:
    @echo "BUILD = {{BUILD}} ({{ if BUILD == 'docker' { 'Colima target' } else { 'native podman' } }})"
    @echo "RUN   = {{RUN}}"
    @echo "TARGET_VOLUME = {{TARGET_VOLUME}}"

# Initialize pgrx state on the macOS host (one-time, for `just dev`).
pgrx-init:
    cargo pgrx init --pg{{PG_MAJOR}} download

# Fast dev loop: pgrx-managed Postgres on the host (native build).
# Currently blocked on a darwin link-flag issue; tracked in ERRATA.
dev:
    cargo pgrx run pg{{PG_MAJOR}}

# Format + lint (host).
fmt:
    cargo fmt --all
clippy:
    cargo clippy --no-default-features --features pg{{PG_MAJOR}} -- -D warnings

# Native pgrx integration tests on the host (same caveat as `just dev`).
test-native:
    cargo pgrx test pg{{PG_MAJOR}}

# pgrx integration tests inside the linux builder container.
# Runs in Colima (BUILD={{BUILD}}) so the heavy cargo cache lives in Colima's
# VM, not the podman one. Source is bind-mounted from the host; the Linux
# build cache lands at $PWD/.target-linux/pg{{PG_MAJOR}} on the host so
# disk pressure is on macOS (lots of space), not the VM.
test:
    {{BUILD}} volume create {{TARGET_VOLUME}} >/dev/null 2>&1 || true
    {{BUILD}} run --rm \
        -v "$PWD:/work" \
        -v {{TARGET_VOLUME}}:/work/target \
        --workdir /work \
        -e CARGO_PGRX_TEST_RUNAS=postgres \
        pgrdf-builder-rust:pg{{PG_MAJOR}} \
        bash -c 'rm -rf /work/target/test-pgdata && chown -R postgres:postgres /work/target && cargo pgrx test --no-default-features --features pg{{PG_MAJOR}} pg{{PG_MAJOR}}'

# pg_regress-style golden tests piped at the compose Postgres on podman.
# Set ACCEPT=1 to re-baseline expected/.
test-regression:
    PGRDF_RUNTIME={{RUN}} bash tests/regression/run.sh

test-regression-accept:
    ACCEPT=1 PGRDF_RUNTIME={{RUN}} bash tests/regression/run.sh

# W3C-shape SPARQL harness against the compose Postgres on podman.
# Each subdir of tests/w3c-sparql/ is one test (data.ttl + query.rq + expected.jsonl).
test-w3c:
    PGRDF_RUNTIME={{RUN}} bash tests/w3c-sparql/run.sh

# W3C SHACL conformance harness against the compose Postgres on
# podman (v0.5-FUTURE §6). Vendored W3C SHACL Core fixtures +
# hand-derived expected {conforms,violations}. The Core suite is the
# v0.5 full-pass gate (§6.1 #1). `--sparql` runs the 'sparql'
# evaluation-engine sub-run and asserts the ERRATA.v0.5 E-012
# known-state (Core-violation parity with 'native'; SHACL-SPARQL
# constraint components are an upstream gap).
test-shacl-manifest *ARGS:
    PGRDF_RUNTIME={{RUN}} bash tests/w3c-shacl/run.sh {{ARGS}}

# LUBM-shape correctness gate against the compose Postgres on podman.
# Same pattern as test-w3c; deferred LUBM-1/10/100 + cross-engine bench
# tracked in tests/perf/README.md.
test-lubm:
    PGRDF_RUNTIME={{RUN}} bash tests/perf/lubm-shape/run.sh

# pg_dump round-trip verification for `_pgrdf_graphs` (LLD v0.4 §3.1
# acceptance criterion). Boots a clean state, seeds two IRI bindings,
# pg_dumps, drops + restores, then re-queries to verify the mapping
# survived. Requires the compose stack to be up and idle.
test-pg-dump-roundtrip:
    PGRDF_RUNTIME={{RUN}} bash tests/regression/scripts/pg-dump-roundtrip.sh

# Prove that compose/extensions/ matches a fresh build from this source tree
# and that the running container has those exact bytes mounted.
test-artifact-parity:
    PGRDF_RUNTIME={{RUN}} PGRDF_BUILD_RUNTIME={{BUILD}} bash tests/regression/scripts/verify-installed-artifacts.sh

# Full local test bar: container-based pgrx tests + compose regression.
# Kept narrow for back-compat; `just test-everything` is the broader sweep.
test-all: test test-regression

# Every test layer that runs against the live compose Postgres
# (no pgrx framework needed — the compose runtime is the only dep).
test-conformance: test-regression test-w3c test-shacl-manifest test-lubm test-pg-dump-roundtrip

# Every test layer end-to-end: pgrx integration + every compose-based harness.
test-everything: test test-conformance

# Build the extension package locally (target/release/pgrdf-pgN/).
package:
    cargo pgrx package --pg-config "$(cargo pgrx info pg-config pg{{PG_MAJOR}})"

# Build extension inside the linux builder container; export artifacts to
# compose/extensions/ on the host. The compose stack (podman) bind-mounts
# them from there.
#
# Tags BOTH multi-stage targets:
#   - pgrdf-builder-rust:pgN (fat, ~5 GB, source of cargo + pgrx + postgres-N)
#   - pgrdf-builder:pgN      (slim, ~100 MB, holds /out artifacts)
build-ext:
    # DOCKER_BUILDKIT=1 enables the # syntax= and --mount=type=cache
    # directives in compose/builder.Containerfile. Without it the
    # build silently falls back to the legacy builder and the
    # cache-mounts are ignored — image layers bloat right back.
    DOCKER_BUILDKIT=1 {{BUILD}} build --target builder \
        -t pgrdf-builder-rust:pg{{PG_MAJOR}} \
        --build-arg PG_MAJOR={{PG_MAJOR}} \
        -f compose/builder.Containerfile .
    DOCKER_BUILDKIT=1 {{BUILD}} build \
        -t pgrdf-builder:pg{{PG_MAJOR}} \
        --build-arg PG_MAJOR={{PG_MAJOR}} \
        -f compose/builder.Containerfile .
    rm -rf compose/extensions/lib compose/extensions/share
    mkdir -p compose/extensions/lib compose/extensions/share/extension
    {{BUILD}} run --rm \
        -v "$PWD/compose/extensions:/export" \
        pgrdf-builder:pg{{PG_MAJOR}}

# Boot the compose stack on podman.
compose-up:
    cd compose && {{RUN}} compose up -d
compose-down:
    cd compose && {{RUN}} compose down
compose-logs:
    cd compose && {{RUN}} compose logs -f postgres

# psql shell against the compose Postgres.
psql:
    cd compose && {{RUN}} compose exec postgres psql -U pgrdf -d pgrdf

# End-to-end smoke: build (Colima), boot (podman), create extension, version.
smoke: build-ext compose-up
    sleep 5
    cd compose && {{RUN}} compose exec postgres psql -U pgrdf -d pgrdf -c "CREATE EXTENSION IF NOT EXISTS pgrdf;"
    cd compose && {{RUN}} compose exec postgres psql -U pgrdf -d pgrdf -c "SELECT pgrdf.version();"

# Fresh-compose smoke — wipe compose, rebuild the extension, boot from
# scratch, then run every compose-based test harness (regression +
# W3C-shape + LUBM-shape). Use this after touching anything in compose/,
# fixtures/, or the test SQL fixtures themselves.
smoke-cold: compose-down build-ext compose-up
    sleep 5
    cd compose && {{RUN}} compose exec postgres psql -U pgrdf -d pgrdf -c "CREATE EXTENSION IF NOT EXISTS pgrdf;"
    just test-artifact-parity
    just test-conformance
