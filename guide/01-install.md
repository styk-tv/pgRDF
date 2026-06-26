# 01 — Install

pgRDF runs inside any unmodified `postgres:14`..`postgres:17`
container. The exact path you take depends on where you're running
it: workstation, Kubernetes, or a managed Postgres service.

## Path A — Workstation (compose)

This is what the project's own `compose/` directory implements.
Stock `postgres:17.4-bookworm`, no image rebuild, the extension
files dropped in via per-file bind mounts.

Prerequisites: `podman` (or `docker`), `just`. On macOS, also
`colima` if you want the build to run somewhere other than the
podman VM.

```bash
git clone https://github.com/styk-tv/pgRDF.git
cd pgRDF
cp compose/.env.example compose/.env       # tweak creds if you want

just build-ext            # produces compose/extensions/{lib, share/extension/}
just compose-up           # podman compose up -d
just psql                 # opens a psql shell to the pgrdf database

pgrdf=# CREATE EXTENSION pgrdf;
pgrdf=# SELECT pgrdf.version();
        --  → 0.6.16
```

That's it. The extension is installed and you can move on to
[loading RDF](02-loading-rdf.md).

To bring the stack down:

```bash
just compose-down
```

PGDATA persists in `compose/pg-data/` between restarts. Wipe it
explicitly if you want a clean database.

## Path B — Production (Kubernetes init-container drop-in)

For real deployments, follow
[`specs/SPEC.pgRDF.INSTALL.v0.2.md`](../specs/SPEC.pgRDF.INSTALL.v0.2.md).
The shape:

1. An `initContainer` fetches the
   `pgrdf-<version>-pg<N>-glibc-<arch>.tar.gz` tarball from the
   GitHub Releases page into an `emptyDir` shared with the postgres
   container.
2. The postgres container's command wrapper copies the files into
   `$libdir` and `$sharedir/extension` before `exec`ing
   `docker-entrypoint.sh postgres -c shared_preload_libraries=pgrdf`.
3. Run `CREATE EXTENSION pgrdf;` against the cluster once
   (via a migration tool, `Job`, or your normal schema-management).

The reference manifest in INSTALL spec §5 is a complete `StatefulSet`
+ `ConfigMap` + `Service` you can copy.

## Path C — PGXN source install

If the host already has PostgreSQL development headers plus the Rust
toolchain, PGXN can build and install pgRDF from source directly:

```bash
pgxn install pgrdf --pg_config /path/to/pg_config
psql -d yourdb -c "CREATE EXTENSION pgrdf;"
```

This is the source-install path. For prerequisites and the direct
`make` fallback from an unpacked source archive, see the repo-root
[`INSTALL.md`](../INSTALL.md).

## Path D — Already-running Postgres (manual install)

If you have a Postgres server you control (RDS isn't this — see the
next section):

```bash
# Download the matching tarball from
# https://github.com/styk-tv/pgRDF/releases/latest
wget https://github.com/styk-tv/pgRDF/releases/download/v0.6.16/pgrdf-0.6.16-pg17-glibc-amd64.tar.gz

tar -xzf pgrdf-0.6.16-pg17-glibc-amd64.tar.gz
sudo cp lib/pgrdf.so                 $(pg_config --pkglibdir)/
sudo cp share/extension/pgrdf.control $(pg_config --sharedir)/extension/
sudo cp share/extension/pgrdf--*.sql  $(pg_config --sharedir)/extension/

# Then in your postgresql.conf:
#   shared_preload_libraries = 'pgrdf'
# Restart Postgres, then:
psql -c "CREATE EXTENSION pgrdf;"
```

## Managed Postgres caveats

Hosted services (RDS, Cloud SQL, Azure Database for Postgres) usually
do not let you install arbitrary extensions. pgRDF is not currently
in any managed-service extension catalogue. Options:

- Run Postgres yourself (on EKS / GKE / AKS) with the K8s manifest
  from Path B.
- Use Crunchy Data's CrunchyBridge or a similar managed Postgres that
  supports custom extensions via Trunk / pgxn.
- Ask your vendor — extension catalogues are growing.

## Verify the install

```sql
\dx pgrdf                                          -- extension is present
SELECT extversion FROM pg_extension                -- matches the tarball you fetched
 WHERE extname = 'pgrdf';
SHOW shared_preload_libraries;                     -- contains 'pgrdf'
SELECT pgrdf.version();                            -- → '0.6.16'
SELECT pgrdf.stats() -> 'shmem_ready';             -- → true (preload OK)
```

If any of these don't match, the most common causes are:

1. Tarball PG major doesn't match the running server. Symptom:
   `CREATE EXTENSION` errors with "could not load library ...
   undefined symbol". Fix: re-download the right `pg<N>` tarball.
2. `shared_preload_libraries` not set / Postgres not restarted.
   Symptom: `pgrdf.stats() -> 'shmem_ready'` returns `false`. The
   extension still works (CRUD, SPARQL, Turtle ingest), but the
   shmem dict cache from LLD §4.1 is disabled and every dictionary
   touch hits the table. Fix: add `pgrdf` to
   `shared_preload_libraries`, restart Postgres.
3. Container is alpine-based, not bookworm. Symptom:
   `not a dynamic executable`. Fix: switch to `postgres:<N>-bookworm`.

The full failure-mode table is in INSTALL spec §9.

## Next

[02-loading-rdf.md](02-loading-rdf.md) — how to get triples in.
