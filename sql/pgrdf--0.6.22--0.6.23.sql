-- pgRDF 0.6.22 -> 0.6.23
--
-- Upgrade path for a database that already carries the extension. Without
-- this file, `ALTER EXTENSION pgrdf UPDATE` fails with "no update path from
-- version 0.6.22 to version 0.6.23" and the only routes forward are a fresh
-- database or DROP/CREATE EXTENSION, which destroys the data.
--
-- Derived from the diff between the released `pgrdf--0.6.22.sql` and the
-- generated `pgrdf--0.6.23.sql`, so it is the actual surface delta rather
-- than a hand-recalled one. Two statements.
--
-- Note for future bumps: a version bump ONLY needs a script when the SQL
-- surface changes. An internal-only rebuild is complete with a library swap,
-- and `extversion` correctly stays where it is.

-- 1. `build_id()` (#92) — reports WHICH build of a version is loaded.
--
-- `version()` returns CARGO_PKG_VERSION, so every build of a version answers
-- the same there; after a library swap it cannot tell you whether the binary
-- now serving queries is the one you installed. This can, because it is
-- compiled in from `git describe --tags --always --dirty`.
--
-- Unqualified on purpose: the extension declares `schema = 'pgrdf'`, so the
-- object lands there, exactly as the generated install SQL declares it.
CREATE FUNCTION "build_id"() RETURNS TEXT /* &'_ str */
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'build_id_wrapper';

-- 2. Invalidate the shared-memory dictionary cache (#88).
--
-- The cache is postmaster-wide and survives this transaction, so slots warmed
-- by the previous binary are still live and still claim to be valid. 0.6.23
-- carries both #88 fixes — the generation bump (#89) and database-scoped
-- fingerprints (#91) — and neither can retroactively invalidate what the
-- OLD binary already interned. Reset once here, at the version boundary,
-- for the same reason the install path resets at CREATE EXTENSION.
SELECT pgrdf.shmem_reset();
