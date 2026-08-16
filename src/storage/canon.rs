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
enum CTerm {
    Iri(String),
    BNode(String),
    Lit {
        val: String,
        dt: Option<String>,
        lang: Option<String>,
    },
}

type Triple = (CTerm, CTerm, CTerm);

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
fn nt_term(t: &CTerm, label: &dyn Fn(&str) -> String) -> String {
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

fn nt_triple(t: &Triple, label: &dyn Fn(&str) -> String) -> String {
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
fn read_asserted_triples(graph_id: i64) -> Vec<Triple> {
    let mut triples: Vec<Triple> = Vec::new();
    Spi::connect(|client| {
        let table = client
            .select(
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
                 WHERE q.graph_id = $1 AND q.is_inferred = FALSE",
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
    let triples = read_asserted_triples(graph_id);
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
    sha256_hex(&lines.concat())
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
}
