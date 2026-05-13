-- pgRDF schema v0.2.0 — bootstrapped from SPEC.pgRDF.LLD.v0.2 §3.
-- Loaded into the install SQL via `extension_sql_file!` in src/lib.rs.

CREATE TABLE IF NOT EXISTS _pgrdf_dictionary (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    term_type       SMALLINT NOT NULL,   -- 1: URI, 2: BlankNode, 3: Literal
    lexical_value   TEXT     NOT NULL,
    datatype_iri_id BIGINT,
    language_tag    TEXT,
    CONSTRAINT unique_term UNIQUE (term_type, lexical_value, datatype_iri_id, language_tag)
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
