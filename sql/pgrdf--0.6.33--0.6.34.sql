-- pgRDF 0.6.33 -> 0.6.34 — SPEC.pgRDF.LIB.v0.6.34: the E-series lands.
--
-- Behaviour shipped in the .so (no DDL of its own):
--
-- * E0 — every deliberate refusal now carries a semantic SQLSTATE
--   instead of XX000 internal_error: 55P03 lock held · 55000 wrong
--   lifecycle state · 22023 invalid argument/content · 0A000 unsupported
--   construct (incl. the #114 fail-closed FILTER gate) · 42704 unknown
--   graph · 42710 rebinding conflict · 2BP01 drop without cascade over
--   inferred rows · 54000 configured budget exceeded. Refusal MESSAGES
--   are byte-identical to 0.6.33 — message-matching clients are
--   unaffected; code-matching finally works. Genuine internal faults
--   remain XX000, deliberately: class XX means "no verdict was
--   reached", everything else means "considered and declined".
-- * graph_digest / structural_digest of an ABSENT graph now refuse with
--   42704 undefined_object. Previously an absent graph silently
--   digested to sha256 of empty input — indistinguishable from a
--   legitimately EMPTY graph (which still answers, as it should).
--
-- Also in this release:
--
-- * E4 — materialization freshness: `materialize` records when it ran
--   and the asserted count it ran over (two new nullable columns on
--   _pgrdf_graphs); graph_inventory() derives materialization =
--   never | unknown | stale | current at read time. Pre-0.6.34 graphs
--   with inferred rows read 'unknown' until their next materialize —
--   the honest value, never a guess. Stated limit: a write leaving the
--   asserted count unchanged reads 'current'.
-- * #107 completed — SPARQL UPDATE now takes the graph lock fence:
--   INSERT DATA / DELETE into a LOCKED graph previously SUCCEEDED
--   while every other write path refused; it now refuses 55P03.
-- * E3 export_graph(graph_id) — asserted triples as canonical
--   N-Triples, byte-sorted; inferred rows never export (closes #36).
-- * E7 graph_manifest(graph_id) — the portable manifest: three digests
--   each with its method (bytes / rdfc-1.0 / pgrdf-fd1), counts,
--   engine identity, and a mandatory not_carried list.
--
-- New functions (DDL below, from the pgrx-generated install script):
--
-- * E1 graph_inventory() / orphan_partitions() — the supported
--   inventory surface; retires every private-table read consumers were
--   forced into (measured: 54 sites in one consumer alone).
-- * E2 last_call_stats() — per-call completeness figures
--   (path_depth_truncations, filter_clauses_dropped) for the most
--   recent query verb in THIS session; backend-local, so another
--   session's truncation cannot appear here. The cumulative stats()
--   counters remain instance health, never a per-call verdict.
-- * E5 surface() — the classified export list, queryable: name,
--   identity args, stability class (stable/internal/spike/deprecated),
--   note. Capability detection becomes a query instead of an inference
--   from absence.
-- * E6 structural_digest(graph_id) — the fleet's first-degree
--   structural pin (method pgrdf-fd1-sha256), byte-for-byte the
--   algorithm consumers already conform against. DIFFERENT is
--   conclusive; SAME is evidence, never proof — use graph_digest
--   (rdfc-1.0-sha256) where proof is required; the two values are
--   never comparable with each other.

ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS last_materialize_at     TIMESTAMPTZ;
ALTER TABLE _pgrdf_graphs ADD COLUMN IF NOT EXISTS materialized_base_count BIGINT;

CREATE FUNCTION "graph_inventory"() RETURNS TABLE (
    "graph_id" bigint,
    "iri" TEXT,
    "asserted" bigint,
    "inferred" bigint,
    "locked" bool,
    "lock_reason" TEXT,
    "materialization" TEXT
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_inventory_wrapper';

CREATE FUNCTION "export_graph"(
    "graph_id" bigint
) RETURNS SETOF TEXT
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'export_graph_wrapper';

CREATE FUNCTION "graph_manifest"(
    "graph_id" bigint
) RETURNS jsonb
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'graph_manifest_wrapper';

CREATE FUNCTION "orphan_partitions"() RETURNS TABLE (
    "relname" TEXT
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'orphan_partitions_wrapper';

CREATE FUNCTION "last_call_stats"() RETURNS jsonb
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'last_call_stats_wrapper';

CREATE FUNCTION "surface"() RETURNS TABLE (
    "name" TEXT,
    "identity_args" TEXT,
    "class" TEXT,
    "note" TEXT
)
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'surface_wrapper';

CREATE FUNCTION "structural_digest"(
    "graph_id" bigint
) RETURNS TEXT
STRICT
LANGUAGE c /* Rust */
AS 'MODULE_PATHNAME', 'structural_digest_wrapper';
