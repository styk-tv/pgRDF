# 06 — Installation

There are three supported install patterns, in order of preference.

## 6.1 PG 18+ — GUC-based drop-in (preferred, INSTALL spec §7)

The postgres:18+ container starts with two GUCs pointed at a directory
containing the pre-built extension:

```bash
postgres \
  -c shared_preload_libraries=pgrdf \
  -c extension_control_path=/pgrdf/share/extension:$system \
  -c dynamic_library_path=/pgrdf/lib:$libdir
```

No image rebuild, no entrypoint wrappers, no init scripts. **This is
what `compose/podman-compose.yml` implements locally.**

## 6.2 PG 14–17 — entrypoint-copy (INSTALL spec §4.3)

The GUCs above don't exist on PG ≤ 17, so the .so / .control / .sql
files must end up at canonical paths (`$libdir` and
`$sharedir/extension`). The supported pattern: a Kubernetes init
container fetches the release tarball into an `emptyDir`, and the
postgres container's command wrapper does:

```bash
cp -f /pgrdf/lib/*.so /usr/lib/postgresql/${PG_MAJOR}/lib/
cp -f /pgrdf/share/extension/* /usr/share/postgresql/${PG_MAJOR}/extension/
exec docker-entrypoint.sh postgres -c shared_preload_libraries=pgrdf
```

This is the K8s reference manifest in INSTALL spec §5. We do not
support it locally because the corresponding entrypoint wrapper was
out of scope per project direction; if you must test on PG 17 locally,
use `cargo pgrx run pg17` instead — pgrx handles file placement.

## 6.3 Container with the extension baked in

Explicitly rejected by INSTALL §10. Couples our release cadence to the
postgres image rebuild cycle.

## CREATE EXTENSION

Once the files are in place and Postgres is running:

```sql
CREATE EXTENSION pgrdf;
SELECT pgrdf.version();   -- → '0.2.0'
```

The extension declares `superuser = true` because we manipulate
`shared_preload_libraries` and (in later phases) create background
workers.

## Verifying conformance (INSTALL spec §12)

After install, run the conformance checklist:

```bash
psql -c "SELECT extversion FROM pg_extension WHERE extname='pgrdf';"   # ≡ Cargo.toml version
psql -c "\dx pgrdf"                                                     # extension present
psql -c "SHOW shared_preload_libraries;"                                # contains pgrdf
```

All three are wired into `tests/regression/sql/00-smoke.sql` so this
is gated in CI.
