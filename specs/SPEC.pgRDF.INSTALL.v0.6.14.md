# SPEC.pgRDF.INSTALL.v0.6.14

| Field | Value |
|---|---|
| Status | Current — tracks the shipped v0.6.14 release |
| Version | 0.6.14 |
| Supersedes | v0.2 (and the v0.1 draft) |
| Scope | Runtime installation of the `pgRDF` PostgreSQL extension into stock PostgreSQL containers on Kubernetes, without rebuilding the container image and without compiling from source. |
| Non-goals | pgRDF internals; build pipeline for pgRDF itself; multi-cluster operator integration; HA / replication topology. |
| Audience | Platform engineers operating PostgreSQL on Kubernetes; pgRDF release maintainers. |
| Conformance | Per RFC 2119: **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, **MAY**. |
| As-built reference | `specs/SPEC.pgRDF.LLD.v0.6.14.md` (the authoritative low-level design this install contract is reconciled against). |

---

## 1. Definitions

| Term | Meaning |
|---|---|
| Stock image | An unmodified `postgres:<N>-bookworm` image from Docker Hub. |
| Drop-in pattern | The init-container fetch + stage + copy approach defined in §4. |
| OCI bundle | The published, SLSA-attested OCI artifact defined in §3; the **primary** release artifact. |
| Extension bundle | The per-PG-major release tarball defined in §3; the **secondary** release artifact. |
| Target | The running PostgreSQL pod. |
| `PG_MAJOR` | The PostgreSQL major version (e.g. `17`). |
| `$libdir` | Resolves to `/usr/lib/postgresql/${PG_MAJOR}/lib` in the stock image. |
| `$sharedir/extension` | Resolves to `/usr/share/postgresql/${PG_MAJOR}/extension` in the stock image. |
| SLSA Build Provenance v1 | The in-toto attestation (`https://slsa.dev/provenance/v1`) bound to each GHCR digest, verifiable with `gh attestation verify`. |

---

## 2. Goals

- **G1.** Run pgRDF inside the official `postgres:<N>-bookworm` image without rebuilding it.
- **G2.** No source compilation at runtime; binaries are pulled pre-built.
- **G3.** Deterministic and reproducible across pod restarts.
- **G4.** Pinnable to an exact pgRDF version × PG major × architecture tuple, by OCI digest.
- **G5.** Compatible with PostgreSQL **14, 15, 16, 17**. The v0.6.14 artifacts target **pg17 head**; pg14–16 builds are PAUSED for a stabilization window (§7). **PostgreSQL 18 is deferred** — blocked on the pgrx 0.16 pin (ERRATA E-006); the `extension_control_path` drop-in (§7) is a *future* path, not the preferred path today.
- **G6.** Zero modification of the upstream `postgres` image tag at the registry level.
- **G7.** Every fetched binary is supply-chain-verifiable: by SLSA Build Provenance v1 attestation for the OCI bundle, and by `SHA256SUMS` for the secondary tarballs (§8.2).

---

## 3. Release Artifact Contract

### 3.1 Primary: OCI bundle (SLSA-attested)

pgRDF **MUST** publish, per release, a multi-arch OCI bundle to GHCR. The aggregate index is:

```
ghcr.io/styk-tv/pgrdf-bundle:0.6.14
```

Per-`(PG_MAJOR, arch)` images are addressed by the conventional tag suffix:

```
ghcr.io/styk-tv/pgrdf-bundle:0.6.14-pg<PG_MAJOR>-<arch>
```

Worked examples:

```
ghcr.io/styk-tv/pgrdf-bundle:0.6.14-pg17-amd64
ghcr.io/styk-tv/pgrdf-bundle:0.6.14-pg17-arm64
```

Each image **MUST** contain, at these relative paths, exactly the extension payload:

```
lib/pgrdf.so
share/extension/pgrdf.control
share/extension/pgrdf--0.6.14.sql
share/extension/pgrdf--<prev>--0.6.14.sql   # zero or more upgrade scripts
```

The bundle is pulled with `oras pull` (§4.2). Consumers **MUST** pin by digest, not by floating tag, in production (§8.2).

Every GHCR digest **MUST** carry a verifiable **SLSA Build Provenance v1** attestation, produced by the release CI and bound to the digest in the GitHub attestation store. Verification is a first-class supply-chain step (§8.2 / §8.6):

```
gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.6.14 --repo styk-tv/pgRDF
```

A successful verification exits `0`. A non-zero exit **MUST** halt any rollout.

### 3.2 Secondary: per-PG tarballs

pgRDF **SHOULD** also publish, per GitHub release, one tarball per `(PG_MAJOR, libc, arch)` tuple as a fallback path for environments without OCI tooling:

```
pgrdf-<version>-pg<PG_MAJOR>-<libc>-<arch>.tar.gz
```

Worked examples:

```
pgrdf-0.6.14-pg17-glibc-amd64.tar.gz
pgrdf-0.6.14-pg17-glibc-arm64.tar.gz
```

The tarball **MUST** contain, at these relative paths:

```
lib/pgrdf.so
share/extension/pgrdf.control
share/extension/pgrdf--0.6.14.sql
share/extension/pgrdf--<prev>--0.6.14.sql   # zero or more upgrade scripts
LICENSE
SHA256SUMS
```

The release **MUST** publish a top-level `SHA256SUMS` covering every tarball in the release. Tarball integrity is verified against `SHA256SUMS`; the OCI bundle is the path that carries the SLSA attestation.

### 3.3 Common ABI rules

The `.so` **MUST** be linked against the Debian `PG_MAJOR` ABI as shipped in `postgres:<N>-bookworm`. A `.so` built against PGDG packages **MUST** be ABI-equivalent. Cross-PG-major `.so` files are **forbidden** in the same image or archive.

The payload **SHOULD** be ≤ 5 MB uncompressed. For v0.6.14 the published matrix is `{pg17} × {amd64, arm64}` (pg14–16 PAUSED, pg18 deferred — §7); the contract **SHOULD** extend to `{pg14, pg15, pg16, pg17} × {amd64, arm64}` once the pg14–16 builds resume.

---

## 4. Reference Architecture: Init-Container Drop-in

### 4.1 Volumes

A single ephemeral volume is shared between the init container and the postgres container:

| Name | Type | Mount path |
|---|---|---|
| `pgrdf-staging` | `emptyDir: {}` | `/pgrdf` (both containers) |

### 4.2 Init container

- **Image:** any minimal image carrying `oras` (default path; recommended: `ghcr.io/oras-project/oras:v1.2.0`). The fallback tarball path requires `curl`, `tar`, `sha256sum` (e.g. `curlimages/curl:8.10.0`).
- **Responsibilities (default, OCI path):**
  1. Resolve `${PGRDF_OCI_REF}` (pinned **by digest**), `${PG_MAJOR}` from environment.
  2. `oras pull` the matching per-`(PG_MAJOR, arch)` image into `/pgrdf`.
  3. Confirm `lib/pgrdf.so`, `share/extension/pgrdf.control`, and `share/extension/pgrdf--0.6.14.sql` are present.
  4. Exit `0`.
- **Responsibilities (fallback, tarball path):**
  1. Resolve `${PGRDF_VERSION}`, `${PG_MAJOR}`, `${ARCH}`, `${RELEASE_URL}`, `${PGRDF_SHA256}` from environment.
  2. Download the matching tarball from the pinned GitHub release URL.
  3. Verify SHA256 against the `ConfigMap`-supplied value.
  4. Extract into `/pgrdf/lib` and `/pgrdf/share/extension`.
  5. Exit `0`.

The init container **MUST** fail with non-zero exit on:
- A pull/HTTP failure or non-2xx response,
- Digest or checksum mismatch,
- Missing required files in the payload,
- Architecture mismatch (`file lib/pgrdf.so` vs expected).

The init container **MUST NOT** mutate any path outside `/pgrdf`.

> **Supply chain.** SLSA attestation verification (§3.1) **SHOULD** be performed in CI/CD at promotion time and **MAY** additionally be performed in the init container. When verified in-cluster, the init container **MUST** fail on a non-zero `gh attestation verify` exit.

### 4.3 Main container

- **Image:** `postgres:<N>-bookworm` (pinned to an exact minor, e.g. `17.4-bookworm`).
- **Entrypoint wrapper:** the manifest **MUST** override `command` to:
  1. Copy `/pgrdf/lib/*.so` → `$libdir`.
  2. Copy `/pgrdf/share/extension/*` → `$sharedir/extension`.
  3. `exec docker-entrypoint.sh postgres "$@"`.
- All copy operations **MUST** be idempotent (`cp -f`) so that pod restarts do not fail on pre-existing files.
- The wrapper **MUST NOT** modify file ownership outside the two target directories.

The postgres args **MUST** include `-c shared_preload_libraries=pgrdf` (see §6 — required for full functionality). A `pg_ctl reload` is insufficient; a full restart is required when this list changes.

### 4.4 Activation

`CREATE EXTENSION pgrdf;` is performed by one of:

- A SQL file mounted at `/docker-entrypoint-initdb.d/10-pgrdf.sql` (first-time bootstrap of a fresh PGDATA), **or**
- An external migration tool (e.g. Flyway, sqitch, Atlas) run as a Kubernetes `Job` after the StatefulSet reports `Ready`.

`ALTER EXTENSION pgrdf UPDATE TO '0.6.14';` **MUST** be used for upgrades, not drop-and-create.

---

## 5. Kubernetes Reference Manifest

The following manifest is normative for v0.6.14. Names, namespace, and storage class are illustrative. It uses the **default OCI path**; the `${PGRDF_OCI_REF}` value **MUST** be pinned by digest in production.

```yaml
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: pgrdf-release
data:
  PG_MAJOR: "17"
  ARCH: "amd64"
  # PRIMARY (OCI) path: pin by digest in production, e.g.
  #   ghcr.io/styk-tv/pgrdf-bundle:0.6.14-pg17-amd64@sha256:<digest>
  PGRDF_OCI_REF: "ghcr.io/styk-tv/pgrdf-bundle:0.6.14-pg17-amd64"
  # FALLBACK (tarball) path — only consumed by the alternate init container:
  PGRDF_VERSION: "0.6.14"
  PGRDF_SHA256: "REPLACE_ME_with_sha256_of_the_chosen_tarball"
  RELEASE_URL: "https://github.com/styk-tv/pgRDF/releases/download/v0.6.14/pgrdf-0.6.14-pg17-glibc-amd64.tar.gz"
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
      # PRIMARY path: oras pull the SLSA-attested OCI bundle (pin by digest).
      - name: fetch-pgrdf
        image: ghcr.io/oras-project/oras:v1.2.0
        envFrom:
        - configMapRef: { name: pgrdf-release }
        command: ["sh", "-c"]
        args:
        - |
          set -euo pipefail
          mkdir -p /pgrdf
          cd /pgrdf
          echo "Pulling ${PGRDF_OCI_REF}"
          oras pull "${PGRDF_OCI_REF}"
          test -f lib/pgrdf.so
          test -f share/extension/pgrdf.control
          test -f share/extension/pgrdf--0.6.14.sql
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
          # shared_preload_libraries=pgrdf is REQUIRED for full functionality:
          # the native staged bulk loader (background-worker pool) and the
          # cross-backend shmem caches (dictionary cache, plan-cache counters,
          # staged jobctl) register only in the postmaster (see §6).
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

> **Fallback init container.** To use the secondary tarball path instead, replace the `fetch-pgrdf` init container with a `curlimages/curl:8.10.0` container that downloads `${RELEASE_URL}`, verifies `${PGRDF_SHA256}` via `sha256sum -c`, and extracts into `/pgrdf` (the v0.2 pattern). Prefer the OCI path; it carries the SLSA attestation and pins by digest.

---

## 6. Required Configuration

| Item | Required | Notes |
|---|---|---|
| `shared_preload_libraries` includes `pgrdf` | **YES** (for full functionality) | Required for the native **staged bulk loader** (background-worker pool) and the cross-backend shared-memory caches (dictionary cache, plan-cache counters, staged jobctl). Their shmem hooks register **only** in the postmaster. Without preload, those shmem facilities no-op and the staged loader is **unavailable**; the in-process loaders (`load_turtle`, `parse_turtle`/`parse_trig`/`parse_nquads`, `load_turtle_streaming`) and the rest of the UDF surface still work. Changing this list needs a **full restart**, not `pg_ctl reload`. |
| `pgrdf.*` GUCs | Per release | Custom GUCs register on both the preload and lazy-load paths. Set via `-c` flags or `postgresql.conf` overlay. |
| Superuser | **YES** | The control file declares `superuser = true` and `trusted = false`; `CREATE EXTENSION pgrdf` requires a superuser. |
| PG minor pin | **YES** | The `postgres` image tag **MUST** include the minor, e.g. `17.4-bookworm`. The `17-bookworm` floating tag is **forbidden**. |

**Control file facts (v0.6.14):** `default_version = '0.6.14'`, `schema = 'pgrdf'`, `relocatable = false`, `superuser = true`, `trusted = false`, `module_pathname = '$libdir/pgrdf'`, and **no external extension dependency** (no `requires =` line).

---

## 7. Forward Path: PostgreSQL 18 `extension_control_path` (DEFERRED)

> **Status: future / not yet supported.** PostgreSQL 18 is **deferred** in v0.6.14, blocked on the pgrx 0.16 pin (ERRATA E-006). The supported majors today are **14, 15, 16, 17** (v0.6.14 ships pg17; pg14–16 PAUSED). This section is forward-looking and **MUST NOT** be used on the shipped majors.

From PG 18, the GUCs `extension_control_path` and `dynamic_library_path` accept additional search directories outside `$sharedir/extension` and `$libdir`. **Once PG 18 lands** (pending the pgrx upgrade and a pg18 artifact in the §3 matrix), implementations **SHOULD** prefer this over §4.3:

- Mount `/pgrdf` into the postgres container.
- Set:
  - `extension_control_path = '/pgrdf/share/extension:$system'`
  - `dynamic_library_path = '/pgrdf/lib:$libdir'`
- Omit the entrypoint copy step entirely.

This is the preferred path *only once PG 18 is supported*, because it leaves the postgres container's writable layer untouched, eliminating the §4.3 entrypoint wrapper and reducing failure surface.

Implementations **MUST NOT** use this path on PG ≤ 17 — the GUCs are absent and silently ignored. Implementations **MUST NOT** treat PG 18 as supported until a pg18 artifact ships per §3.

---

## 8. Operational Considerations

### 8.1 Persistence

Extension binaries live in `emptyDir` and are re-fetched on every pod restart. PGDATA persists across restarts; the catalog rows from `CREATE EXTENSION` remain valid as long as the post-restart `.so` is ABI-compatible.

### 8.2 Caching and supply chain

The primary artifact is the GHCR OCI bundle, which natively supports digest pinning, registry mirroring, and SLSA attestation. Production deployments **SHOULD**:

- Pin the OCI reference **by digest**, not by tag, e.g. `…@sha256:<digest>`.
- Mirror the bundle into a node-local or internal OCI registry on release; pull the init container payload from the mirror.
- Verify the **SLSA Build Provenance v1** attestation at promotion time (and **MAY** re-verify in-cluster):
  ```
  gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.6.14 --repo styk-tv/pgRDF
  ```
  A non-zero exit **MUST** halt the rollout.

The secondary tarball path pulls from `github.com/styk-tv/pgRDF/releases/download/...`, which exposes the workload to GitHub availability, GitHub rate limits, and asset-deletion incidents. When the tarball path is used, the workload **SHOULD** mirror the tarballs internally and **MUST** verify each tarball against the release `SHA256SUMS` before extraction.

### 8.3 Upgrades

1. Update `ConfigMap/pgrdf-release` with the new `PGRDF_OCI_REF` (new digest) — and, for the fallback path, `PGRDF_VERSION`, `PGRDF_SHA256`, `RELEASE_URL`.
2. (Recommended) Verify the new digest's SLSA attestation (§8.2) before promotion.
3. `kubectl rollout restart statefulset/pg-pgrdf`.
4. After Ready: `ALTER EXTENSION pgrdf UPDATE;` per database.
5. Because `shared_preload_libraries=pgrdf` is in use, step 3 already restarted Postgres, picking up the new `.so` for the preload.

### 8.4 Downgrades

Not supported in v0.6.14. Operational procedure:
1. `DROP EXTENSION pgrdf CASCADE;`
2. Roll back the ConfigMap.
3. `kubectl rollout restart statefulset/pg-pgrdf`.
4. `CREATE EXTENSION pgrdf VERSION '<old>';`

Data stored in pgRDF-managed objects will be lost. Operators **MUST** back up before downgrading.

### 8.5 Telemetry

The init container **SHOULD** log: the resolved OCI reference (or source URL), resolved version, resolved digest (or computed SHA256), and the list of extracted files. These are inspectable via `kubectl logs <pod> -c fetch-pgrdf`.

### 8.6 Security

- Init container **MUST** run as non-root (`runAsUser: 1000` or any non-zero UID; the destination volume is `emptyDir` with default ownership).
- Postgres container retains its standard UID (999).
- Network egress from the init container **SHOULD** be restricted by `NetworkPolicy` to the OCI registry (or mirror) host only.
- **Supply-chain attestation is first-class.** Every GHCR digest carries a verifiable **SLSA Build Provenance v1** attestation; the promotion pipeline **MUST** run `gh attestation verify oci://ghcr.io/styk-tv/pgrdf-bundle:0.6.14 --repo styk-tv/pgRDF` and treat a non-zero exit as a release-blocking failure. The secondary tarballs are integrity-checked against `SHA256SUMS`.

---

## 9. Failure Modes

| Failure | Symptom | Resolution |
|---|---|---|
| Wrong `PG_MAJOR` in payload | `ERROR: could not load library ... undefined symbol` at `CREATE EXTENSION` | Fix `ConfigMap.PG_MAJOR` / `PGRDF_OCI_REF`. |
| Alpine-based postgres image | `ELF interpreter not found` or `not a dynamic executable` | Use `postgres:<N>-bookworm`, not `-alpine`. |
| Floating `postgres:17-bookworm` tag drift | Periodic `LOAD` crashes after upstream image rebuild | Pin minor: `postgres:17.4-bookworm`. |
| OCI tag pinned (not digest) and re-pushed | Silent payload drift between restarts | Pin `PGRDF_OCI_REF` by `@sha256:<digest>` (§8.2). |
| SLSA attestation verify fails | `gh attestation verify` exits non-zero | Halt promotion; investigate provenance before deploying (§8.6). |
| GitHub release deletion / renaming (tarball path) | Init container 404s, pod stuck in `Init:CrashLoopBackOff` | Use the OCI bundle, or an internal mirror (§8.2). |
| Tarball checksum mismatch | Init container exits with `sha256sum: WARNING` | Halt rollout; investigate supply chain before retrying. |
| `shared_preload_libraries` not set | Staged loader refuses with a clear `error!` ("pgRDF is not in `shared_preload_libraries`"); shmem caches read 0 and no-op; in-process loaders still work | Add `-c shared_preload_libraries=pgrdf` and **restart** (not reload) the cluster (§6). |
| PG 18 image used | `extension_control_path` path attempted with no pg18 artifact; `CREATE EXTENSION` fails on a missing/incompatible `.so` | PG 18 is deferred (§7); use a supported major (14–17, pg17 shipped). |
| Stale `.so` after upgrade rollback | Catalog version mismatch error | Re-run `ALTER EXTENSION pgrdf UPDATE`; or `DROP` + `CREATE` if downgrading per §8.4. |
| Init container OOM / slow link | Pod timeout | Increase `activeDeadlineSeconds`; pre-populate the OCI mirror. |

---

## 10. Alternatives Considered

| Approach | Verdict | Rationale |
|---|---|---|
| Custom-built image with pgRDF baked in | **Rejected** | Explicitly out of scope; couples pgRDF release cadence to image rebuild. |
| Tembo Trunk (`trunk install pgrdf`) | **Possible future** once pgRDF is published to the Trunk registry | Equivalent semantics, simpler init container, central registry. Not the v0.6.14 path; the OCI bundle is. |
| PGDG `apt-get install postgresql-${PG_MAJOR}-pgrdf` in init container | **Acceptable** if pgRDF is packaged for PGDG | Heavier image but well-understood supply chain. |
| StackGres or CloudNativePG operator-managed extensions | **Recommended at fleet scale** (>5 clusters) | Out of scope for this spec; defer to operator-specific docs. |
| HostPath bind-mount of pre-staged binaries | **Rejected** | Node-coupling; breaks portability; conflicts with PSP/PSA restrictions. |
| Sidecar that lazily downloads on `LOAD` | **Rejected** | Race conditions; Postgres expects files present at startup for `shared_preload_libraries`. |

---

## 11. Open Questions

- **OQ1.** *(Resolved in v0.6.14.)* Source of truth for binaries: the **OCI bundle** (`ghcr.io/styk-tv/pgrdf-bundle:0.6.14`) is the primary, SLSA-attested artifact; the GitHub-release tarballs are the secondary fallback. The init container default is `oras pull`, pinned by digest; the `curl`-from-GitHub path is the fallback.
- **OQ2.** Multi-arch handling: auto-detect via `uname -m` in the init container, or hard-code arch via `ConfigMap` plus `nodeAffinity`. Default: hard-code, simpler reasoning. The OCI index permits arch selection by manifest where the runtime supports it.
- **OQ3.** Digest pinning: `ConfigMap` (GitOps-friendly) vs in-line in the manifest. Default: `ConfigMap`, pinned by `@sha256:<digest>`.
- **OQ4.** *(Resolved in v0.6.14.)* Attestation is **SLSA Build Provenance v1** on every GHCR digest, verified with `gh attestation verify`. Tarballs carry `SHA256SUMS`.
- **OQ5.** Re-enable of the pg14–16 build matrix and PG 18 support — both gated (§7); pg14–16 on the stabilization window, pg18 on the pgrx 0.16 pin (ERRATA E-006).
- **OQ6.** Backup/restore semantics when pgRDF stores opaque blobs. Deferred to a sibling spec `SPEC.pgRDF.BACKUP.v0.x`.

---

## 12. Conformance Checklist

A deployment is conformant with v0.6.14 if and only if:

- [ ] The postgres image is unmodified and pulled from Docker Hub by digest or pinned minor tag.
- [ ] No `Dockerfile` exists in the deployment repo for the postgres workload.
- [ ] An init container fetches the §3 payload — the OCI bundle by digest (primary) or a tarball verified against `SHA256SUMS` (fallback).
- [ ] For the OCI path, the digest's SLSA Build Provenance v1 attestation verifies (`gh attestation verify` exit 0) at promotion time.
- [ ] Extension files land in `$libdir` and `$sharedir/extension` (PG 14–17).
- [ ] `shared_preload_libraries=pgrdf` is set and the cluster was restarted (required for the staged loader and shmem caches — §6).
- [ ] `CREATE EXTENSION pgrdf;` succeeds against the running cluster as a superuser.
- [ ] `SELECT extversion FROM pg_extension WHERE extname='pgrdf';` returns `0.6.14`.

---

## 13. Changelog

- **v0.6.14** (this document)
  - Versioned the spec to track the shipped v0.6.14 release; bumped all examples from `0.4.1` to `0.6.14`.
  - Made the **OCI bundle** (`ghcr.io/styk-tv/pgrdf-bundle:0.6.14`) the **primary** release artifact (§3.1); demoted the per-PG tarballs to a secondary fallback (§3.2). Rewrote the init container (§4.2) and reference manifest (§5) to `oras pull` by digest by default.
  - Replaced "GPG signature deferred to v0.3" with first-class **SLSA Build Provenance v1** verification via `gh attestation verify` (§3.1, §8.2, §8.6); resolved OQ1 and OQ4.
  - Corrected G5 and §7: supported majors are **14, 15, 16, 17** (v0.6.14 ships pg17; pg14–16 PAUSED); **PG 18 is deferred** behind the pgrx 0.16 pin (ERRATA E-006), and the `extension_control_path` drop-in is future-gated, not preferred today.
  - Made `shared_preload_libraries=pgrdf` **required for full functionality** (§6, manifest comment, §9): needed for the native staged bulk loader and the cross-backend shmem caches; in-process loaders still work without it.
  - Replaced all `ORG`/`ghcr.io/ORG/...` placeholders with `styk-tv/pgRDF` and `ghcr.io/styk-tv/pgrdf-bundle`; pinned the control-file facts in §6.
  - Cross-referenced the as-built `specs/SPEC.pgRDF.LLD.v0.6.14.md`.
- **v0.2**
  - Added §7 PG 18 `extension_control_path` path.
  - Added §10 alternatives table.
  - Tightened §3 release artifact naming and contents.
  - Added §12 conformance checklist.
  - Added security subsection §8.6.
- **v0.1**
  - Initial draft: init-container pattern, reference manifest, failure modes.
