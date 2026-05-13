# SPEC.pgRDF.INSTALL.v0.2

| Field | Value |
|---|---|
| Status | Draft |
| Version | 0.2 |
| Supersedes | v0.1 |
| Scope | Runtime installation of the `pgRDF` PostgreSQL extension into stock PostgreSQL containers on Kubernetes, without rebuilding the container image and without compiling from source. |
| Non-goals | pgRDF internals; build pipeline for pgRDF itself; multi-cluster operator integration; HA / replication topology. |
| Audience | Platform engineers operating PostgreSQL on Kubernetes; pgRDF release maintainers. |
| Conformance | Per RFC 2119: **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, **MAY**. |

---

## 1. Definitions

| Term | Meaning |
|---|---|
| Stock image | An unmodified `postgres:<N>-bookworm` image from Docker Hub. |
| Drop-in pattern | The init-container fetch + stage + copy approach defined in §4. |
| Extension bundle | The release artifact defined in §3. |
| Target | The running PostgreSQL pod. |
| `PG_MAJOR` | The PostgreSQL major version (e.g. `17`). |
| `$libdir` | Resolves to `/usr/lib/postgresql/${PG_MAJOR}/lib` in the stock image. |
| `$sharedir/extension` | Resolves to `/usr/share/postgresql/${PG_MAJOR}/extension` in the stock image. |

---

## 2. Goals

- **G1.** Run pgRDF inside the official `postgres:<N>-bookworm` image without rebuilding it.
- **G2.** No source compilation at runtime; binaries are pulled pre-built.
- **G3.** Deterministic and reproducible across pod restarts.
- **G4.** Pinnable to an exact pgRDF version × PG minor version × architecture tuple.
- **G5.** Compatible with PG 14 through 18, with the PG-18 path (§7) being preferred.
- **G6.** Zero modification of the upstream `postgres` image tag at the registry level.

---

## 3. Release Artifact Contract

pgRDF **MUST** publish, per GitHub release, one tarball per `(PG_MAJOR, libc, arch)` tuple:

```
pgrdf-<version>-pg<PG_MAJOR>-<libc>-<arch>.tar.gz
```

Worked examples:

```
pgrdf-0.4.1-pg17-glibc-amd64.tar.gz
pgrdf-0.4.1-pg17-glibc-arm64.tar.gz
pgrdf-0.4.1-pg18-glibc-amd64.tar.gz
```

The tarball **MUST** contain, at these relative paths:

```
lib/pgrdf.so
share/extension/pgrdf.control
share/extension/pgrdf--<version>.sql
share/extension/pgrdf--<prev>--<version>.sql   # zero or more upgrade scripts
LICENSE
SHA256SUMS
```

The release **MUST** also publish `SHA256SUMS` and `SHA256SUMS.asc` (detached GPG signature) as separate top-level assets, covering every tarball in the release.

The `.so` **MUST** be linked against the Debian `PG_MAJOR` ABI as shipped in `postgres:<N>-bookworm`. A `.so` built against PGDG packages **MUST** be ABI-equivalent. Cross-PG-major `.so` files are **forbidden** in the same archive.

The tarball **SHOULD** be ≤ 5 MB uncompressed. The release **SHOULD** publish the matrix `{pg14, pg15, pg16, pg17, pg18} × {amd64, arm64}`.

---

## 4. Reference Architecture: Init-Container Drop-in

### 4.1 Volumes

A single ephemeral volume is shared between the init container and the postgres container:

| Name | Type | Mount path |
|---|---|---|
| `pgrdf-staging` | `emptyDir: {}` | `/pgrdf` (both containers) |

### 4.2 Init container

- **Image:** any minimal image with `curl`, `tar`, `sha256sum` (recommended: `curlimages/curl:8.10.0`).
- **Responsibilities:**
  1. Resolve `${PGRDF_VERSION}`, `${PG_MAJOR}`, `${ARCH}` from environment.
  2. Download the matching tarball from the pinned GitHub release URL.
  3. Verify SHA256 against the value supplied via `ConfigMap`.
  4. Extract into `/pgrdf/lib` and `/pgrdf/share/extension`.
  5. Exit `0`.

The init container **MUST** fail with non-zero exit on:
- HTTP non-2xx response,
- Checksum mismatch,
- Missing required files in the archive,
- Architecture mismatch (`file lib/pgrdf.so` vs expected).

The init container **MUST NOT** mutate any path outside `/pgrdf`.

### 4.3 Main container

- **Image:** `postgres:<N>-bookworm` (pinned to an exact minor, e.g. `17.4-bookworm`).
- **Entrypoint wrapper:** the manifest **MUST** override `command` to:
  1. Copy `/pgrdf/lib/*.so` → `$libdir`.
  2. Copy `/pgrdf/share/extension/*` → `$sharedir/extension`.
  3. `exec docker-entrypoint.sh postgres "$@"`.
- All copy operations **MUST** be idempotent (`cp -f`) so that pod restarts do not fail on pre-existing files.
- The wrapper **MUST NOT** modify file ownership outside the two target directories.

If pgRDF requires `shared_preload_libraries` (see §6), the postgres args **MUST** include
`-c shared_preload_libraries=pgrdf`. A `pg_ctl reload` is insufficient; a full restart is required when this list changes.

### 4.4 Activation

`CREATE EXTENSION pgrdf;` is performed by one of:

- A SQL file mounted at `/docker-entrypoint-initdb.d/10-pgrdf.sql` (first-time bootstrap of a fresh PGDATA), **or**
- An external migration tool (e.g. Flyway, sqitch, Atlas) run as a Kubernetes `Job` after the StatefulSet reports `Ready`.

`ALTER EXTENSION pgrdf UPDATE TO '<version>';` **MUST** be used for upgrades, not drop-and-create.

---

## 5. Kubernetes Reference Manifest

The following manifest is normative for v0.2. Names, namespace, and storage class are illustrative.

```yaml
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: pgrdf-release
data:
  PGRDF_VERSION: "0.4.1"
  PG_MAJOR: "17"
  ARCH: "amd64"
  # Pin checksums; one per supported tarball.
  PGRDF_SHA256: "REPLACE_ME_with_sha256_of_the_chosen_tarball"
  RELEASE_URL: "https://github.com/ORG/pgrdf/releases/download/v0.4.1/pgrdf-0.4.1-pg17-glibc-amd64.tar.gz"
---
apiVersion: v1
kind: Secret
metadata:
  name: pg-secret
type: Opaque
stringData:
  POSTGRES_PASSWORD: changeMe
---
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: pg-pgrdf
spec:
  serviceName: pg-pgrdf
  replicas: 1
  selector:
    matchLabels: { app: pg-pgrdf }
  template:
    metadata:
      labels: { app: pg-pgrdf }
    spec:
      volumes:
      - name: pgrdf-staging
        emptyDir: {}
      initContainers:
      - name: fetch-pgrdf
        image: curlimages/curl:8.10.0
        envFrom:
        - configMapRef: { name: pgrdf-release }
        command: ["sh", "-c"]
        args:
        - |
          set -euo pipefail
          cd /tmp
          echo "Fetching ${RELEASE_URL}"
          curl -fsSL -o pgrdf.tar.gz "${RELEASE_URL}"
          echo "${PGRDF_SHA256}  pgrdf.tar.gz" | sha256sum -c -
          mkdir -p /pgrdf/lib /pgrdf/share/extension
          tar -xzf pgrdf.tar.gz
          cp -f lib/pgrdf.so /pgrdf/lib/
          cp -f share/extension/* /pgrdf/share/extension/
          ls -la /pgrdf/lib /pgrdf/share/extension
        volumeMounts:
        - { name: pgrdf-staging, mountPath: /pgrdf }
      containers:
      - name: postgres
        image: postgres:17.4-bookworm
        envFrom:
        - secretRef: { name: pg-secret }
        - configMapRef: { name: pgrdf-release }
        command: ["bash", "-c"]
        args:
        - |
          set -euo pipefail
          cp -f /pgrdf/lib/*.so /usr/lib/postgresql/${PG_MAJOR}/lib/
          cp -f /pgrdf/share/extension/* /usr/share/postgresql/${PG_MAJOR}/extension/
          exec docker-entrypoint.sh postgres \
            -c shared_preload_libraries=pgrdf
        ports:
        - containerPort: 5432
        volumeMounts:
        - { name: pgrdf-staging, mountPath: /pgrdf, readOnly: true }
        - { name: pgdata, mountPath: /var/lib/postgresql/data }
  volumeClaimTemplates:
  - metadata: { name: pgdata }
    spec:
      accessModes: ["ReadWriteOnce"]
      resources: { requests: { storage: 10Gi } }
---
apiVersion: v1
kind: Service
metadata:
  name: pg-pgrdf
spec:
  selector: { app: pg-pgrdf }
  ports: [{ port: 5432, targetPort: 5432 }]
```

---

## 6. Required Configuration

| Item | Required | Notes |
|---|---|---|
| `shared_preload_libraries` includes `pgrdf` | Conditional | Required only if pgRDF registers planner/executor hooks, custom types with on-disk image, or background workers. Declared by pgRDF release notes. |
| `pgrdf.*` GUCs | Per release | Set via `-c` flags or `postgresql.conf` overlay. |
| Superuser | YES | Required for `CREATE EXTENSION` unless the `.control` file declares `trusted = true`. |
| PG minor pin | YES | The `postgres` image tag **MUST** include the minor, e.g. `17.4-bookworm`. `17-bookworm` floating tag is **forbidden**. |

---

## 7. Forward Path: PostgreSQL 18 `extension_control_path`

From PG 18, the GUCs `extension_control_path` and `dynamic_library_path` accept additional search directories outside `$sharedir/extension` and `$libdir`. When `PG_MAJOR ≥ 18`, implementations **SHOULD** prefer this over §4.3:

- Mount `/pgrdf` into the postgres container.
- Set:
  - `extension_control_path = '/pgrdf/share/extension:$system'`
  - `dynamic_library_path = '/pgrdf/lib:$libdir'`
- Omit the entrypoint copy step entirely.

This is preferred because the postgres container's writable layer is not mutated, eliminating the §4.3 entrypoint wrapper and reducing failure surface.

Implementations **MUST NOT** use this path on PG ≤ 17 — the GUCs are absent and silently ignored.

---

## 8. Operational Considerations

### 8.1 Persistence

Extension binaries live in `emptyDir` and are re-fetched on every pod restart. PGDATA persists across restarts; the catalog rows from `CREATE EXTENSION` remain valid as long as the post-restart `.so` is ABI-compatible.

### 8.2 Caching and supply chain

A node-local registry mirror **SHOULD** be used in production. Pulling directly from `github.com/releases/download/...` exposes the workload to GitHub availability, GitHub rate limits, and asset-deletion incidents.

Recommended hardening:
- Mirror tarballs into an internal HTTP server, S3 bucket, or OCI registry on release.
- Replace the init container's `curl` step with `oras pull` or `crane export` against the internal mirror.
- Pin by digest, not by tag, when using OCI artifacts.

### 8.3 Upgrades

1. Update `ConfigMap/pgrdf-release` with new `PGRDF_VERSION`, `PGRDF_SHA256`, `RELEASE_URL`.
2. `kubectl rollout restart statefulset/pg-pgrdf`.
3. After Ready: `ALTER EXTENSION pgrdf UPDATE;` per database.
4. If `shared_preload_libraries` is in use, step 2 already restarted Postgres; otherwise no restart required.

### 8.4 Downgrades

Not supported in v0.2. Operational procedure:
1. `DROP EXTENSION pgrdf CASCADE;`
2. Roll back the ConfigMap.
3. `kubectl rollout restart statefulset/pg-pgrdf`.
4. `CREATE EXTENSION pgrdf VERSION '<old>';`

Data stored in pgRDF-managed objects will be lost. Operators **MUST** back up before downgrading.

### 8.5 Telemetry

The init container **SHOULD** log: source URL, resolved version, computed SHA256, list of extracted files. These are inspectable via `kubectl logs <pod> -c fetch-pgrdf`.

### 8.6 Security

- Init container **MUST** run as non-root (`runAsUser: 1000` or any non-zero UID; the destination volume is `emptyDir` with default ownership).
- Postgres container retains its standard UID (999).
- Network egress from the init container **SHOULD** be restricted by `NetworkPolicy` to the mirror host only.
- Signature verification (GPG against `SHA256SUMS.asc`) **SHOULD** be added in v0.3.

---

## 9. Failure Modes

| Failure | Symptom | Resolution |
|---|---|---|
| Wrong `PG_MAJOR` in tarball | `ERROR: could not load library ... undefined symbol` at `CREATE EXTENSION` | Fix `ConfigMap.PG_MAJOR` / `RELEASE_URL`. |
| Alpine-based postgres image | `ELF interpreter not found` or `not a dynamic executable` | Use `postgres:<N>-bookworm`, not `-alpine`. |
| Floating `postgres:17-bookworm` tag drift | Periodic `LOAD` crashes after upstream image rebuild | Pin minor: `postgres:17.4-bookworm`. |
| GitHub release deletion / renaming | Init container 404s, pod stuck in `Init:CrashLoopBackOff` | Use an internal mirror (§8.2). |
| Checksum mismatch | Init container exits with `sha256sum: WARNING` | Halt rollout; investigate supply chain before retrying. |
| `shared_preload_libraries` not set when required | Extension functions return `ERROR: pgrdf must be loaded via shared_preload_libraries` | Add `-c shared_preload_libraries=pgrdf` to postgres args. |
| Stale `.so` after upgrade rollback | Catalog version mismatch error | Re-run `ALTER EXTENSION pgrdf UPDATE`; or `DROP` + `CREATE` if downgrading per §8.4. |
| Init container OOM on slow link | Pod timeout | Increase `activeDeadlineSeconds`; pre-populate mirror. |

---

## 10. Alternatives Considered

| Approach | Verdict | Rationale |
|---|---|---|
| Custom-built image with pgRDF baked in | **Rejected** | Explicitly out of scope; couples pgRDF release cadence to image rebuild. |
| Tembo Trunk (`trunk install pgrdf`) | **Recommended** once pgRDF is published to the Trunk registry | Equivalent semantics, simpler init container, central registry. v0.3 target. |
| PGDG `apt-get install postgresql-${PG_MAJOR}-pgrdf` in init container | **Acceptable** if pgRDF is packaged for PGDG | Heavier image but well-understood supply chain. |
| StackGres or CloudNativePG operator-managed extensions | **Recommended at fleet scale** (>5 clusters) | Out of scope for this spec; defer to operator-specific docs. |
| HostPath bind-mount of pre-staged binaries | **Rejected** | Node-coupling; breaks portability; conflicts with PSP/PSA restrictions. |
| Sidecar that lazily downloads on `LOAD` | **Rejected** | Race conditions; Postgres expects files present at startup for preload libs. |

---

## 11. Open Questions

- **OQ1.** Source of truth for binaries: GitHub Releases (current) vs OCI artifact (`ghcr.io/ORG/pgrdf-bundle:<ver>`). Default v0.2: GitHub. Revisit in v0.3 once OCI artifact tooling matures in CI.
- **OQ2.** Multi-arch handling: auto-detect via `uname -m` in the init container, or hard-code arch via `ConfigMap` plus `nodeAffinity`. Default v0.2: hard-code, simpler reasoning.
- **OQ3.** Checksum pinning: `ConfigMap` (GitOps-friendly, current) vs in-line in the manifest. Default v0.2: `ConfigMap`.
- **OQ4.** Signature verification path. Deferred to v0.3; tracked separately.
- **OQ5.** Backup/restore semantics when pgRDF stores opaque blobs. Deferred to a sibling spec `SPEC.pgRDF.BACKUP.v0.x`.

---

## 12. Conformance Checklist

A deployment is conformant with v0.2 if and only if:

- [ ] The postgres image is unmodified and pulled from Docker Hub by digest or pinned minor tag.
- [ ] No `Dockerfile` exists in the deployment repo for the postgres workload.
- [ ] An init container fetches a tarball matching §3 and verifies SHA256.
- [ ] Extension files land in `$libdir` and `$sharedir/extension` (PG ≤ 17) or are referenced via `extension_control_path` / `dynamic_library_path` (PG ≥ 18).
- [ ] `CREATE EXTENSION pgrdf;` succeeds against the running cluster.
- [ ] `SELECT extversion FROM pg_extension WHERE extname='pgrdf';` matches the value in `ConfigMap.PGRDF_VERSION`.

---

## 13. Changelog

- **v0.2** (this document)
  - Added §7 PG 18 `extension_control_path` path.
  - Added §10 alternatives table.
  - Tightened §3 release artifact naming and contents.
  - Added §12 conformance checklist.
  - Added security subsection §8.6.
- **v0.1**
  - Initial draft: init-container pattern, reference manifest, failure modes.
