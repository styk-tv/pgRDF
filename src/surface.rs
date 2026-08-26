//! E5 (SPEC.pgRDF.LIB.v0.6.34) — `pgrdf.surface()`: the SQL surface,
//! queryable from the engine itself.
//!
//! Before this, "what can this engine do" was answered by guessing:
//! `to_regprocedure` probes that establish a function EXISTS and can
//! never establish a behaviour changed — which is how a consumer kept
//! medicating against a defect for days after the fix shipped (L8), and
//! how another consumer's weaker digest outlived its need for two
//! releases (L4). Capability detection becomes a query.
//!
//! Surface membership needs BOTH predicates: extension OWNERSHIP
//! (pg_depend 'e' — the pgrdf schema is writable, test helpers live
//! beside the extension) AND the pgrdf NAMESPACE (under `cargo pgrx
//! test` the #[pg_test] functions are extension-owned too, in their
//! own schema — measured when K9-2 first ran in the harness).
//!
//! The classification is AUTHORED JUDGMENT, reviewed, shipped in-crate
//! (`src/surface_manifest.tsv`, `include_str!`-ed so the .so can never
//! disagree with its manifest), and enforced complete by a `#[pg_test]`
//! diffing it against `pg_proc`'s extension-owned rows in BOTH
//! directions (K9-2: an unclassified export fails the suite; a stale
//! row fails it too). Membership is extension OWNERSHIP (`pg_depend`
//! deptype 'e'), never schema — the pgrdf schema is writable and test
//! helpers legitimately live beside the extension (measured when the
//! regression suite's own plpgsql helpers broke a namespace-based
//! census).
//!
//! Classes: `stable` (contract; removal or signature change is a
//! breaking release) · `internal` (reachable but not API) · `spike`
//! (dev artifact; may vanish) · `deprecated` (names its replacement).

use pgrx::prelude::*;

const MANIFEST: &str = include_str!("surface_manifest.tsv");

/// Every export of this extension with its stability class. The
/// version a verb appeared in is recorded in `note` where known
/// (e.g. `graph_digest` → 0.6.32; the five LIB emissions → 0.6.34);
/// an absent version means "not recorded", never a guess.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn surface() -> TableIterator<
    'static,
    (
        name!(name, String),
        name!(identity_args, String),
        name!(class, String),
        name!(note, Option<String>),
    ),
> {
    let rows: Vec<_> = MANIFEST
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|l| {
            let mut f = l.splitn(3, '\t');
            let sig = f.next().unwrap_or_default().trim();
            let class = f.next().unwrap_or_default().trim().to_string();
            let note = f
                .next()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let (name, args) = match sig.find('(') {
                Some(i) => (
                    sig[..i].to_string(),
                    sig[i + 1..sig.len().saturating_sub(1)].to_string(),
                ),
                None => (sig.to_string(), String::new()),
            };
            (name, args, class, note)
        })
        .collect();
    TableIterator::new(rows.into_iter())
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// K9-2, enforced: every extension-owned export is classified, and
    /// no classification row is stale — both directions, zero rows in
    /// each EXCEPT. A new `#[pg_extern]` without a manifest row fails
    /// HERE, which is the build-gate in the only place it can live
    /// (classification needs a database; build.rs has none).
    #[pg_test]
    fn surface_covers_every_export_both_directions() {
        let (unclassified, stale) = Spi::get_two::<i64, i64>(
            "WITH live AS (
                 SELECT p.proname || '(' || pg_get_function_identity_arguments(p.oid) || ')' AS sig
                 FROM pg_proc p
                 JOIN pg_namespace ns ON ns.oid = p.pronamespace
                                     AND ns.nspname = 'pgrdf'
                 JOIN pg_depend d ON d.classid = 'pg_proc'::regclass
                                 AND d.objid = p.oid AND d.deptype = 'e'
                 JOIN pg_extension e ON e.oid = d.refobjid AND e.extname = 'pgrdf'
             ),
             declared AS (
                 SELECT name || '(' || identity_args || ')' AS sig FROM pgrdf.surface()
             )
             SELECT
                 (SELECT count(*) FROM (SELECT sig FROM live EXCEPT SELECT sig FROM declared) u),
                 (SELECT count(*) FROM (SELECT sig FROM declared EXCEPT SELECT sig FROM live) s)",
        )
        .unwrap();
        assert_eq!(
            unclassified,
            Some(0),
            "live exports missing from surface() — classify them (K9-2)"
        );
        assert_eq!(
            stale,
            Some(0),
            "surface() declares exports that no longer exist — prune the manifest"
        );
    }

    /// The classes are a closed set; a typo'd class is not a new class.
    #[pg_test]
    fn surface_classes_are_closed() {
        let bad = Spi::get_one::<i64>(
            "SELECT count(*) FROM pgrdf.surface()
             WHERE class NOT IN ('stable', 'internal', 'spike', 'deprecated')",
        )
        .unwrap();
        assert_eq!(bad, Some(0));
    }
}
