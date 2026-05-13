# Linux builder for pgRDF — produces glibc-bookworm artifacts that the
# postgres:17.4-bookworm container can load directly via bind mount.
#
# Run via `just build-ext` (preferred) or:
#   podman build -t pgrdf-builder -f compose/builder.Containerfile .
#   podman run --rm -v "$PWD/compose/extensions:/out:Z" pgrdf-builder
#
# Output:
#   /out/lib/pgrdf.so
#   /out/share/extension/pgrdf.control
#   /out/share/extension/pgrdf--<version>.sql

FROM docker.io/library/rust:1.91-bookworm AS builder

ARG PG_MAJOR=17
ARG PGRX_VERSION=0.16

# Postgres headers + build deps for both the builder and pgrx.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl gnupg lsb-release \
        build-essential pkg-config libssl-dev libclang-dev \
    && curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc \
        | gpg --dearmor -o /usr/share/keyrings/postgresql-archive-keyring.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/postgresql-archive-keyring.gpg] http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" \
        > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update && apt-get install -y --no-install-recommends \
        postgresql-server-dev-${PG_MAJOR} \
        postgresql-${PG_MAJOR} \
        sudo \
    && rm -rf /var/lib/apt/lists/* \
    && echo 'postgres ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/postgres-nopasswd

# PGRX_HOME goes to /opt/pgrx so both root (build orchestration) and
# postgres (initdb / managed PG via CARGO_PGRX_TEST_RUNAS) can read/write.
ENV PGRX_HOME=/opt/pgrx

RUN cargo install cargo-pgrx --locked --version "^${PGRX_VERSION}"
RUN cargo pgrx init --pg${PG_MAJOR} "$(which pg_config)" \
    && chown -R postgres:postgres /opt/pgrx

WORKDIR /work
COPY . .

RUN cargo pgrx package --pg-config "$(which pg_config)"

# Stage 2: minimal export image. `podman run` copies /out into the host.
FROM debian:bookworm-slim AS export
ARG PG_MAJOR=17
COPY --from=builder /work/target/release/pgrdf-pg${PG_MAJOR}/usr/lib/postgresql/${PG_MAJOR}/lib/pgrdf.so /out/lib/pgrdf.so
COPY --from=builder /work/target/release/pgrdf-pg${PG_MAJOR}/usr/share/postgresql/${PG_MAJOR}/extension/ /out/share/extension/
CMD ["sh", "-c", "cp -r /out/* /export/ && ls -laR /export"]
