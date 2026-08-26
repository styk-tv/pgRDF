//! E6 (SPEC.pgRDF.LIB.v0.6.34) — `pgrdf.structural_digest`: the fleet's
//! first-degree structural pin, in the engine that owns the graphs.
//!
//! Method label: **`pgrdf-fd1-sha256`**. This is the algorithm at least
//! three consumers already implement and conform against each other
//! (finding L7: the load-bearing interop digest existed everywhere
//! EXCEPT here, so every consumer reimplemented it and one published a
//! method string citing a function that did not exist). The engine now
//! owns the number; the algorithm below is reproduced **byte-for-byte**
//! from the fleet's reference implementation, including its escaping —
//! which is deliberately MINIMAL and NOT this crate's canonical
//! N-Triples escaping (`canon::nt_escape` also escapes `\r`/`\t`; the
//! fleet renderer does not, and a digest that "improved" the escaping
//! would silently diverge from every existing consumer — L7 again,
//! wearing a conformant-sounding name):
//!
//! ```text
//! render   IRI <v> · bnode _:label · literal "v" with \→\\ and "→\"
//!          (object position additionally \n→\n, plus @lang, or ^^<dt>
//!          suppressed for xsd:string; suffix on OBJECT position only)
//! ground   lines with NO blank node, byte-sorted, joined \n ('' if none)
//! sig(b)   sha256 hex over b's incident lines with b→_:a and every
//!          other bnode→_:z, byte-sorted, joined \n
//! digest   sha256 hex over: ground ++ "\n--\n" ++ sig hexes byte-sorted
//!          joined \n
//! ```
//!
//! Asserted triples only (inferred is a check value, never content).
//!
//! THE ASYMMETRY IS NORMATIVE (LIB K6): unequal digests PROVE the graphs
//! differ; equal digests are EVIDENCE of isomorphism, never proof —
//! first-degree signing covers each node's immediate neighbourhood only,
//! so symmetric structures collide (constructed minimal case: a 4-cycle
//! of blank nodes vs two 2-cycles — 8 triples, non-isomorphic, equal
//! fd1). Where proof is required, `pgrdf.graph_digest` (rdfc-1.0-sha256)
//! is the answer; the two values are never comparable with each other.

use crate::storage::canon::{CTerm, Triple, read_asserted_triples};
use pgrx::prelude::*;
use sha2::{Digest, Sha256};

/// Fleet-minimal literal escaping: backslash and quote always; newline
/// only when `object_position`. Byte-for-byte the reference renderer.
fn fleet_escape(s: &str, object_position: bool) -> String {
    let mut out = s.replace('\\', "\\\\").replace('"', "\\\"");
    if object_position {
        out = out.replace('\n', "\\n");
    }
    out
}

/// Render one term in the fleet line format. `bnode` maps a blank-node
/// label to its substitution (the real label for ground-detection is
/// never rendered — ground lines contain no bnodes by construction).
fn fleet_term(t: &CTerm, object_position: bool, bnode: &dyn Fn(&str) -> String) -> String {
    match t {
        CTerm::Iri(i) => format!("<{i}>"),
        CTerm::BNode(b) => format!("_:{}", bnode(b)),
        CTerm::Lit { val, dt, lang } => {
            let esc = fleet_escape(val, object_position);
            if !object_position {
                return format!("\"{esc}\"");
            }
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

fn sha256_hex(data: &str) -> String {
    let mut h = Sha256::new();
    h.update(data.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn is_bnode(t: &CTerm) -> bool {
    matches!(t, CTerm::BNode(_))
}

fn bnode_label(t: &CTerm) -> Option<&str> {
    match t {
        CTerm::BNode(b) => Some(b.as_str()),
        _ => None,
    }
}

/// The fd1 computation over an already-read triple set. Pure — no SPI,
/// shared verbatim by the SQL surface and the unit tests.
fn fd1_digest(triples: &[Triple]) -> String {
    // ground: lines whose subject and object are both non-bnode.
    let mut ground: Vec<String> = triples
        .iter()
        .filter(|(s, _, o)| !is_bnode(s) && !is_bnode(o))
        .map(|(s, p, o)| {
            format!(
                "{} {} {} .",
                fleet_term(s, false, &|b| b.to_string()),
                fleet_term(p, false, &|b| b.to_string()),
                fleet_term(o, true, &|b| b.to_string()),
            )
        })
        .collect();
    ground.sort(); // Rust str ordering is byte-wise == COLLATE "C"
    let ground_text = ground.join("\n");

    // every distinct bnode label appearing as subject or object
    let mut bnodes: Vec<&str> = triples
        .iter()
        .flat_map(|(s, _, o)| [bnode_label(s), bnode_label(o)])
        .flatten()
        .collect();
    bnodes.sort_unstable();
    bnodes.dedup();

    // sig(b): sha256 over b's incident lines, self→_:a, other bnode→_:z
    let mut sigs: Vec<String> = bnodes
        .iter()
        .map(|b| {
            let sub = |label: &str| -> String { if label == *b { "a".into() } else { "z".into() } };
            let mut lines: Vec<String> = triples
                .iter()
                .filter(|(s, _, o)| bnode_label(s) == Some(*b) || bnode_label(o) == Some(*b))
                .map(|(s, p, o)| {
                    format!(
                        "{} {} {} .",
                        fleet_term(s, false, &sub),
                        fleet_term(p, false, &sub),
                        fleet_term(o, true, &sub),
                    )
                })
                .collect();
            lines.sort();
            sha256_hex(&lines.join("\n"))
        })
        .collect();
    sigs.sort();
    let sigs_text = sigs.join("\n");

    sha256_hex(&format!("{ground_text}\n--\n{sigs_text}"))
}

/// First-degree structural digest of a graph's ASSERTED triples.
/// Method: `pgrdf-fd1-sha256`. Survives blank-node relabelling (a
/// reload changes every label and not this value); `DIFFERENT` is
/// conclusive, `SAME` is evidence only — see the module docs and use
/// `pgrdf.graph_digest` where proof of identity is required.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn structural_digest(graph_id: i64) -> String {
    // Same existence discipline as graph_digest (T-NULL/L9): an absent
    // graph REFUSES — it has no digest; an EMPTY graph answers. Graph 0
    // (the default graph) always exists.
    if graph_id != 0 {
        let registered: bool = Spi::get_one_with_args(
            "SELECT EXISTS(SELECT 1 FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)",
            &[graph_id.into()],
        )
        .expect("structural_digest: registry existence check failed")
        .unwrap_or(false);
        if !registered {
            crate::refuse(
                pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
                format!(
                    "structural_digest: graph {graph_id} does not exist — an absent \
                     graph has no digest (an EMPTY graph does: pgrdf.add_graph it first)"
                ),
            );
        }
    }
    fd1_digest(&read_asserted_triples(graph_id))
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use super::*;
    use pgrx::prelude::*;

    fn iri(v: &str) -> CTerm {
        CTerm::Iri(v.into())
    }
    fn bn(v: &str) -> CTerm {
        CTerm::BNode(v.into())
    }

    /// K6-1 as a pure unit test: the canonical collision pair — a
    /// 4-cycle of blank nodes vs two 2-cycles — is non-isomorphic and
    /// MUST collide under fd1 (every node's first-degree view is
    /// identical). An fd1 that separates this pair is not the fleet
    /// algorithm, whatever its name says.
    #[test]
    fn fd1_collides_on_the_automorphic_pair() {
        let p = || iri("http://e/p");
        let cycle4 = vec![
            (bn("x1"), p(), bn("x2")),
            (bn("x2"), p(), bn("x3")),
            (bn("x3"), p(), bn("x4")),
            (bn("x4"), p(), bn("x1")),
        ];
        let two_2cycles = vec![
            (bn("y1"), p(), bn("y2")),
            (bn("y2"), p(), bn("y1")),
            (bn("y3"), p(), bn("y4")),
            (bn("y4"), p(), bn("y3")),
        ];
        assert_eq!(
            fd1_digest(&cycle4),
            fd1_digest(&two_2cycles),
            "SAME under fd1 is evidence, not proof — this pair is why"
        );
    }

    /// The conclusive direction: one changed edge is DIFFERENT.
    #[test]
    fn fd1_different_is_conclusive() {
        let p = || iri("http://e/p");
        let a = vec![(bn("x1"), p(), bn("x2")), (bn("x2"), p(), bn("x1"))];
        let b = vec![
            (bn("x1"), p(), bn("x2")),
            (bn("x2"), iri("http://e/q"), bn("x1")),
        ];
        assert_ne!(fd1_digest(&a), fd1_digest(&b));
    }

    /// Relabel invariance: identical structure under fresh labels gives
    /// the identical digest — the whole reason the pin is portable.
    #[test]
    fn fd1_survives_relabelling() {
        let p = || iri("http://e/p");
        let a = vec![
            (bn("m"), p(), iri("http://e/o")),
            (iri("http://e/s"), p(), bn("m")),
        ];
        let b = vec![
            (bn("q7"), p(), iri("http://e/o")),
            (iri("http://e/s"), p(), bn("q7")),
        ];
        assert_eq!(fd1_digest(&a), fd1_digest(&b));
    }

    /// E6 negative control (K10): absent graph refuses 42704, same
    /// discipline as graph_digest. No SPI after the catch.
    #[pg_test]
    fn structural_digest_absent_graph_refuses() {
        use pgrx::pg_sys::errcodes::PgSqlErrorCode;
        use pgrx::pg_sys::panic::CaughtError;
        let code = pgrx::PgTryBuilder::new(|| {
            Spi::run("SELECT pgrdf.structural_digest(983777)").unwrap();
            None
        })
        .catch_others(|e| match &e {
            CaughtError::PostgresError(r)
            | CaughtError::ErrorReport(r)
            | CaughtError::RustPanic { ereport: r, .. } => Some(r.sql_error_code()),
        })
        .execute();
        assert_eq!(code, Some(PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT));
    }

    /// End-to-end through the store: two loads of the same bnode-bearing
    /// source (labels relabelled by the parser) — equal structural
    /// digest; RDFC agrees; and the collision pair collides under fd1
    /// while RDFC separates it (the two-digest design, both directions,
    /// one test).
    #[pg_test]
    fn structural_digest_end_to_end() {
        Spi::run("SELECT pgrdf.add_graph(983778)").unwrap();
        Spi::run("SELECT pgrdf.add_graph(983779)").unwrap();
        let src = "'@prefix e: <http://e/> . _:m e:p e:o . e:s e:p _:m .'";
        Spi::run(&format!("SELECT pgrdf.parse_turtle({src}, 983778)")).unwrap();
        Spi::run(&format!("SELECT pgrdf.parse_turtle({src}, 983779)")).unwrap();
        let (a, b) = Spi::get_two::<String, String>(
            "SELECT pgrdf.structural_digest(983778), pgrdf.structural_digest(983779)",
        )
        .unwrap();
        assert_eq!(a, b, "fd1 survives the parser's relabelling");

        Spi::run("SELECT pgrdf.add_graph(983780)").unwrap();
        Spi::run("SELECT pgrdf.add_graph(983781)").unwrap();
        Spi::run("SELECT pgrdf.parse_turtle('@prefix e: <http://e/> . _:x1 e:p _:x2 . _:x2 e:p _:x3 . _:x3 e:p _:x4 . _:x4 e:p _:x1 .', 983780)").unwrap();
        Spi::run("SELECT pgrdf.parse_turtle('@prefix e: <http://e/> . _:y1 e:p _:y2 . _:y2 e:p _:y1 . _:y3 e:p _:y4 . _:y4 e:p _:y3 .', 983781)").unwrap();
        let (fd_a, fd_b) = Spi::get_two::<String, String>(
            "SELECT pgrdf.structural_digest(983780), pgrdf.structural_digest(983781)",
        )
        .unwrap();
        assert_eq!(fd_a, fd_b, "fd1 collides on the pair (SAME = evidence)");
        let (rd_a, rd_b) = Spi::get_two::<String, String>(
            "SELECT pgrdf.graph_digest(983780), pgrdf.graph_digest(983781)",
        )
        .unwrap();
        assert_ne!(rd_a, rd_b, "RDFC separates the same pair (proof plane)");
    }
}
