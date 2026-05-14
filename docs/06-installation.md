# 06 — Installation

There are three supported install patterns, in order of preference.

## 6.1 PG 18+ — GUC-based drop-in (forward path, INSTALL spec §7)

A postgres:18+ container starts with two GUCs pointed at a directory
containing the pre-built extension:

```bash
postgres \
  -c shared_preload_libraries=pgrdf \
  -c extension_control_path=/pgrdf/share/extension:$system \
  -c dynamic_library_path=/pgrdf/lib:$libdir
```

No image rebuild, no entrypoint wrappers, no init scripts. This is
the spec-preferred forward path; pgRDF will switch to it once pgrx
ships a stable 0.17+/0.18 line that builds on current rustc (see
ERRATA E-006, E-007). Until then v0.3 pins to PG 17 + pgrx 0.16.1
and uses the per-file bind-mount pattern below.

## 6.2 PG 14–17 — file placement (INSTALL spec §4.3)

The GUCs above don't exist on PG ≤ 17, so the .so / .control / .sql
files must end up at canonical paths (`$libdir` and
`$sharedir/extension`). Two supported flavours:

**Local compose (what `compose/compose.yml` implements):** stock
`postgres:17.4-bookworm` boots with per-file bind mounts placing the
locally-built artifacts directly at `/usr/lib/postgresql/17/lib/` and
`/usr/share/postgresql/17/extension/`. No image rebuild, no
entrypoint wrapper, no init script — the spec-compliant local
incarnation of "drop-in extension files".

**K8s reference manifest (INSTALL spec §5):** a Kubernetes init
container fetches the release tarball into an `emptyDir`, and the
postgres container's command wrapper does:

```bash
cp -f /pgrdf/lib/*.so /usr/lib/postgresql/${PG_MAJOR}/lib/
cp -f /pgrdf/share/extension/* /usr/share/postgresql/${PG_MAJOR}/extension/
exec docker-entrypoint.sh postgres -c shared_preload_libraries=pgrdf
```

The corresponding entrypoint-wrapper container is out of scope for
v0.3 local dev; if you must reproduce an entrypoint-copy layout
locally, use `cargo pgrx run pg17` — pgrx handles file placement.

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
