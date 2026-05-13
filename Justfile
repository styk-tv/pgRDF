set shell := ["bash", "-uc"]

PG_MAJOR := env_var_or_default("PG_MAJOR", "17")

# List recipes.
default:
    @just --list

# Initialize pgrx state (one-time; downloads + compiles PG sources locally).
pgrx-init:
    cargo pgrx init --pg{{PG_MAJOR}} download

# Fast dev loop: pgrx-managed Postgres, hot-reload of the extension.
dev:
    cargo pgrx run pg{{PG_MAJOR}}

# Format + lint.
fmt:
    cargo fmt --all
clippy:
    cargo clippy --no-default-features --features pg{{PG_MAJOR}} -- -D warnings

# Unit + pgrx integration tests against the chosen PG major.
test:
    cargo pgrx test pg{{PG_MAJOR}}

# Build the extension package into target/release/pgrdf-pg{{PG_MAJOR}}/.
package:
    cargo pgrx package --pg-config "$(cargo pgrx info pg-config pg{{PG_MAJOR}})"

# Build extension inside a linux builder container; export artifacts to compose/extensions/.
build-ext:
    podman build -t pgrdf-builder:pg{{PG_MAJOR}} \
        --build-arg PG_MAJOR={{PG_MAJOR}} \
        -f compose/builder.Containerfile .
    rm -rf compose/extensions/lib compose/extensions/share
    mkdir -p compose/extensions/lib compose/extensions/share/extension
    podman run --rm \
        -v "$PWD/compose/extensions:/export:Z" \
        pgrdf-builder:pg{{PG_MAJOR}}

# Boot the compose stack.
compose-up:
    cd compose && podman compose up -d
compose-down:
    cd compose && podman compose down
compose-logs:
    cd compose && podman compose logs -f postgres

# psql shell against the compose Postgres.
psql:
    cd compose && podman compose exec postgres psql -U pgrdf -d pgrdf

# End-to-end smoke: build, boot, create extension, check version.
smoke: build-ext compose-up
    sleep 5
    cd compose && podman compose exec postgres psql -U pgrdf -d pgrdf -c "CREATE EXTENSION IF NOT EXISTS pgrdf;"
    cd compose && podman compose exec postgres psql -U pgrdf -d pgrdf -c "SELECT pgrdf.version();"
