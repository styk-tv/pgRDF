-- pgrdf--0.5.1--0.6.10.sql
--
-- Upgrade-path declaration from v0.5.1 (the earliest installable version) to v0.6.10. PostgreSQL
-- requires this file to exist for `ALTER EXTENSION pgrdf UPDATE TO '0.6.10'` to be a valid path.
--
-- Most v0.5.1 -> v0.6.x deltas are runtime / `.so` changes (the M4 join-order pin, auto-ANALYZE after
-- materialize, the batched materialize write-back, the v0.6.2 parallel bulk loader, the v0.6.3/v0.6.4
-- deferred-index + deferred-constraint path, v0.6.5 parallel in-Rust dedup, v0.6.6 larger quad batch,
-- v0.6.7 concurrency-safe id reservation, v0.6.8 streaming/windowed loader + lenient parse).
--
-- v0.6.10 is the FIRST release in the 0.6.x line with a real SCHEMA change, so this upgrade carries DDL:
--
--   R1 (the dictionary 2704-byte btree fix). The `unique_term` UNIQUE key used to embed the full
--   `lexical_value`; a Wikidata literal longer than PostgreSQL's 2704-byte btree key limit aborts the
--   index build (measured: a 3312-byte literal rolled back an 8.2 B-triple load at the final rebuild).
--   The fix hashes the value into a generated `lexical_md5 BYTEA` (md5, 128-bit, fixed 16 bytes) and
--   keys `unique_term` on that instead. An in-place upgrade MUST add the column + re-key the constraint
--   or the v0.6.10 `.so` (whose bulk-rebuild references `lexical_md5`) breaks against the old schema.
--   The ADD COLUMN computes md5 for existing rows (a table rewrite); for the small v0.5.1-era dicts this
--   is the earliest installable path serves, that is cheap.
--
--   R2 (the staged background-worker loader foundation). Adds the `_pgrdf_staged_ping` proof table used
--   by `pgrdf.load_turtle_staged_ping` to verify the bgworker pool end-to-end.
--
-- The authoritative full surface ships in the base install script `pgrdf--0.6.10.sql`, which a fresh
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
