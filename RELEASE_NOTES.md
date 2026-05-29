# pgRDF v0.5.1 — PGXN, artifact parity, and MIT cleanup

**pgRDF v0.5.1 is a maintenance release on top of the v0.5.0 engine
surface.** There is no RDF / SPARQL / SHACL / OWL engine delta in this
cut. The focus is packaging and delivery: PGXN source-distribution
support, release-asset parity checks, current install docs, and the
license transition to MIT.

## What changed

- **PGXN source distribution** — the repo root now carries `META.json`,
  `Makefile`, `INSTALL.md`, and a PGXN-facing `README.pgxn.md`. GitHub
  releases now attach `pgrdf-0.5.1.zip` alongside the existing binary
  tarball matrix, so the PGXN archive is cut from the same tagged
  source as the release tarballs.
- **Artifact parity verification** — the compose/runtime test bar now
  proves that the running container is mounting the exact extension
  bytes built from this source tree, not stale local artifacts. This is
  exposed locally via `just test-artifact-parity` and folded into the
  cold-boot smoke path.
- **Docs and SHACL gate sync** — the install guides now use current
  versioned examples, document the PGXN path, and the W3C SHACL Core
  docs are aligned with the genuine 25 / 25 `sh:conforms` full-pass and
  the `--sparql` E-012 known-state contract.
- **MIT alignment** — crate metadata, README surfaces, release assets,
  and the root license file are now MIT-aligned. `NOTICE` is no longer
  shipped in the release tarballs.

## Release assets

This release publishes:

- `pgrdf-0.5.1.zip` — PGXN source archive
- `pgrdf-0.5.1-pg14-glibc-amd64.tar.gz`
- `pgrdf-0.5.1-pg14-glibc-arm64.tar.gz`
- `pgrdf-0.5.1-pg15-glibc-amd64.tar.gz`
- `pgrdf-0.5.1-pg15-glibc-arm64.tar.gz`
- `pgrdf-0.5.1-pg16-glibc-amd64.tar.gz`
- `pgrdf-0.5.1-pg16-glibc-arm64.tar.gz`
- `pgrdf-0.5.1-pg17-glibc-amd64.tar.gz`
- `pgrdf-0.5.1-pg17-glibc-arm64.tar.gz`
- `SHA256SUMS`

## Install paths

### GitHub release tarballs

```bash
tar xzf pgrdf-0.5.1-pg17-glibc-amd64.tar.gz
( cd pgrdf-0.5.1-pg17-glibc-amd64 && sha256sum -c SHA256SUMS )

docker run -d --name pg \
  -e POSTGRES_PASSWORD=pw \
  -v "$PWD/pgrdf-0.5.1-pg17-glibc-amd64/lib/pgrdf.so:/usr/lib/postgresql/17/lib/pgrdf.so:ro" \
  -v "$PWD/pgrdf-0.5.1-pg17-glibc-amd64/share/extension/pgrdf.control:/usr/share/postgresql/17/extension/pgrdf.control:ro" \
  -v "$PWD/pgrdf-0.5.1-pg17-glibc-amd64/share/extension/pgrdf--0.5.1.sql:/usr/share/postgresql/17/extension/pgrdf--0.5.1.sql:ro" \
  postgres:17.4
docker exec -it pg psql -U postgres -c 'CREATE EXTENSION pgrdf;'
```

### PGXN source install

```bash
pgxn install pgrdf --pg_config /path/to/pg_config
psql -d yourdb -c 'CREATE EXTENSION pgrdf;'
```

For prerequisites and the `make` fallback from an unpacked archive, see
`INSTALL.md`.

### OCI artifacts

```
oras pull ghcr.io/styk-tv/pgrdf-bundle:v0.5.1-pg17-amd64
oras manifest fetch ghcr.io/styk-tv/pgrdf-bundle:v0.5.1
```

The `oci-publish` workflow downloads the tagged release assets,
verifies `SHA256SUMS`, and publishes one OCI artifact per PG ×
architecture plus the aggregate `:0.5.1` / `:v0.5.1` index manifests.

## Engine and compatibility

- **No engine delta from v0.5.0.** The v0.5.0 RDF / SPARQL / SHACL /
  OWL feature surface is unchanged in v0.5.1.
- **No in-place extension upgrade path.** pgRDF v0.x still does not
  support `ALTER EXTENSION pgrdf UPDATE`; drop and recreate between v0.x
  releases.
- **Documented upstream gates remain.**
  - `E-011`: crates.io publish stays blocked by the patched
    `reasonable` dependency.
  - `E-012`: SHACL-SPARQL constraint execution remains upstream-gated.

## License

MIT. Copyright 2026 Peter Styk — see [`LICENSE`](LICENSE) for the canonical attribution.

Full changelog: [`CHANGELOG.md`](CHANGELOG.md). Spec:
[`specs/SPEC.pgRDF.LLD.v0.5.md`](specs/SPEC.pgRDF.LLD.v0.5.md).
