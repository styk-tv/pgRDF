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
pgrdf-0.6.37-pg18-glibc-amd64.tar.gz
pgrdf-0.6.37-pg18-glibc-arm64.tar.gz
SHA256SUMS
```

Inside each archive:

```
pgrdf-0.6.37-pg18-glibc-amd64/
├── lib/pgrdf.so
└── share/extension/
    ├── pgrdf.control
    ├── pgrdf--0.6.37.sql
    └── pgrdf--<old>--<new>.sql     (upgrade scripts)
```

[LATEST.md](../LATEST.md) always names the current version.

## A. Docker in two minutes

No image build: copy the files into a stock `postgres:18` container
before its first start.

```sh
VER=0.6.41
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
ARG PGRDF_VERSION=0.6.37
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
VER=0.6.41
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
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.6.37 --repo styk-tv/pgRDF

# fetch the files for your architecture (oras CLI)
oras pull ghcr.io/styk-tv/pgrdf-bundle:0.6.37-pg18-amd64      # or -arm64
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
--  0.6.37  | v0.6.37  | 0.6.37

SELECT pgrdf.stats() -> 'shmem_ready';   -- true when shared_preload_libraries is set
```

`stats()` is for superusers and the role that installed the extension
unless you [grant it](#functions-that-are-not-public).

`shmem_ready` is `false` when pgRDF is installed but not preloaded.
Loading, querying, reasoning and validation still work then. What does
not: the staged loader refuses to start, `graph_activity()` returns no
rows, and the term cache is not shared between connections.

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

Those grants are also enough to create, clear and drop graphs:
`add_graph` needs `SELECT` and `INSERT` on `pgrdf._pgrdf_quads` and
`pgrdf._pgrdf_graphs`, `drop_graph` needs `SELECT` and `DELETE` on them,
and `clear_graph` needs `SELECT` and `DELETE` on `pgrdf._pgrdf_quads`.
The steps PostgreSQL reserves for the table owner (creating, emptying or
removing a graph's storage partition) run as the owner inside those
three functions and nowhere else. A role without the grants gets `42501`
with the exact `GRANT` in the hint; `pgrdf.can_create_graphs()`,
`pgrdf.can_clear_graphs()` and `pgrdf.can_drop_graphs()` answer the
question up front. For a read-only role, grant only `USAGE` and
`SELECT`.

The same grants cover the [act record](05-graphs.md#the-act-record):
any role with `USAGE` on the schema can read it through
`pgrdf.graph_acts()`, and no grant lets a role write it. pgRDF writes
the rows itself; a direct `INSERT`, `UPDATE`, `DELETE` or `TRUNCATE` by
a role that does not hold the table owner's privileges is refused with
`42501`, whatever table grants the role has.

### Functions that are not PUBLIC

Three functions act on state that every database on the server shares,
so since 0.6.41 only superusers and the role that installed the
extension may call them:

| Function | Acts on |
|---|---|
| `pgrdf.stats()` | server-wide counters |
| `pgrdf.shmem_reset()` | the shared term cache |
| `pgrdf.shmem_cache_prewarm(bigint)` | the shared term cache |

Any other role gets `42501: permission denied for function stats`. Open
`stats()` to a monitoring role with a plain grant:

```sql
GRANT EXECUTE ON FUNCTION pgrdf.stats() TO monitor;
```

`pgrdf.last_call_stats()` reports only the caller's own last call and
stays PUBLIC.

### Giving pgRDF's storage to another role

Some deployments make a dedicated role own pgRDF's tables with
`ALTER TABLE … OWNER TO`. The steps above that run as the owner then run
as that role, the owner of `pgrdf._pgrdf_quads` (the *storage owner*),
and so does writing the act record. That role needs the storage
relations, and two of them are sequences that are not tied to a table,
so `ALTER TABLE` does not move them: the graph-id sequence
`pgrdf._pgrdf_graph_id_seq` and the act-id sequence
`pgrdf._pgrdf_graph_act_id_seq`. It also needs `USAGE` and `CREATE` on
the `pgrdf` schema, where it creates partitions.

`pgrdf.ownership_drift()` lists each object the storage owner does not
own, whether that stops pgRDF, and the statement that fixes it. After
every table was handed to a role `store` that was also granted `USAGE`
and `CREATE` on the schema:

```sql
SELECT relname, kind, owner, storage_owner, blocking, cure FROM pgrdf.ownership_drift();
--          relname         |   kind   |  owner   | storage_owner | blocking |                            cure
-- -------------------------+----------+----------+---------------+----------+-------------------------------------------------------------
--  _pgrdf_graph_act_id_seq | sequence | postgres | store         | t        | ALTER SEQUENCE pgrdf._pgrdf_graph_act_id_seq OWNER TO store
--  _pgrdf_graph_id_seq     | sequence | postgres | store         | t        | ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO store
--  pgrdf                   | schema   | postgres | store         | f        | ALTER SCHEMA pgrdf OWNER TO store
```

Run every `cure` whose row says `blocking`. A row that is not blocking
is listed for completeness: here the schema still belongs to
`postgres`, and `store` holds what it needs on it. No rows means
ownership agrees. The rule behind `blocking` is in
[errors and diagnostics](08-errors-and-diagnostics.md#ownership_drift).

Run it after handing the storage over and after every
`ALTER EXTENSION pgrdf UPDATE`. If creating a graph refuses with `55000`,
its hint is the statement to run.

The file loaders (`load_turtle` and its variants) read files from the
database server's filesystem, so they require the built-in role
`pg_read_server_files`, as `COPY … FROM 'file'` does:

```sql
GRANT pg_read_server_files TO app;   -- only for roles you trust with the server's files
```

The staged loader additionally needs the privileges of the owner of the
pgRDF tables. An application sending data over the connection needs
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

Between steps 2 and 3 the library is newer than the SQL objects in the
database. Queries and loads keep working. Since 0.6.41, `export_graph`,
`graph_manifest`, `graph_digest` and `structural_digest` refuse in that
window, and name the cure:

```
ERROR:  55000: pgrdf: exports and digests need pgrdf._nt_line, which is missing: the installed extension SQL is older than the loaded library
HINT:  ALTER EXTENSION pgrdf UPDATE
```

Graph-level acts made in that window are not written to the act
record, which the update creates.

### Upgrading to 0.6.41

`ALTER EXTENSION pgrdf UPDATE` rewrites no data. It builds one index
per graph (about the cost of the index it replaces), adds the graph
owner columns and the act record, and takes `stats()`, `shmem_reset()`
and `shmem_cache_prewarm()` away from PUBLIC.

- **Read the WARNINGs.** From 0.6.41 a graph is a set of triples. A
  graph that already stores some triple more than once is left exactly
  as it is and named:

  ```
  WARNING:  pgrdf: _pgrdf_quads_g2 holds repeated triples; it is left as it is, without the unique index. See pgrdf.duplicate_report(); pgrdf.dedup_graph(graph_id) cures it.
  ```

  The number after `_g` is the graph id. Afterwards run
  `SELECT * FROM pgrdf.duplicate_report();` and follow
  [repeated triples from earlier versions](05-graphs.md#repeated-triples-from-earlier-versions).
  No rows means there is nothing to do.
- **A digest you recorded over a graph that held repeats will not
  match** what 0.6.41 computes for it, because 0.6.41 digests the set
  of triples. Digests of every other graph are byte-identical to
  0.6.40. See
  [identity and export](07-identity-and-export.md#digests-describe-the-set-of-triples).
- **Graphs that exist before the update have no recorded owner** and no
  earlier acts: `owner` and `created_at` read `NULL` in
  `graph_inventory()`. The act record starts with the update.
- **Callers of `stats()`, `shmem_reset()` or `shmem_cache_prewarm()`**
  that are neither a superuser nor the role that installed the
  extension need `EXECUTE` granted first, or the call fails `42501`.
  See [above](#functions-that-are-not-public).
- **The `pgrdf.` setting prefix is reserved.** A client that sets a
  `pgrdf.*` name pgRDF does not define now gets an error instead of a
  silent placeholder:

  ```
  ERROR:  42602: invalid configuration parameter name "pgrdf.no_such_setting"
  DETAIL:  "pgrdf" is a reserved prefix.
  ```

## Dump and restore

`pg_dump` and `pg_restore` carry a pgRDF database whole since 0.6.41:
every graph's registry row (IRI, owner, lock state, source digest), the
triples of every graph including the default graph, the dictionary, the
act record, and the id allocators, so a restored database hands out
graph, term and act ids above the ones it already holds. A locked graph
is locked after the restore. A database created under an earlier
version dumps complete once it has been updated to 0.6.41. A dump taken
before that update lacks what earlier versions left out: the graph
registry, the default graph's triples and the dictionary's id
allocator (#152).

The target server needs the same pgRDF files installed; the dump
contains `CREATE EXTENSION pgrdf`, not the library.

One thing is not clean yet (#166): the restore prints two errors per
graph, for indexes the partition already received from its parent
table. With one graph in the database:

```
pg_restore: error: could not execute query: ERROR:  relation "_pgrdf_quads_g1_object_id_subject_id_predicate_id_is_inferr_idx" already exists
Command was: CREATE INDEX _pgrdf_quads_g1_object_id_subject_id_predicate_id_is_inferr_idx ON pgrdf._pgrdf_quads_g1 USING btree (object_id, subject_id, predicate_id) INCLUDE (is_inferred);

pg_restore: error: could not execute query: ERROR:  relation "_pgrdf_quads_g1_predicate_id_object_id_subject_id_is_inferr_idx" already exists
Command was: CREATE INDEX _pgrdf_quads_g1_predicate_id_object_id_subject_id_is_inferr_idx ON pgrdf._pgrdf_quads_g1 USING btree (predicate_id, object_id, subject_id) INCLUDE (is_inferred);

pg_restore: warning: errors ignored on restore: 2
```

The restored data is complete, and `pg_restore` exits with status 1
because it counted errors. Two options turn these errors into a failed
restore: with `--single-transaction` the first one rolls the whole
restore back, and with `--exit-on-error` the restore stops there and
leaves the rest of the dump unrestored. Run it without those options
and check that the errors are only these.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `stats() -> 'shmem_ready'` is `false` | `pgrdf` is not in `shared_preload_libraries`, or the server wasn't restarted. Most functions still work, but the shared caches, the parallel staged loader and `graph_activity()` don't. Add it and restart. |
| `permission denied for function stats` | `stats()` is not PUBLIC since 0.6.41. See [functions that are not PUBLIC](#functions-that-are-not-public). |
| `export_graph` or a digest refuses with `55000` after new files were installed | The database has not been updated. Run `ALTER EXTENSION pgrdf UPDATE`. |
| Server won't start: `could not access file "pgrdf"` | The library isn't where PostgreSQL looks. Check the copy step, or `dynamic_library_path` (route D). |
| `could not load library … GLIBC_2.39 not found` | The system's glibc is too old. Use Debian 13, Ubuntu 24.04+, or the `postgres:18` image. |
| `CREATE EXTENSION` can't find `pgrdf.control` | The `share/extension` files weren't copied, or were copied for a different PostgreSQL installation than the running one. |
| `version()` and `extversion` differ | New files installed but the database not updated. Run `ALTER EXTENSION pgrdf UPDATE`. |
| Alpine-based image | Not supported (musl). Use `postgres:18` (Debian). |

## Next

[02 — Loading RDF](02-loading-rdf.md), or the [ten-minute tour](tour.md).
