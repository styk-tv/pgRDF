-- pgRDF schema v0.2.0 — bootstrapped from SPEC.pgRDF.LLD.v0.2 §3.
-- Loaded into the install SQL via `extension_sql_file!` in src/lib.rs.

CREATE TABLE IF NOT EXISTS _pgrdf_dictionary (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    term_type       SMALLINT NOT NULL,   -- 1: URI, 2: BlankNode, 3: Literal
    lexical_value   TEXT     NOT NULL,
    datatype_iri_id BIGINT,
    language_tag    TEXT,
    -- R1 (v0.6.9): the uniqueness key hashes the lexical value so the btree key is FIXED-SIZE.
    -- A raw-lexical_value unique btree key exceeds PostgreSQL's 2704-byte limit on long Wikidata
    -- literals (measured: a 3312-byte literal aborted the 8.2B-triple full-truthy load at the final
    -- index rebuild, rolling back the whole transaction). md5 is 128-bit (collision ~1e-20 even at
    -- billion-term scale), built-in (no extension dependency), 16 bytes as bytea. The generated column
    -- is computed by PostgreSQL, so the loader insert paths are unchanged (they list explicit columns).
    lexical_md5     BYTEA GENERATED ALWAYS AS (decode(md5(lexical_value), 'hex')) STORED,
    CONSTRAINT unique_term UNIQUE (term_type, lexical_md5, datatype_iri_id, language_tag)
);

-- HASH index is significantly faster for exact string matching during ingestion.
CREATE INDEX IF NOT EXISTS _pgrdf_dict_val_idx
    ON _pgrdf_dictionary USING HASH (lexical_value);

CREATE TABLE IF NOT EXISTS _pgrdf_quads (
    subject_id   BIGINT NOT NULL,
    predicate_id BIGINT NOT NULL,
    object_id    BIGINT NOT NULL,
    graph_id     BIGINT NOT NULL DEFAULT 0,
    is_inferred  BOOLEAN NOT NULL DEFAULT FALSE
) PARTITION BY LIST (graph_id);

CREATE TABLE IF NOT EXISTS _pgrdf_quads_default
    PARTITION OF _pgrdf_quads DEFAULT;

-- Hexastore covering indexes (Index-Only Scan via INCLUDE).
CREATE INDEX IF NOT EXISTS _pgrdf_idx_spo
    ON _pgrdf_quads (subject_id, predicate_id, object_id) INCLUDE (is_inferred);
CREATE INDEX IF NOT EXISTS _pgrdf_idx_pos
    ON _pgrdf_quads (predicate_id, object_id, subject_id) INCLUDE (is_inferred);
CREATE INDEX IF NOT EXISTS _pgrdf_idx_osp
    ON _pgrdf_quads (object_id, subject_id, predicate_id) INCLUDE (is_inferred);

-- R2.0 — staged-loader background-worker pool proof table. Each spawned worker commits one row from
-- its OWN backend transaction; the coordinator (`pgrdf.load_turtle_staged_ping`) counts rows for its
-- job_id as the end-to-end "every worker ran + committed" proof. Shipped here (not created by the
-- coordinator) so it is committed at CREATE EXTENSION and visible to every worker backend WITHOUT
-- the coordinator holding a TRUNCATE/ACCESS-EXCLUSIVE lock that would deadlock against
-- wait_for_shutdown. Rows are job-scoped; safe to leave between runs. R2.1's real staged pipeline
-- supersedes this; the table is harmless to keep.
CREATE TABLE IF NOT EXISTS _pgrdf_staged_ping (
    job_id      BIGINT NOT NULL,
    worker_slot BIGINT NOT NULL,
    pid         BIGINT NOT NULL,
    noted_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
