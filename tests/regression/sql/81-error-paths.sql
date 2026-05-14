-- 81-error-paths.sql
--
-- Regression signals for the *error-message* contract of the UDF
-- surface — distinct from `80-unsupported-shapes.sql` which locks
-- the failure-mode of SPARQL translator gaps. This file pins the
-- stable error prefix each UDF emits when given an invalid input
-- (missing file, bad graph id, malformed Turtle, etc.). Downstream
-- callers (CloudNativePG operators, client libraries, CI scripts)
-- match on these prefixes; a silent rename would break them
-- without any pgRDF-side test firing.
--
-- Out of scope: the exact tail of SQLERRM — OS-level `os error N`
-- numbers, parser line:col coordinates, `std::io::Error` Display
-- strings — all of which are platform/build/locale dependent. Only
-- the stable Rust-side error prefix our UDF emits IS pinned.
--
-- Like `80`, each check is wrapped in plpgsql `BEGIN ... EXCEPTION
-- ... END;` so the captured baseline is a clean boolean (`t` =
-- expected substring present in SQLERRM); the volatile tail isn't.
--
-- One check per commit. This file accretes locked rows as the
-- 66 → 1 countdown toward v0.3.0 progresses:
--   error-66 load_turtle missing file    → 'load_turtle: failed to open'
--   error-65 load_turtle invalid base IRI → 'load_turtle: invalid base IRI'

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

-- The check helper:
--   * Runs `sql` inside a try/catch via EXECUTE (so it works for
--     arbitrary statements, not just `pgrdf.sparql()` SETOF queries
--     like `_check_gap` in `80`).
--   * If the statement SUCCEEDED, emits `unexpected success` (which
--     fires the diff against the baseline).
--   * If the statement failed AND SQLERRM contains the stable
--     `expected_fragment` substring, emits `t`.
--   * If the statement failed but the SQLERRM message changed shape,
--     emits `f` plus the new SQLERRM (so the diff carries
--     diagnostics).
CREATE OR REPLACE FUNCTION _check_error(label TEXT, sql TEXT, expected_fragment TEXT)
RETURNS TEXT
LANGUAGE plpgsql AS $$
DECLARE
  msg TEXT;
BEGIN
  BEGIN
    EXECUTE sql;
    RETURN format('%s: !!! unexpected success !!!', label);
  EXCEPTION WHEN OTHERS THEN
    msg := SQLERRM;
  END;
  IF position(expected_fragment IN msg) > 0 THEN
    RETURN format('%s: t', label);
  ELSE
    RETURN format('%s: f (got: %s)', label, left(msg, 80));
  END IF;
END
$$;

-- ─── Error 66: load_turtle missing file ──────────────────────────
-- `src/storage/loader.rs::load_turtle` opens the path with
-- `File::open(path)`. The `unwrap_or_else` panics with the literal
-- prefix `load_turtle: failed to open` followed by the path and the
-- underlying `std::io::Error` Display. Downstream tooling matches
-- the prefix to decide whether to retry / surface to the operator;
-- the tail is informational only.
SELECT _check_error(
  'error-66 load_turtle missing file',
  'SELECT * FROM pgrdf.load_turtle(''/nonexistent/path/i/promise.ttl'', 9981);',
  'load_turtle: failed to open'
);

-- ─── Error 65: load_turtle invalid base IRI ──────────────────────
-- `src/storage/loader.rs::ingest_turtle_with_stats` (shared by both
-- `load_turtle` and `parse_turtle`) calls `parser.with_base_iri(base)`
-- and `unwrap_or_else` panics with the literal prefix
-- `load_turtle: invalid base IRI` followed by the bad value Debug-
-- formatted and the underlying oxiri Display. The prefix says
-- `load_turtle:` regardless of which UDF triggered it — that name
-- IS the locked contract surface; the tail (oxiri's specific
-- complaint, e.g. `Invalid IRI code point ' '`) is informational.
--
-- We trigger via `parse_turtle` (not `load_turtle`) so the test does
-- not depend on a fixture file; the panic prefix is identical
-- because both UDFs route through the same ingest function.
-- Empty-string base_iri is filtered out before `with_base_iri` is
-- called (see `loader.rs` `.filter(|s| !s.is_empty())`), so the bad
-- value must be syntactically invalid yet non-empty —
-- `'not an iri at all'` (whitespace + no scheme) fits.
SELECT _check_error(
  'error-65 load_turtle invalid base IRI',
  $$SELECT pgrdf.parse_turtle('@prefix ex: <http://e/> . ex:a ex:b ex:c .', 9982, 'not an iri at all')$$,
  'load_turtle: invalid base IRI'
);

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
