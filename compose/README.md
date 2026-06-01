# compose/ — local-dev runtime for pgRDF

Stock `postgres:17.4-bookworm` image, no image rebuild, no entrypoint
wrapper. The locally-built extension files are placed at the
canonical Postgres paths via **per-file bind mounts**:

    ./extensions/lib/pgrdf.so                       → /usr/lib/postgresql/17/lib/pgrdf.so
    ./extensions/share/extension/pgrdf.control      → /usr/share/postgresql/17/extension/pgrdf.control
    ./extensions/share/extension/pgrdf--<ver>.sql   → /usr/share/postgresql/17/extension/pgrdf--<ver>.sql

A one-shot **`pgrdf-parity` init container** runs before postgres
starts (TG-3 v2). It hashes the mounted files and verifies internal
consistency — `.control`'s `default_version` matches the
`pgrdf--<ver>.sql` filename; neither `.so` nor `.sql` is empty. If
anything mismatches (the realistic case: a release cut bumped
`pgrdf.control`'s `default_version` but `compose.yml` still mounts
the previous SQL file), the parity check exits non-zero and postgres
never starts — `docker compose up` fails at startup rather than
later at `CREATE EXTENSION` time with a confusing
"`pgrdf--<old>.sql` not found" error.

## Layout

    compose/
    ├── compose.yml                 # services definition (postgres + pgrdf-parity init)
    ├── parity-check.sh             # TG-3 v2 compose-startup gate (runs inside pgrdf-parity)
    ├── builder.Containerfile       # linux/glibc-bookworm builder
    ├── .env.example
    ├── extensions/                 # built artifacts (gitignored, populated by `just build-ext`)
    │   ├── lib/pgrdf.so
    │   └── share/extension/{pgrdf.control, pgrdf--<ver>.sql}
    └── pg-data/                    # PGDATA bind mount (gitignored)

## One-time setup

    cp compose/.env.example compose/.env
    # edit .env if you want non-default creds

## Boot sequence

From the repo root:

    just build-ext        # builds the linux .so + .control + .sql into compose/extensions/
    just compose-up       # boots Postgres
    just psql             # connects as pgrdf/pgrdf to the pgrdf database
    pgrdf=# CREATE EXTENSION pgrdf;
    pgrdf=# SELECT pgrdf.version();    -- → "0.5.32"
    just test-artifact-parity          # prove mounted bytes match a fresh build

By default the compose container is named `pgrdf-pgrdf-postgres`.
Override it with `PGRDF_CONTAINER=...` if you need a workstation-local
name.

## Why PG 17 (not 18)

The forward path in SPEC.pgRDF.INSTALL.v0.2 §7 is to use PG 18+'s
`extension_control_path` GUC, which lets us point Postgres at a
side directory without touching `$libdir`/`$sharedir/extension`. That
is the long-term shape this compose will adopt.

Today it pins to PG 17.4-bookworm because pgrx 0.17 and 0.18 (the
versions that add PG 18 support) fail to build on current Rust
toolchains (stable 1.95, nightly 1.97) — they reference unstable
APIs without enabling the corresponding feature flags. See
[`specs/ERRATA.v0.2.md`](../specs/ERRATA.v0.2.md) item E-006. Until
pgrx publishes a fixed 0.17.x or 0.18.x, we pin pgrx 0.16 and PG 17.

## Why per-file bind mounts (no init script, no entrypoint wrapper)

On PG 17 there's no `extension_control_path` GUC, so the files must
land at canonical Postgres paths. The three supported options per
INSTALL spec are:

1. Custom-built image with pgRDF baked in — rejected by §10.
2. Init container + entrypoint wrapper that copies files at boot —
   §4.3, used in K8s manifests. Out of scope for this local compose
   per project direction.
3. **Per-file bind mounts targeting `$libdir` and
   `$sharedir/extension` directly.**

Option 3 is what this compose does. It has the same observable
end-state as option 2 (files at canonical paths, no image rebuild,
no source compile at runtime), with one fewer moving part. The
files are produced on the host by `just build-ext` (a Linux
builder container) before `compose up`.

## Why a Linux builder container (not native cargo)

We're cross-platform (macOS host, Linux Postgres container). Native
`cargo pgrx run` works on macOS for fast iteration but produces a
`.dylib`, which the Linux postgres container cannot load. The
builder container produces a glibc-bookworm `.so` matching the
target environment exactly.

## Resetting state

    just compose-down
    rm -rf compose/pg-data/*                            # discard PGDATA
    rm -rf compose/extensions/lib compose/extensions/share  # discard built artifacts
