-- pgRDF 0.6.34 -> 0.6.35 — SPEC.pgRDF.v0.6.35: capability custody.
--
-- The release closes one question: pgRDF could not state, provably and
-- discoverably, which SHACL features it enforces.
--
-- New export (created by the .so's generated schema, no DDL here):
--
-- * pgrdf.shacl_capability() — which SHACL features validate enforces,
--   one row per feature of SHACL Core as the W3C Recommendation
--   enumerates it. verdict is enforced / not enforced / unprobed; mode
--   is set only where a feature is evaluated by some validation modes
--   and not others (sh:sparql, evaluated by 'pgrdf' only) and NULL
--   otherwise; measured_on names the build the probes ran against.
--
--   Unprobed features are LISTED, never omitted, so the row count is
--   the denominator. That direction is the point: the capability
--   harness previously measured 17 of 46 features and reported
--   "nothing unenforced", which read as a clean bill of health for the
--   whole surface. A consumer counting rows here cannot mistake a
--   partial measurement for a complete one.
--
-- Behaviour change in validate:
--
-- * sh:oneOrMorePath, sh:zeroOrMorePath and sh:path ( … ) sequences now
--   validate LITERAL values reached through them. Previously a literal
--   endpoint was absent from the value set, so no constraint fired on
--   it, and a literal partway along a multi-hop path discarded the
--   whole value set — including sibling values that were still
--   reachable, which could drop a genuine violation. Root cause was
--   upstream in the shacl crate (rudof-project/rudof#818); pgRDF pins
--   the fix by commit until a release carries it.
--
--   A shapes graph that previously reported conforms:true over data
--   with literals on such a path may now report violations. That is
--   the correction, not a regression: those violations were always
--   real and were not being reported.
--
-- No catalog changes. No migration of existing graphs is required, and
-- no existing report shape changes.

-- The new export. pgrx generates this into the full install script; an
-- upgrade has to carry it explicitly, or `ALTER EXTENSION ... UPDATE`
-- advances the version without creating the function.
CREATE FUNCTION "shacl_capability"() RETURNS TABLE (
	"feature" TEXT,
	"kind" TEXT,
	"spec_section" TEXT,
	"verdict" TEXT,
	"mode" TEXT,
	"measured_on" TEXT
)
STRICT
LANGUAGE c
AS 'MODULE_PATHNAME', 'shacl_capability_wrapper';
