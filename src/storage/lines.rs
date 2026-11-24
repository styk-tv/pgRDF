//! Whole-graph reads in bounded memory: one line formatter, one ordered
//! stream.
//!
//! `export_graph`, `graph_manifest`, `graph_digest` and `structural_digest`
//! each read every asserted triple of a graph. They used to pull the graph
//! into Rust vectors and sort it there, so backend memory grew with the
//! graph (about a gigabyte per million triples for a manifest) and nothing
//! PostgreSQL controls bounded it.
//!
//! Here the graph stays in PostgreSQL:
//!
//! * [`nt_line`] (SQL: `pgrdf._nt_line`) serialises one stored triple to
//!   one line, with the same term serialisers the digests always used;
//! * the order is `ORDER BY <line> COLLATE "C"`, PostgreSQL's own sort,
//!   which spills to disk under `work_mem`;
//! * Rust reads the sorted lines through a cursor, a fixed batch at a
//!   time, and hashes or returns each line as it arrives.
//!
//! What a caller holds is a batch of lines, plus, for the two digests that
//! relabel blank nodes, the distinct triples that involve a blank node,
//! under `pgrdf.digest_max_bnode_quads`.
//!
//! **A graph is a set.** The same triple stored twice is one triple: the
//! sorted stream drops adjacent equal lines, and the blank-node triples are
//! de-duplicated before they are labelled (a repeat would change a blank
//! node's first-degree hash). Equality is equality of the serialised
//! triple, so one term stored under two dictionary ids is still one term.
//!
//! **No term containing U+0000 is emitted.** Versions before 0.6.41 could
//! store one. The formatter refuses it (XX001), and because PostgreSQL
//! formats every row before it returns the first sorted one, a refusal
//! arrives before any line does.

use crate::storage::canon::{CTerm, TermRef, Triple, nt_term_ref};
use crate::storage::dict::term_type;
use pgrx::prelude::*;
use std::collections::{HashSet, VecDeque};

const XSD_STRING: &str = "http://www.w3.org/2001/XMLSchema#string";

/// Rows fetched from the cursor per round trip. The lines of one batch are
/// what a streaming read holds.
const BATCH: std::ffi::c_long = 4096;

/// The line forms `pgrdf._nt_line` produces.
pub(crate) mod style {
    /// Canonical N-Triples, the stored blank-node labels, no newline:
    /// `export_graph` and the manifest's bytes digest.
    pub const EXPORT: i32 = 0;
    /// Canonical N-Triples with the trailing newline, ground triples only
    /// (a triple that involves a blank node yields an empty line): the
    /// RDFC-1.0 document, which sorts its lines newline included.
    pub const RDFC: i32 = 1;
    /// The fleet's first-degree line format, ground triples only: the
    /// ground part of `structural_digest`.
    pub const FD1: i32 = 2;
}

/// Why a whole-graph digest was not computed.
pub(crate) enum Refusal {
    /// More distinct blank-node-bearing triples than
    /// `pgrdf.digest_max_bnode_quads`.
    Budget { limit: usize },
}

impl Refusal {
    /// Raise this refusal as `verb`'s own error.
    pub(crate) fn raise(&self, verb: &str, graph_id: i64) -> ! {
        match self {
            Refusal::Budget { limit } => crate::refuse_with_hint(
                pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_PROGRAM_LIMIT_EXCEEDED,
                format!(
                    "{verb}: graph {graph_id} holds more than {limit} distinct triples that \
                     involve a blank node (pgrdf.digest_max_bnode_quads={limit})"
                ),
                "Blank-node labelling needs those triples in memory together; ground \
                 triples are streamed and do not count. Raise \
                 pgrdf.digest_max_bnode_quads deliberately, with the memory to hold them."
                    .to_string(),
            ),
        }
    }
}

/// Refuse to emit a stored term that contains U+0000: XX001, with the
/// dictionary id(s), the graph and the cure in the HINT.
fn refuse_stored_nul(graph_id: i64, ids: &[i64], sample: &str) -> ! {
    let ids = ids
        .iter()
        .map(|i| i.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let shown = if sample.is_empty() {
        String::new()
    } else {
        format!(": \"{}\"", crate::storage::dict::nul_term_preview(sample))
    };
    crate::refuse_with_hint(
        pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_DATA_CORRUPTED,
        format!(
            "pgrdf: graph {graph_id} holds a term containing U+0000 (dictionary id {ids}), \
             which cannot be exported or digested{shown}"
        ),
        format!(
            "Dictionary id {ids} in graph {graph_id} was stored by a version before 0.6.41; \
             pgrdf.nul_term_report() lists every such term and the graphs using it. \
             Reload graph {graph_id} from a source with the character removed or replaced."
        ),
    )
}

/// One stored triple as one line (SQL: `pgrdf._nt_line`).
///
/// Takes the triple the way the dictionary join yields it: per term its
/// id, type and lexical value, and for the object its datatype IRI and
/// language tag. Returns NULL for a row that is not an RDF triple (a
/// literal in subject position, an unknown term type): such a row was
/// never exported or digested. `style` picks the line form, see
/// [`style`].
///
/// Refuses XX001 when a term of the row contains U+0000, naming the
/// dictionary id and the graph.
#[allow(clippy::too_many_arguments)]
#[pg_extern(immutable, parallel_safe, name = "_nt_line")]
fn nt_line(
    style: i32,
    graph_id: i64,
    s_id: i64,
    s_type: i16,
    s_value: &str,
    p_id: i64,
    p_value: &str,
    o_id: i64,
    o_type: i16,
    o_value: &str,
    o_datatype: Option<&str>,
    o_language: Option<&str>,
) -> Option<String> {
    let s = match s_type {
        term_type::URI => TermRef::Iri(s_value),
        term_type::BLANK_NODE => TermRef::BNode(s_value),
        _ => return None,
    };
    let p = TermRef::Iri(p_value);
    let o = match o_type {
        term_type::URI => TermRef::Iri(o_value),
        term_type::BLANK_NODE => TermRef::BNode(o_value),
        term_type::LITERAL => TermRef::Lit {
            val: o_value,
            dt: o_datatype,
            lang: o_language,
        },
        _ => return None,
    };

    let has_nul = |v: &str| v.as_bytes().contains(&0);
    let o_nul =
        has_nul(o_value) || o_datatype.is_some_and(has_nul) || o_language.is_some_and(has_nul);
    if has_nul(s_value) || has_nul(p_value) || o_nul {
        let mut ids = Vec::new();
        let mut sample = "";
        for (id, value, bad) in [
            (s_id, s_value, has_nul(s_value)),
            (p_id, p_value, has_nul(p_value)),
            (o_id, o_value, o_nul),
        ] {
            if bad {
                ids.push(id);
                sample = value;
            }
        }
        refuse_stored_nul(graph_id, &ids, sample);
    }

    let stored = |label: &str| label.to_string();
    match style {
        style::EXPORT => Some(format!(
            "{} {} {} .",
            nt_term_ref(&s, &stored),
            nt_term_ref(&p, &stored),
            nt_term_ref(&o, &stored)
        )),
        style::RDFC | style::FD1 => {
            if matches!(s, TermRef::BNode(_)) || matches!(o, TermRef::BNode(_)) {
                // Read as a structured row by the caller; the line is unused.
                return Some(String::new());
            }
            Some(if style == style::RDFC {
                format!(
                    "{} {} {} .\n",
                    nt_term_ref(&s, &stored),
                    nt_term_ref(&p, &stored),
                    nt_term_ref(&o, &stored)
                )
            } else {
                crate::storage::fd1::fleet_line(&s, &p, &o, &stored)
            })
        }
        other => crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
            format!("_nt_line: unknown style {other}"),
        ),
    }
}

/// A literal as a [`CTerm`], with the datatype that never shows in a
/// serialised term left out: `"x"` and `"x"^^xsd:string` are one term, and
/// a language-tagged literal has no other datatype. Two rows that
/// serialise alike are then equal as values too.
pub(crate) fn literal(val: String, dt: Option<String>, lang: Option<String>) -> CTerm {
    let dt = match (&lang, dt) {
        (Some(_), _) => None,
        (None, Some(d)) if d == XSD_STRING => None,
        (None, d) => d,
    };
    CTerm::Lit { val, dt, lang }
}

/// The join every whole-graph read runs: the asserted rows of graph `$1`
/// with their terms, and the line `pgrdf._nt_line` makes of each in style
/// `$2`. `OFFSET 0` keeps the planner from inlining the subquery, so the
/// formatter runs once per row.
fn stream_sql(split: bool) -> String {
    let blank = "(s.term_type = 2 OR o.term_type = 2)";
    let parts = if split {
        format!(
            ", {blank} AS bn, \
             CASE WHEN {blank} THEN s.term_type END AS st, \
             CASE WHEN {blank} THEN s.lexical_value END AS sv, \
             CASE WHEN {blank} THEN p.lexical_value END AS pv, \
             CASE WHEN {blank} THEN o.term_type END AS ot, \
             CASE WHEN {blank} THEN o.lexical_value END AS ov, \
             CASE WHEN {blank} THEN dt.lexical_value END AS od, \
             CASE WHEN {blank} THEN o.language_tag END AS ol"
        )
    } else {
        String::new()
    };
    let (cols, order) = if split {
        (
            "x.l, x.bn, x.st, x.sv, x.pv, x.ot, x.ov, x.od, x.ol",
            "x.bn DESC, x.l COLLATE \"C\"",
        )
    } else {
        ("x.l", "x.l COLLATE \"C\"")
    };
    format!(
        "SELECT {cols} FROM ( \
            SELECT pgrdf._nt_line($2, q.graph_id, \
                       q.subject_id, s.term_type, s.lexical_value, \
                       q.predicate_id, p.lexical_value, \
                       q.object_id, o.term_type, o.lexical_value, \
                       dt.lexical_value, o.language_tag) AS l{parts} \
              FROM pgrdf._pgrdf_quads q \
              JOIN pgrdf._pgrdf_dictionary s ON s.id = q.subject_id \
              JOIN pgrdf._pgrdf_dictionary p ON p.id = q.predicate_id \
              JOIN pgrdf._pgrdf_dictionary o ON o.id = q.object_id \
              LEFT JOIN pgrdf._pgrdf_dictionary dt ON dt.id = o.datatype_iri_id \
             WHERE q.graph_id = $1 AND q.is_inferred = FALSE \
            OFFSET 0 \
         ) x WHERE x.l IS NOT NULL ORDER BY {order}"
    )
}

/// A cursor left open in the transaction and read one batch per SPI
/// session, so each batch's rows are freed before the next is fetched.
struct Portal {
    name: Option<String>,
}

impl Portal {
    fn open(sql: &str, graph_id: i64, style: i32) -> Self {
        let name = Spi::connect(|c| {
            c.open_cursor(sql, &[graph_id.into(), style.into()])
                .detach_into_name()
        });
        Portal { name: Some(name) }
    }

    /// Fetch the next batch, handing each row to `each`. Returns the
    /// number of rows; 0 means the cursor is exhausted, and closed.
    fn fetch(&mut self, mut each: impl FnMut(&pgrx::spi::SpiHeapTupleData<'_>)) -> usize {
        let Some(name) = self.name.take() else {
            return 0;
        };
        let (n, name) = Spi::connect(|c| {
            let mut cursor = c
                .find_cursor(&name)
                .expect("whole-graph read: the cursor vanished");
            let table = cursor.fetch(BATCH).expect("whole-graph read: fetch failed");
            let n = table.len();
            for row in table {
                each(&row);
            }
            if n == 0 {
                // Dropping the cursor closes it.
                (0, None)
            } else {
                (n, Some(cursor.detach_into_name()))
            }
        });
        self.name = name;
        n
    }

    /// Close the cursor before it is exhausted.
    fn close(&mut self) {
        if let Some(name) = self.name.take() {
            Spi::connect(|c| drop(c.find_cursor(&name)));
        }
    }
}

/// The sorted lines of a graph with adjacent repeats dropped: an iterator
/// over the SET of serialised triples, in byte order.
pub(crate) struct Lines {
    portal: Portal,
    pending: VecDeque<String>,
    /// The next line to return, read ahead to see where its repeats end.
    ahead: Option<String>,
    /// Lines read, repeats included.
    pub(crate) rows: i64,
    /// Lines returned.
    pub(crate) distinct: i64,
}

impl Lines {
    /// Every asserted triple of `graph_id` as a canonical N-Triples line.
    /// PostgreSQL sorts before the first batch returns, so a stored
    /// U+0000 is refused here, before any line is handed out.
    pub(crate) fn export(graph_id: i64) -> Self {
        let mut lines = Lines {
            portal: Portal::open(&stream_sql(false), graph_id, style::EXPORT),
            pending: VecDeque::new(),
            ahead: None,
            rows: 0,
            distinct: 0,
        };
        lines.refill();
        lines
    }

    fn refill(&mut self) -> bool {
        let pending = &mut self.pending;
        self.portal.fetch(|row| {
            pending.push_back(
                row.get::<String>(1)
                    .ok()
                    .flatten()
                    .expect("whole-graph read: line"),
            );
        }) > 0
    }

    fn raw(&mut self) -> Option<String> {
        if self.pending.is_empty() && !self.refill() {
            return None;
        }
        self.rows += 1;
        self.pending.pop_front()
    }
}

impl Iterator for Lines {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        let current = match self.ahead.take() {
            Some(line) => line,
            None => self.raw()?,
        };
        // The stream is sorted: the repeats of a line follow it directly.
        while let Some(line) = self.raw() {
            if line != current {
                self.ahead = Some(line);
                break;
            }
        }
        self.distinct += 1;
        Some(current)
    }
}

/// A graph read for a digest that relabels blank nodes: the distinct
/// triples involving a blank node, held; and the ground triples as sorted
/// lines, streamed.
pub(crate) struct Split {
    /// The set of blank-node-bearing triples, in no particular order.
    pub(crate) blank: Vec<Triple>,
    /// The ground lines in `style`, sorted, repeats dropped.
    pub(crate) ground: Lines,
}

impl Split {
    /// Read `graph_id` in a ground-only `style`. One cursor: the query
    /// returns the blank-node rows first, then the sorted ground lines.
    pub(crate) fn open(graph_id: i64, style: i32) -> Result<Self, Refusal> {
        let limit = crate::query::guc::digest_max_bnode_quads();
        let mut portal = Portal::open(&stream_sql(true), graph_id, style);
        let mut blank: HashSet<Triple> = HashSet::new();
        let mut pending: VecDeque<String> = VecDeque::new();
        let mut over = false;
        loop {
            let n = portal.fetch(|row| {
                let is_blank: bool = row.get(2).ok().flatten().expect("whole-graph read: bn");
                if !is_blank {
                    pending.push_back(
                        row.get::<String>(1)
                            .ok()
                            .flatten()
                            .expect("whole-graph read: line"),
                    );
                    return;
                }
                if over {
                    return;
                }
                let text = |n: usize| row.get::<String>(n).ok().flatten();
                let s_val = text(4).expect("whole-graph read: s.value");
                let s = match row.get::<i16>(3).ok().flatten() {
                    Some(term_type::URI) => CTerm::Iri(s_val),
                    _ => CTerm::BNode(s_val),
                };
                let p = CTerm::Iri(text(5).expect("whole-graph read: p.value"));
                let o_val = text(7).expect("whole-graph read: o.value");
                let o = match row.get::<i16>(6).ok().flatten() {
                    Some(term_type::URI) => CTerm::Iri(o_val),
                    Some(term_type::BLANK_NODE) => CTerm::BNode(o_val),
                    _ => literal(o_val, text(8), text(9)),
                };
                blank.insert((s, p, o));
                if blank.len() > limit {
                    over = true;
                }
            });
            if over {
                portal.close();
                return Err(Refusal::Budget { limit });
            }
            // Blank-node rows sort first: once a ground line has arrived,
            // or the cursor is exhausted, the held set is complete.
            if n == 0 || !pending.is_empty() {
                break;
            }
        }
        Ok(Split {
            blank: blank.into_iter().collect(),
            ground: Lines {
                portal,
                pending,
                ahead: None,
                rows: 0,
                distinct: 0,
            },
        })
    }
}

/// Stored terms that contain U+0000, with the graphs using them.
///
/// Versions before 0.6.41 could store such a term (PostgreSQL `text`
/// cannot hold U+0000, and reading one back fails or truncates). Ingest
/// now refuses them, and the whole-graph reads refuse a graph that still
/// holds one. This lists what is left: one row per term, its dictionary
/// id, its type (1 IRI, 2 blank node, 3 literal), a preview with the
/// character written `\u0000`, and the ids of the graphs with a triple
/// that uses the term in any position (asserted or inferred). The cure is
/// to reload those graphs from a corrected source.
///
/// Reads the whole dictionary: an occasional check, not a per-request one.
#[search_path(pgrdf, pg_temp)]
#[pg_extern(stable)]
fn nul_term_report() -> TableIterator<
    'static,
    (
        name!(term_id, i64),
        name!(term_type, i16),
        name!(preview, String),
        name!(graph_ids, Vec<i64>),
    ),
> {
    let mut rows = Vec::new();
    Spi::connect(|c| {
        // length(), LIKE and the regex engine all stop at a NUL, so the
        // test is on the value's bytes.
        let table = c
            .select(
                "SELECT d.id, d.term_type, d.lexical_value,
                        ARRAY(SELECT u.g FROM (
                                  SELECT q.graph_id AS g FROM pgrdf._pgrdf_quads q
                                   WHERE q.subject_id = d.id
                                  UNION
                                  SELECT q.graph_id FROM pgrdf._pgrdf_quads q
                                   WHERE q.predicate_id = d.id
                                  UNION
                                  SELECT q.graph_id FROM pgrdf._pgrdf_quads q
                                   WHERE q.object_id = d.id
                                  UNION
                                  SELECT q.graph_id FROM pgrdf._pgrdf_quads q
                                    JOIN pgrdf._pgrdf_dictionary l ON l.id = q.object_id
                                   WHERE d.term_type = 1 AND l.datatype_iri_id = d.id
                              ) u ORDER BY u.g)
                   FROM pgrdf._pgrdf_dictionary d
                  WHERE position('\\x00'::bytea IN pg_catalog.textsend(d.lexical_value)) > 0
                  ORDER BY d.id",
                None,
                &[],
            )
            .expect("nul_term_report: dictionary read failed");
        for row in table {
            let value: String = row.get(3).ok().flatten().unwrap_or_default();
            rows.push((
                row.get::<i64>(1)
                    .ok()
                    .flatten()
                    .expect("nul_term_report: id"),
                row.get::<i16>(2)
                    .ok()
                    .flatten()
                    .expect("nul_term_report: type"),
                crate::storage::dict::nul_term_preview(&value),
                row.get::<Vec<i64>>(4).ok().flatten().unwrap_or_default(),
            ));
        }
    });
    TableIterator::new(rows)
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use super::XSD_STRING;
    use pgrx::prelude::*;

    const XSD_INTEGER: &str = "http://www.w3.org/2001/XMLSchema#integer";

    /// One term of a fixture triple.
    #[derive(Clone, Copy)]
    pub(crate) enum T {
        /// An IRI.
        I(&'static str),
        /// A blank node with this stored label.
        B(&'static str),
        /// A literal stored the way the loaders store a simple literal
        /// (datatype xsd:string).
        L(&'static str),
        /// A literal stored with no datatype and no language.
        Bare(&'static str),
        /// A typed literal.
        D(&'static str, &'static str),
        /// A language-tagged literal.
        Lang(&'static str, &'static str),
    }
    use T::*;

    /// The dictionary id of a term, inserting the row when it is not
    /// there. Plain SQL on the dictionary, so a fixture stores exactly the
    /// rows it names whatever the ingest code does.
    fn dict_id(ty: i16, val: &str, dt: Option<i64>, lang: Option<&str>) -> i64 {
        let found: Option<i64> = Spi::get_one_with_args(
            "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary \
                      WHERE term_type = $1 AND lexical_value = $2 \
                        AND datatype_iri_id IS NOT DISTINCT FROM $3 \
                        AND language_tag IS NOT DISTINCT FROM $4 \
                      ORDER BY id LIMIT 1)",
            &[ty.into(), val.into(), dt.into(), lang.into()],
        )
        .unwrap();
        if let Some(id) = found {
            return id;
        }
        Spi::get_one_with_args(
            "INSERT INTO pgrdf._pgrdf_dictionary \
               (term_type, lexical_value, datatype_iri_id, language_tag) \
             VALUES ($1, $2, $3, $4) RETURNING id",
            &[ty.into(), val.into(), dt.into(), lang.into()],
        )
        .unwrap()
        .unwrap()
    }

    pub(crate) fn term_id(t: T) -> i64 {
        match t {
            I(v) => dict_id(1, v, None, None),
            B(v) => dict_id(2, v, None, None),
            L(v) => {
                let dt = dict_id(1, XSD_STRING, None, None);
                dict_id(3, v, Some(dt), None)
            }
            Bare(v) => dict_id(3, v, None, None),
            D(v, d) => {
                let dt = dict_id(1, d, None, None);
                dict_id(3, v, Some(dt), None)
            }
            Lang(v, l) => dict_id(3, v, None, Some(l)),
        }
    }

    /// Store one row of a fixture.
    pub(crate) fn put(graph_id: i64, s: T, p: T, o: T, inferred: bool) {
        let (s, p, o) = (term_id(s), term_id(p), term_id(o));
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_quads \
               (subject_id, predicate_id, object_id, graph_id, is_inferred) \
             VALUES ($1, $2, $3, $4, $5)",
            &[
                s.into(),
                p.into(),
                o.into(),
                graph_id.into(),
                inferred.into(),
            ],
        )
        .unwrap();
    }

    pub(crate) fn fill(graph_id: i64, triples: &[(T, T, T)]) {
        for (s, p, o) in triples {
            put(graph_id, *s, *p, *o, false);
        }
    }

    pub(crate) fn new_graph(graph_id: i64) {
        Spi::run(&format!("SELECT pgrdf.add_graph({graph_id})")).unwrap();
    }

    // ── fixtures ───────────────────────────────────────────────────────

    /// Ground triples only: every literal form, the escapes, non-ASCII,
    /// and the pairs where one serialised term is a prefix of another.
    pub(crate) fn ground_triples() -> Vec<(T, T, T)> {
        let p = I("urn:wg:p");
        vec![
            (I("urn:wg:s1"), p, I("urn:wg:o1")),
            (I("urn:wg:s1"), p, I("urn:wg:o10")),
            (I("urn:wg:s10"), p, I("urn:wg:o1")),
            (I("urn:wg:s1"), p, L("x")),
            (I("urn:wg:s1"), p, L("x y")),
            (I("urn:wg:s1"), p, Lang("x", "en")),
            (I("urn:wg:s1"), p, Lang("x", "en-GB")),
            (I("urn:wg:s1"), p, D("x", "urn:wg:dt")),
            (I("urn:wg:s1"), p, Bare("bare")),
            (I("urn:wg:s1"), p, L("")),
            (I("urn:wg:s1"), p, D("42", XSD_INTEGER)),
            (I("urn:wg:s1"), I("urn:wg:p2"), L("x")),
            (
                I("urn:wg:s2"),
                p,
                L("line1\nline2\ttab\rcr \"quoted\" back\\slash"),
            ),
            (I("urn:wg:s2"), p, L("a\" .")),
            (
                I("urn:wg:s2"),
                p,
                L("h\u{e9}llo w\u{f6}rld \u{2014} \u{65e5}\u{672c}\u{8a9e} \u{1f389}"),
            ),
            (I("urn:wg:s2"), p, L("ctl\u{1}and\u{7f}del")),
            (I("urn:wg:\u{fc}"), p, I("urn:wg:\u{65e5}\u{672c}")),
            (I("urn:wg:s2"), I("urn:wg:q"), Lang("\u{e9}t\u{e9}", "fr")),
        ]
    }

    /// Blank nodes with fixed stored labels, in subject and object
    /// position: shared, symmetric, a 4-cycle (needs the n-degree hash),
    /// a label that is a prefix of another, and ground triples that sort
    /// between the blank-node lines.
    pub(crate) fn bnode_triples() -> Vec<(T, T, T)> {
        let p = I("urn:wg:p");
        let q = I("urn:wg:q");
        vec![
            (I("urn:wg:s1"), p, I("urn:wg:o")),
            (I("urn:wg:s1"), p, B("b1")),
            (I("urn:wg:s1"), p, L("lit")),
            (I("urn:wg:s1"), q, L("z")),
            (I("urn:wg:s2"), q, B("sh")),
            (I("urn:wg:s3"), q, B("sh")),
            (B("sh"), p, Lang("v", "en")),
            (B("b1"), p, I("urn:wg:o")),
            (B("b1"), p, L("t\tab")),
            (B("b10"), p, I("urn:wg:o")),
            (B("b2"), p, B("b3")),
            (B("b3"), p, B("b2")),
            (B("c1"), q, B("c2")),
            (B("c2"), q, B("c3")),
            (B("c3"), q, B("c4")),
            (B("c4"), q, B("c1")),
            (B("c1"), p, L("marked")),
            (B("lone"), p, B("lone")),
        ]
    }

    /// A small graph for the default graph, the inferred-rows fixture and
    /// the locked one.
    pub(crate) fn small_triples() -> Vec<(T, T, T)> {
        let p = I("urn:wg:p");
        vec![
            (I("urn:wg:a"), p, I("urn:wg:b")),
            (I("urn:wg:a"), p, L("one")),
            (I("urn:wg:a"), p, B("k")),
            (B("k"), p, D("7", XSD_INTEGER)),
        ]
    }

    /// What the whole-graph functions say about one graph.
    #[derive(Debug, PartialEq)]
    pub(crate) struct Reading {
        pub graph_digest: String,
        pub structural_digest: String,
        pub manifest_bytes: String,
        pub manifest_identity: String,
        pub manifest_structure: String,
        pub asserted: i64,
        pub inferred: i64,
        pub export: Vec<String>,
    }

    pub(crate) fn export(graph_id: i64) -> Vec<String> {
        let mut out = Vec::new();
        Spi::connect(|c| {
            for r in c
                .select(
                    "SELECT l FROM pgrdf.export_graph($1) WITH ORDINALITY AS e(l, n) ORDER BY n",
                    None,
                    &[graph_id.into()],
                )
                .unwrap()
            {
                out.push(r.get::<String>(1).unwrap().unwrap());
            }
        });
        out
    }

    pub(crate) fn manifest(graph_id: i64) -> serde_json::Value {
        Spi::get_one_with_args::<pgrx::JsonB>("SELECT pgrdf.graph_manifest($1)", &[graph_id.into()])
            .unwrap()
            .unwrap()
            .0
    }

    pub(crate) fn read(graph_id: i64) -> Reading {
        let one = |f: &str| -> String {
            Spi::get_one_with_args(&format!("SELECT pgrdf.{f}($1)"), &[graph_id.into()])
                .unwrap()
                .unwrap()
        };
        let m = manifest(graph_id);
        let s = |v: &serde_json::Value| v.as_str().unwrap().to_string();
        Reading {
            graph_digest: one("graph_digest"),
            structural_digest: one("structural_digest"),
            manifest_bytes: s(&m["digests"]["bytes"]["value"]),
            manifest_identity: s(&m["digests"]["identity"]["value"]),
            manifest_structure: s(&m["digests"]["structure"]["value"]),
            asserted: m["counts"]["asserted"].as_i64().unwrap(),
            inferred: m["counts"]["inferred"].as_i64().unwrap(),
            export: export(graph_id),
        }
    }

    /// Build every pinned fixture and return its reading, by name.
    fn build_all() -> Vec<(&'static str, Reading)> {
        let mut out = Vec::new();

        new_graph(986101);
        fill(986101, &ground_triples());
        out.push(("ground", read(986101)));

        new_graph(986102);
        fill(986102, &bnode_triples());
        out.push(("bnodes", read(986102)));

        new_graph(986103);
        out.push(("empty", read(986103)));

        // The default graph. Other tests' rows are uncommitted and so
        // invisible; anything committed there is removed for this
        // transaction only.
        Spi::run("DELETE FROM pgrdf._pgrdf_quads WHERE graph_id = 0").unwrap();
        fill(0, &small_triples());
        out.push(("default", read(0)));

        new_graph(986104);
        fill(986104, &small_triples());
        put(
            986104,
            I("urn:wg:a"),
            I("urn:wg:p"),
            I("urn:wg:derived"),
            true,
        );
        put(986104, I("urn:wg:a"), I("urn:wg:p"), I("urn:wg:b"), true);
        put(986104, B("k"), I("urn:wg:p"), I("urn:wg:derived"), true);
        out.push(("inferred", read(986104)));

        new_graph(986105);
        fill(986105, &bnode_triples());
        fill(986105, &ground_triples()[3..]);
        Spi::run("SELECT pgrdf.lock_graph(986105, 'pinned')").unwrap();
        let first = read(986105);
        let again = read(986105);
        assert_eq!(first, again, "a locked graph reads the same twice");
        out.push(("locked", first));

        // A row whose subject is a literal: never exported, never
        // digested.
        new_graph(986106);
        fill(986106, &small_triples());
        put(
            986106,
            L("literal subject"),
            I("urn:wg:p"),
            I("urn:wg:b"),
            false,
        );
        out.push(("residue", read(986106)));

        out
    }

    /// One pinned fixture: name, `graph_digest`, `structural_digest`, the
    /// manifest's bytes / identity / structure digests, its asserted and
    /// inferred counts, and the full `export_graph` output.
    type Pinned = (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        i64,
        i64,
        &'static [&'static str],
    );

    /// What 0.6.40 answers for each fixture, recorded from that code
    /// before the whole-graph functions were changed. These values are the
    /// acceptance oracle: a graph that holds no repeated triple must keep
    /// reading exactly this, byte for byte. Never regenerate them from the
    /// code under test.
    const PINNED: &[Pinned] = &[
        (
            "ground",
            "29dfff2905a2653ad83fb507e81755506c2bee0d83f6b3aea385064d2f0ceef5",
            "896b4bdc14bfa30f055eda36e8bd49341a9f469d1252037314ec471549a0c5ba",
            "29dfff2905a2653ad83fb507e81755506c2bee0d83f6b3aea385064d2f0ceef5",
            "29dfff2905a2653ad83fb507e81755506c2bee0d83f6b3aea385064d2f0ceef5",
            "896b4bdc14bfa30f055eda36e8bd49341a9f469d1252037314ec471549a0c5ba",
            18,
            0,
            &[
                "<urn:wg:s10> <urn:wg:p> <urn:wg:o1> .",
                "<urn:wg:s1> <urn:wg:p2> \"x\" .",
                "<urn:wg:s1> <urn:wg:p> \"\" .",
                "<urn:wg:s1> <urn:wg:p> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                "<urn:wg:s1> <urn:wg:p> \"bare\" .",
                "<urn:wg:s1> <urn:wg:p> \"x y\" .",
                "<urn:wg:s1> <urn:wg:p> \"x\" .",
                "<urn:wg:s1> <urn:wg:p> \"x\"@en .",
                "<urn:wg:s1> <urn:wg:p> \"x\"@en-GB .",
                "<urn:wg:s1> <urn:wg:p> \"x\"^^<urn:wg:dt> .",
                "<urn:wg:s1> <urn:wg:p> <urn:wg:o10> .",
                "<urn:wg:s1> <urn:wg:p> <urn:wg:o1> .",
                "<urn:wg:s2> <urn:wg:p> \"a\\\" .\" .",
                "<urn:wg:s2> <urn:wg:p> \"ctl\u{1}and\u{7f}del\" .",
                "<urn:wg:s2> <urn:wg:p> \"héllo wörld — 日本語 🎉\" .",
                "<urn:wg:s2> <urn:wg:p> \"line1\\nline2\\ttab\\rcr \\\"quoted\\\" back\\\\slash\" .",
                "<urn:wg:s2> <urn:wg:q> \"été\"@fr .",
                "<urn:wg:ü> <urn:wg:p> <urn:wg:日本> .",
            ],
        ),
        (
            "bnodes",
            "5e944bb9d0ce344fd101322259e5756ae77b428e8e488ef8360c114178d95f80",
            "2e3118ad5f8766fc5ce9572733d55e2aafe4009c085fe27de6d01d9c6e9d762b",
            "e9b5adb12ddf256e6a9024bdf864b8d54879589b519ba98f9b725ff428c186f7",
            "5e944bb9d0ce344fd101322259e5756ae77b428e8e488ef8360c114178d95f80",
            "2e3118ad5f8766fc5ce9572733d55e2aafe4009c085fe27de6d01d9c6e9d762b",
            18,
            0,
            &[
                "<urn:wg:s1> <urn:wg:p> \"lit\" .",
                "<urn:wg:s1> <urn:wg:p> <urn:wg:o> .",
                "<urn:wg:s1> <urn:wg:p> _:b1 .",
                "<urn:wg:s1> <urn:wg:q> \"z\" .",
                "<urn:wg:s2> <urn:wg:q> _:sh .",
                "<urn:wg:s3> <urn:wg:q> _:sh .",
                "_:b1 <urn:wg:p> \"t\\tab\" .",
                "_:b1 <urn:wg:p> <urn:wg:o> .",
                "_:b10 <urn:wg:p> <urn:wg:o> .",
                "_:b2 <urn:wg:p> _:b3 .",
                "_:b3 <urn:wg:p> _:b2 .",
                "_:c1 <urn:wg:p> \"marked\" .",
                "_:c1 <urn:wg:q> _:c2 .",
                "_:c2 <urn:wg:q> _:c3 .",
                "_:c3 <urn:wg:q> _:c4 .",
                "_:c4 <urn:wg:q> _:c1 .",
                "_:lone <urn:wg:p> _:lone .",
                "_:sh <urn:wg:p> \"v\"@en .",
            ],
        ),
        (
            "empty",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "0e7d07ee6ebef2a84399f1e308a9f4ab5afa93a21400c792ac85fba53456b145",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "0e7d07ee6ebef2a84399f1e308a9f4ab5afa93a21400c792ac85fba53456b145",
            0,
            0,
            &[],
        ),
        (
            "default",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            "87f00edf8b048040a71659c243de762d6b11cb56a149905b7178a79ccfc339b2",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            4,
            0,
            &[
                "<urn:wg:a> <urn:wg:p> \"one\" .",
                "<urn:wg:a> <urn:wg:p> <urn:wg:b> .",
                "<urn:wg:a> <urn:wg:p> _:k .",
                "_:k <urn:wg:p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            ],
        ),
        (
            "inferred",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            "87f00edf8b048040a71659c243de762d6b11cb56a149905b7178a79ccfc339b2",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            4,
            3,
            &[
                "<urn:wg:a> <urn:wg:p> \"one\" .",
                "<urn:wg:a> <urn:wg:p> <urn:wg:b> .",
                "<urn:wg:a> <urn:wg:p> _:k .",
                "_:k <urn:wg:p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            ],
        ),
        (
            "locked",
            "aa29e8bcdd392335be51c3f66ecc05c65ab976c5a5641269d708a1ce0f03d458",
            "8111dcafbbece2b29b97e0ee6b80c2cc2adb0cdb8cf27df4e8e2be132a7f6fae",
            "38afff29761e1e52a33d8d5592d08e79433b44cffd8c776193f7465710ee021d",
            "aa29e8bcdd392335be51c3f66ecc05c65ab976c5a5641269d708a1ce0f03d458",
            "8111dcafbbece2b29b97e0ee6b80c2cc2adb0cdb8cf27df4e8e2be132a7f6fae",
            33,
            0,
            &[
                "<urn:wg:s1> <urn:wg:p2> \"x\" .",
                "<urn:wg:s1> <urn:wg:p> \"\" .",
                "<urn:wg:s1> <urn:wg:p> \"42\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                "<urn:wg:s1> <urn:wg:p> \"bare\" .",
                "<urn:wg:s1> <urn:wg:p> \"lit\" .",
                "<urn:wg:s1> <urn:wg:p> \"x y\" .",
                "<urn:wg:s1> <urn:wg:p> \"x\" .",
                "<urn:wg:s1> <urn:wg:p> \"x\"@en .",
                "<urn:wg:s1> <urn:wg:p> \"x\"@en-GB .",
                "<urn:wg:s1> <urn:wg:p> \"x\"^^<urn:wg:dt> .",
                "<urn:wg:s1> <urn:wg:p> <urn:wg:o> .",
                "<urn:wg:s1> <urn:wg:p> _:b1 .",
                "<urn:wg:s1> <urn:wg:q> \"z\" .",
                "<urn:wg:s2> <urn:wg:p> \"a\\\" .\" .",
                "<urn:wg:s2> <urn:wg:p> \"ctl\u{1}and\u{7f}del\" .",
                "<urn:wg:s2> <urn:wg:p> \"héllo wörld — 日本語 🎉\" .",
                "<urn:wg:s2> <urn:wg:p> \"line1\\nline2\\ttab\\rcr \\\"quoted\\\" back\\\\slash\" .",
                "<urn:wg:s2> <urn:wg:q> \"été\"@fr .",
                "<urn:wg:s2> <urn:wg:q> _:sh .",
                "<urn:wg:s3> <urn:wg:q> _:sh .",
                "<urn:wg:ü> <urn:wg:p> <urn:wg:日本> .",
                "_:b1 <urn:wg:p> \"t\\tab\" .",
                "_:b1 <urn:wg:p> <urn:wg:o> .",
                "_:b10 <urn:wg:p> <urn:wg:o> .",
                "_:b2 <urn:wg:p> _:b3 .",
                "_:b3 <urn:wg:p> _:b2 .",
                "_:c1 <urn:wg:p> \"marked\" .",
                "_:c1 <urn:wg:q> _:c2 .",
                "_:c2 <urn:wg:q> _:c3 .",
                "_:c3 <urn:wg:q> _:c4 .",
                "_:c4 <urn:wg:q> _:c1 .",
                "_:lone <urn:wg:p> _:lone .",
                "_:sh <urn:wg:p> \"v\"@en .",
            ],
        ),
        (
            "residue",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            "87f00edf8b048040a71659c243de762d6b11cb56a149905b7178a79ccfc339b2",
            "6517b00daa3d58af9b3bd737ab34da060024370ca84a538c59d0b40aa370b5c0",
            "c1e31c857d3744bfdf8a5b847edbdd1c71d2bbd0af2b5379adb59e70278b5e98",
            5,
            0,
            &[
                "<urn:wg:a> <urn:wg:p> \"one\" .",
                "<urn:wg:a> <urn:wg:p> <urn:wg:b> .",
                "<urn:wg:a> <urn:wg:p> _:k .",
                "_:k <urn:wg:p> \"7\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
            ],
        ),
    ];

    /// A graph without repeats reads byte-identically to 0.6.40: both
    /// digests, the three manifest digests, the counts and every export
    /// line, in order. (One count differs for one malformed fixture; see
    /// below.)
    #[pg_test]
    fn clean_graphs_read_as_before() {
        let got = build_all();
        assert_eq!(got.len(), PINNED.len());
        for ((name, r), pin) in got.iter().zip(PINNED) {
            assert_eq!(*name, pin.0);
            assert_eq!(r.graph_digest, pin.1, "{name}: graph_digest");
            assert_eq!(r.structural_digest, pin.2, "{name}: structural_digest");
            assert_eq!(r.manifest_bytes, pin.3, "{name}: manifest bytes digest");
            assert_eq!(r.manifest_identity, pin.4, "{name}: manifest identity");
            assert_eq!(r.manifest_structure, pin.5, "{name}: manifest structure");
            // The one value that deliberately moved: `asserted` is the
            // number of triples the export returns. 0.6.40 counted stored
            // rows, so for `residue` it included the literal-subject row
            // that it never exported or digested.
            let asserted = if *name == "residue" {
                assert_eq!(pin.6, pin.8.len() as i64 + 1);
                pin.8.len() as i64
            } else {
                pin.6
            };
            assert_eq!(r.asserted, asserted, "{name}: asserted count");
            assert_eq!(r.inferred, pin.7, "{name}: inferred count");
            let want: Vec<String> = pin.8.iter().map(|l| l.to_string()).collect();
            assert_eq!(r.export, want, "{name}: export_graph");
        }
    }

    // ── helpers for the behaviour tests ────────────────────────────────

    /// Make `graph_id` hold triples more than once: every asserted row is
    /// stored a second time, every third one four times in all. Returns
    /// the number of surplus rows.
    ///
    /// THE ONE PLACE a test creates repeated rows. Whatever keeps a graph
    /// duplicate-free on the write side changes here and nowhere else.
    pub(crate) fn repeat_rows(graph_id: i64) -> i64 {
        Spi::get_one_with_args::<i64>(
            "WITH ins AS (
                 INSERT INTO pgrdf._pgrdf_quads
                        (subject_id, predicate_id, object_id, graph_id, is_inferred)
                 SELECT r.subject_id, r.predicate_id, r.object_id, r.graph_id, false
                   FROM (SELECT q.subject_id, q.predicate_id, q.object_id, q.graph_id,
                                row_number() OVER (ORDER BY q.subject_id, q.predicate_id,
                                                            q.object_id) AS n
                           FROM pgrdf._pgrdf_quads q
                          WHERE q.graph_id = $1 AND NOT q.is_inferred) r,
                        LATERAL generate_series(1, CASE WHEN r.n % 3 = 0 THEN 3 ELSE 1 END)
                 RETURNING 1)
             SELECT count(*) FROM ins",
            &[graph_id.into()],
        )
        .unwrap()
        .unwrap()
    }

    /// Store a literal whose lexical form contains U+0000 and return its
    /// dictionary id. Versions before 0.6.41 could store one; ingest now
    /// refuses it, and an SQL text literal cannot carry one, so the row is
    /// written from Rust.
    ///
    /// THE ONE PLACE a test stores such a term.
    pub(crate) fn store_nul_literal(lexical_with_nul: &str) -> i64 {
        assert!(lexical_with_nul.contains('\0'));
        Spi::get_one_with_args(
            "INSERT INTO pgrdf._pgrdf_dictionary (term_type, lexical_value) \
             VALUES (3, $1) RETURNING id",
            &[lexical_with_nul.into()],
        )
        .unwrap()
        .unwrap()
    }

    fn put_ids(graph_id: i64, s: i64, p: i64, o: i64) {
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_quads \
               (subject_id, predicate_id, object_id, graph_id, is_inferred) \
             VALUES ($1, $2, $3, $4, false)",
            &[s.into(), p.into(), o.into(), graph_id.into()],
        )
        .unwrap();
    }

    /// Run `q` in a subtransaction: `("ok", "", "")`, or the SQLSTATE,
    /// message and hint of what it raised. The test's own transaction
    /// stays usable either way.
    pub(crate) fn attempt(q: &str) -> (String, String, String) {
        Spi::run(
            "CREATE OR REPLACE FUNCTION pg_temp.wg_attempt(q text) RETURNS text[] \
             LANGUAGE plpgsql AS $$ DECLARE st text; m text; h text; BEGIN \
               BEGIN \
                 EXECUTE q; \
                 RETURN ARRAY['ok', '', '']; \
               EXCEPTION WHEN OTHERS THEN \
                 GET STACKED DIAGNOSTICS st = RETURNED_SQLSTATE, m = MESSAGE_TEXT, \
                                         h = PG_EXCEPTION_HINT; \
                 RETURN ARRAY[st, m, coalesce(h, '')]; \
               END; \
             END $$",
        )
        .unwrap();
        let r: Vec<Option<String>> =
            Spi::get_one_with_args("SELECT pg_temp.wg_attempt($1)", &[q.into()])
                .unwrap()
                .unwrap();
        let mut it = r.into_iter().map(|x| x.unwrap_or_default());
        (it.next().unwrap(), it.next().unwrap(), it.next().unwrap())
    }

    /// The four whole-graph reads, as statements over one graph.
    fn four_reads(graph_id: i64) -> [(&'static str, String); 4] {
        [
            (
                "export_graph",
                format!("SELECT count(*) FROM pgrdf.export_graph({graph_id})"),
            ),
            (
                "graph_manifest",
                format!("SELECT pgrdf.graph_manifest({graph_id})"),
            ),
            (
                "graph_digest",
                format!("SELECT pgrdf.graph_digest({graph_id})"),
            ),
            (
                "structural_digest",
                format!("SELECT pgrdf.structural_digest({graph_id})"),
            ),
        ]
    }

    // ── set semantics (#158) ───────────────────────────────────────────

    /// A graph that physically holds a triple several times reads exactly
    /// like the same set stored once: both digests, the manifest's three
    /// digests and its asserted count, and every export line. Blank-node
    /// rows are repeated too: a repeat would otherwise change a blank
    /// node's first-degree hash and with it the labelling.
    #[pg_test]
    fn repeats_do_not_change_any_reading() {
        for (clean, dirty, triples) in [
            (986111, 986112, ground_triples()),
            (986113, 986114, bnode_triples()),
        ] {
            new_graph(clean);
            fill(clean, &triples);
            new_graph(dirty);
            fill(dirty, &triples);
            let surplus = repeat_rows(dirty);
            assert!(surplus > triples.len() as i64, "the helper stored repeats");

            assert_eq!(read(dirty), read(clean), "graph {dirty} against {clean}");

            let (mc, md) = (manifest(clean), manifest(dirty));
            assert!(
                mc["counts"].get("repeats").is_none(),
                "no repeats key on a graph without repeats: {}",
                mc["counts"]
            );
            assert_eq!(md["counts"]["repeats"].as_i64(), Some(surplus));
            assert_eq!(
                md["counts"]["asserted"].as_i64(),
                Some(triples.len() as i64)
            );
        }
    }

    /// The set is a set of TRIPLES, not of id rows: one term stored under
    /// two dictionary ids (here a simple literal with and without the
    /// explicit xsd:string datatype) is one triple.
    #[pg_test]
    fn one_term_under_two_ids_is_one_triple() {
        new_graph(986115);
        put(986115, I("urn:wg:s"), I("urn:wg:p"), L("x"), false);
        new_graph(986116);
        put(986116, I("urn:wg:s"), I("urn:wg:p"), L("x"), false);
        put(986116, I("urn:wg:s"), I("urn:wg:p"), Bare("x"), false);
        assert_ne!(term_id(L("x")), term_id(Bare("x")));
        assert_eq!(read(986116), read(986115));
        assert_eq!(
            export(986116),
            vec!["<urn:wg:s> <urn:wg:p> \"x\" .".to_string()]
        );
        assert_eq!(manifest(986116)["counts"]["repeats"].as_i64(), Some(1));
    }

    /// `graph_diff` agrees with `graph_digest`: a graph and the same set
    /// with repeats differ in nothing, blank-node components included.
    #[pg_test]
    fn graph_diff_sees_no_difference_in_repeats() {
        new_graph(986117);
        fill(986117, &bnode_triples());
        new_graph(986118);
        fill(986118, &bnode_triples());
        repeat_rows(986118);
        let s = Spi::get_one::<pgrx::JsonB>("SELECT pgrdf.graph_diff_summary(986117, 986118)")
            .unwrap()
            .unwrap()
            .0;
        for k in ["removed", "added"] {
            assert_eq!(s["ground"][k].as_i64(), Some(0), "ground {k}: {s}");
            assert_eq!(s["components"][k].as_i64(), Some(0), "components {k}: {s}");
        }
        let rows: i64 = Spi::get_one("SELECT count(*) FROM pgrdf.graph_diff(986117, 986118)")
            .unwrap()
            .unwrap();
        assert_eq!(rows, 0, "the diff of a set with itself is empty");
    }

    /// Each manifest pass is its own read. `consistent` says whether they
    /// are known to have read the same data: the graph is locked, or the
    /// transaction keeps one snapshot (REPEATABLE READ or stricter).
    #[pg_test]
    fn manifest_says_whether_its_passes_agree() {
        new_graph(986119);
        fill(986119, &small_triples());
        let iso: String = Spi::get_one("SHOW transaction_isolation").unwrap().unwrap();
        assert_eq!(iso, "read committed", "the test harness default");
        assert_eq!(manifest(986119)["consistent"].as_bool(), Some(false));
        Spi::run("SELECT pgrdf.lock_graph(986119, 'pinned')").unwrap();
        assert_eq!(manifest(986119)["consistent"].as_bool(), Some(true));
    }

    /// A locked graph's cached digest records the engine version that
    /// computed it; unlocking clears both. A cache written before the
    /// column existed (engine NULL) keeps being served untouched.
    #[pg_test]
    fn locked_digest_records_its_engine() {
        let engine = |g: i64| -> (Option<String>, Option<String>) {
            Spi::get_two_with_args(
                "SELECT locked_digest, locked_digest_engine FROM pgrdf._pgrdf_graphs \
                  WHERE graph_id = $1",
                &[g.into()],
            )
            .unwrap()
        };
        let digest = |g: i64| -> String {
            Spi::get_one_with_args("SELECT pgrdf.graph_digest($1)", &[g.into()])
                .unwrap()
                .unwrap()
        };
        let version: String = Spi::get_one("SELECT pgrdf.version()").unwrap().unwrap();

        new_graph(986120);
        fill(986120, &bnode_triples());
        Spi::run("SELECT pgrdf.lock_graph(986120, 'pinned')").unwrap();
        assert_eq!(engine(986120), (None, None), "locking computes nothing");
        let d = digest(986120);
        assert_eq!(engine(986120), (Some(d.clone()), Some(version)));
        Spi::run("SELECT pgrdf.unlock_graph(986120, 'reopen')").unwrap();
        assert_eq!(engine(986120), (None, None), "unlock clears both");

        // A digest cached by an earlier version: served as recorded.
        Spi::run("SELECT pgrdf.lock_graph(986120, 'pinned again')").unwrap();
        Spi::run(
            "UPDATE pgrdf._pgrdf_graphs SET locked_digest = 'recorded-before-0.6.41' \
              WHERE graph_id = 986120",
        )
        .unwrap();
        assert_eq!(digest(986120), "recorded-before-0.6.41");
        assert_eq!(
            engine(986120),
            (Some("recorded-before-0.6.41".to_string()), None),
            "an existing cache is left as it is"
        );
    }

    // ── bounded memory ─────────────────────────────────────────────────

    /// The digests hold the distinct blank-node-bearing triples and
    /// nothing else. Past `pgrdf.digest_max_bnode_quads` they refuse
    /// 54000; the export and a graph within the budget are unaffected,
    /// and repeats do not count against it.
    #[pg_test]
    fn blank_node_budget_refuses_54000() {
        new_graph(986121);
        fill(986121, &bnode_triples());
        repeat_rows(986121);
        let held = bnode_triples()
            .iter()
            .filter(|(s, _, o)| matches!(s, B(_)) || matches!(o, B(_)))
            .count();
        assert_eq!(held, 15);

        Spi::run("SET LOCAL pgrdf.digest_max_bnode_quads = 15").unwrap();
        for (name, q) in four_reads(986121) {
            assert_eq!(attempt(&q).0, "ok", "{name} within the budget");
        }
        Spi::run("SET LOCAL pgrdf.digest_max_bnode_quads = 14").unwrap();
        for (name, q) in four_reads(986121) {
            let (state, msg, hint) = attempt(&q);
            if name == "export_graph" {
                assert_eq!(state, "ok", "the export holds no triples at all");
                continue;
            }
            assert_eq!(state, "54000", "{name}: {msg}");
            assert!(
                msg.contains("986121") && msg.contains("14"),
                "{name}: {msg}"
            );
            assert!(
                msg.contains("pgrdf.digest_max_bnode_quads"),
                "{name} names the setting: {msg}"
            );
            assert!(
                hint.contains("pgrdf.digest_max_bnode_quads"),
                "{name}: {hint}"
            );
        }
    }

    /// A few hundred thousand ground triples plus blank-node ones: with a
    /// `work_mem` far below the graph's size (PostgreSQL's sort spills to
    /// disk) every read completes and equals the reading under default
    /// settings.
    #[pg_test]
    fn large_graph_reads_the_same_under_low_work_mem() {
        const N: i64 = 100_000;
        const BLANKS: i64 = 1_000;
        new_graph(986122);
        let xsd = term_id(I(XSD_STRING));
        // New terms go in with an id map, so the quads are built by
        // integer joins.
        Spi::run("CREATE TEMP TABLE wg_map (kind int, i int, id bigint) ON COMMIT DROP").unwrap();
        // (kind, term type, lexical form, datatype id, language, how many)
        let kinds = [
            (1, 1i16, "'urn:wgbig:s' || i", None, None, N),
            (2, 3, "'value ' || i || E' \\t\"q\"'", Some(xsd), None, N),
            (3, 3, "'wert ' || i", None, Some("de"), N),
            (4, 2, "'wgbig' || i", None, None, BLANKS),
        ];
        for (kind, ty, lexical, dt, lang, n) in kinds {
            Spi::run_with_args(
                &format!(
                    "WITH ins AS (
                         INSERT INTO pgrdf._pgrdf_dictionary
                                (term_type, lexical_value, datatype_iri_id, language_tag)
                         SELECT $1, {lexical}, $2, $3 FROM generate_series(1, $4) i
                         RETURNING id, lexical_value)
                     INSERT INTO wg_map
                     SELECT {kind}, substring(lexical_value from '[0-9]+')::int, id FROM ins"
                ),
                &[ty.into(), dt.into(), lang.into(), n.into()],
            )
            .unwrap();
        }
        Spi::run("ANALYZE wg_map").unwrap();
        let p: Vec<i64> = (1..=5)
            .map(|k| dict_id(1, &format!("urn:wgbig:p{k}"), None, None))
            .collect();
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred)
             SELECT s.id, x.p, x.o, 986122, false
               FROM wg_map s
               JOIN wg_map o1 ON o1.kind = 1 AND o1.i = $1 + 1 - s.i
               JOIN wg_map o2 ON o2.kind = 2 AND o2.i = s.i
               JOIN wg_map o3 ON o3.kind = 3 AND o3.i = s.i
              CROSS JOIN LATERAL (VALUES ($2::bigint, o1.id), ($3::bigint, o2.id),
                                         ($4::bigint, o3.id)) x(p, o)
              WHERE s.kind = 1",
            &[N.into(), p[0].into(), p[1].into(), p[2].into()],
        )
        .unwrap();
        Spi::run_with_args(
            "INSERT INTO pgrdf._pgrdf_quads (subject_id, predicate_id, object_id, graph_id, is_inferred)
             SELECT x.s, x.p, x.o, 986122, false
               FROM wg_map b
               JOIN wg_map s ON s.kind = 1 AND s.i = b.i
               JOIN wg_map v ON v.kind = 3 AND v.i = b.i
              CROSS JOIN LATERAL (VALUES (s.id, $1::bigint, b.id), (b.id, $2::bigint, v.id)) x(s, p, o)
              WHERE b.kind = 4",
            &[p[3].into(), p[4].into()],
        )
        .unwrap();
        let rows: i64 =
            Spi::get_one("SELECT count(*) FROM pgrdf._pgrdf_quads WHERE graph_id = 986122")
                .unwrap()
                .unwrap();
        assert_eq!(rows, 3 * N + 2 * BLANKS);

        let reference = read(986122);
        assert_eq!(reference.asserted, rows);
        assert_eq!(reference.export.len() as i64, rows);
        assert!(
            reference.export.windows(2).all(|w| w[0] < w[1]),
            "the export is strictly byte-sorted"
        );

        Spi::run("SET LOCAL work_mem = '64kB'").unwrap();
        // The query the reads run, under this work_mem: PostgreSQL sorts
        // on disk, which is what keeps the graph out of backend memory.
        let sql = super::stream_sql(false)
            .replace("$1", "986122")
            .replace("$2", &super::style::EXPORT.to_string());
        let mut plan = String::new();
        Spi::connect(|c| {
            for r in c
                .select(
                    &format!("EXPLAIN (ANALYZE, COSTS OFF, TIMING OFF, SUMMARY OFF) {sql}"),
                    None,
                    &[],
                )
                .unwrap()
            {
                plan.push_str(&r.get::<String>(1).unwrap().unwrap());
                plan.push('\n');
            }
        });
        assert!(plan.contains("Sort Method: external"), "{plan}");

        let squeezed = read(986122);
        assert!(
            squeezed == reference,
            "the same reading under work_mem = 64kB"
        );
    }

    // ── stored U+0000 ──────────────────────────────────────────────────

    /// A term containing U+0000, stored by an earlier version, never
    /// leaves pgRDF: each whole-graph read refuses XX001 (data corrupted)
    /// and the HINT names the dictionary id, the graph and the cure.
    /// The term is refused wherever it sits: in a ground triple, or in a
    /// triple that involves a blank node.
    #[pg_test]
    fn stored_nul_refuses_every_whole_graph_read() {
        let nul = store_nul_literal("before\0after");
        let (s, p, b) = (
            term_id(I("urn:wg:s")),
            term_id(I("urn:wg:p")),
            term_id(B("holder")),
        );
        new_graph(986131);
        fill(986131, &small_triples());
        put_ids(986131, s, p, nul);
        new_graph(986132);
        fill(986132, &small_triples());
        put_ids(986132, b, p, nul);

        for g in [986131i64, 986132] {
            for (name, q) in four_reads(g) {
                let (state, msg, hint) = attempt(&q);
                assert_eq!(state, "XX001", "{name} on {g}: {msg}");
                assert!(msg.contains("U+0000"), "{name}: {msg}");
                assert!(
                    hint.contains(&nul.to_string()) && hint.contains(&g.to_string()),
                    "{name}: the hint names the term id and the graph: {hint}"
                );
                assert!(
                    hint.contains("eload"),
                    "{name}: the hint gives the cure: {hint}"
                );
            }
        }
        // Not even the first line: the refusal comes before any row.
        let (state, _, _) = attempt("SELECT pgrdf.export_graph(986131) LIMIT 1");
        assert_eq!(state, "XX001", "no partial export");

        // A graph that does not use the term is unaffected.
        new_graph(986133);
        fill(986133, &small_triples());
        for (name, q) in four_reads(986133) {
            assert_eq!(attempt(&q).0, "ok", "{name} on a clean graph");
        }
    }

    /// `nul_term_report()` lists each stored term containing U+0000 with
    /// the graphs that use it, in any position.
    #[pg_test]
    fn nul_term_report_lists_terms_and_their_graphs() {
        let used = store_nul_literal("a\0b");
        let unused = store_nul_literal("\0");
        let (s, p) = (term_id(I("urn:wg:s")), term_id(I("urn:wg:p")));
        new_graph(986134);
        put_ids(986134, s, p, used);
        new_graph(986135);
        put_ids(986135, s, p, used);
        put_ids(986135, s, p, term_id(L("clean")));

        let graphs = |id: i64| -> Option<Vec<i64>> {
            Spi::get_one_with_args::<Vec<i64>>(
                "SELECT (SELECT graph_ids FROM pgrdf.nul_term_report() WHERE term_id = $1)",
                &[id.into()],
            )
            .unwrap()
        };
        assert_eq!(graphs(used), Some(vec![986134, 986135]));
        assert_eq!(graphs(unused), Some(vec![]), "stored, used by no graph");
        assert_eq!(
            graphs(term_id(L("clean"))),
            None,
            "a clean term is not listed"
        );

        let preview: String = Spi::get_one_with_args(
            "SELECT preview FROM pgrdf.nul_term_report() WHERE term_id = $1",
            &[used.into()],
        )
        .unwrap()
        .unwrap();
        assert_eq!(preview, "a\\u0000b", "the preview itself carries no U+0000");
    }

    /// The streamed digests hash the same bytes as the in-memory
    /// algorithms do for the graph's set of triples: RDFC-1.0 over the
    /// whole set (`canonicalize`), and the fleet's first-degree reference
    /// (`fd1_digest`). Checked on graphs with and without repeats.
    #[pg_test]
    fn streamed_digests_equal_the_in_memory_reference() {
        use crate::storage::canon::{Triple, canonicalize, read_asserted_triples};
        use std::collections::HashSet;

        let mut mixed = bnode_triples();
        mixed.extend(ground_triples());
        for (graph_id, triples, repeats) in [
            (986141, ground_triples(), false),
            (986142, bnode_triples(), false),
            (986143, mixed.clone(), false),
            (986144, mixed, true),
            (986145, Vec::new(), false),
        ] {
            new_graph(graph_id);
            fill(graph_id, &triples);
            if repeats {
                repeat_rows(graph_id);
            }
            let set: Vec<Triple> = read_asserted_triples(graph_id)
                .into_iter()
                .collect::<HashSet<Triple>>()
                .into_iter()
                .collect();
            assert_eq!(set.len(), triples.len(), "graph {graph_id}: the set");
            let r = read(graph_id);
            assert_eq!(
                r.graph_digest,
                canonicalize(set.clone()).1,
                "graph {graph_id}: rdfc-1.0"
            );
            assert_eq!(
                r.structural_digest,
                crate::storage::fd1::fd1_digest(&set),
                "graph {graph_id}: fd1"
            );
        }
    }
}
