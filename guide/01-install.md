# 01 — Install

pgRDF is a PostgreSQL 18 extension: one shared library (`pgrdf.so`)
plus its SQL and control files. Installing it means putting those
files where PostgreSQL can find them, adding `pgrdf` to
`shared_preload_libraries`, restarting, and running
`CREATE EXTENSION pgrdf`.

Pick the route that fits:

| Route | Best for |
|---|---|
| [A. Docker, no build](#a-docker-in-two-minutes) | Trying it out |
| [B. Your own Docker image](#b-your-own-docker-image) | Repeatable local and CI setups |
| [C. An existing PostgreSQL server](#c-an-existing-postgresql-server) | VMs and bare metal |
| [D. A directory of its own](#d-keep-pgrdf-in-its-own-directory) | Kubernetes volumes, read-only images |
| [E. The signed OCI bundle](#e-the-signed-oci-bundle) | Supply-chain-verified deployments |
| [F. From source](#f-from-source) | Development, other platforms |

Requirements for every route: PostgreSQL 18 on Linux (x86-64 or
arm64) with glibc 2.39 or newer, which includes the official
`postgres:18` image, Debian 13, and Ubuntu 24.04 or later.

## Release files

Each release on the [releases page](https://github.com/styk-tv/pgRDF/releases)
has one archive per CPU architecture, plus checksums:

```
pgrdf-0.6.34-pg18-glibc-amd64.tar.gz
pgrdf-0.6.34-pg18-glibc-arm64.tar.gz
SHA256SUMS
```

Inside each archive:

```
pgrdf-0.6.34-pg18-glibc-amd64/
├── lib/pgrdf.so
└── share/extension/
    ├── pgrdf.control
    ├── pgrdf--0.6.34.sql
    └── pgrdf--<old>--<new>.sql     (upgrade scripts)
```

[LATEST.md](../LATEST.md) always names the current version.

## A. Docker in two minutes

No image build: copy the files into a stock `postgres:18` container
before its first start.

```sh
VER=0.6.34
ARCH=$(uname -m | sed -e 's/x86_64/amd64/' -e 's/aarch64/arm64/')
curl -fsSL https://github.com/styk-tv/pgRDF/releases/download/v$VER/pgrdf-$VER-pg18-glibc-$ARCH.tar.gz | tar -xz

docker create --name pgrdf -e POSTGRES_PASSWORD=pgrdf -p 5432:5432 \
  postgres:18 -c shared_preload_libraries=pgrdf
docker cp pgrdf-$VER-pg18-glibc-$ARCH/lib/pgrdf.so       pgrdf:/usr/lib/postgresql/18/lib/
docker cp pgrdf-$VER-pg18-glibc-$ARCH/share/extension/.  pgrdf:/usr/share/postgresql/18/extension/
docker start pgrdf

until docker exec pgrdf pg_isready -h localhost -U postgres -q; do sleep 1; done
docker exec -it pgrdf psql -U postgres -c 'CREATE EXTENSION pgrdf' -c 'SELECT pgrdf.version()'
```

Then `docker exec -it pgrdf psql -U postgres` opens a shell. Continue
with the [tour](tour.md). If port 5432 is taken, change `-p 5432:5432`
to, say, `-p 5433:5432`.

## B. Your own Docker image

```dockerfile
FROM postgres:18
ARG PGRDF_VERSION=0.6.34
ARG TARGETARCH
ADD https://github.com/styk-tv/pgRDF/releases/download/v${PGRDF_VERSION}/pgrdf-${PGRDF_VERSION}-pg18-glibc-${TARGETARCH}.tar.gz /tmp/pgrdf.tar.gz
RUN tar -xzf /tmp/pgrdf.tar.gz -C /tmp \
 && cp /tmp/pgrdf-*/lib/pgrdf.so /usr/lib/postgresql/18/lib/ \
 && cp /tmp/pgrdf-*/share/extension/pgrdf* /usr/share/postgresql/18/extension/ \
 && rm -rf /tmp/pgrdf*
CMD ["postgres", "-c", "shared_preload_libraries=pgrdf"]
```

```sh
docker build -t my-postgres-pgrdf .
docker run -d --name pgrdf -e POSTGRES_PASSWORD=pgrdf -p 5432:5432 my-postgres-pgrdf
```

`TARGETARCH` is filled in by Docker, so the same file builds on x86-64
and arm64. To create the extension automatically on first start, add a
script to `/docker-entrypoint-initdb.d/` that runs
`CREATE EXTENSION pgrdf;`.

## C. An existing PostgreSQL server

```sh
VER=0.6.34
ARCH=amd64                       # or arm64
curl -fLO https://github.com/styk-tv/pgRDF/releases/download/v$VER/pgrdf-$VER-pg18-glibc-$ARCH.tar.gz
curl -fLO https://github.com/styk-tv/pgRDF/releases/download/v$VER/SHA256SUMS
sha256sum -c SHA256SUMS --ignore-missing       # macOS: shasum -a 256 -c SHA256SUMS --ignore-missing

tar -xzf pgrdf-$VER-pg18-glibc-$ARCH.tar.gz
cd pgrdf-$VER-pg18-glibc-$ARCH
sudo cp lib/pgrdf.so              "$(pg_config --pkglibdir)/"
sudo cp share/extension/pgrdf*    "$(pg_config --sharedir)/extension/"
```

`pg_config` must be the one belonging to your PostgreSQL 18
installation. On Debian and Ubuntu the paths are
`/usr/lib/postgresql/18/lib` and `/usr/share/postgresql/18/extension`.

Add pgRDF to `postgresql.conf`, keeping any libraries already listed:

```ini
shared_preload_libraries = 'pgrdf'
```

**Restart** PostgreSQL (a reload is not enough), then:

```sql
CREATE EXTENSION pgrdf;
```

## D. Keep pgRDF in its own directory

PostgreSQL 18 can load extensions from extra directories, so the files
don't need to go into the system paths. Unpack the archive anywhere,
for example `/opt/pgrdf`, a mounted volume, or an init-container
`emptyDir` in Kubernetes, and point PostgreSQL at it:

```sh
postgres -c shared_preload_libraries=pgrdf \
         -c 'extension_control_path=/opt/pgrdf/share:$system' \
         -c 'dynamic_library_path=/opt/pgrdf/lib:$libdir'
```

(`/opt/pgrdf` is the unpacked archive directory: it contains `lib/` and
`share/extension/`.) The same three settings work in
`postgresql.conf`.

## E. The signed OCI bundle

Every release is also published to GitHub Container Registry as an
OCI artifact with a verifiable SLSA build-provenance attestation. The
artifact holds the same files as the archives.

```sh
# check where it came from (GitHub CLI)
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.6.34 --repo styk-tv/pgRDF

# fetch the files for your architecture (oras CLI)
oras pull ghcr.io/styk-tv/pgrdf-bundle:0.6.34-pg18-amd64      # or -arm64
```

A successful verification means the artifact was built by this
repository's release workflow from the tagged commit, and recorded in
the Sigstore transparency log. Digests for every architecture are in
[LATEST.md](../LATEST.md).
## F. From source

Building needs Rust and `cargo-pgrx`. See [INSTALL.md](../INSTALL.md)
for building against your own PostgreSQL, and
[compose/README.md](../compose/README.md) for the Docker-based
development setup.

## Check the installation

```sql
SELECT pgrdf.version(), pgrdf.build_id(),
       (SELECT extversion FROM pg_extension WHERE extname = 'pgrdf');
--  version | build_id | extversion
-- ---------+----------+------------
--  0.6.34  | v0.6.34  | 0.6.34

SELECT pgrdf.stats() -> 'shmem_ready';   -- true when shared_preload_libraries is set
```

## Using pgRDF from an application role

`CREATE EXTENSION` needs a superuser, but your application doesn't
have to be one. pgRDF keeps its data in tables in the `pgrdf` schema,
and its functions run with the caller's privileges. To let a role
query graphs and write into existing ones:

```sql
GRANT USAGE ON SCHEMA pgrdf TO app;
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pgrdf TO app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pgrdf TO app;
```

Creating a graph (`add_graph`) also creates a table partition. Do that
as the extension owner, or additionally grant `CREATE` on schema
`pgrdf`. For a read-only role, grant only `USAGE` and `SELECT`.

The file loaders (`load_turtle` and its variants) read files from the
database server's filesystem. Give write access only to roles you
trust with that. An application sending data over the connection needs
only `parse_turtle`, `parse_trig` and `parse_nquads`.

## Upgrading

1. Install the new release's files (any route above). The archive
   includes the upgrade scripts.
2. Restart PostgreSQL so the new library is loaded.
3. In every database that uses pgRDF:

   ```sql
   ALTER EXTENSION pgrdf UPDATE;
   ```

4. Check that `version()`, `build_id()` and `extversion` agree again.

Upgrade scripts cover versions 0.6.22 and later. Your graphs stay in
place.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `stats() -> 'shmem_ready'` is `false` | `pgrdf` is not in `shared_preload_libraries`, or the server wasn't restarted. Most functions still work, but the shared caches and the parallel staged loader don't. Add it and restart. |
| Server won't start: `could not access file "pgrdf"` | The library isn't where PostgreSQL looks. Check the copy step, or `dynamic_library_path` (route D). |
| `could not load library … GLIBC_2.39 not found` | The system's glibc is too old. Use Debian 13, Ubuntu 24.04+, or the `postgres:18` image. |
| `CREATE EXTENSION` can't find `pgrdf.control` | The `share/extension` files weren't copied, or were copied for a different PostgreSQL installation than the running one. |
| `version()` and `extversion` differ | New files installed but the database not updated. Run `ALTER EXTENSION pgrdf UPDATE`. |
| Alpine-based image | Not supported (musl). Use `postgres:18` (Debian). |

## Next

[02 — Loading RDF](02-loading-rdf.md), or the [ten-minute tour](tour.md).
