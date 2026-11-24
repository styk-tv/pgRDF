//! #159 — typed refusals on the query path.
//!
//! When `pgrdf.sparql`, `construct`, `describe`, `sparql_sql` or
//! `sparql_parse` fails, the SQLSTATE says who has to act:
//!
//! | class             | SQLSTATE        | meaning                                        |
//! |-------------------|-----------------|------------------------------------------------|
//! | unsupported shape | `0A000`         | valid SPARQL that pgRDF does not execute       |
//! | limit             | `54000`         | the request exceeds a bound of the engine      |
//! | request error     | `22xxx`/`42xxx` | the request is wrong, or names what is absent  |
//! | invariant         | `XX000`         | pgRDF or the store is broken — never a request |
//!
//! A refusal depends on the request, never on the data: the same text
//! refuses over an empty store and over a full one. The one input that
//! is not the text is the graph catalogue — a request naming a graph
//! that does not exist (`42704`) or already does (`42710`).
//!
//! Deliberate refusals go through `crate::refuse`; what is left as
//! `panic!` / `expect` / `unreachable!` on the query path is an
//! invariant, and reaching one is a bug to report. The tests below pin
//! one SQLSTATE per refusal family.

use pgrx::prelude::*;
use spargebra::algebra::{GraphPattern, GraphTarget};
use spargebra::term::{GraphNamePattern, NamedNode, NamedNodePattern, TermPattern};
use spargebra::{GraphUpdateOperation, Update};

/// Refuse (22P05) a parsed query or update that holds U+0000 in a term.
///
/// A SPARQL string literal may spell the character as `\u0000`;
/// PostgreSQL `text` cannot hold it. Left alone it went two ways. Where
/// the term reached SQL text, SPI failed with a Rust `NulError` dump
/// under XX000. Where it reached a bound parameter nothing failed at
/// all: `INSERT DATA` stored a term with a NUL in it.
///
/// `entry` prefixes the message, `text` is the request as received and
/// `parsed` its algebra. Only two escapes produce the character, so a
/// request without them passes without a second look. With one, the
/// algebra decides: the escape may sit in a comment or follow an escaped
/// backslash, and then no term holds U+0000.
pub(crate) fn refuse_nul_term(entry: &str, text: &str, parsed: &impl std::fmt::Debug) {
    if may_spell_nul(text) && debug_holds_nul(parsed) {
        crate::refuse(
            PgSqlErrorCode::ERRCODE_UNTRANSLATABLE_CHARACTER,
            format!(
                "{entry}: a term contains U+0000 (written \\u0000), \
                 which PostgreSQL text cannot hold"
            ),
        );
    }
}

/// Whether `text` could denote U+0000 once parsed: the 4- or 8-digit
/// escape, or the character itself.
fn may_spell_nul(text: &str) -> bool {
    text.contains("\\u0000") || text.contains("\\U00000000") || text.contains('\0')
}

/// Whether any string inside `value` holds U+0000, read off its `Debug`
/// rendering. `Debug` writes that character as `\0` and a backslash as
/// `\\`, so walking the escapes pairwise tells the two apart — and
/// covers every string of the algebra, whichever variant carries it,
/// without a hand-written walk that a new variant would slip past.
fn debug_holds_nul(value: &impl std::fmt::Debug) -> bool {
    let dump = format!("{value:?}");
    let mut chars = dump.chars();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.next() == Some('0') {
            return true;
        }
    }
    false
}

// ── ADD / COPY / MOVE from a graph that does not exist ──────────────

/// An ADD, COPY or MOVE of a parsed update whose source is a named
/// graph.
pub(crate) struct Transfer {
    /// Index of the first operation spargebra rewrote it into. The
    /// source has to exist before this one runs.
    first_op: usize,
    /// `ADD`, `COPY` or `MOVE` — or all three when only the shape of the
    /// algebra identified the operation.
    keyword: &'static str,
    /// IRI of the source graph.
    source: String,
}

/// Find the ADD / COPY / MOVE operations of `update` that read a named
/// graph, so the executor can refuse one whose source does not exist
/// before it writes anything.
///
/// Why this is needed. spargebra has no algebra for the three; it
/// rewrites them while parsing, as W3C SPARQL 1.1 Update §3.2.3–3.2.5
/// defines them:
///
/// ```text
/// ADD  a TO b   INSERT { GRAPH b { ?s ?p ?o } } WHERE { GRAPH a { ?s ?p ?o } }
/// COPY a TO b   DROP SILENT b ; <the ADD>
/// MOVE a TO b   DROP SILENT b ; <the ADD> ; DROP a
/// ```
///
/// A pattern over a graph that does not exist has no solutions, so with
/// a mistyped source COPY emptied the destination and reported success,
/// ADD did nothing and reported success, and MOVE was saved only by its
/// last DROP failing. The rewrite also throws `SILENT` away, so nothing
/// downstream can tell `COPY SILENT` from `COPY`.
///
/// **`SILENT` is therefore not honoured here: `COPY SILENT`,
/// `ADD SILENT` and `MOVE SILENT` from a graph that does not exist
/// refuse like the plain forms.** That is stricter than W3C, which lets
/// `SILENT` turn the error into success. Success would mean the
/// destination silently emptied; pgRDF refuses instead.
///
/// How the operations are found. The rewritten algebra has an exact
/// shape, and every ADD / COPY / MOVE has it — recognising the shape
/// cannot miss one, whatever the text looks like. But a caller may write
/// the same INSERT … WHERE by hand, and that request is not an ADD: W3C
/// gives it zero solutions and success. So when the shape is present,
/// the update text is read for the keyword that opens each operation
/// (`update_heads`) and the operations are matched to the algebra
/// one by one; only those written as ADD / COPY / MOVE are returned.
///
/// The two sources are not trusted equally. If the text cannot be read
/// with confidence, or does not line up with the algebra, the shape
/// alone decides and every operation that has it is returned: the
/// failure mode is one refusal too many, never a destination emptied.
///
/// The default graph as source always exists and is never returned.
pub(crate) fn transfers(text: &str, update: &Update) -> Vec<Transfer> {
    let ops = &update.operations;
    let reads_named_graph =
        |op: &GraphUpdateOperation| matches!(transfer_shape(op), Some((Some(_), _)));
    if !ops.iter().any(reads_named_graph) {
        return Vec::new();
    }
    update_heads(text)
        .and_then(|heads| transfers_by_keyword(&heads, ops))
        .unwrap_or_else(|| transfers_by_shape(ops))
}

/// Refuse 42704 if operation `index` opens a transfer whose source
/// graph `exists` says is absent. Called by the executor before each
/// operation runs, so a source created earlier in the same request
/// (`CREATE GRAPH <a> ; COPY <a> TO <b>`) counts.
pub(crate) fn refuse_transfer_from_unknown_graph(
    transfers: &[Transfer],
    index: usize,
    exists: impl Fn(&str) -> bool,
) {
    for t in transfers.iter().filter(|t| t.first_op == index) {
        if !exists(&t.source) {
            crate::refuse_with_hint(
                PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
                format!(
                    "sparql: {}: source graph <{}> does not exist",
                    t.keyword, t.source
                ),
                "check the source IRI, or CREATE GRAPH it first; SILENT does not \
                 suppress this refusal"
                    .to_string(),
            );
        }
    }
}

fn is_spo(s: &TermPattern, p: &NamedNodePattern, o: &TermPattern) -> bool {
    matches!(s, TermPattern::Variable(v) if v.as_str() == "s")
        && matches!(p, NamedNodePattern::Variable(v) if v.as_str() == "p")
        && matches!(o, TermPattern::Variable(v) if v.as_str() == "o")
}

/// `Some((source, destination))` when `op` is exactly the INSERT … WHERE
/// that spargebra's rewrite builds; a `None` source is the default
/// graph.
fn transfer_shape(op: &GraphUpdateOperation) -> Option<(Option<&NamedNode>, &GraphNamePattern)> {
    let GraphUpdateOperation::DeleteInsert {
        delete,
        insert,
        using: None,
        pattern,
    } = op
    else {
        return None;
    };
    let [quad] = insert.as_slice() else {
        return None;
    };
    if !delete.is_empty()
        || !is_spo(&quad.subject, &quad.predicate, &quad.object)
        || matches!(quad.graph_name, GraphNamePattern::Variable(_))
    {
        return None;
    }
    let (source, bgp) = match pattern.as_ref() {
        GraphPattern::Graph {
            name: NamedNodePattern::NamedNode(source),
            inner,
        } => (Some(source), inner.as_ref()),
        bgp => (None, bgp),
    };
    let GraphPattern::Bgp { patterns } = bgp else {
        return None;
    };
    let [tp] = patterns.as_slice() else {
        return None;
    };
    is_spo(&tp.subject, &tp.predicate, &tp.object).then_some((source, &quad.graph_name))
}

/// The target of `op` when it is the `DROP SILENT` a COPY / MOVE opens
/// with.
fn silent_drop(op: &GraphUpdateOperation) -> Option<&GraphTarget> {
    match op {
        GraphUpdateOperation::Drop {
            silent: true,
            graph,
        } => Some(graph),
        _ => None,
    }
}

fn target_is_destination(target: &GraphTarget, destination: &GraphNamePattern) -> bool {
    match (target, destination) {
        (GraphTarget::DefaultGraph, GraphNamePattern::DefaultGraph) => true,
        (GraphTarget::NamedNode(a), GraphNamePattern::NamedNode(b)) => a == b,
        _ => false,
    }
}

fn target_is_source(target: &GraphTarget, source: Option<&NamedNode>) -> bool {
    match (target, source) {
        (GraphTarget::DefaultGraph, None) => true,
        (GraphTarget::NamedNode(a), Some(b)) => a == b,
        _ => false,
    }
}

/// The keyword that opens one operation of an update.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Head {
    Load,
    Clear,
    Drop,
    Create,
    /// `INSERT`, `DELETE` or `WITH`.
    Modify,
    Add,
    Copy,
    Move,
}

const HEADS: [(&str, Head); 10] = [
    ("LOAD", Head::Load),
    ("CLEAR", Head::Clear),
    ("DROP", Head::Drop),
    ("CREATE", Head::Create),
    ("INSERT", Head::Modify),
    ("DELETE", Head::Modify),
    ("WITH", Head::Modify),
    ("ADD", Head::Add),
    ("COPY", Head::Copy),
    ("MOVE", Head::Move),
];

impl Head {
    fn keyword(self) -> &'static str {
        match self {
            Head::Add => "ADD",
            Head::Copy => "COPY",
            _ => "MOVE",
        }
    }

    /// Whether `op` is what this (non-transfer) keyword parses into.
    fn parses_into(self, op: &GraphUpdateOperation) -> bool {
        matches!(
            (self, op),
            (Head::Load, GraphUpdateOperation::Load { .. })
                | (Head::Clear, GraphUpdateOperation::Clear { .. })
                | (Head::Drop, GraphUpdateOperation::Drop { .. })
                | (Head::Create, GraphUpdateOperation::Create { .. })
                | (
                    Head::Modify,
                    GraphUpdateOperation::InsertData { .. }
                        | GraphUpdateOperation::DeleteData { .. }
                        | GraphUpdateOperation::DeleteInsert { .. }
                )
        )
    }

    /// How many of `rest`'s leading operations this ADD / COPY / MOVE
    /// was rewritten into, and its source. `None` when they do not have
    /// the rewritten form — which is how `COPY <g> TO <g>` looks:
    /// spargebra emits nothing for a transfer onto itself.
    fn rewritten(self, rest: &[GraphUpdateOperation]) -> Option<(usize, Option<&NamedNode>)> {
        if self == Head::Add {
            let (source, _) = transfer_shape(rest.first()?)?;
            return Some((1, source));
        }
        let dropped = silent_drop(rest.first()?)?;
        let (source, destination) = transfer_shape(rest.get(1)?)?;
        if !target_is_destination(dropped, destination) {
            return None;
        }
        if self == Head::Copy {
            return Some((2, source));
        }
        match rest.get(2)? {
            GraphUpdateOperation::Drop { graph, .. } if target_is_source(graph, source) => {
                Some((3, source))
            }
            _ => None,
        }
    }
}

/// Match the keywords of the text to the operations of the algebra and
/// return the transfers that read a named graph. `None` when the two do
/// not line up one for one — the caller then falls back to the shape.
fn transfers_by_keyword(heads: &[Head], ops: &[GraphUpdateOperation]) -> Option<Vec<Transfer>> {
    let mut found = Vec::new();
    let mut at = 0;
    for &head in heads {
        if matches!(head, Head::Add | Head::Copy | Head::Move) {
            if let Some((len, source)) = head.rewritten(&ops[at..]) {
                if let Some(source) = source {
                    found.push(Transfer {
                        first_op: at,
                        keyword: head.keyword(),
                        source: source.as_str().to_string(),
                    });
                }
                at += len;
            }
        } else if ops.get(at).is_some_and(|op| head.parses_into(op)) {
            at += 1;
        } else {
            return None;
        }
    }
    (at == ops.len()).then_some(found)
}

/// Every operation with the rewritten shape and a named source, without
/// consulting the text. A preceding `DROP SILENT` of the destination is
/// taken as part of it, so the check still runs before that DROP.
fn transfers_by_shape(ops: &[GraphUpdateOperation]) -> Vec<Transfer> {
    let mut found = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let Some((Some(source), destination)) = transfer_shape(op) else {
            continue;
        };
        let opened_by_drop = i > 0
            && silent_drop(&ops[i - 1]).is_some_and(|t| target_is_destination(t, destination));
        found.push(Transfer {
            first_op: if opened_by_drop { i - 1 } else { i },
            keyword: "ADD/COPY/MOVE",
            source: source.as_str().to_string(),
        });
    }
    found
}

/// Read the keyword that opens each operation of an update text that
/// spargebra has already accepted. `None` when the text cannot be read
/// with confidence.
///
/// Operations are separated by `;` outside any `{ … }`. Getting there
/// means stepping over what may contain a `;` or a brace without being
/// one: comments, string literals, IRIs and `\`-escapes. Nothing else is
/// interpreted. spargebra's grammar lets keywords run together
/// (`COPYSILENT<a>TO<b>` parses), so a keyword is matched as a prefix of
/// the word it starts.
fn update_heads(text: &str) -> Option<Vec<Head>> {
    let b = text.as_bytes();
    let mut heads = Vec::new();
    let mut depth = 0usize;
    let mut at_head = true;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'#' => i = skip_comment(b, i),
            b'"' | b'\'' => i = skip_string(b, i)?,
            // `<` opens an IRI, or is the comparison operator.
            b'<' => i = skip_iri(b, i).unwrap_or(i + 1),
            b'\\' => i += 2,
            b'{' => {
                depth += 1;
                i += 1;
            }
            b'}' => {
                depth = depth.checked_sub(1)?;
                i += 1;
            }
            b';' if depth == 0 => {
                at_head = true;
                i += 1;
            }
            c if c.is_ascii_alphabetic() && depth == 0 && at_head => {
                let start = i;
                while i < b.len() && b[i].is_ascii_alphabetic() {
                    i += 1;
                }
                let word = text[start..i].to_ascii_uppercase();
                if word == "BASE" {
                    // its IRI is stepped over as any IRI is
                } else if word.starts_with("PREFIX") {
                    // step over the prefix label, which may look like a
                    // keyword (`PREFIX copy: <…>`)
                    while i < b.len() && (b[i].is_ascii_whitespace() || b[i] == b'#') {
                        i = if b[i] == b'#' {
                            skip_comment(b, i)
                        } else {
                            i + 1
                        };
                    }
                    while *b.get(i)? != b':' {
                        if b[i].is_ascii_whitespace() {
                            return None;
                        }
                        i += 1;
                    }
                    i += 1;
                } else {
                    heads.push(HEADS.iter().find(|(kw, _)| word.starts_with(kw))?.1);
                    at_head = false;
                }
            }
            _ => i += 1,
        }
    }
    (depth == 0).then_some(heads)
}

/// Index just past a `#` comment, which runs to the end of the line.
fn skip_comment(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i] != b'\n' && b[i] != b'\r' {
        i += 1;
    }
    i
}

/// Index just past the string literal that opens at `i`; `None` if it
/// does not close.
fn skip_string(b: &[u8], mut i: usize) -> Option<usize> {
    let quote = b[i];
    let long = b.len() >= i + 3 && b[i + 1] == quote && b[i + 2] == quote;
    i += if long { 3 } else { 1 };
    loop {
        let c = *b.get(i)?;
        if c == b'\\' {
            i += 2;
        } else if long {
            if c == quote && b.get(i + 1) == Some(&quote) && b.get(i + 2) == Some(&quote) {
                return Some(i + 3);
            }
            i += 1;
        } else if c == quote {
            return Some(i + 1);
        } else if c == b'\n' || c == b'\r' {
            return None;
        } else {
            i += 1;
        }
    }
}

/// Index just past the IRI that opens at `i`; `None` if what follows the
/// `<` is not one (it is then the comparison operator).
fn skip_iri(b: &[u8], mut i: usize) -> Option<usize> {
    loop {
        i += 1;
        match *b.get(i)? {
            b'>' => return Some(i + 1),
            c if c <= b' ' || b"<\"{}|^`\\".contains(&c) => return None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod unit {
    use super::*;
    use spargebra::SparqlParser;

    fn query_holds_nul(q: &str) -> bool {
        let parsed = SparqlParser::new()
            .parse_query(q)
            .expect("test query parses");
        may_spell_nul(q) && debug_holds_nul(&parsed)
    }

    #[test]
    fn nul_is_found_wherever_a_string_sits() {
        assert!(debug_holds_nul(&"a\0b"));
        assert!(debug_holds_nul(&vec![Some(("x", "\0"))]));
        assert!(query_holds_nul(r#"SELECT ?s WHERE { ?s ?p "a\u0000b" }"#));
        assert!(query_holds_nul(
            r#"SELECT ?s WHERE { ?s ?p "\U00000000"@en }"#
        ));
        assert!(query_holds_nul(
            r#"SELECT (GROUP_CONCAT(?o; separator="\u0000") AS ?x) WHERE { ?s ?p ?o }"#
        ));
        assert!(query_holds_nul(
            r#"SELECT ?s WHERE { ?s ?p ?o FILTER(REGEX(STR(?o), "\u0000")) }"#
        ));
    }

    #[test]
    fn a_backslash_before_a_zero_is_not_nul() {
        // backslash, then the digit zero
        assert!(!debug_holds_nul(&"a\\0b"));
        assert!(!debug_holds_nul(&"\\\\0"));
        // two backslashes then `u0000`: an escaped backslash and five
        // ordinary characters
        assert!(!query_holds_nul(r#"SELECT ?s WHERE { ?s ?p "a\\u0000b" }"#));
        // the escape in a comment is in no term
        assert!(!query_holds_nul("SELECT ?s WHERE { ?s ?p ?o } # \\u0000"));
        assert!(!query_holds_nul(r#"SELECT ?s WHERE { ?s ?p "0" }"#));
    }

    fn found(update: &str) -> Vec<(usize, &'static str, String)> {
        let parsed = SparqlParser::new()
            .parse_update(update)
            .expect("test update parses");
        transfers(update, &parsed)
            .into_iter()
            .map(|t| (t.first_op, t.keyword, t.source))
            .collect()
    }

    fn one(
        first_op: usize,
        keyword: &'static str,
        source: &str,
    ) -> Vec<(usize, &'static str, String)> {
        vec![(first_op, keyword, source.to_string())]
    }

    #[test]
    fn each_transfer_is_found_with_its_keyword_and_first_operation() {
        assert_eq!(found("ADD <urn:a> TO <urn:b>"), one(0, "ADD", "urn:a"));
        assert_eq!(found("COPY <urn:a> TO <urn:b>"), one(0, "COPY", "urn:a"));
        assert_eq!(found("MOVE <urn:a> TO <urn:b>"), one(0, "MOVE", "urn:a"));
        assert_eq!(
            found("copy silent graph <urn:a> to default"),
            one(0, "COPY", "urn:a")
        );
        assert_eq!(found("COPYSILENT<urn:a>TO<urn:b>"), one(0, "COPY", "urn:a"));
        assert_eq!(
            found("PREFIX copy: <urn:> # move it\nMOVE copy:a TO copy:b"),
            one(0, "MOVE", "urn:a")
        );
        // the check belongs in front of the operation's own first step
        assert_eq!(
            found("INSERT DATA { <urn:s> <urn:p> \"a ; } COPY\" } ; COPY <urn:a> TO <urn:b>"),
            one(1, "COPY", "urn:a")
        );
        assert_eq!(
            found("CREATE GRAPH <urn:a> ; ADD <urn:a> TO <urn:b> ; MOVE <urn:c> TO <urn:d>"),
            vec![
                (1, "ADD", "urn:a".to_string()),
                (2, "MOVE", "urn:c".to_string())
            ]
        );
    }

    #[test]
    fn what_is_not_a_transfer_from_a_named_graph_is_left_alone() {
        // the default graph always exists
        assert!(found("COPY DEFAULT TO <urn:b>").is_empty());
        assert!(found("ADD DEFAULT TO <urn:b>").is_empty());
        // a transfer onto itself parses to nothing
        assert!(found("MOVE <urn:a> TO <urn:a>").is_empty());
        // the same algebra, written by hand: not an ADD
        let by_hand = "INSERT { GRAPH <urn:b> { ?s ?p ?o } } WHERE { GRAPH <urn:a> { ?s ?p ?o } }";
        assert!(found(by_hand).is_empty());
        assert!(found(&format!("# COPY it\n{by_hand}")).is_empty());
        assert!(found(&format!("DROP SILENT GRAPH <urn:b> ; {by_hand}")).is_empty());
        // and told apart from a real one in the same request
        assert_eq!(
            found(&format!("{by_hand} ; ADD <urn:c> TO <urn:d>")),
            one(1, "ADD", "urn:c")
        );
        // other shapes never qualify
        assert!(
            found("INSERT { ?s ?p ?o } WHERE { GRAPH <urn:a> { ?s ?p ?o FILTER(?o < 3) } }")
                .is_empty()
        );
    }

    #[test]
    fn when_text_and_algebra_disagree_the_shape_decides() {
        // `COPY <a> TO <a>` parses to nothing, so the keyword is first
        // matched to the hand-written pair behind it; the count then does
        // not add up and the shape rule takes over. One refusal too many
        // is the safe direction.
        let q = "COPY <urn:a> TO <urn:a> ; DROP SILENT GRAPH <urn:x> ; \
                 INSERT { GRAPH <urn:x> { ?s ?p ?o } } WHERE { GRAPH <urn:y> { ?s ?p ?o } }";
        assert_eq!(found(q), one(0, "ADD/COPY/MOVE", "urn:y"));
    }

    #[test]
    fn heads_are_read_past_everything_that_may_hide_a_separator() {
        use Head::*;
        let heads = |q: &str| update_heads(q);
        assert_eq!(
            heads("BASE <urn:x#a;b> PREFIX add: <urn:> CLEAR ALL"),
            Some(vec![Clear])
        );
        assert_eq!(
            heads(
                "INSERT DATA { <urn:s> <urn:p> \"\"\"a \" } ; COPY\"\"\" , 'it\\'s }' } ;\n\
                 # ; DROP ALL\n DELETE { ?s ?p ?o } WHERE { ?s ?p ?o FILTER(?o < 3 && ?o > 1) } ;\
                 WITH <urn:g> DELETE { ?s ?p ?o } WHERE { ?s ?p ?o } ; LOAD <urn:l> ;"
            ),
            Some(vec![Modify, Modify, Modify, Load])
        );
        assert_eq!(
            heads("create graph <urn:a>;drop graph <urn:a>"),
            Some(vec![Create, Drop])
        );
        // unbalanced, unterminated or unknown: no confident reading
        assert_eq!(heads("COPY <urn:a> TO <urn:b> }"), None);
        assert_eq!(heads("INSERT DATA { <urn:s> <urn:p> \"open }"), None);
        assert_eq!(heads("FROBNICATE <urn:a>"), None);
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    /// Run `sql` in a PL/pgSQL subtransaction and report how it ended:
    /// `("00000", "")` on success, else `(SQLSTATE, message)`. The
    /// subtransaction is rolled back on error, so the caller may keep
    /// using SPI afterwards — which is what lets a test assert that a
    /// refused request wrote nothing.
    fn outcome(sql: &str) -> (String, String) {
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.refusal_of(q text) RETURNS text \
             LANGUAGE plpgsql AS $$ DECLARE st text; m text; BEGIN \
               EXECUTE q; RETURN '00000|'; \
             EXCEPTION WHEN OTHERS THEN \
               GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, m = MESSAGE_TEXT; \
               RETURN st || '|' || m; END $$",
        )
        .unwrap();
        let got: String = Spi::get_one_with_args("SELECT pg_temp.refusal_of($1)", &[sql.into()])
            .unwrap()
            .unwrap();
        let (st, msg) = got.split_once('|').unwrap();
        (st.to_string(), msg.to_string())
    }

    /// `outcome` of running `q` through `pgrdf.sparql` to the last row.
    fn sparql(q: &str) -> (String, String) {
        outcome(&format!(
            "SELECT count(*) FROM pgrdf.sparql($sparql${q}$sparql$)"
        ))
    }

    #[track_caller]
    fn assert_outcome(got: (String, String), state: &str, fragment: &str, what: &str) {
        assert_eq!(got.0, state, "{what}: SQLSTATE {} — {}", got.0, got.1);
        assert!(
            got.1.contains(fragment),
            "{what}: message {:?} lacks {fragment:?}",
            got.1
        );
    }

    /// One named graph with a two-hop chain and a number, plus one
    /// default-graph triple. Returns the named graph's id.
    fn seed(iri: &str) -> i64 {
        let g: i64 = Spi::get_one_with_args("SELECT pgrdf.add_graph($1)", &[iri.into()])
            .unwrap()
            .unwrap();
        Spi::get_one_with_args::<i64>(
            "SELECT pgrdf.parse_turtle($1, $2)",
            &[
                "<urn:tr:a> <urn:tr:p> <urn:tr:b> . <urn:tr:b> <urn:tr:p> \"x\" . \
                 <urn:tr:a> <urn:tr:n> 5 ."
                    .into(),
                g.into(),
            ],
        )
        .unwrap();
        Spi::get_one::<i64>("SELECT pgrdf.parse_turtle('<urn:tr:d> <urn:tr:p> <urn:tr:e> .', 0)")
            .unwrap();
        g
    }

    /// Every shape pgRDF parses but does not execute refuses 0A000 —
    /// one request per message family. None of these depends on what
    /// the store holds: the same request refuses over an empty store.
    #[pg_test]
    fn unsupported_shapes_refuse_0a000() {
        const P: &str = "<urn:tr:p>";
        let cases: Vec<(String, &str)> = vec![
            // query forms and empty groups
            ("DESCRIBE <urn:tr:a>".into(), "use pgrdf.describe(q)"),
            (
                "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }".into(),
                "query form not supported yet",
            ),
            ("SELECT * WHERE { }".into(), "sparql: empty BGP"),
            ("ASK { }".into(), "sparql: ASK with empty BGP"),
            // variables no pattern binds
            (
                "SELECT ?zz WHERE { ?s ?p ?o }".into(),
                "projected variable ?zz is not bound in any BGP pattern",
            ),
            (
                "SELECT ?s WHERE { ?s ?p ?o } ORDER BY ?zz".into(),
                "ORDER BY variable ?zz not bound in any BGP pattern",
            ),
            (
                "SELECT ?zz (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?zz".into(),
                "GROUP BY variable ?zz not bound in any BGP pattern",
            ),
            (
                "SELECT ?s (COUNT(?zz) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "aggregate COUNT(?zz) but variable not bound",
            ),
            (
                "SELECT ?s (SUM(?zz) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "numeric aggregate over ?zz but variable not bound",
            ),
            (
                "SELECT ?s (GROUP_CONCAT(?zz) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "aggregate over ?zz but variable not bound",
            ),
            // ORDER BY
            (
                "SELECT ?s WHERE { ?s ?p ?o } ORDER BY (<urn:x:fn>(?o))".into(),
                "ORDER BY expression not translatable",
            ),
            (
                "SELECT DISTINCT ?s WHERE { ?s ?p ?o } ORDER BY (STR(?o))".into(),
                "ORDER BY over an expression is not supported together with DISTINCT",
            ),
            (
                "SELECT DISTINCT ?s WHERE { ?s ?p ?o } ORDER BY ?o".into(),
                "requires ?o to be in the SELECT clause when DISTINCT is used",
            ),
            (
                "SELECT ?s (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s ORDER BY (STR(?s))"
                    .into(),
                "ORDER BY expression sort keys are not supported on an aggregate query",
            ),
            (
                "SELECT ?s (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s ORDER BY ?p".into(),
                "ORDER BY ?p on an aggregate query must reference a projected variable",
            ),
            (
                format!(
                    "SELECT ?s WHERE {{ {{ ?s {P} ?o }} UNION {{ ?s <urn:tr:n> ?o }} }} ORDER BY ?o"
                ),
                "ORDER BY ?o on UNION must reference a projected variable",
            ),
            (
                format!(
                    "SELECT ?s (COUNT(?o) AS ?n) WHERE {{ {{ ?s {P} ?o }} UNION \
                     {{ ?s <urn:tr:n> ?o }} }} GROUP BY ?s ORDER BY ?o"
                ),
                "on an aggregate-over-UNION query must reference a projected variable",
            ),
            // grouping and aggregates
            (
                "SELECT (?s AS ?t) (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "?t is neither a GROUP BY variable nor an aggregate output",
            ),
            (
                "SELECT (COUNT(?o) AS ?n) WHERE { ?s ?p ?o } HAVING (<urn:x:fn>(?n))".into(),
                "HAVING expression not translatable",
            ),
            (
                format!(
                    "SELECT (COUNT(?o) AS ?n) WHERE {{ {{ ?s {P} ?o }} UNION \
                     {{ ?s <urn:tr:n> ?o }} }} HAVING (<urn:x:fn>(?n))"
                ),
                "HAVING expression not translatable",
            ),
            (
                "SELECT ?s (COUNT(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "COUNT over expression not translatable",
            ),
            (
                "SELECT ?s (SUM(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "SUM over expression not numerically translatable",
            ),
            (
                "SELECT ?s (MIN(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "MIN over expression not translatable",
            ),
            (
                "SELECT ?s (MAX(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "MAX over expression not translatable",
            ),
            (
                "SELECT ?s (GROUP_CONCAT(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s"
                    .into(),
                "GROUP_CONCAT over expression not translatable",
            ),
            (
                "SELECT ?s (SAMPLE(<urn:x:fn>(?o)) AS ?n) WHERE { ?s ?p ?o } GROUP BY ?s".into(),
                "SAMPLE over expression not translatable",
            ),
            (
                format!(
                    "SELECT ?g (SUM(STRLEN(STR(?g))) AS ?n) WHERE {{ {{ GRAPH ?g {{ ?s {P} ?o }} }} \
                     UNION {{ GRAPH ?g {{ ?s <urn:tr:n> ?o }} }} }} GROUP BY ?g"
                ),
                "UNION graph-text-lane variable ?g not supported yet",
            ),
            (
                format!(
                    "SELECT ?g (COUNT(?s) AS ?n) WHERE {{ {{ GRAPH ?g {{ ?s {P} ?o }} }} \
                     UNION {{ ?s <urn:tr:n> ?g }} }} GROUP BY ?g"
                ),
                "GRAPH-scoped in one UNION branch but term-bound in another",
            ),
            // OPTIONAL
            (
                format!(
                    "SELECT ?s WHERE {{ ?s {P} ?o OPTIONAL {{ ?o {P} ?x \
                     {{ SELECT ?y WHERE {{ ?y ?q ?z }} }} }} }}"
                ),
                "OPTIONAL group contains an unsupported sub-pattern",
            ),
            (
                format!(
                    "SELECT ?s WHERE {{ ?s {P} ?o OPTIONAL {{ ?o {P} ?x . \
                     GRAPH ?g {{ ?x {P} ?y . ?y {P} ?z }} }} }}"
                ),
                "introduced inside a multi-triple OPTIONAL is not yet supported",
            ),
            // VALUES on a graph variable (#109)
            (
                "SELECT ?g ?s WHERE { VALUES ?g { <urn:tr:g1> } GRAPH ?g { ?s ?p ?o } }".into(),
                "pgRDF#109",
            ),
            // UPDATE shapes refused before the WHERE runs
            (
                format!(
                    "INSERT {{ ?s {P} ?n }} WHERE {{ SELECT ?s (COUNT(?o) AS ?n) \
                     WHERE {{ ?s {P} ?o }} GROUP BY ?s }}"
                ),
                "INSERT WHERE template feature 'aggregate/GROUP BY in WHERE'",
            ),
            (
                format!(
                    "DELETE {{ ?s {P} ?n }} WHERE {{ SELECT ?s (COUNT(?o) AS ?n) \
                     WHERE {{ ?s {P} ?o }} GROUP BY ?s }}"
                ),
                "DELETE WHERE template feature 'aggregate/GROUP BY in WHERE'",
            ),
            (
                format!(
                    "DELETE {{ ?s {P} ?o }} INSERT {{ ?s {P} ?n }} WHERE {{ SELECT ?s \
                     (COUNT(?o) AS ?n) WHERE {{ ?s {P} ?o }} GROUP BY ?s }}"
                ),
                "DELETE/INSERT WHERE template feature 'aggregate/GROUP BY in WHERE'",
            ),
            (
                format!("INSERT {{ ?s {P} ?zz }} WHERE {{ ?s {P} ?o }}"),
                "INSERT WHERE template feature 'unbound template variable ?zz'",
            ),
            (
                format!("DELETE {{ ?s {P} ?zz }} WHERE {{ ?s {P} ?o }}"),
                "DELETE WHERE template feature 'unbound template variable ?zz'",
            ),
            (
                format!("DELETE {{ ?s {P} ?zz }} INSERT {{ ?s {P} ?o }} WHERE {{ ?s {P} ?o }}"),
                "DELETE/INSERT WHERE template feature 'unbound template variable ?zz'",
            ),
        ];
        for (q, fragment) in &cases {
            assert_outcome(sparql(q), "0A000", fragment, q);
        }
        // The same class at the other entry point.
        for (q, fragment) in [
            (
                "CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o } ORDER BY ?s",
                "DISTINCT / ORDER BY / GROUP BY / aggregates not supported",
            ),
            (
                "CONSTRUCT { ?s ?p ?o } WHERE { SELECT DISTINCT ?s ?p ?o WHERE { ?s ?p ?o } }",
                "DISTINCT / ORDER BY / GROUP BY / aggregates not supported",
            ),
            (
                "CONSTRUCT { ?s ?p ?x } WHERE { ?s ?p ?o BIND(ISIRI(?o) AS ?x) }",
                "BIND expression for template ?x not translatable",
            ),
        ] {
            let got = outcome(&format!(
                "SELECT count(*) FROM pgrdf.construct($sparql${q}$sparql$)"
            ));
            assert_outcome(got, "0A000", fragment, q);
        }
    }

    /// #114: a group-level FILTER beside a UNION refuses 0A000, the
    /// message is the one consumers already match on, and the refusal
    /// still counts in `filter_clauses_dropped`.
    #[pg_test]
    fn filter_over_union_refuses_0a000_and_still_counts() {
        let before = crate::storage::shmem_cache::snapshot().filter_clauses_dropped;
        let got = sparql(
            "SELECT ?c WHERE { { ?c a <urn:t:A> } UNION { ?c a <urn:t:B> } \
             FILTER(?c = <urn:t:x>) }",
        );
        assert_eq!(got.0, "0A000", "{}", got.1);
        assert_eq!(
            got.1,
            "sparql: group-level FILTER alongside a UNION is not applied on the \
             SELECT/CONSTRUCT path (pgRDF#114) — it would be silently dropped and widen the \
             result set. Move the FILTER into every UNION branch (or restructure the query). \
             Refusing instead of returning a wrong answer."
        );
        let after = crate::storage::shmem_cache::snapshot().filter_clauses_dropped;
        assert_eq!(after, before + 1, "the refusal must count");
    }

    /// `ISIRI()` and the other boolean-valued forms in a SELECT
    /// expression are not translated to a value; that is an unsupported
    /// shape (0A000), not an engine fault. The same test inside FILTER
    /// keeps working.
    #[pg_test]
    fn boolean_valued_select_expressions_refuse_0a000() {
        seed("urn:tr:bool");
        for expr in [
            "ISIRI(?o)",
            "ISURI(?o)",
            "ISBLANK(?o)",
            "ISLITERAL(?o)",
            "ISNUMERIC(?o)",
            "BOUND(?o)",
            "sameTerm(?s, ?o)",
            "?s = ?o",
            "REGEX(STR(?o), \"a\")",
        ] {
            let q = format!("SELECT ({expr} AS ?x) WHERE {{ ?s ?p ?o }}");
            assert_outcome(
                sparql(&q),
                "0A000",
                "BIND expression for ?x not translatable",
                &q,
            );
        }
        let (state, msg) = sparql("SELECT ?s WHERE { ?s ?p ?o FILTER(ISIRI(?o)) }");
        assert_eq!(state, "00000", "ISIRI in FILTER is supported: {msg}");
    }

    /// A FILTER pgRDF cannot translate refuses with the same class
    /// wherever it sits: top level, as an OPTIONAL's join condition, or
    /// inside a nested OPTIONAL.
    #[pg_test]
    fn untranslatable_filter_refuses_0a000_inside_optional_too() {
        let top = sparql("SELECT ?s WHERE { ?s <urn:tr:p> ?o FILTER(<urn:x:fn>(?o)) }");
        assert_outcome(
            top,
            "0A000",
            "FILTER expression not translatable",
            "top level",
        );
        let join = sparql(
            "SELECT ?s WHERE { ?s <urn:tr:p> ?o \
             OPTIONAL { ?o <urn:tr:p> ?x FILTER(<urn:x:fn>(?x)) } }",
        );
        assert_outcome(
            join,
            "0A000",
            "OPTIONAL join FILTER not translatable",
            "join",
        );
        let nested = sparql(
            "SELECT ?s WHERE { ?s <urn:tr:p> ?o OPTIONAL { ?o <urn:tr:p> ?x \
             OPTIONAL { ?x <urn:tr:p> ?y FILTER(<urn:x:fn>(?y)) } } }",
        );
        assert_eq!(nested.0, "0A000", "nested: {}", nested.1);
    }

    /// Text that is not SPARQL is the caller's error: 42601 at every
    /// entry point that parses.
    #[pg_test]
    fn syntax_errors_refuse_42601() {
        for (sql, prefix) in [
            (
                "SELECT count(*) FROM pgrdf.sparql('SELEC ?s WHERE { ?s ?p ?o }')",
                "sparql: parse error: ",
            ),
            (
                "SELECT pgrdf.sparql_sql('SELEC ?s')",
                "sparql: parse error: ",
            ),
            ("SELECT pgrdf.sparql_parse('SELEC ?s')", "sparql_parse: "),
            (
                "SELECT count(*) FROM pgrdf.describe('SELEC ?s')",
                "pgrdf.describe: parse error: ",
            ),
        ] {
            let got = outcome(sql);
            assert_eq!(got.0, "42601", "{sql}: {}", got.1);
            assert!(got.1.starts_with(prefix), "{sql}: {}", got.1);
        }
    }

    /// A request that parses but is not valid for the function called,
    /// or that RDF forbids, is 22023.
    #[pg_test]
    fn invalid_requests_refuse_22023() {
        for (sql, fragment) in [
            (
                "SELECT count(*) FROM pgrdf.describe('SELECT ?s WHERE { ?s ?p ?o }')",
                "pgrdf.describe: not a DESCRIBE query",
            ),
            (
                "SELECT count(*) FROM pgrdf.construct('CONSTRUCT WHERE { ?s ?p _:b }')",
                "WHERE-shorthand prohibits blank nodes in the pattern",
            ),
            (
                "SELECT count(*) FROM pgrdf.construct(\
                 'CONSTRUCT { \"lit\" ?p ?o } WHERE { ?s ?p ?o }')",
                "literal not allowed in subject/predicate position",
            ),
        ] {
            assert_outcome(outcome(sql), "22023", fragment, sql);
        }
    }

    /// `"\u0000"` in a query or update is a term PostgreSQL text cannot
    /// hold. It refuses 22P05 before anything runs — no Rust `NulError`
    /// dump, and no row written with a NUL in it.
    #[pg_test]
    fn nul_escape_refuses_22p05_and_writes_nothing() {
        seed("urn:tr:nul");
        let dict_before: i64 = Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary")
            .unwrap()
            .unwrap();
        for q in [
            r#"SELECT ?s WHERE { ?s ?p "a\u0000b" }"#,
            r#"SELECT ?s WHERE { ?s ?p "\U00000000" }"#,
            r#"SELECT ?s WHERE { ?s ?p ?o FILTER(CONTAINS(STR(?o), "\u0000")) }"#,
            r#"SELECT (CONCAT(STR(?o), "\u0000") AS ?x) WHERE { ?s ?p ?o }"#,
            r#"SELECT ?s WHERE { VALUES ?o { "a\u0000b" } ?s ?p ?o }"#,
            r#"ASK { ?s ?p "\u0000"@en }"#,
            r#"INSERT DATA { <urn:tr:a> <urn:tr:p> "a\u0000b" }"#,
            r#"DELETE DATA { <urn:tr:a> <urn:tr:p> "a\u0000b" }"#,
            r#"INSERT { ?s <urn:tr:q> "a\u0000b" } WHERE { ?s <urn:tr:p> ?o }"#,
            r#"DELETE { ?s ?p ?o } WHERE { ?s ?p ?o FILTER(?o = "\u0000") }"#,
        ] {
            assert_outcome(sparql(q), "22P05", "U+0000", q);
        }
        for call in [
            r#"SELECT count(*) FROM pgrdf.construct($q$CONSTRUCT { ?s ?p "a\u0000b" } WHERE { ?s ?p ?o }$q$)"#,
            r#"SELECT count(*) FROM pgrdf.describe($q$DESCRIBE ?s WHERE { ?s ?p "\u0000" }$q$)"#,
            r#"SELECT pgrdf.sparql_sql($q$SELECT ?s WHERE { ?s ?p "\u0000" }$q$)"#,
            r#"SELECT pgrdf.sparql_parse($q$SELECT ?s WHERE { ?s ?p "\u0000" }$q$)"#,
        ] {
            assert_outcome(outcome(call), "22P05", "U+0000", call);
        }
        let dict_after: i64 = Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_dictionary")
            .unwrap()
            .unwrap();
        assert_eq!(dict_after, dict_before, "a refused request interns nothing");

        // The six characters `\u0000` are not the escape when the
        // backslash is itself escaped, and a comment is not a term.
        for q in [
            r#"SELECT ?s WHERE { ?s ?p "a\\u0000b" }"#,
            "SELECT ?s WHERE { ?s ?p ?o } # \\u0000",
        ] {
            let (state, msg) = sparql(q);
            assert_eq!(state, "00000", "{q}: {msg}");
        }
    }

    /// A derived table carries at most 64 distinct variables. Past that
    /// the request hits a limit of the implementation: 54000.
    #[pg_test]
    fn projection_arity_limit_refuses_54000() {
        let vars: Vec<String> = (0..65).map(|i| format!("?v{i}")).collect();
        let vals: Vec<String> = (0..65).map(|i| i.to_string()).collect();
        let q = format!(
            "SELECT ?s WHERE {{ ?s ?p ?o VALUES ({}) {{ ({}) }} }}",
            vars.join(" "),
            vals.join(" ")
        );
        assert_outcome(
            sparql(&q),
            "54000",
            "sparql: derived-table arity exceeded 64 distinct variables",
            "65-variable VALUES",
        );
        let within = format!(
            "SELECT ?s WHERE {{ ?s ?p ?o VALUES ({}) {{ ({}) }} }}",
            vars[..64].join(" "),
            vals[..64].join(" ")
        );
        let (state, msg) = sparql(&within);
        assert_eq!(state, "00000", "64 variables are within the limit: {msg}");
    }
    fn quads_in(graph_id: i64) -> i64 {
        Spi::get_one_with_args(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads WHERE graph_id = $1",
            &[graph_id.into()],
        )
        .unwrap()
        .unwrap()
    }

    /// An UPDATE template pgRDF cannot instantiate refuses whether or
    /// not the WHERE pattern matches anything: the outcome is a property
    /// of the request. Each shape runs twice — over a pattern that
    /// matches nothing and over one that matches — and must refuse the
    /// same way both times, leaving the store untouched.
    #[pg_test]
    fn update_template_refusals_do_not_depend_on_the_data() {
        let g = seed("urn:tr:tpl");
        let total_before: i64 = Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads")
            .unwrap()
            .unwrap();
        // (template-and-form with a `{W}` slot for the WHERE predicate,
        //  SQLSTATE, message fragment)
        let shapes = [
            (
                "INSERT { _:b <urn:tr:q> ?o } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "INSERT WHERE template feature 'blank node in template (b)'",
            ),
            (
                "INSERT { ?s <urn:tr:q> _:b } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "INSERT WHERE template feature 'blank node in template (b)'",
            ),
            (
                "INSERT { GRAPH ?o { ?s <urn:tr:q> ?o } } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "INSERT WHERE template feature 'variable GRAPH ?o'",
            ),
            (
                "INSERT { \"lit\" <urn:tr:q> ?o } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "22023",
                "sparql: literal subject is invalid in RDF",
            ),
            (
                "DELETE { GRAPH ?o { ?s <urn:tr:p> ?o } } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "DELETE WHERE template feature 'variable GRAPH ?o'",
            ),
            // A constant the dictionary has never seen used to end the
            // row's instantiation before the GRAPH check was reached.
            (
                "DELETE { GRAPH ?o { <urn:tr:never-seen> <urn:tr:p> ?o } } \
                 WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "DELETE WHERE template feature 'variable GRAPH ?o'",
            ),
            (
                "DELETE { \"lit\" <urn:tr:p> ?o } WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "22023",
                "sparql: literal subject is invalid in RDF",
            ),
            (
                "DELETE { ?s <urn:tr:p> ?o } INSERT { _:b <urn:tr:q> ?o } \
                 WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "INSERT WHERE template feature 'blank node in template (b)'",
            ),
            (
                "DELETE { GRAPH ?o { ?s <urn:tr:p> ?o } } INSERT { ?s <urn:tr:q> ?o } \
                 WHERE { GRAPH <urn:tr:tpl> { ?s {W} ?o } }",
                "0A000",
                "DELETE WHERE template feature 'variable GRAPH ?o'",
            ),
        ];
        for (shape, state, fragment) in shapes {
            for (label, predicate) in [("no match", "<urn:tr:nomatch>"), ("match", "<urn:tr:p>")] {
                let q = shape.replace("{W}", predicate);
                assert_outcome(sparql(&q), state, fragment, &format!("{label}: {q}"));
            }
        }
        let total_after: i64 = Spi::get_one("SELECT count(*)::bigint FROM pgrdf._pgrdf_quads")
            .unwrap()
            .unwrap();
        assert_eq!(total_after, total_before, "no refused UPDATE wrote a quad");
        assert_eq!(quads_in(g), 3);
    }

    /// CLEAR and DROP of a graph that does not exist: 42704. CREATE of
    /// one that does: 42710. `SILENT` suppresses each, as W3C SPARQL 1.1
    /// Update §3.2 requires.
    #[pg_test]
    fn graph_management_refusals_are_typed_and_silent_is_honoured() {
        let g = seed("urn:tr:mgmt");
        assert_outcome(
            sparql("CLEAR GRAPH <urn:tr:absent>"),
            "42704",
            "sparql: CLEAR GRAPH <urn:tr:absent>: graph not bound",
            "CLEAR",
        );
        assert_outcome(
            sparql("DROP GRAPH <urn:tr:absent>"),
            "42704",
            "sparql: DROP GRAPH <urn:tr:absent>: graph not bound",
            "DROP",
        );
        assert_outcome(
            sparql("CREATE GRAPH <urn:tr:mgmt>"),
            "42710",
            "sparql: CREATE GRAPH <urn:tr:mgmt>: graph already exists",
            "CREATE",
        );
        for q in [
            "CLEAR SILENT GRAPH <urn:tr:absent>",
            "DROP SILENT GRAPH <urn:tr:absent>",
            "CREATE SILENT GRAPH <urn:tr:mgmt>",
        ] {
            let (state, msg) = sparql(q);
            assert_eq!(state, "00000", "{q}: {msg}");
        }
        assert_eq!(
            quads_in(g),
            3,
            "none of the six touched the graph's triples"
        );
    }

    /// `graph_integrity` of a graph id nobody registered: 42704.
    #[pg_test]
    fn graph_integrity_of_unknown_graph_refuses_42704() {
        assert_outcome(
            outcome("SELECT pgrdf.graph_integrity(970998)"),
            "42704",
            "graph_integrity: no graph with id 970998 (see pgrdf._pgrdf_graphs)",
            "graph_integrity",
        );
    }

    /// COPY / ADD / MOVE from a graph that does not exist refuse 42704
    /// and leave the destination exactly as it was. Before: COPY emptied
    /// the destination and reported success, ADD did nothing and
    /// reported success.
    #[pg_test]
    fn copy_add_move_from_unknown_graph_refuse_42704_before_any_write() {
        let g = seed("urn:tr:dst");
        for q in [
            "COPY <urn:tr:typo> TO <urn:tr:dst>",
            "ADD <urn:tr:typo> TO <urn:tr:dst>",
            "MOVE <urn:tr:typo> TO <urn:tr:dst>",
            // spargebra drops SILENT while rewriting these three, so it
            // cannot be honoured: stricter than W3C, on purpose.
            "COPY SILENT <urn:tr:typo> TO <urn:tr:dst>",
            "ADD SILENT <urn:tr:typo> TO <urn:tr:dst>",
            "MOVE SILENT <urn:tr:typo> TO <urn:tr:dst>",
            "COPY GRAPH <urn:tr:typo> TO GRAPH <urn:tr:dst>",
            "copy<urn:tr:typo>to<urn:tr:dst>",
            "PREFIX tr: <urn:tr:> COPY tr:typo TO tr:dst",
            // The refusal covers the whole request: the INSERT DATA
            // before it is rolled back with it.
            "INSERT DATA { GRAPH <urn:tr:dst> { <urn:tr:x> <urn:tr:p> 1 } } ; \
             COPY <urn:tr:typo> TO <urn:tr:dst>",
        ] {
            let got = sparql(q);
            assert_outcome(got, "42704", "source graph <urn:tr:typo> does not exist", q);
            assert_eq!(quads_in(g), 3, "{q}: the destination keeps its triples");
        }
        // The default graph as destination is protected the same way.
        let default_before = quads_in(0);
        assert!(default_before > 0);
        assert_outcome(
            sparql("COPY <urn:tr:typo> TO DEFAULT"),
            "42704",
            "source graph <urn:tr:typo> does not exist",
            "COPY … TO DEFAULT",
        );
        assert_eq!(quads_in(0), default_before);
        let absent: bool = Spi::get_one(
            "SELECT NOT EXISTS (SELECT 1 FROM pgrdf._pgrdf_graphs WHERE iri = 'urn:tr:typo')",
        )
        .unwrap()
        .unwrap();
        assert!(absent, "a refused source is not created as a side effect");
    }

    /// Triples in the graph named `iri` (0 when no such graph). By IRI,
    /// not id: COPY and MOVE drop the destination and re-create it.
    fn quads_of(iri: &str) -> i64 {
        Spi::get_one_with_args(
            "SELECT count(*)::bigint FROM pgrdf._pgrdf_quads q \
               JOIN pgrdf._pgrdf_graphs g USING (graph_id) WHERE g.iri = $1",
            &[iri.into()],
        )
        .unwrap()
        .unwrap()
    }

    /// What keeps working: an existing source, the default graph as
    /// source, a source created earlier in the same request, the
    /// identity case, and an INSERT … WHERE the caller wrote by hand —
    /// that one is not COPY/ADD/MOVE, and W3C gives a pattern over a
    /// missing graph zero solutions, not an error.
    #[pg_test]
    fn copy_add_move_with_a_real_source_still_work() {
        seed("urn:tr:src");
        let ok = |q: &str| {
            let (state, msg) = sparql(q);
            assert_eq!(state, "00000", "{q}: {msg}");
        };
        ok("COPY <urn:tr:src> TO <urn:tr:c1>");
        assert_eq!(quads_of("urn:tr:c1"), 3);
        ok("ADD <urn:tr:src> TO <urn:tr:c1>");
        assert_eq!(
            quads_of("urn:tr:c1"),
            3,
            "ADD of the same triples adds none"
        );

        // The default graph always exists, so it is never an unknown
        // source.
        ok("COPY DEFAULT TO <urn:tr:c2>");
        assert!(quads_of("urn:tr:c2") > 0);
        ok("ADD DEFAULT TO <urn:tr:c2>");

        // The source exists by the time the COPY runs.
        ok("CREATE GRAPH <urn:tr:fresh> ; COPY <urn:tr:fresh> TO <urn:tr:c2>");
        assert_eq!(
            quads_of("urn:tr:c2"),
            0,
            "copying an empty graph leaves the target without triples"
        );

        ok("COPY <urn:tr:src> TO <urn:tr:src>");
        assert_eq!(
            quads_of("urn:tr:src"),
            3,
            "a copy onto itself changes nothing"
        );

        ok("MOVE <urn:tr:src> TO <urn:tr:c3>");
        assert_eq!(quads_of("urn:tr:c3"), 3);
        assert_eq!(quads_of("urn:tr:src"), 0);

        for q in [
            "INSERT { GRAPH <urn:tr:c1> { ?s ?p ?o } } WHERE { GRAPH <urn:tr:typo> { ?s ?p ?o } }",
            // Even beside a real COPY the hand-written one is told apart.
            "COPY <urn:tr:c1> TO <urn:tr:c4> ; \
             INSERT { GRAPH <urn:tr:c1> { ?s ?p ?o } } WHERE { GRAPH <urn:tr:typo> { ?s ?p ?o } }",
        ] {
            ok(q);
            assert_eq!(quads_of("urn:tr:c1"), 3);
        }
        assert_eq!(quads_of("urn:tr:c4"), 3);
    }
}
