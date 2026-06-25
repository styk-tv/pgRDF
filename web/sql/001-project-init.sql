-- pgRDF v0.2 — per-project database initialiser.
--
-- Connect AS SUPERUSER to the project database (e.g. pgrdf_styk) and
-- run this. Wires the pgRDF extension into the `pgrdf` schema. AGE
-- support is gated behind the AGE_AVAILABLE substitution — the
-- vendored extension binaries only contain pgRDF in v0.1, so AGE
-- creation is normally skipped.
--
-- Used by tools/provision.py after CREATE DATABASE.

CREATE SCHEMA IF NOT EXISTS pgrdf;
CREATE EXTENSION IF NOT EXISTS pgrdf SCHEMA pgrdf;

-- AGE optional — uncomment when the image actually carries age.so:
-- CREATE EXTENSION IF NOT EXISTS age;
-- LOAD 'age';

-- postgres_fdw for bridged mode — installed unconditionally; wiring
-- happens via tools/provision.py --mode bridged.
CREATE EXTENSION IF NOT EXISTS postgres_fdw;

-- Sanity selects so the provisioner can verify the extension landed.
SELECT pgrdf.version();
