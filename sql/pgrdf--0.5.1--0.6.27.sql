-- pgrdf--0.5.1--0.6.20.sql
--
-- Upgrade-path declaration from v0.5.1 (the earliest installable version) to v0.6.20. PostgreSQL
-- requires this file to exist for `ALTER EXTENSION pgrdf UPDATE TO '0.6.20'` to be a valid path.
--
-- Most v0.5.1 -> v0.6.x deltas are runtime / `.so` changes (the M4 join-order pin, auto-ANALYZE after
-- materialize, the batched materialize write-back, the v0.6.2 parallel bulk loader, the v0.6.3/v0.6.4
-- deferred-index + deferred-constraint path, v0.6.5 parallel in-Rust dedup, v0.6.6 larger quad batch,
-- v0.6.7 concurrency-safe id reservation, v0.6.8 streaming/windowed loader + lenient parse).
--
-- The 0.6.x line's first real SCHEMA change landed in v0.6.10 (R1 + R2 below); this cumulative 0.5.1
-- -> 0.6.18 path carries that DDL. v0.6.11 (R2.1) adds only the `load_turtle_staged_run` coordinator
-- FUNCTION (+ the CALL-able `load_turtle_staged` wrapper), v0.6.12 only corrects that loader's
-- literal-dictionary keying, v0.6.13 only hardens the staged worker's panic-reporting + RESOLVE
-- memory, and v0.6.14 only adds the out-of-the-box staged-ingest tuning levers (T1–T5: temp routing,
-- resolve-strategy GUC, parallel STAGE, format dispatch, adaptive self-tune) — all ship in the base
-- `.so` SQL, no new schema. The DDL this upgrade
-- carries (cumulative from v0.5.1):
--
--   R1 (the dictionary 2704-byte btree fix). The `unique_term` UNIQUE key used to embed the full
--   `lexical_value`; a Wikidata literal longer than PostgreSQL's 2704-byte btree key limit aborts the
--   index build (measured: a 3312-byte literal rolled back an 8.2 B-triple load at the final rebuild).
--   The fix hashes the value into a generated `lexical_md5 BYTEA` (md5, 128-bit, fixed 16 bytes) and
--   keys `unique_term` on that instead. An in-place upgrade MUST add the column + re-key the constraint
--   or the v0.6.14 `.so` (whose bulk-rebuild references `lexical_md5`) breaks against the old schema.
--   The ADD COLUMN computes md5 for existing rows (a table rewrite); for the small v0.5.1-era dicts this
--   is the earliest installable path serves, that is cheap.
--
--   R2 (the staged background-worker loader foundation). Adds the `_pgrdf_staged_ping` proof table used
--   by `pgrdf.load_turtle_staged_ping` to verify the bgworker pool end-to-end.
--
--   R2.1 (the staged loader coordinator) — NO schema delta. `pgrdf.load_turtle_staged_run` drives the
--   real STAGE -> DICT -> RESOLVE -> INDEX pipeline over the pool (commit-per-phase lives in the
--   workers' own transactions); `pgrdf.load_turtle_staged` is the CALL-able PROCEDURE wrapper. Both are
--   functions and ship in the base `.so` SQL, so this upgrade carries no DDL for them.
--
--   v0.6.12 (the staged loader literal-dictionary full-key fix) — NO schema delta. The staged loader
--   now keys its literal dictionary on the full literal identity (lexical value + datatype + language),
--   not the lexical value alone, so distinct literals that share a value no longer collapse; the fix is
--   internal to the loader's set-based SQL (runtime / `.so`), carrying no DDL.
--
--   v0.6.13 (staged-worker panic-reporting + RESOLVE memory hardening) — NO schema delta. A staged worker
--   that hits a PostgreSQL ERROR now surfaces the real message instead of an opaque `unknown panic`, and
--   RESOLVE's `work_mem` / `maintenance_work_mem` scale to host RAM instead of a fixed 2 GB so it spills
--   rather than risking OOM; both are runtime / `.so` changes, carrying no DDL.
--
--   v0.6.14 (out-of-the-box at-scale staged ingest, T1–T5) — NO schema delta. Adds the staged-ingest
--   tuning levers: T1 `pgrdf.staged_temp_tablespaces` (route RESOLVE temp spill off PGDATA), T2
--   `pgrdf.staged_resolve_strategy` (hash|index|auto; DEFAULT NOW index — the at-scale-validated
--   low-spill index-nested-loop path), T3 parallel multi-backend STAGE COPY, T4 format-aware staged
--   dispatch, and T5 adaptive self-tune of work_mem/maintenance_work_mem with a self-tune log. All are
--   GUC + runtime / `.so` changes, carrying no DDL.
--
--   v0.6.15 (the staged-loader cross-load corruption fix, issue #8) — NO schema delta. The staged
--   loader's STAGE_PREP worker now detects a non-empty dictionary and falls back to the combined
--   path, so a second load into a populated dict no longer fabricates duplicate dict rows; a
--   runtime / `.so` change, carrying no DDL.
--
--   v0.6.16 (the carve_graph subview UDF, issue #10) — NO schema delta. `pgrdf.carve_graph(src,
--   predicate, dst)` carves a predicated slice of a graph into a new graph in the same database
--   (shared dictionary, no decode) — a new FUNCTION that ships in the base `.so` SQL, carrying no DDL
--   (the same way the v0.6.11 staged-loader coordinator function above ships with no DDL).
--
--   v0.6.17 (the carve_graph neighbourhood overload, issue #30) — NO schema delta. `pgrdf.carve_graph(
--   src, seeds[], dst, max_hops)` carves the K-hop neighbourhood of a seed set into a new graph
--   (shared dictionary, id-space BFS over the source partition) — a new FUNCTION overload that ships in
--   the base `.so` SQL, carrying no DDL.
--
--   v0.6.18 (carve hardening + the pg_dump dictionary fix). #33 adds carve guard /
--   edge-case regression coverage; #32 rewrites the neighbourhood carve EXTRACT to an
--   index-only split UNION (~35x) — both runtime / `.so` + test, NO schema delta. #35
--   is the one DDL delta this release carries (R3 below): it registers
--   `_pgrdf_dictionary` so `pg_dump` includes its row data.
--
--   v0.6.19 (SPARQL expression surface + differential oracle + fail-closed truncation) — NO schema
--   delta. #51 adds `IF` / `ABS` / `ROUND` / `CEIL` / `FLOOR` / `RAND` + the XPath `math#` extension
--   tier (`exp` / `log` / `sqrt` / `pow`) to the executor; #50 lets aggregates take an arbitrary
--   expression or a BIND-produced variable (`SUM(?a * ?b)`); #14 adds the `pgrdf.on_path_truncation`
--   GUC (count | warn | error) so a depth-truncated property-path walk is never a silent partial
--   result. All are executor + GUC (`_PG_init`) + test-harness changes in the base `.so`, carrying no
--   DDL. #17 lands the W3C differential oracle (a standalone test crate, no runtime surface).
--
--   v0.6.20 (pgrx 0.16.1 -> 0.19.1 + PostgreSQL 18, #63) — NO schema delta. Resolves E-006: the
--   framework moves to pgrx 0.19.1 and PG 18 joins the supported matrix. A pure build / toolchain /
--   `.so` change (drop the two-pass `pgrx_embed` bin; Rust 1.96 MSRV; CI + attested release chain
--   retarget PG 18), carrying no DDL and no query-surface change.
--
-- The authoritative full surface ships in the base install script `pgrdf--0.6.20.sql`, which a fresh
-- `CREATE EXTENSION pgrdf` installs. Tables here use unqualified names (the extension schema is in
-- search_path during ALTER EXTENSION UPDATE), matching `sql/schema_v0_2_0.sql`.

-- R1 — generated lexical_md5 + re-keyed unique_term (idempotent; safe to re-run).
ALTER TABLE _pgrdf_dictionary
    ADD COLUMN IF NOT EXISTS lexical_md5 BYTEA
        GENERATED ALWAYS AS (decode(md5(lexical_value), 'hex')) STORED;
ALTER TABLE _pgrdf_dictionary DROP CONSTRAINT IF EXISTS unique_term;
ALTER TABLE _pgrdf_dictionary
    ADD CONSTRAINT unique_term UNIQUE (term_type, lexical_md5, datatype_iri_id, language_tag);

-- R2 — staged-loader background-worker pool proof table.
CREATE TABLE IF NOT EXISTS _pgrdf_staged_ping (
    job_id      BIGINT NOT NULL,
    worker_slot BIGINT NOT NULL,
    pid         BIGINT NOT NULL,
    noted_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- R3 (v0.6.18, #35) — register _pgrdf_dictionary for pg_dump. The base install
-- (schema_v0_4_0_graphs.sql) adds this for a fresh CREATE EXTENSION; this carries
-- it to a v0.5.1 install upgrading directly. Without it pg_dump skips the dict's
-- row data and a restore rebuilds quads pointing at an empty dictionary. O(1),
-- idempotent (appends the table OID to extconfig).
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_dictionary', '');


-- ---------------------------------------------------------------
-- 0.6.22 -> 0.6.25 delta, replayed.
--
-- This bridge is a DIRECT 0.5.1 -> 0.6.25 path. PostgreSQL picks the
-- shortest update route, so an install on 0.5.1 takes this file and
-- never sees pgrdf--0.6.22--0.6.25.sql. Everything that file adds must
-- therefore also appear here, or a 0.5.1 install would land labelled
-- 0.6.25 without build_id() and without the partition ACL backfill.
-- ---------------------------------------------------------------

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


-- ---------------------------------------------------------------
-- 0.6.25 -> 0.6.26 delta, replayed. This bridge is a DIRECT path;
-- postgres takes the shortest route, so a 0.5.1 install never reads
-- pgrdf--0.6.25--0.6.26.sql.
-- ---------------------------------------------------------------
CREATE FUNCTION "graph_integrity"("graph_id" bigint) RETURNS jsonb
STRICT
LANGUAGE c
AS 'MODULE_PATHNAME', 'graph_integrity_wrapper';

-- ---------------------------------------------------------------
-- 0.6.26 -> 0.6.27: no catalog delta (see pgrdf--0.6.26--0.6.27.sql).
-- The bridge target moves so a 0.5.1 install lands on the current
-- version label; nothing further to replay.
-- ---------------------------------------------------------------
