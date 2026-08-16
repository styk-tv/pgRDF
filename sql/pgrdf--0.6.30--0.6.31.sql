-- pgRDF 0.6.30 -> 0.6.31 — the loader records the source byte digest (#118).
--
-- Downstream systems pin the file bytes they loaded (a sourceDigest recorded
-- beside an adoption). Once triples are in the store nothing can recompute
-- file bytes — so the pin was recorded, pattern-checked, and consulted by
-- nothing. The loader is the ONLY party that ever sees the input bytes; from
-- this release every load through the turtle funnel records their sha256.
--
-- Semantics, stated where the DDL lands:
--   source_sha256  digest of the MOST RECENT load's input bytes
--   source_loads   count of recorded loads; > 1 self-reports that whole-graph
--                  byte identity no longer holds (consumers needing content
--                  identity use the canonical graph digest, #117, instead)
--   NULL in both   never recorded — distinct from every possible digest;
--                  pre-existing graphs read NULL after this upgrade
--
-- Not covered by this release (recorded, not implied): the staged loader,
-- the v0.6.2 parallel bulk path, and TriG/N-Quads quad ingest do not yet
-- record — the turtle funnel (parse_/load_turtle family) does.

ALTER TABLE pgrdf._pgrdf_graphs ADD COLUMN IF NOT EXISTS source_sha256 TEXT;
ALTER TABLE pgrdf._pgrdf_graphs ADD COLUMN IF NOT EXISTS source_loads  INTEGER;
