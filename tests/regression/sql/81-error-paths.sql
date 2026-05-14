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
-- One check per commit. This commit locks #66 of the 66 → 1
-- countdown toward v0.3.0:
--   error-66 load_turtle missing file → 'load_turtle: failed to open'

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

DROP FUNCTION _check_error(TEXT, TEXT, TEXT);

-- Cleanup
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
