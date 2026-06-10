-- 133-verbose-path-field.sql
--
-- TA-5 correctness gate. v0.5.41 adds a `path` field to the
-- verbose-ingest JSONB output of the dispatched ingest UDFs
-- (`parse_turtle_verbose` / `load_turtle_verbose` / `parse_trig` /
-- `parse_nquads`). The field reports which `pgrdf.ingest_dict_path`
-- route the dispatcher actually selected for the call —
-- `baseline` / `batched` / `shmem_warm` / `combined`. This lets
-- callers (benchmark harness, operators) confirm the route a given
-- ingest took without inferring it from timing.
--
-- This test sets each GUC value in turn and asserts the verbose
-- JSONB echoes it back, for both the Turtle path (parse_turtle_verbose)
-- and the quad path (parse_nquads + parse_trig). It also locks the
-- default (`combined`) and the unrecognised-value fallback.
--
-- Expected output: 8 boolean assertions all evaluating to `t`.

SET client_min_messages = WARNING;
CREATE EXTENSION IF NOT EXISTS pgrdf;

BEGIN;

-- ─── Turtle path echoes each of the four routes ─────────────────
SET LOCAL pgrdf.ingest_dict_path = 'baseline';
SELECT (
  pgrdf.parse_turtle_verbose('@prefix ex: <http://x/> . ex:a ex:p "1" .',
    pgrdf.add_graph('urn:test/ta-5/t-baseline')) ->> 'path' = 'baseline'
) AS a_turtle_baseline;

SET LOCAL pgrdf.ingest_dict_path = 'batched';
SELECT (
  pgrdf.parse_turtle_verbose('@prefix ex: <http://x/> . ex:a ex:p "2" .',
    pgrdf.add_graph('urn:test/ta-5/t-batched')) ->> 'path' = 'batched'
) AS b_turtle_batched;

SET LOCAL pgrdf.ingest_dict_path = 'shmem_warm';
SELECT (
  pgrdf.parse_turtle_verbose('@prefix ex: <http://x/> . ex:a ex:p "3" .',
    pgrdf.add_graph('urn:test/ta-5/t-shmem')) ->> 'path' = 'shmem_warm'
) AS c_turtle_shmem_warm;

SET LOCAL pgrdf.ingest_dict_path = 'combined';
SELECT (
  pgrdf.parse_turtle_verbose('@prefix ex: <http://x/> . ex:a ex:p "4" .',
    pgrdf.add_graph('urn:test/ta-5/t-combined')) ->> 'path' = 'combined'
) AS d_turtle_combined;

-- ─── Quad path (N-Quads) echoes the route ───────────────────────
SET LOCAL pgrdf.ingest_dict_path = 'baseline';
SELECT (
  pgrdf.parse_nquads('<http://x/a> <http://x/p> "1" .',
    pgrdf.add_graph('urn:test/ta-5/nq-baseline')) ->> 'path' = 'baseline'
) AS e_nquads_baseline;

SET LOCAL pgrdf.ingest_dict_path = 'combined';
SELECT (
  pgrdf.parse_nquads('<http://x/a> <http://x/p> "2" .',
    pgrdf.add_graph('urn:test/ta-5/nq-combined')) ->> 'path' = 'combined'
) AS f_nquads_combined;

-- ─── Quad path (TriG) echoes the route ──────────────────────────
SET LOCAL pgrdf.ingest_dict_path = 'combined';
SELECT (
  pgrdf.parse_trig('@prefix ex: <http://x/> . ex:a ex:p "1" .',
    pgrdf.add_graph('urn:test/ta-5/trig-combined')) ->> 'path' = 'combined'
) AS g_trig_combined;

-- ─── Unrecognised GUC value falls back to combined ──────────────
SET LOCAL pgrdf.ingest_dict_path = 'no-such-path';
SELECT (
  pgrdf.parse_turtle_verbose('@prefix ex: <http://x/> . ex:a ex:p "5" .',
    pgrdf.add_graph('urn:test/ta-5/t-fallback')) ->> 'path' = 'combined'
) AS h_unknown_falls_back_to_combined;

ROLLBACK;
