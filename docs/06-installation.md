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
ERRATA E-006, E-007). Until then the current v0.5.x line pins to
PG 17 + pgrx 0.16.1
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
SELECT pgrdf.version();   -- → '0.5.25'
```

The extension declares `superuser = true` because we manipulate
`shared_preload_libraries` and (in later phases) create background
workers.

## Upgrade between v0.x versions

**pgRDF v0.x reserves the right to break schema and UDF signatures
between minor releases.** There is no in-place upgrade path.
`ALTER EXTENSION pgrdf UPDATE` is not supported in v0.x and is
deferred until v1.0.

### Upgrade procedure

To move from one v0.x version to the next:

1. **Export your data** while still on the old version. v0.3 has no
   `CONSTRUCT` UDF, so dump the dictionary-decoded quads directly via
   SQL — per graph:

   ```sql
   SELECT s.lexical_value AS subject,
          p.lexical_value AS predicate,
          o.lexical_value AS object
   FROM pgrdf._pgrdf_quads q
   JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id
   JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id
   JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id
   WHERE q.graph_id = <YOUR_GRAPH>;
   ```

   Serialise the result to Turtle externally (psycopg → `oxttl`,
   `rdflib`, etc.). A future release is expected to add a
   `pgrdf.construct_turtle(…)` UDF for direct Turtle output (see
   [`specs/SPEC.pgRDF.LLD.v0.4.md`](../specs/SPEC.pgRDF.LLD.v0.4.md)
   §6); until that lands the SQL dump above is the supported path.

2. **Drop the extension**:

   ```sql
   DROP EXTENSION pgrdf CASCADE;
   ```

   `CASCADE` is required because the partitioned `_pgrdf_quads` table
   and the SPO/POS/OSP indexes depend on the extension.

3. **Install the new version** per §6.1 / §6.2 above.

4. **Re-create + re-load**:

   ```sql
   CREATE EXTENSION pgrdf;
   SELECT pgrdf.add_graph(<YOUR_GRAPH>);
   SELECT pgrdf.load_turtle('/path/to/your/export.ttl', <YOUR_GRAPH>);
   ```

### Why no in-place upgrade?

- Pre-1.0 schema is fluid. Anticipated additions (named-graph
  scoping, an IRI ↔ `graph_id` mapping table per LLD v0.4 §3,
  CONSTRUCT, SPARQL UPDATE) require migrations the v0.3 schema can't
  describe ahead of time.
- The dictionary `id` space is not stable across versions — the same
  Turtle re-ingested under a different build gets different integer
  ids.
- The `is_inferred` boolean column on `_pgrdf_quads` may grow or
  shift semantics as the OWL 2 RL surface evolves.

Once v1.0 cuts, `ALTER EXTENSION pgrdf UPDATE` migrations will land
alongside a frozen on-disk schema. No date is committed yet.

### Cluster-managed installations

Operators running pgRDF on top of CloudNativePG, StackGres, Apache
AGE, or similar:

- Schedule the upgrade as a planned maintenance window — `DROP
  EXTENSION pgrdf CASCADE` is destructive to pgRDF data, even though
  surrounding Postgres tables are untouched.
- Snapshot the Postgres volume before `DROP EXTENSION`.
- Test on a staging database first; verify the dump-and-reload
  round-trip produces the expected triple count.
- File issues at <https://github.com/styk-tv/pgRDF/issues> if the
  upgrade friction is high — we want to smooth this path before v1.0.

## Verifying conformance (INSTALL spec §12)

After install, run the conformance checklist:

```bash
psql -c "SELECT extversion FROM pg_extension WHERE extname='pgrdf';"   # ≡ Cargo.toml version
psql -c "\dx pgrdf"                                                     # extension present
psql -c "SHOW shared_preload_libraries;"                                # contains pgrdf
```

All three are wired into `tests/regression/sql/00-smoke.sql` so this
is gated in CI.
