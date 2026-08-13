-- pgrdf 0.6.29 -> 0.6.30
--
-- #114 (fail-closed filter custody) lives in the .so: group-level
-- constructs alongside a UNION now refuse instead of being silently
-- dropped, and pgrdf.stats() gains "filter_clauses_dropped". No DDL.
--
-- #115: honest volatility classes so the planner folds one lookup per
-- statement instead of re-evaluating per scanned row (measured 63,833
-- calls of graph_id for a single census before this).
ALTER FUNCTION pgrdf.graph_id(text) STABLE;
ALTER FUNCTION pgrdf.graph_iri(bigint) STABLE;
ALTER FUNCTION pgrdf.version() IMMUTABLE;
ALTER FUNCTION pgrdf.build_id() IMMUTABLE;
