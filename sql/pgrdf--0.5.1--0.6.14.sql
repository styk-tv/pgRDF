-- pgrdf--0.5.1--0.6.14.sql
--
-- Upgrade-path declaration from v0.5.1 (the earliest installable version) to v0.6.14. PostgreSQL
-- requires this file to exist for `ALTER EXTENSION pgrdf UPDATE TO '0.6.14'` to be a valid path.
--
-- Most v0.5.1 -> v0.6.x deltas are runtime / `.so` changes (the M4 join-order pin, auto-ANALYZE after
-- materialize, the batched materialize write-back, the v0.6.2 parallel bulk loader, the v0.6.3/v0.6.4
-- deferred-index + deferred-constraint path, v0.6.5 parallel in-Rust dedup, v0.6.6 larger quad batch,
-- v0.6.7 concurrency-safe id reservation, v0.6.8 streaming/windowed loader + lenient parse).
--
-- The 0.6.x line's first real SCHEMA change landed in v0.6.10 (R1 + R2 below); this cumulative 0.5.1
-- -> 0.6.14 path carries that DDL. v0.6.11 (R2.1) adds only the `load_turtle_staged_run` coordinator
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
-- The authoritative full surface ships in the base install script `pgrdf--0.6.14.sql`, which a fresh
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
