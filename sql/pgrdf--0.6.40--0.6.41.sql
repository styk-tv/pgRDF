-- pgRDF 0.6.40 -> 0.6.41
--
-- No data is rewritten by this script. graph_ids, partitions and consumer
-- schemas survive.

-- Every upgrade script repeats this alignment (0.6.40, #153): an upgrade
-- must leave the engine's own allocator usable by the storage owner.
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

-- #161: functions that act on state shared by every database on the server
-- are not PUBLIC. stats() is opened with GRANT EXECUTE ON FUNCTION
-- pgrdf.stats() TO <role>; last_call_stats() stays PUBLIC.
REVOKE EXECUTE ON FUNCTION shmem_reset() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION shmem_cache_prewarm(bigint) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION stats() FROM PUBLIC;

-- #152: the registry is dumped without its seed row (graph_id 0). A restore
-- creates that row itself, and the dumped copy made the registry COPY fail,
-- which lost every graph's registry row. Calling the function again replaces
-- the filter.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_graphs', 'WHERE graph_id <> 0');

-- #152: the default partition's rows (the default graph, graph_id 0, and any
-- graph id without its own partition) were not dumped at all.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_quads_default', '');

-- #152: the dictionary's rows were dumped but its id allocator was not, so a
-- restored database handed out dictionary ids from 1 again and the first
-- load of a new term failed 23505 on the primary key.
SELECT pg_catalog.pg_extension_config_dump('_pgrdf_dictionary_id_seq', '');
