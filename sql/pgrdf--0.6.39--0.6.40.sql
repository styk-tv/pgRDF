-- pgRDF 0.6.39 -> 0.6.40 — the graph-id sequence follows the storage owner (#153).
--
-- * Graph creation allocates ids from _pgrdf_graph_id_seq as the storage
--   owner (the owner of _pgrdf_quads). The 0.6.39 upgrade created the
--   sequence owned by whoever ran ALTER EXTENSION UPDATE. Where the storage
--   tables had been handed to another role before that upgrade, the storage
--   owner could not use the sequence and every new graph failed 42501 on
--   nextval. The sequence now takes the storage owner. A database where the
--   two already agree (every fresh install) is unchanged.
-- * New export: ownership_drift(), the storage relations not owned by the
--   storage owner, whether each blocks the engine, and the statement that
--   realigns it. Empty when ownership agrees.
--
-- Behaviour changes carried by the .so (no DDL):
-- * graph creation refuses 55000 with HINT
--   `ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO <storage owner>`
--   when the storage owner cannot use the sequence, instead of a bare 42501.
--
-- Every later upgrade script repeats the alignment below: an upgrade must
-- leave the engine's own allocator usable by the storage owner.

DO $pgrdf_153$
DECLARE
  storage_owner oid := (SELECT relowner FROM pg_catalog.pg_class
                         WHERE oid = 'pgrdf._pgrdf_quads'::regclass);
BEGIN
  IF (SELECT relowner FROM pg_catalog.pg_class
       WHERE oid = 'pgrdf._pgrdf_graph_id_seq'::regclass) <> storage_owner THEN
    EXECUTE pg_catalog.format('ALTER SEQUENCE pgrdf._pgrdf_graph_id_seq OWNER TO %I',
                              pg_catalog.pg_get_userbyid(storage_owner));
  END IF;
END
$pgrdf_153$;

CREATE  FUNCTION "ownership_drift"() RETURNS TABLE (
	"relname" TEXT,
	"kind" TEXT,
	"owner" TEXT,
	"storage_owner" TEXT,
	"blocking" bool,
	"cure" TEXT
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'ownership_drift_wrapper';
