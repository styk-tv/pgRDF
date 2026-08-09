-- pgRDF 0.6.22 -> 0.6.25
--
-- Upgrade path for a database that already carries the extension. Without
-- this file, `ALTER EXTENSION pgrdf UPDATE` fails with "no update path from
-- version 0.6.22 to version 0.6.25" and the only routes forward are a fresh
-- database or DROP/CREATE EXTENSION, which destroys the data.
--
-- Derived from the diff between the released `pgrdf--0.6.22.sql` and the
-- generated `pgrdf--0.6.25.sql`, so it is the actual surface delta rather
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
-- by the previous binary are still live and still claim to be valid. 0.6.25
-- carries both #88 fixes — the generation bump (#89) and database-scoped
-- fingerprints (#91) — and neither can retroactively invalidate what the
-- OLD binary already interned. Reset once here, at the version boundary,
-- for the same reason the install path resets at CREATE EXTENSION.
SELECT pgrdf.shmem_reset();

-- 3. Backfill partition grants (#96).
--
-- Postgres does not propagate ACLs to partitions, so every
-- `_pgrdf_quads_g<id>` created before 0.6.25 is owner-only regardless of
-- what was granted on the parent. A downstream SECURITY DEFINER function
-- owned by a non-superuser role reads the parent fine and fails on the
-- partition holding the rows.
--
-- 0.6.25 replicates the parent's ACL at creation time; this pass applies
-- the same rule to partitions that already exist, so an upgrade does not
-- leave the graphs a database already has behind the ones it makes next.
--
-- Copies only what the parent carries. If the parent has no grants
-- (`relacl IS NULL`) this does nothing.
DO $pgrdf_acl_backfill$
DECLARE
  p record;
  r record;
BEGIN
  FOR p IN
    SELECT c.relname AS part
    FROM pg_class c
    JOIN pg_inherits i     ON i.inhrelid = c.oid
    JOIN pg_class parent   ON parent.oid = i.inhparent
    WHERE parent.oid = 'pgrdf._pgrdf_quads'::regclass
  LOOP
    FOR r IN
      SELECT a.privilege_type AS priv,
             CASE WHEN a.grantee = 0 THEN 'PUBLIC'
                  ELSE quote_ident(pg_get_userbyid(a.grantee)) END AS grantee
      FROM pg_class c, aclexplode(c.relacl) a
      WHERE c.oid = 'pgrdf._pgrdf_quads'::regclass
    LOOP
      EXECUTE format('GRANT %s ON pgrdf.%I TO %s', r.priv, p.part, r.grantee);
    END LOOP;
  END LOOP;
END
$pgrdf_acl_backfill$;
