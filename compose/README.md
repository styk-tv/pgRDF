# compose/ — development setup

A local PostgreSQL 18 server running a pgRDF you built from this
checkout. It's for working on pgRDF itself. To just try pgRDF, use the
prebuilt release instead ([install guide](../guide/01-install.md)).

It uses a stock `postgres:18-trixie` image. The extension files you
build are mounted into it file by file, so there's no custom image to
rebuild.

## Prerequisites

- Docker (or Podman) with Compose.
- [`just`](https://github.com/casey/just).
- About 5 GB of disk for the builder image, the first time.

## Boot

From the repository root:

```sh
cp compose/.env.example compose/.env      # optional: change credentials / port

just build-ext                            # build pgrdf.so + SQL in a Linux builder container
PGRDF_RUN_RUNTIME=docker just compose-up  # start PostgreSQL (the default runtime is podman)
PGRDF_RUN_RUNTIME=docker just psql        # psql as pgrdf/pgrdf on the pgrdf database
```

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version(), pgrdf.build_id();
```

`just build-ext` compiles inside a Linux container, so it also works on
macOS. The output lands in `compose/extensions/`.

A small check container runs before PostgreSQL starts. It verifies
that the mounted files belong together; the version in `pgrdf.control`
must match the `pgrdf--<version>.sql` file. If they don't, `compose up`
stops with an error rather than failing later at `CREATE EXTENSION`.

## What's where

```
compose/
├── compose.yml              # postgres + the file check
├── parity-check.sh          # the file check
├── builder.Containerfile    # Linux builder image
├── .env.example
└── extensions/              # build output (gitignored)
    ├── lib/pgrdf.so
    └── share/extension/{pgrdf.control, pgrdf--<version>.sql}
```

- `./fixtures` from the repository is mounted read-only at `/fixtures`
  in the container, so `pgrdf.load_turtle('/fixtures/…', …)` works for
  the bundled test ontologies.
- Data lives in the Docker volume `pgrdf-pg18-data`.
- The container is named `pgrdf-pgrdf-postgres`; override with
  `PGRDF_CONTAINER=…`.

When the version changes, update the `pgrdf--<version>.sql` mount line
in `compose.yml` to match.

## Useful targets

```sh
just smoke                  # build, boot, CREATE EXTENSION, print the version
just test-regression        # SQL regression suite against the running server
just test-conformance       # regression + W3C SPARQL / SHACL + LUBM checks
just test-artifact-parity   # prove the mounted files match a fresh build
just compose-logs           # follow the server log
just compose-down           # stop
```

## Reset

```sh
just compose-down
docker volume rm compose_pgrdf-pg18-data                    # discard the database
rm -rf compose/extensions/lib compose/extensions/share      # discard build output
```

(The volume name carries your Compose project prefix; `docker volume ls`
shows it.)
