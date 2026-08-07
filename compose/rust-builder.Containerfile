# syntax=docker/dockerfile:1.4
#
# ck-rust-builder — the shared Rust builder, per SPEC.RUST-BUILDER.CK.v3.11.
#
# One image for every Rust artifact that loads into pgck-localhost.
# Unlike compose/builder.Containerfile (pgRDF-only, source COPYed in at
# BUILD time), this image contains NO source and NO cargo-pgrx: the
# repo arrives as a read-only mount at RUN time and the entrypoint
# resolves cargo-pgrx from that crate's own pin.
#
# Why cargo-pgrx is not baked: it must EXACTLY equal the crate's pgrx
# pin. pgRDF is on 0.19.2 and pgCK on 0.16.1, so a baked version locks
# the image to one crate. Resolving per build lets both share it today.
#
#   docker build -t ck-rust-builder:trixie-pg18 -f compose/rust-builder.Containerfile .
#
# See the entrypoint for the run-time contract.

FROM docker.io/library/rust:1.97.1-trixie

ARG PG_MAJOR=18
ENV PG_MAJOR=${PG_MAJOR}

# Postgres dev headers + full server (initdb, for crates that run
# pgrx tests) and sudo for the pgrx-tests RUNAS path. Same pgdg setup
# as the pgRDF builder — the userland here, not the host, determines
# the .so's glibc floor, and trixie is what ck-allinone runs.
RUN rm -f /etc/apt/apt.conf.d/docker-clean && \
    apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl gnupg lsb-release git jq \
        build-essential pkg-config libssl-dev libclang-dev \
    && curl -fsSL https://www.postgresql.org/media/keys/ACCC4CF8.asc \
        | gpg --dearmor -o /usr/share/keyrings/postgresql-archive-keyring.gpg \
    && echo "deb [signed-by=/usr/share/keyrings/postgresql-archive-keyring.gpg] http://apt.postgresql.org/pub/repos/apt $(lsb_release -cs)-pgdg main" \
        > /etc/apt/sources.list.d/pgdg.list \
    && apt-get update && apt-get install -y --no-install-recommends \
        postgresql-server-dev-${PG_MAJOR} \
        postgresql-${PG_MAJOR} \
        sudo \
    && echo 'postgres ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/postgres-nopasswd \
    && rm -rf /var/lib/apt/lists/*

# PGRX_HOME and the version-scoped cargo-pgrx root are both cache
# volumes at run time (see the spec §4), so multiple pgrx pins coexist
# and a cold install happens once per fleet rather than once per repo.
ENV PGRX_HOME=/pgrx/home \
    CARGO_TARGET_DIR=/target

COPY compose/rust-builder-entrypoint.sh /usr/local/bin/ck-build
RUN chmod +x /usr/local/bin/ck-build

WORKDIR /src
ENTRYPOINT ["/usr/local/bin/ck-build"]
