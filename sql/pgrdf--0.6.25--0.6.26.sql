-- pgRDF 0.6.25 -> 0.6.26 — the verification cut (#102, #103, #104).
--
-- One new SQL object; two behaviour changes ride in the library:
--
--   #103  strict validate refusals now RAISE instead of returning
--         {"conforms": null, "error": ...}. No catalog change — but a
--         caller that parsed the in-band refusal must catch the error
--         instead. strict => false keeps the old in-band contract.
--   #102  validate(g, g) warns on self-validation, in the log and in a
--         "warnings" array on the report. No catalog change.
--
-- #104: the integrity probe. Positions are legality-checked against
-- the dictionary: predicate must be an IRI, subject must not be a
-- literal, and every id must resolve to a dictionary row.
CREATE FUNCTION "graph_integrity"("graph_id" bigint) RETURNS jsonb
STRICT
LANGUAGE c
AS 'MODULE_PATHNAME', 'graph_integrity_wrapper';
