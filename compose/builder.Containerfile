# syntax=docker/dockerfile:1.4
#
# Linux builder for pgRDF — produces glibc-bookworm artifacts that the
# postgres:17.4-bookworm container can load directly via bind mount.
#
# This Containerfile uses BuildKit cache mounts (the `# syntax=` line
# at the top + `--mount=type=cache` on RUN steps). Cargo's registry,
# git checkout cache, and the project's target/ directory all live in
# build-scoped cache volumes that persist across builds without
# accumulating in image layers. Net effect: rebuilding after a source
# change produces a ~200 MB "dangling" intermediate image instead of
# the ~5.7 GB we used to leave behind every time.
#
# Requires BuildKit. The Justfile sets DOCKER_BUILDKIT=1 already; for
# manual builds use `DOCKER_BUILDKIT=1 docker build ...`.
#
# Run via `just build-ext` (preferred) or:
#   DOCKER_BUILDKIT=1 docker build -t pgrdf-builder -f compose/builder.Containerfile .
#   docker run --rm -v "$PWD/compose/extensions:/out" pgrdf-builder

FROM docker.io/library/rust:1.91-bookworm AS builder

ARG PG_MAJOR=17
ARG PGRX_VERSION=0.16

# Postgres dev headers + full server (initdb for pgrx tests) + sudo
# for the pgrx-tests RUNAS path. apt cache lives in a BuildKit cache
# mount so repeated builds don't redo the index update + downloads.
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    rm -f /etc/apt/apt.conf.d/docker-clean && \
    apt-get update && apt-get install -y --no-install-recommends \
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
    && echo 'postgres ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/postgres-nopasswd

ENV PGRX_HOME=/opt/pgrx

# cargo-pgrx install + pgrx init. Both use the cargo cache mounts so
# the downloaded crate sources and built dep state stay out of the
# image layer.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo install cargo-pgrx --locked --version "^${PGRX_VERSION}"

RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    cargo pgrx init --pg${PG_MAJOR} "$(which pg_config)" \
    && chown -R postgres:postgres /opt/pgrx

WORKDIR /work
COPY . .

# Package the extension. target/ is a cache mount so the multi-GB
# cargo dep build state lives in a build-scoped cache volume, not
# the image layer. The artifacts we actually need land in /artifacts
# which is a tiny dir in the image (~700 KB).
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/work/target,sharing=locked \
    cargo pgrx package --pg-config "$(which pg_config)" \
    && mkdir -p /artifacts/lib /artifacts/share/extension \
    && cp /work/target/release/pgrdf-pg${PG_MAJOR}/usr/lib/postgresql/${PG_MAJOR}/lib/pgrdf.so \
          /artifacts/lib/pgrdf.so \
    && cp /work/target/release/pgrdf-pg${PG_MAJOR}/usr/share/postgresql/${PG_MAJOR}/extension/pgrdf.control \
          /artifacts/share/extension/ \
    && cp /work/target/release/pgrdf-pg${PG_MAJOR}/usr/share/postgresql/${PG_MAJOR}/extension/*.sql \
          /artifacts/share/extension/

# Stage 2: minimal export image. `podman run` copies /out into the
# host (compose/extensions/).
FROM debian:bookworm-slim AS export
COPY --from=builder /artifacts/lib/pgrdf.so /out/lib/pgrdf.so
COPY --from=builder /artifacts/share/extension/ /out/share/extension/
CMD ["sh", "-c", "cp -r /out/* /export/ && ls -laR /export"]
