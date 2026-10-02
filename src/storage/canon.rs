//! #117 — RDFC-1.0 canonical graph digest: identity that survives reload.
//!
//! A byte digest over exported N-Triples identifies one stored copy: blank
//! node labels are minted per parse, so the same source loads to different
//! bytes forever (measured: 531/948 of core's triples ride bnodes; two
//! loads of one file differ only in labels). `pgrdf.graph_digest(graph)`
//! answers identity of MEANING: canonical blank-node relabelling per
//! RDFC-1.0 (W3C), canonical N-Quads serialization, sha256. Algorithm
//! label, per the sealed interface contract: `rdfc-1.0-sha256` — values
//! are NOT comparable with first-degree structural pins, by design.
//!
//! Complexity guard: RDFC-1.0 is worst-case exponential on adversarial
//! automorphic bnode structures; the guard RAISES (never degrades) — the
//! fail-closed direction, as everywhere in this engine.
//!
//! Contract (v0.6.32 rechecks, stated before the code):
//!   - same source parsed into two fresh graphs ⇒ equal `graph_digest`,
//!     while their byte serializations differ (label variance);
//!   - genuinely different graphs ⇒ unequal digests (the conclusive
//!     direction);
//!   - W3C rdf-canon conformance subset passes as regression fixtures.

use crate::storage::dict::term_type;
use pgrx::prelude::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};

// ─────────────────────────────────────────────────────────────────────
// RDFC-1.0 (W3C RDF Dataset Canonicalization) over pgRDF's quad store.
// Asserted triples only (inferred is a check value, never content —
// I13). Algorithm label: `rdfc-1.0-sha256`; values are NOT comparable
// with first-degree structural pins, by design.
// ─────────────────────────────────────────────────────────────────────

/// Hard budgets — the complexity guard RAISES, never degrades. RDFC-1.0
/// is worst-case exponential on adversarial automorphic blank-node
/// structures (the W3C suite's "poison" tests expect an abort: refusing
/// IS the conforming behaviour there).
const MAX_NDEGREE_CALLS: usize = 10_000;
const MAX_PERMUTATION_GROUP: usize = 7;
const MAX_RECURSION_DEPTH: usize = 32;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) enum CTerm {
    Iri(String),
    BNode(String),
    Lit {
        val: String,
        dt: Option<String>,
        lang: Option<String>,
    },
}

pub(crate) type Triple = (CTerm, CTerm, CTerm);

/// Canonical N-Triples escaping for the literal lexical form.
fn nt_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

/// Serialize one term; blank nodes go through `label` so callers control
/// the substitution (`_:a`/`_:z` during first-degree hashing, issued
/// canonical ids at the end).
pub(crate) fn nt_term(t: &CTerm, label: &dyn Fn(&str) -> String) -> String {
    match t {
        CTerm::Iri(i) => format!("<{i}>"),
        CTerm::BNode(b) => format!("_:{}", label(b)),
        CTerm::Lit { val, dt, lang } => {
            let esc = nt_escape(val);
            match (lang, dt) {
                (Some(l), _) => format!("\"{esc}\"@{l}"),
                (None, Some(d)) if d != "http://www.w3.org/2001/XMLSchema#string" => {
                    format!("\"{esc}\"^^<{d}>")
                }
                _ => format!("\"{esc}\""),
            }
        }
    }
}

pub(crate) fn nt_triple(t: &Triple, label: &dyn Fn(&str) -> String) -> String {
    format!(
        "{} {} {} .\n",
        nt_term(&t.0, label),
        nt_term(&t.1, label),
        nt_term(&t.2, label)
    )
}

fn sha256_hex(data: &str) -> String {
    let mut h = Sha256::new();
    h.update(data.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Read a graph's ASSERTED triples with full term structure — the same
/// join shape `serialise_graph_to_ntriples` uses (shacl.rs), minus the
/// inferred rows.
pub(crate) fn read_asserted_triples(graph_id: i64) -> Vec<Triple> {
    read_asserted_triples_where(graph_id, "")
}

/// The term-structured triple read behind `graph_digest`, narrowed by an
/// extra SQL condition over the aliases `q` (quad), `s` / `p` / `o`
/// (dictionary rows) — `graph_diff` reads only the blank-node-bearing
/// triples this way. `extra` is engine-built, never user input.
pub(crate) fn read_asserted_triples_where(graph_id: i64, extra: &str) -> Vec<Triple> {
    let mut triples: Vec<Triple> = Vec::new();
    let sql = format!(
        "SELECT
            s.term_type,        s.lexical_value,
            p.lexical_value     AS p_iri,
            o.term_type,        o.lexical_value,
            dt.lexical_value    AS o_dt,
            o.language_tag      AS o_lang
         FROM pgrdf._pgrdf_quads q
         JOIN pgrdf._pgrdf_dictionary s  ON s.id  = q.subject_id
         JOIN pgrdf._pgrdf_dictionary p  ON p.id  = q.predicate_id
         JOIN pgrdf._pgrdf_dictionary o  ON o.id  = q.object_id
         LEFT JOIN pgrdf._pgrdf_dictionary dt ON dt.id = o.datatype_iri_id
         WHERE q.graph_id = $1 AND q.is_inferred = FALSE {extra}"
    );
    Spi::connect(|client| {
        let table = client
            .select(
                &sql,
                None,
                &[unsafe {
                    pgrx::datum::DatumWithOid::new(
                        graph_id,
                        pgrx::pg_sys::PgBuiltInOids::INT8OID.into(),
                    )
                }],
            )
            .expect("graph_digest: triple read failed");
        for row in table {
            let s_type: i16 = row
                .get(1)
                .ok()
                .flatten()
                .expect("graph_digest: s.term_type");
            let s_val: String = row.get(2).ok().flatten().expect("graph_digest: s.value");
            let p_iri: String = row.get(3).ok().flatten().expect("graph_digest: p.iri");
            let o_type: i16 = row
                .get(4)
                .ok()
                .flatten()
                .expect("graph_digest: o.term_type");
            let o_val: String = row.get(5).ok().flatten().expect("graph_digest: o.value");
            let o_dt: Option<String> = row.get(6).ok().flatten();
            let o_lang: Option<String> = row.get(7).ok().flatten();

            let s = match s_type {
                term_type::URI => CTerm::Iri(s_val),
                term_type::BLANK_NODE => CTerm::BNode(s_val),
                _ => continue, // literal subject — #88 residue, not canonicalizable
            };
            let p = CTerm::Iri(p_iri);
            let o = match o_type {
                term_type::URI => CTerm::Iri(o_val),
                term_type::BLANK_NODE => CTerm::BNode(o_val),
                term_type::LITERAL => CTerm::Lit {
                    val: o_val,
                    dt: o_dt,
                    lang: o_lang,
                },
                _ => continue,
            };
            triples.push((s, p, o));
        }
    });
    triples
}

/// RDFC-1.0 identifier issuer: stable prefix + counter, remembering
/// issue order (the order canonical ids are handed out in matters for
/// hash-n-degree results).
#[derive(Clone)]
struct Issuer {
    prefix: String,
    counter: usize,
    issued: HashMap<String, String>,
    order: Vec<String>,
}

impl Issuer {
    fn new(prefix: &str) -> Self {
        Issuer {
            prefix: prefix.to_string(),
            counter: 0,
            issued: HashMap::new(),
            order: Vec::new(),
        }
    }
    fn issue(&mut self, id: &str) -> String {
        if let Some(v) = self.issued.get(id) {
            return v.clone();
        }
        let v = format!("{}{}", self.prefix, self.counter);
        self.counter += 1;
        self.issued.insert(id.to_string(), v.clone());
        self.order.push(id.to_string());
        v
    }
    fn issued_for(&self, id: &str) -> Option<&String> {
        self.issued.get(id)
    }
}

struct CanonState {
    triples: Vec<Triple>,
    bnode_quads: HashMap<String, Vec<usize>>,
    ndegree_calls: usize,
}

impl CanonState {
    /// 4.6 Hash First Degree Quads: serialize every quad mentioning `n`
    /// with `n → _:a` and every other bnode `→ _:z`; sort; hash.
    fn hash_first_degree(&self, n: &str) -> String {
        let mut lines: Vec<String> = self.bnode_quads[n]
            .iter()
            .map(|&i| {
                nt_triple(&self.triples[i], &|b: &str| {
                    if b == n {
                        "a".to_string()
                    } else {
                        "z".to_string()
                    }
                })
            })
            .collect();
        lines.sort();
        sha256_hex(&lines.concat())
    }

    /// 4.7 Hash Related Blank Node.
    fn hash_related(
        &mut self,
        related: &str,
        quad_idx: usize,
        issuer: &Issuer,
        canonical: &Issuer,
        position: char,
    ) -> String {
        let mut input = String::new();
        input.push(position);
        if position != 'g' {
            input.push('<');
            if let CTerm::Iri(p) = &self.triples[quad_idx].1 {
                input.push_str(p);
            }
            input.push('>');
        }
        if let Some(c) = canonical.issued_for(related) {
            input.push_str("_:");
            input.push_str(c);
        } else if let Some(t) = issuer.issued_for(related) {
            input.push_str("_:");
            input.push_str(t);
        } else {
            input.push_str(&self.hash_first_degree(related));
        }
        sha256_hex(&input)
    }

    /// 4.8 Hash N-Degree Quads — the gossip-path tie-breaker, with the
    /// fail-closed budget.
    fn hash_n_degree(
        &mut self,
        id: &str,
        issuer: Issuer,
        canonical: &Issuer,
        depth: usize,
    ) -> (String, Issuer) {
        self.ndegree_calls += 1;
        if depth > MAX_RECURSION_DEPTH || self.ndegree_calls > MAX_NDEGREE_CALLS {
            error!(
                "pgRDF#117: canonicalization budget exceeded (adversarial blank-node \
                 structure); refusing rather than degrading"
            );
        }
        let mut issuer = issuer;
        // Group related bnodes by their related-hash.
        let mut hn: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let quad_ids = self.bnode_quads[id].clone();
        for qi in quad_ids {
            let (s, _p, o) = self.triples[qi].clone();
            for (t, pos) in [(s, 's'), (o, 'o')] {
                if let CTerm::BNode(b) = t
                    && b != id
                {
                    let h = self.hash_related(&b, qi, &issuer, canonical, pos);
                    let e = hn.entry(h).or_default();
                    if !e.contains(&b) {
                        e.push(b);
                    }
                }
            }
        }
        let mut data = String::new();
        for (related_hash, group) in hn {
            data.push_str(&related_hash);
            if group.len() > MAX_PERMUTATION_GROUP {
                error!(
                    "pgRDF#117: canonicalization budget exceeded (adversarial blank-node \
                     structure); refusing rather than degrading"
                );
            }
            let mut chosen_path = String::new();
            let mut chosen_issuer: Option<Issuer> = None;
            for perm in permutations(&group) {
                let mut copy = issuer.clone();
                let mut path = String::new();
                let mut recursion: Vec<String> = Vec::new();
                let mut aborted = false;
                for related in &perm {
                    if let Some(c) = canonical.issued_for(related) {
                        path.push_str("_:");
                        path.push_str(c);
                    } else {
                        if copy.issued_for(related).is_none() {
                            recursion.push(related.clone());
                        }
                        path.push_str("_:");
                        path.push_str(&copy.issue(related));
                    }
                    if !chosen_path.is_empty()
                        && path.len() >= chosen_path.len()
                        && path > chosen_path
                    {
                        aborted = true;
                        break;
                    }
                }
                if aborted {
                    continue;
                }
                for related in &recursion {
                    let (rh, ri) = self.hash_n_degree(related, copy.clone(), canonical, depth + 1);
                    path.push_str("_:");
                    path.push_str(&copy.issue(related));
                    path.push('<');
                    path.push_str(&rh);
                    path.push('>');
                    copy = ri;
                    if !chosen_path.is_empty()
                        && path.len() >= chosen_path.len()
                        && path > chosen_path
                    {
                        aborted = true;
                        break;
                    }
                }
                if aborted {
                    continue;
                }
                if chosen_path.is_empty() || path < chosen_path {
                    chosen_path = path;
                    chosen_issuer = Some(copy);
                }
            }
            data.push_str(&chosen_path);
            issuer = chosen_issuer.unwrap_or(issuer);
        }
        (sha256_hex(&data), issuer)
    }
}

fn permutations(items: &[String]) -> Vec<Vec<String>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for (i, x) in items.iter().enumerate() {
        let mut rest: Vec<String> = items.to_vec();
        rest.remove(i);
        for mut p in permutations(&rest) {
            let mut v = vec![x.clone()];
            v.append(&mut p);
            out.push(v);
        }
    }
    out
}

/// Canonicalize the asserted triples of `graph_id` per RDFC-1.0 and
/// return the sha256 (hex) of the sorted canonical N-Triples document.
/// Algorithm label: `rdfc-1.0-sha256`. Isomorphic graphs — same meaning,
/// any blank-node labels — produce EQUAL digests; unequal digests prove
/// the graphs differ. The complexity guard raises `pgRDF#117` on
/// adversarial structures rather than degrading.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn graph_digest(graph_id: i64) -> String {
    // T-NULL-1/2 (LIB v0.6.34, finding L9): an ABSENT graph REFUSES —
    // 42704 undefined_object — instead of silently hashing zero triples.
    // Measured 2026-08-25: a typo'd id and a legitimately EMPTY graph
    // both returned sha256 of empty input (e3b0c44…), indistinguishable
    // in the identity plane — the one place silence is most expensive.
    // Graph 0 (the default graph) always exists and is exempt; every
    // other id must hold a `_pgrdf_graphs` row (Slice 119 guarantees
    // one for every add_graph path, including the raw-id form).
    if graph_id != 0 {
        let registered: bool = Spi::get_one_with_args(
            "SELECT EXISTS(SELECT 1 FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)",
            &[graph_id.into()],
        )
        .expect("graph_digest: registry existence check failed")
        .unwrap_or(false);
        if !registered {
            crate::refuse(
                pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
                format!(
                    "graph_digest: graph {graph_id} does not exist — an absent graph \
                     has no digest (an EMPTY graph does: pgrdf.add_graph it first)"
                ),
            );
        }
    }
    // 0.6.37 (#142/#143): a locked graph whose lock custody holds — the
    // engine fence plus both partition triggers — cannot change, so its
    // digest is cached on first computation (lazily: locking stays cheap)
    // and served from the cache after. Custody drift (a trigger dropped by
    // hand) means "do not trust the cache": recompute. Open graphs are
    // never cached. A caller who cannot write the cache — read-only
    // transaction, no UPDATE on _pgrdf_graphs — still gets the digest.
    let (locked, cached): (Option<bool>, Option<String>) = Spi::get_two_with_args(
        "SELECT locked, locked_digest FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
        &[graph_id.into()],
    )
    .unwrap_or((None, None));
    let custody_holds =
        locked == Some(true) && crate::storage::lock::lock_trigger_count(graph_id) == Some(2);
    if custody_holds && let Some(d) = cached {
        return d;
    }
    let digest = canonicalize(read_asserted_triples(graph_id)).1;
    if custody_holds {
        let can_cache = Spi::get_one::<bool>(
            "SELECT current_setting('transaction_read_only') = 'off' \
                AND has_table_privilege('pgrdf._pgrdf_graphs', 'UPDATE')",
        )
        .expect("graph_digest: cache-write check failed")
        .unwrap_or(false);
        if can_cache {
            Spi::run_with_args(
                "UPDATE pgrdf._pgrdf_graphs SET locked_digest = $2 \
                 WHERE graph_id = $1 AND locked",
                &[graph_id.into(), digest.as_str().into()],
            )
            .expect("graph_digest: cache write failed");
        }
    }
    digest
}

/// RDFC-1.0 over an arbitrary triple set: returns the sorted canonical
/// N-Triples lines (each `\n`-terminated, blank nodes labelled `_:c14nN`)
/// and the `rdfc-1.0-sha256` digest of their concatenation. This is the
/// algorithm `graph_digest` runs over a whole graph, factored out so a
/// caller can canonicalize a SUBSET — `graph_diff` canonicalizes each
/// blank-node component on its own (SPEC 0.6.37 §3.5). The complexity
/// budget is per call: an adversarial input raises `pgRDF#117`.
pub(crate) fn canonicalize(triples: Vec<Triple>) -> (Vec<String>, String) {
    let (lines, digest, _labels) = canonicalize_labeled(triples);
    (lines, digest)
}

/// [`canonicalize`] plus the label map: each input blank-node label →
/// its canonical `c14nN` label. `graph_diff` renders a component's rows
/// with these labels.
pub(crate) fn canonicalize_labeled(
    triples: Vec<Triple>,
) -> (Vec<String>, String, HashMap<String, String>) {
    let mut bnode_quads: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, t) in triples.iter().enumerate() {
        for term in [&t.0, &t.2] {
            if let CTerm::BNode(b) = term {
                bnode_quads.entry(b.clone()).or_default().push(i);
            }
        }
    }
    let mut state = CanonState {
        triples,
        bnode_quads,
        ndegree_calls: 0,
    };
    let mut canonical = Issuer::new("c14n");

    // Steps 3–4: first-degree hashes; issue canonical ids for uniques in
    // hash order.
    let bnodes: Vec<String> = state.bnode_quads.keys().cloned().collect();
    let mut by_hash: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for b in &bnodes {
        by_hash
            .entry(state.hash_first_degree(b))
            .or_default()
            .push(b.clone());
    }
    let mut shared: Vec<(String, Vec<String>)> = Vec::new();
    for (h, mut group) in by_hash {
        if group.len() == 1 {
            canonical.issue(&group[0]);
        } else {
            group.sort();
            shared.push((h, group));
        }
    }
    // Step 5: ties via hash-n-degree; results in hash order, canonical
    // ids issued in each result issuer's issue order.
    for (_h, group) in shared {
        let mut results: Vec<(String, Issuer)> = Vec::new();
        for b in &group {
            if canonical.issued_for(b).is_some() {
                continue;
            }
            let mut temp = Issuer::new("b");
            temp.issue(b);
            let (hash, temp_issuer) = state.hash_n_degree(b, temp, &canonical, 0);
            results.push((hash, temp_issuer));
        }
        results.sort_by(|a, b| a.0.cmp(&b.0));
        for (_hash, temp_issuer) in results {
            for old in &temp_issuer.order {
                canonical.issue(old);
            }
        }
    }

    // Final: serialize with canonical labels, sort, hash.
    let mut lines: Vec<String> = state
        .triples
        .iter()
        .map(|t| {
            nt_triple(t, &|b: &str| {
                canonical
                    .issued_for(b)
                    .cloned()
                    .unwrap_or_else(|| format!("MISSING-{b}"))
            })
        })
        .collect();
    lines.sort();
    let digest = sha256_hex(&lines.concat());
    (lines, digest, canonical.issued.clone())
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// Two anonymous bnodes — each parse mints fresh internal labels, so
    /// byte-level content differs across loads while structure is fixed.
    const BNODE_TTL: &str = "@prefix c: <urn:c:> .\n[ c:p c:o1 ] .\n[ c:p c:o2 ] .\n";

    fn seed(graph_id: i64, ttl: &str) {
        Spi::run(&format!("SELECT pgrdf.add_graph({graph_id})")).expect("add_graph failed");
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[ttl.into(), graph_id.into()],
        )
        .expect("seed parse failed");
    }

    fn digest(graph_id: i64) -> String {
        Spi::get_one_with_args("SELECT pgrdf.graph_digest($1)", &[graph_id.into()])
            .expect("graph_digest failed")
            .expect("graph_digest returned NULL")
    }

    /// The core promise: canonical identity survives the reload that a
    /// fork, a spore-germination, or a plain re-load necessarily is.
    #[pg_test]
    fn reload_equality_survives_relabelling() {
        seed(982201, BNODE_TTL);
        seed(982202, BNODE_TTL);
        let d1 = digest(982201);
        let d2 = digest(982202);
        assert_eq!(d1, d2, "isomorphic graphs must share a canonical digest");
        assert_eq!(d1.len(), 64, "sha256 hex");
    }

    /// The conclusive direction: different meaning, different digest.
    #[pg_test]
    fn different_graphs_differ() {
        seed(982203, BNODE_TTL);
        seed(
            982204,
            "@prefix c: <urn:c:> .\n[ c:p c:o1 ] .\n[ c:q c:o2 ] .\n",
        );
        assert_ne!(digest(982203), digest(982204));
    }

    /// W3C rdf-canon conformance subset (vendored; attribution in
    /// tests/fixtures/rdfc10/LICENSE.md). Each case loads the suite's
    /// input and asserts our digest equals sha256 of the suite's OWN
    /// expected canonical document — byte-for-byte: our canonical
    /// serialization is the spec's, or this fails naming the case.
    #[pg_test]
    fn w3c_rdfc10_conformance_subset() {
        const CASES: &[(&str, &str, &str)] = &[
            (
                "001",
                include_str!("../../tests/fixtures/rdfc10/test001-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test001-rdfc10.nq"),
            ),
            (
                "002",
                include_str!("../../tests/fixtures/rdfc10/test002-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test002-rdfc10.nq"),
            ),
            (
                "003",
                include_str!("../../tests/fixtures/rdfc10/test003-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test003-rdfc10.nq"),
            ),
            (
                "004",
                include_str!("../../tests/fixtures/rdfc10/test004-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test004-rdfc10.nq"),
            ),
            (
                "005",
                include_str!("../../tests/fixtures/rdfc10/test005-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test005-rdfc10.nq"),
            ),
            (
                "008",
                include_str!("../../tests/fixtures/rdfc10/test008-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test008-rdfc10.nq"),
            ),
            (
                "009",
                include_str!("../../tests/fixtures/rdfc10/test009-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test009-rdfc10.nq"),
            ),
            (
                "010",
                include_str!("../../tests/fixtures/rdfc10/test010-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test010-rdfc10.nq"),
            ),
            (
                "017",
                include_str!("../../tests/fixtures/rdfc10/test017-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test017-rdfc10.nq"),
            ),
            (
                "020",
                include_str!("../../tests/fixtures/rdfc10/test020-in.nq"),
                include_str!("../../tests/fixtures/rdfc10/test020-rdfc10.nq"),
            ),
        ];
        for (i, (name, input, expected)) in CASES.iter().enumerate() {
            let gid = 983300 + i as i64;
            Spi::run(&format!("SELECT pgrdf.add_graph({gid})")).expect("add_graph failed");
            Spi::get_one_with_args::<i64>(
                "SELECT pgrdf.parse_turtle($1, $2)",
                &[(*input).into(), gid.into()],
            )
            .expect("fixture load failed");
            let got = digest(gid);
            let want = super::sha256_hex(expected);
            assert_eq!(got, want, "W3C rdfc10 test{name} diverged from the suite");
        }
    }

    /// The extracted `canonicalize` is the same algorithm `graph_digest`
    /// runs, exposed for per-component use (graph_diff, 0.6.37 §3.5): its
    /// sorted canonical lines must equal the W3C suite's expected document
    /// line-for-line, and its digest must equal `graph_digest` of the same
    /// graph.
    #[pg_test]
    fn canonicalize_matches_w3c_lines_and_graph_digest() {
        let input = include_str!("../../tests/fixtures/rdfc10/test017-in.nq");
        let expected = include_str!("../../tests/fixtures/rdfc10/test017-rdfc10.nq");
        let gid = 983399;
        Spi::run(&format!("SELECT pgrdf.add_graph({gid})")).expect("add_graph failed");
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[input.into(), gid.into()],
        )
        .expect("fixture load failed");
        let (lines, digest_hex) = super::canonicalize(super::read_asserted_triples(gid));
        let want: Vec<String> = expected.lines().map(|l| format!("{l}\n")).collect();
        assert_eq!(
            lines, want,
            "canonical lines must equal the W3C expected document"
        );
        assert_eq!(
            digest_hex,
            digest(gid),
            "canonicalize digest must equal graph_digest"
        );
    }

    fn cached_digest(graph_id: i64) -> Option<String> {
        Spi::get_one_with_args(
            "SELECT locked_digest FROM pgrdf._pgrdf_graphs WHERE graph_id = $1",
            &[graph_id.into()],
        )
        .unwrap()
    }

    /// #142/#143 (0.6.37): a locked graph cannot change, so its digest is
    /// cached on first computation and served from the cache after; unlock
    /// clears it. Open graphs are never cached.
    #[pg_test]
    fn digest_is_cached_while_locked_and_cleared_on_unlock() {
        seed(983501, BNODE_TTL);
        let open = digest(983501);
        assert_eq!(cached_digest(983501), None, "open graphs are never cached");
        Spi::run("SELECT pgrdf.lock_graph(983501, 'checkpoint')").unwrap();
        let first = digest(983501);
        assert_eq!(first, open, "locking does not change identity");
        assert_eq!(cached_digest(983501).as_deref(), Some(first.as_str()));
        assert_eq!(digest(983501), first, "served from the cache");
        Spi::run("SELECT pgrdf.unlock_graph(983501, 'reopen')").unwrap();
        assert_eq!(cached_digest(983501), None, "unlock clears the cache");
    }

    /// The cache is trusted only while lock custody holds. If an owner
    /// drops a lock trigger and changes the graph, graph_digest recomputes
    /// instead of serving the stale value.
    #[pg_test]
    fn cached_digest_is_not_trusted_after_custody_drift() {
        seed(983502, BNODE_TTL);
        Spi::run("SELECT pgrdf.lock_graph(983502, 'checkpoint')").unwrap();
        let cached = digest(983502);
        Spi::run("DROP TRIGGER pgrdf_lock_row ON pgrdf._pgrdf_quads_g983502").unwrap();
        Spi::run(
            "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred) \
             SELECT subject_id, predicate_id, predicate_id, graph_id, false \
             FROM pgrdf._pgrdf_quads WHERE graph_id = 983502 LIMIT 1",
        )
        .unwrap();
        assert_ne!(
            digest(983502),
            cached,
            "drift: recompute, never serve stale"
        );
    }

    /// A caller without UPDATE on _pgrdf_graphs still gets the digest of a
    /// locked graph; the cache is simply not written.
    #[pg_test]
    fn digest_of_locked_graph_works_for_a_reader() {
        seed(983503, BNODE_TTL);
        Spi::run("SELECT pgrdf.lock_graph(983503, 'checkpoint')").unwrap();
        Spi::run("CREATE ROLE pgrdf_digest_reader NOLOGIN").unwrap();
        Spi::run("GRANT USAGE ON SCHEMA pgrdf TO pgrdf_digest_reader").unwrap();
        Spi::run(
            "GRANT SELECT ON pgrdf._pgrdf_quads, pgrdf._pgrdf_graphs, pgrdf._pgrdf_dictionary \
             TO pgrdf_digest_reader",
        )
        .unwrap();
        Spi::run("GRANT SELECT ON pgrdf._pgrdf_quads_g983503 TO pgrdf_digest_reader").unwrap();
        Spi::run("SET ROLE pgrdf_digest_reader").unwrap();
        let d = digest(983503);
        Spi::run("RESET ROLE").unwrap();
        assert_eq!(d.len(), 64);
        assert_eq!(
            cached_digest(983503),
            None,
            "no UPDATE privilege: no cache write"
        );
    }

    /// test074 — the poison graph (RDFC10NegativeEvalTest): a highly
    /// automorphic structure where completing normally under resource
    /// limits is NON-conforming. Our budgets raise — refusing IS the
    /// spec's expected behaviour here, and the fail-closed doctrine and
    /// the conformance requirement are the same sentence.
    #[pg_test(
        error = "pgRDF#117: canonicalization budget exceeded (adversarial blank-node structure); refusing rather than degrading"
    )]
    fn w3c_rdfc10_poison_refuses() {
        let input = include_str!("../../tests/fixtures/rdfc10/test074-in.nq");
        Spi::run("SELECT pgrdf.add_graph(983399)").expect("add_graph failed");
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[input.into(), 983399i64.into()],
        )
        .expect("poison load failed");
        digest(983399);
    }

    /// T-NULL-1 negative control (LIB K10): digesting an ABSENT graph
    /// refuses with 42704 undefined_object — never the sha256 of empty
    /// input. Code asserted by ENUM. No SPI after the catch.
    #[pg_test]
    fn graph_digest_absent_graph_refuses_undefined_object() {
        use pgrx::pg_sys::errcodes::PgSqlErrorCode;
        use pgrx::pg_sys::panic::CaughtError;
        let code = pgrx::PgTryBuilder::new(|| {
            Spi::run("SELECT pgrdf.graph_digest(983888)").unwrap();
            None
        })
        .catch_others(|e| match &e {
            CaughtError::PostgresError(r)
            | CaughtError::ErrorReport(r)
            | CaughtError::RustPanic { ereport: r, .. } => Some(r.sql_error_code()),
        })
        .execute();
        assert_eq!(
            code,
            Some(PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT),
            "absent graph must refuse 42704, never silently digest nothing"
        );
    }

    /// T-NULL-2 companion: an EMPTY (registered) graph still digests —
    /// a digest is an ANSWER there. Empty and absent must be
    /// distinguishable: one answers, the other refuses.
    #[pg_test]
    fn graph_digest_empty_graph_still_digests() {
        Spi::run("SELECT pgrdf.add_graph(983889)").expect("add_graph failed");
        let d = digest(983889);
        assert_eq!(d.len(), 64, "empty graph digest is a real sha256 answer");
    }
}
