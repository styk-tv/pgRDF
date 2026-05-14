-- 62-materialize-empty.sql
--
-- Edge-case correctness regression — `pgrdf.materialize(N)` on an
-- empty graph (zero base triples) must NOT panic and must return a
-- well-formed JSONB stats object. This opens the edge-case track
-- (slices 62 → onward) below the error-path track (66 → 63) above.
--
-- Three invariants locked here:
--
--   1. `base_triples = 0` when no triples have been added.
--   2. `inferred_triples_written ≥ 0` — OWL 2 RL axiomatic triples
--      (owl:Thing self-statements etc.) are emitted by `reasonable`
--      on the empty input. The exact count is upstream-defined and
--      NOT pinned here; only non-negativity is part of the contract.
--   3. Idempotency — calling materialize twice on the same empty
--      graph wipes the first run's is_inferred=TRUE rows before
--      re-deriving. So run 2's `previous_inferred_dropped` MUST
--      equal run 1's `inferred_triples_written`.
--
-- Companion to `60-materialize-owl-rl.sql` (happy path with base
-- triples + application-level entailments) and
-- `61-materialize-then-sparql.sql` (integration round-trip).
-- All expected values hand-computed; never ACCEPT=1 baselined.

DROP EXTENSION IF EXISTS pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();

SELECT pgrdf.add_graph(9962);

-- ─── Invariant 1+2: base = 0, inferred ≥ 0 on empty graph ─────────
WITH r1 AS (
  SELECT (pgrdf.materialize(9962) - 'elapsed_ms') AS j
)
SELECT (j->>'base_triples')::bigint = 0           AS base_is_zero,
       (j->>'inferred_triples_written')::bigint >= 0 AS inferred_nonneg,
       (j->>'previous_inferred_dropped')::bigint = 0 AS first_run_dropped_zero
  FROM r1;

-- ─── Invariant 3: idempotency across two runs ────────────────────
-- Run materialize twice; the second call's `previous_inferred_dropped`
-- must equal the first call's `inferred_triples_written` because the
-- UDF wipes its own prior is_inferred=TRUE rows before re-deriving.
WITH r1 AS (
  SELECT (pgrdf.materialize(9962)->>'inferred_triples_written')::bigint AS written
),
r2 AS (
  SELECT (pgrdf.materialize(9962)->>'previous_inferred_dropped')::bigint AS dropped
)
SELECT r1.written = r2.dropped AS idempotent
  FROM r1, r2;

-- ─── Cleanup ────────────────────────────────────────────────────
DROP EXTENSION pgrdf CASCADE;
CREATE EXTENSION pgrdf;
SELECT pgrdf.shmem_reset();
SELECT pgrdf.plan_cache_clear();
