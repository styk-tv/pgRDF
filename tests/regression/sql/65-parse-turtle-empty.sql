-- 65-parse-turtle-empty.sql
--
-- Edge-case correctness regression — `pgrdf.parse_turtle()` MUST
-- accept *triple-free* Turtle input without panicking and MUST
-- return `0` as the inserted-triple count. Continues the edge-case
-- track opened by `62-materialize-empty.sql` (62 → forward, now at
-- 65 after 63 / 64).
--
-- The parser path in `src/storage/loader.rs::ingest_turtle_with_stats`
-- drives an oxttl `TurtleParser` iterator; the for-loop body that
-- interns dict ids and pushes onto the `batch_s/p/o` vectors runs
-- ONCE PER TRIPLE. Inputs that contain no triples — empty string,
-- whitespace-only, comment-only, bare `@prefix` declarations — yield
-- zero iterator items: the loop body never executes, `stats.triples`
-- stays `0`, the trailing `flush_batch()` flushes empty vectors (no
-- SQL is emitted to `_pgrdf_quads`), and the function returns `0`.
--
-- A refactor that wraps the loop in a "fast-path" that panics on
-- empty input, or that emits a placeholder dictionary row / quad
-- row "to seed the graph", or that mishandles the trailing
-- `flush_batch()` of empty vectors and tries to bind a zero-length
-- array as a non-nullable column, would surface as a regression
-- failure here. The dictionary side of the contract is part of the
-- lock: triple-free input MUST NOT add rows to `_pgrdf_dictionary`,
-- because interning only happens inside the per-triple loop body
-- (the `@prefix` declaration in case 4 is scanned by the parser to
-- expand subsequent CURIEs but does NOT emit a triple, so it does
-- NOT intern the IRI either).
--
-- Six invariants locked (each projects a single boolean):
--
--   1. `parse_turtle('', g)`                  returns `0`
--   2. `parse_turtle(E'   \n   \t  ', g)`     returns `0` (whitespace)
--   3. `parse_turtle(E'# c1\n# c2\n', g)`     returns `0` (comments)
--   4. `parse_turtle('@prefix ex: <…> .', g)` returns `0` (prefix only)
--   5. `count_quads(g) = 0` after all four calls
--   6. `_pgrdf_dictionary` has zero rows after all four calls
--
-- This is the orthogonal correct-path companion to the
-- malformed-input case noted in `81-error-paths.sql` (where the
-- parser panics with `load_turtle: turtle parse error: …`): the
-- six invariants here lock the contract that an EMPTY parser
-- iterator is NOT a parse error — it returns 0 cleanly.
--
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

SELECT pgrdf.add_graph(9959);

-- ── Invariant 1: empty string → 0 triples ─────────────────────────
SELECT pgrdf.parse_turtle('', 9959) AS empty_string \gset
SELECT :empty_string = 0 AS empty_string_returns_zero;

-- ── Invariant 2: whitespace-only → 0 triples ──────────────────────
SELECT pgrdf.parse_turtle(E'   \n   \t  ', 9959) AS whitespace_only \gset
SELECT :whitespace_only = 0 AS whitespace_only_returns_zero;

-- ── Invariant 3: comment-only → 0 triples ─────────────────────────
SELECT pgrdf.parse_turtle(E'# just a comment\n# another\n', 9959)
  AS comment_only \gset
SELECT :comment_only = 0 AS comment_only_returns_zero;

-- ── Invariant 4: bare `@prefix` declaration → 0 triples ───────────
-- The parser still SCANS the directive (a refactor that fails to
-- accept directive-only input would surface here as a parse error),
-- but no triple is emitted, so the loop body never runs.
SELECT pgrdf.parse_turtle('@prefix ex: <http://example.org/> .', 9959)
  AS prefix_only \gset
SELECT :prefix_only = 0 AS prefix_only_returns_zero;

-- ── Invariant 5: no rows landed in `_pgrdf_quads` for graph 9959 ──
SELECT pgrdf.count_quads(9959) = 0 AS quads_after_is_zero;

-- ── Invariant 6: no rows landed in `_pgrdf_dictionary` ────────────
-- Interning only happens inside the per-triple loop body in
-- `ingest_turtle_with_stats`. With zero triples emitted across all
-- four inputs, the dictionary stays empty — including the case 4
-- `@prefix` IRI, which is parser-scope state, not a dict write.
SELECT count(*) = 0 AS dictionary_stayed_empty
  FROM pgrdf._pgrdf_dictionary;

-- ── Cleanup ────────────────────────────────────────────────────
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
