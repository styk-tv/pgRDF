//! Blank-node-aware graph difference (SPEC 0.6.37 §3.5, issue #141).
//!
//! Comparing stored ids is exact only when neither graph has blank
//! nodes: every load mints fresh blank-node ids, so an id-level diff
//! reports each blank-node triple as both removed and added. Whole-graph
//! canonical labels do not fix it either — RDFC-1.0 numbers blank nodes
//! across the whole graph, so one change elsewhere renumbers them.
//!
//! So the diff has two parts:
//!
//! * **ground triples** (no blank node in subject or object) are compared
//!   by exact set difference over dictionary ids, in SQL — `EXCEPT` /
//!   `INTERSECT`, which spill to temp and so stay under `temp_file_limit`;
//! * **blank-node components** — triples connected through shared blank
//!   nodes (one RDF list, one SHACL property shape) — are each
//!   canonicalised on their own with RDFC-1.0 and compared as a multiset
//!   of component digests.
//!
//! The diff is empty iff the graphs are isomorphic. A change inside a
//! component reports the whole component as removed and added — the most
//! precise answer that holds in general, since matching blank nodes
//! across non-isomorphic graphs has no unique solution.

use crate::storage::canon::{
    CTerm, Triple, canonicalize_labeled, nt_term, read_asserted_triples_where,
};
use crate::storage::dict::term_type;
use pgrx::prelude::*;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

const METHOD: &str = "ground set difference + blank-node components by RDFC-1.0 component digest; \
                      a change inside a component reports the whole component";
const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";

fn require_graph(g: i64) {
    if g == 0 {
        return;
    }
    let known = Spi::get_one_with_args::<bool>(
        "SELECT EXISTS(SELECT 1 FROM pgrdf._pgrdf_graphs WHERE graph_id = $1)",
        &[g.into()],
    )
    .expect("graph_diff: graph lookup failed")
    .unwrap_or(false);
    if !known {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_UNDEFINED_OBJECT,
            format!("graph_diff: graph {g} does not exist"),
        );
    }
}

fn iri_id(iri: &str) -> Option<i64> {
    Spi::get_one_with_args::<i64>(
        // Scalar subquery: SPI sees exactly one row (NULL when absent).
        "SELECT (SELECT id FROM pgrdf._pgrdf_dictionary \
                  WHERE term_type = 1 AND lexical_value = $1 LIMIT 1)",
        &[iri.into()],
    )
    .expect("graph_diff: dictionary lookup failed")
}

fn budget() -> usize {
    crate::query::guc::diff_max_rows().max(1) as usize
}

fn over_budget(what: &str, n: usize) -> ! {
    crate::refuse_with_hint(
        pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_PROGRAM_LIMIT_EXCEEDED,
        format!(
            "graph_diff: {what} ({n}) exceeds pgrdf.diff_max_rows={}",
            budget()
        ),
        "narrow the diff (side, predicate, class) or raise pgrdf.diff_max_rows deliberately"
            .to_string(),
    )
}

/// Row narrowing for `graph_diff`. Pushed into the ground comparison;
/// applied to components whole (a component matches if ANY triple does).
struct Narrow {
    predicate: Option<String>,
    class: Option<String>,
}

impl Narrow {
    /// Extra SQL over the quad alias `q` for the ground sets. Unknown IRIs
    /// match nothing.
    fn ground_sql(&self) -> String {
        let mut c = String::new();
        if let Some(p) = &self.predicate {
            match iri_id(p) {
                Some(id) => c.push_str(&format!(" AND q.predicate_id = {id}")),
                None => c.push_str(" AND false"),
            }
        }
        if let Some(cls) = &self.class {
            match (iri_id(RDF_TYPE), iri_id(cls)) {
                (Some(t), Some(o)) => {
                    c.push_str(&format!(" AND q.predicate_id = {t} AND q.object_id = {o}"))
                }
                _ => c.push_str(" AND false"),
            }
        }
        c
    }

    fn matches(&self, t: &Triple) -> bool {
        let pred_ok = match &self.predicate {
            Some(p) => matches!(&t.1, CTerm::Iri(i) if i == p),
            None => true,
        };
        let class_ok = match &self.class {
            Some(c) => {
                matches!(&t.1, CTerm::Iri(i) if i == RDF_TYPE)
                    && matches!(&t.2, CTerm::Iri(o) if o == c)
            }
            None => true,
        };
        pred_ok && class_ok
    }
}

/// The ground set of graph `$n`: asserted triples with no blank node.
fn ground_set(param: &str, extra: &str) -> String {
    format!(
        "SELECT q.subject_id, q.predicate_id, q.object_id \
           FROM pgrdf._pgrdf_quads q \
           JOIN pgrdf._pgrdf_dictionary ds ON ds.id = q.subject_id \
           JOIN pgrdf._pgrdf_dictionary dob ON dob.id = q.object_id \
          WHERE q.graph_id = {param} AND NOT q.is_inferred \
            AND ds.term_type <> 2 AND dob.term_type <> 2{extra}"
    )
}

/// One blank-node component of a graph, canonicalised on its own.
struct Component {
    digest: String,
    triples: Vec<Triple>,
    labels: HashMap<String, String>,
}

impl Component {
    fn label(&self, b: &str) -> String {
        format!(
            "{}_{}",
            &self.digest[..12],
            self.labels
                .get(b)
                .map(String::as_str)
                .unwrap_or("unlabelled")
        )
    }
}

/// Split a graph's blank-node-bearing triples into components (union-find
/// over shared blank nodes) and canonicalise each.
fn components(graph_id: i64) -> Vec<Component> {
    let triples = read_asserted_triples_where(graph_id, "AND (s.term_type = 2 OR o.term_type = 2)");
    if triples.len() > budget() {
        over_budget("blank-node-bearing triples", triples.len());
    }
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut parent: Vec<usize> = Vec::new();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    let mut id_of = |b: &str, parent: &mut Vec<usize>| -> usize {
        *index.entry(b.to_string()).or_insert_with(|| {
            parent.push(parent.len());
            parent.len() - 1
        })
    };
    let mut anchor: Vec<usize> = Vec::with_capacity(triples.len());
    for t in &triples {
        let s = match &t.0 {
            CTerm::BNode(b) => Some(id_of(b, &mut parent)),
            _ => None,
        };
        let o = match &t.2 {
            CTerm::BNode(b) => Some(id_of(b, &mut parent)),
            _ => None,
        };
        match (s, o) {
            (Some(a), Some(b)) => {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[ra] = rb;
                }
                anchor.push(a);
            }
            (Some(a), None) | (None, Some(a)) => anchor.push(a),
            (None, None) => unreachable!("the read selects blank-node-bearing triples only"),
        }
    }
    let mut groups: BTreeMap<usize, Vec<Triple>> = BTreeMap::new();
    for (t, a) in triples.into_iter().zip(anchor) {
        let root = find(&mut parent, a);
        groups.entry(root).or_default().push(t);
    }
    let mut out = Vec::with_capacity(groups.len());
    for (_, group) in groups {
        pgrx::check_for_interrupts!();
        let (_lines, digest, labels) = canonicalize_labeled(group.clone());
        out.push(Component {
            digest,
            triples: group,
            labels,
        });
    }
    out
}

/// Multiset comparison by digest: (removed from A, added in B, common).
fn compare_components(
    a: Vec<Component>,
    b: Vec<Component>,
) -> (Vec<Component>, Vec<Component>, i64) {
    let mut by_a: BTreeMap<String, Vec<Component>> = BTreeMap::new();
    for c in a {
        by_a.entry(c.digest.clone()).or_default().push(c);
    }
    let mut by_b: BTreeMap<String, Vec<Component>> = BTreeMap::new();
    for c in b {
        by_b.entry(c.digest.clone()).or_default().push(c);
    }
    let mut removed = Vec::new();
    let mut added = Vec::new();
    let mut common = 0i64;
    for (d, mut ca) in by_a {
        let cb = by_b.remove(&d).unwrap_or_default();
        let shared = ca.len().min(cb.len());
        common += shared as i64;
        removed.extend(ca.drain(shared..));
        added.extend(cb.into_iter().skip(shared));
    }
    for (_, cb) in by_b {
        added.extend(cb);
    }
    (removed, added, common)
}

fn term_key(t: &CTerm, c: &Component) -> String {
    match t {
        CTerm::Iri(i) => i.clone(),
        other => nt_term(other, &|b| c.label(b)),
    }
}

/// `pgrdf.graph_diff_summary(a, b)` — counts of what changed from graph
/// `a` to graph `b`. Counting units: `ground.*` and `components.triples_*`
/// are triples; `components.removed/added/common` count components as a
/// multiset of digests; `by_predicate` counts ground AND component triples
/// per predicate; `by_type` counts rdf:type triples per class.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
fn graph_diff_summary(a: i64, b: i64) -> pgrx::JsonB {
    require_graph(a);
    require_graph(b);
    let type_id = iri_id(RDF_TYPE).unwrap_or(-1);
    let sql = format!(
        "WITH a AS MATERIALIZED ({ga}), b AS MATERIALIZED ({gb}), \
              rem AS MATERIALIZED (SELECT * FROM a EXCEPT SELECT * FROM b), \
              ad AS MATERIALIZED (SELECT * FROM b EXCEPT SELECT * FROM a), \
              u AS (SELECT subject_id, predicate_id, object_id, 1 AS r, 0 AS d FROM rem \
                    UNION ALL SELECT subject_id, predicate_id, object_id, 0, 1 FROM ad) \
         SELECT (SELECT count(*) FROM rem), (SELECT count(*) FROM ad), \
                (SELECT count(*) FROM (SELECT * FROM a INTERSECT SELECT * FROM b) i), \
                (SELECT coalesce(jsonb_agg(jsonb_build_object( \
                    'p', d.lexical_value, 'removed', x.r, 'added', x.d)), '[]'::jsonb) \
                   FROM (SELECT predicate_id, sum(r) r, sum(d) d FROM u GROUP BY 1) x \
                   JOIN pgrdf._pgrdf_dictionary d ON d.id = x.predicate_id), \
                (SELECT coalesce(jsonb_agg(jsonb_build_object( \
                    'class', d.lexical_value, 'removed', x.r, 'added', x.d)), '[]'::jsonb) \
                   FROM (SELECT object_id, sum(r) r, sum(d) d FROM u \
                          WHERE predicate_id = {type_id} GROUP BY 1) x \
                   JOIN pgrdf._pgrdf_dictionary d ON d.id = x.object_id)",
        ga = ground_set("$1", ""),
        gb = ground_set("$2", ""),
    );
    let (g_rem, g_add, g_common, by_p_json, by_t_json) = Spi::connect(|c| {
        let row = c
            .select(&sql, Some(1), &[a.into(), b.into()])
            .expect("graph_diff_summary: ground comparison failed")
            .first();
        (
            row.get::<i64>(1).unwrap().unwrap_or(0),
            row.get::<i64>(2).unwrap().unwrap_or(0),
            row.get::<i64>(3).unwrap().unwrap_or(0),
            row.get::<pgrx::JsonB>(4)
                .unwrap()
                .map(|j| j.0)
                .unwrap_or(json!([])),
            row.get::<pgrx::JsonB>(5)
                .unwrap()
                .map(|j| j.0)
                .unwrap_or(json!([])),
        )
    });

    let mut by_p: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    let mut by_t: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for (json_list, key, map) in [
        (&by_p_json, "p", &mut by_p),
        (&by_t_json, "class", &mut by_t),
    ] {
        for e in json_list.as_array().into_iter().flatten() {
            let k = e[key].as_str().unwrap_or_default().to_string();
            let entry = map.entry(k).or_insert((0, 0));
            entry.0 += e["removed"].as_i64().unwrap_or(0);
            entry.1 += e["added"].as_i64().unwrap_or(0);
        }
    }

    let (removed, added, common) = compare_components(components(a), components(b));
    let mut triples_removed = 0i64;
    let mut triples_added = 0i64;
    for (comps, side) in [(&removed, 0usize), (&added, 1usize)] {
        for c in comps {
            for t in &c.triples {
                if side == 0 {
                    triples_removed += 1;
                } else {
                    triples_added += 1;
                }
                let pe = by_p.entry(term_key(&t.1, c)).or_insert((0, 0));
                if side == 0 {
                    pe.0 += 1
                } else {
                    pe.1 += 1
                }
                if matches!(&t.1, CTerm::Iri(i) if i == RDF_TYPE) {
                    let te = by_t.entry(term_key(&t.2, c)).or_insert((0, 0));
                    if side == 0 { te.0 += 1 } else { te.1 += 1 }
                }
            }
        }
    }
    let list = |m: &BTreeMap<String, (i64, i64)>, key: &str| -> Value {
        Value::Array(
            m.iter()
                .map(|(k, (r, d))| json!({ key: k, "removed": r, "added": d }))
                .collect(),
        )
    };
    pgrx::JsonB(json!({
        "method": METHOD,
        "a": a,
        "b": b,
        "ground": { "removed": g_rem, "added": g_add, "common": g_common },
        "components": {
            "removed": removed.len(),
            "added": added.len(),
            "common": common,
            "triples_removed": triples_removed,
            "triples_added": triples_added,
        },
        "by_predicate": list(&by_p, "p"),
        "by_type": list(&by_t, "class"),
    }))
}

fn ground_term(kind: i16, val: String, dt: Option<String>, lang: Option<String>) -> CTerm {
    match kind {
        term_type::URI => CTerm::Iri(val),
        term_type::BLANK_NODE => CTerm::BNode(val),
        _ => CTerm::Lit { val, dt, lang },
    }
}

/// `pgrdf.graph_diff(a, b [, side, predicate, class])` — the triples that
/// differ from graph `a` to graph `b`: `side` '-' (only in a) or '+' (only
/// in b); terms in N-Triples form; `component` NULL for a ground triple,
/// else the digest of the blank-node component it belongs to (component
/// blank nodes print as `_:<digest12>_c14nN`). Ordered by side ('-' first)
/// then canonical N-Triples line, so OFFSET/LIMIT paging is deterministic.
/// `predicate` / `class` narrow the ground comparison itself; a component
/// is returned whole if any of its triples matches.
#[search_path(pgrdf, pg_temp)]
#[pg_extern]
#[allow(clippy::type_complexity)]
fn graph_diff(
    a: i64,
    b: i64,
    side: default!(Option<&str>, "NULL"),
    predicate: default!(Option<&str>, "NULL"),
    class: default!(Option<&str>, "NULL"),
) -> TableIterator<
    'static,
    (
        name!(side, String),
        name!(subject, String),
        name!(predicate, String),
        name!(object, String),
        name!(component, Option<String>),
    ),
> {
    if !matches!(side, None | Some("-") | Some("+")) {
        crate::refuse(
            pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_INVALID_PARAMETER_VALUE,
            "graph_diff: side must be '-', '+' or NULL".to_string(),
        );
    }
    require_graph(a);
    require_graph(b);
    let narrow = Narrow {
        predicate: predicate.map(str::to_string),
        class: class.map(str::to_string),
    };
    let extra = narrow.ground_sql();
    // (side rank, line, side, s, p, o, component)
    let mut out: Vec<(u8, String, String, String, String, String, Option<String>)> = Vec::new();

    let sql = format!(
        "WITH a AS MATERIALIZED ({ga}), b AS MATERIALIZED ({gb}), \
              d AS (SELECT '-'::text AS side, * FROM (SELECT * FROM a EXCEPT SELECT * FROM b) x \
                    UNION ALL \
                    SELECT '+'::text, * FROM (SELECT * FROM b EXCEPT SELECT * FROM a) y) \
         SELECT d.side, s.term_type, s.lexical_value, p.lexical_value, \
                o.term_type, o.lexical_value, dt.lexical_value, o.language_tag \
           FROM d \
           JOIN pgrdf._pgrdf_dictionary s ON s.id = d.subject_id \
           JOIN pgrdf._pgrdf_dictionary p ON p.id = d.predicate_id \
           JOIN pgrdf._pgrdf_dictionary o ON o.id = d.object_id \
           LEFT JOIN pgrdf._pgrdf_dictionary dt ON dt.id = o.datatype_iri_id",
        ga = ground_set("$1", &extra),
        gb = ground_set("$2", &extra),
    );
    Spi::connect(|c| {
        for r in c
            .select(&sql, None, &[a.into(), b.into()])
            .expect("graph_diff: ground comparison failed")
        {
            let sd: String = r.get(1).unwrap().unwrap();
            if side.is_some_and(|want| want != sd) {
                continue;
            }
            let s = ground_term(
                r.get(2).unwrap().unwrap(),
                r.get(3).unwrap().unwrap(),
                None,
                None,
            );
            let p = CTerm::Iri(r.get(4).unwrap().unwrap());
            let o = ground_term(
                r.get(5).unwrap().unwrap(),
                r.get(6).unwrap().unwrap(),
                r.get(7).unwrap(),
                r.get(8).unwrap(),
            );
            let label = |x: &str| x.to_string();
            let (st, pt, ot) = (
                nt_term(&s, &label),
                nt_term(&p, &label),
                nt_term(&o, &label),
            );
            let line = format!("{st} {pt} {ot} .");
            out.push((u8::from(sd == "+"), line, sd, st, pt, ot, None));
            if out.len() > budget() {
                over_budget("diff rows", out.len());
            }
        }
    });

    let (removed, added, _common) = compare_components(components(a), components(b));
    for (comps, sd) in [(removed, "-"), (added, "+")] {
        if side.is_some_and(|want| want != sd) {
            continue;
        }
        for comp in comps {
            pgrx::check_for_interrupts!();
            if !comp.triples.iter().any(|t| narrow.matches(t)) {
                continue;
            }
            for t in &comp.triples {
                let label = |x: &str| comp.label(x);
                let (st, pt, ot) = (
                    nt_term(&t.0, &label),
                    nt_term(&t.1, &label),
                    nt_term(&t.2, &label),
                );
                let line = format!("{st} {pt} {ot} .");
                out.push((
                    u8::from(sd == "+"),
                    line,
                    sd.to_string(),
                    st,
                    pt,
                    ot,
                    Some(comp.digest.clone()),
                ));
            }
            if out.len() > budget() {
                over_budget("diff rows", out.len());
            }
        }
    }
    out.sort_by(|x, y| (x.0, &x.1).cmp(&(y.0, &y.1)));
    TableIterator::new(
        out.into_iter()
            .map(|(_, _, sd, s, p, o, comp)| (sd, s, p, o, comp)),
    )
}

#[cfg(any(test, feature = "pg_test"))]
#[pgrx::pg_schema]
mod tests {
    use pgrx::prelude::*;

    fn load(iri: &str, ttl: &str) -> i64 {
        let g: i64 = Spi::get_one_with_args("SELECT pgrdf.add_graph($1)", &[iri.into()])
            .unwrap()
            .unwrap();
        Spi::get_one_with_args::<i64>("SELECT pgrdf.parse_turtle($1, $2)", &[ttl.into(), g.into()])
            .unwrap();
        g
    }

    fn summary(a: i64, b: i64) -> serde_json::Value {
        Spi::get_one::<pgrx::JsonB>(&format!("SELECT pgrdf.graph_diff_summary({a}, {b})"))
            .unwrap()
            .unwrap()
            .0
    }

    fn n(v: &serde_json::Value, path: &[&str]) -> i64 {
        let mut cur = v;
        for k in path {
            cur = &cur[*k];
        }
        cur.as_i64().unwrap_or_else(|| panic!("{path:?} in {v}"))
    }

    fn rows(sql_args: &str) -> Vec<(String, String, String, String, Option<String>)> {
        let mut out = Vec::new();
        Spi::connect(|c| {
            for r in c
                .select(
                    &format!(
                        "SELECT side, subject, predicate, object, component \
                         FROM pgrdf.graph_diff({sql_args})"
                    ),
                    None,
                    &[],
                )
                .unwrap()
            {
                out.push((
                    r.get::<String>(1).unwrap().unwrap(),
                    r.get::<String>(2).unwrap().unwrap(),
                    r.get::<String>(3).unwrap().unwrap(),
                    r.get::<String>(4).unwrap().unwrap(),
                    r.get::<String>(5).unwrap(),
                ));
            }
        });
        out
    }

    fn digest(g: i64) -> String {
        Spi::get_one(&format!("SELECT pgrdf.graph_digest({g})"))
            .unwrap()
            .unwrap()
    }

    /// Without blank nodes the diff is the exact set difference, removed
    /// rows first, in canonical N-Triples order.
    #[pg_test]
    fn ground_diff_is_set_difference() {
        let a = load(
            "urn:tdd:diff:a1",
            "<urn:d:s1> <urn:d:p> <urn:d:o> . <urn:d:s2> <urn:d:p> <urn:d:o> . <urn:d:s3> <urn:d:p> \"x\" .",
        );
        let b = load(
            "urn:tdd:diff:b1",
            "<urn:d:s2> <urn:d:p> <urn:d:o> . <urn:d:s3> <urn:d:p> \"x\" . <urn:d:s4> <urn:d:p> <urn:d:o> .",
        );
        let s = summary(a, b);
        assert_eq!(n(&s, &["ground", "removed"]), 1);
        assert_eq!(n(&s, &["ground", "added"]), 1);
        assert_eq!(n(&s, &["ground", "common"]), 2);
        assert_eq!(n(&s, &["components", "removed"]), 0);
        assert!(s["method"].as_str().unwrap().contains("RDFC-1.0"));
        let r = rows(&format!("{a}, {b}"));
        assert_eq!(
            r,
            vec![
                (
                    "-".into(),
                    "<urn:d:s1>".into(),
                    "<urn:d:p>".into(),
                    "<urn:d:o>".into(),
                    None
                ),
                (
                    "+".into(),
                    "<urn:d:s4>".into(),
                    "<urn:d:p>".into(),
                    "<urn:d:o>".into(),
                    None
                ),
            ]
        );
    }

    const SHAPE: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> . @prefix ex: <urn:ex:> . \
        ex:S a sh:NodeShape ; sh:targetClass ex:C ; \
          sh:property [ sh:path ex:p ; sh:minCount 1 ] , [ sh:path ex:q ; sh:in ( ex:a ex:b ) ] .";

    /// The same blank-node content loaded twice diffs EMPTY, and its
    /// graph_digest agrees — stored blank-node ids differ, meaning does not.
    #[pg_test]
    fn same_blank_node_content_loaded_twice_diffs_empty() {
        let a = load("urn:tdd:diff:a2", SHAPE);
        let b = load("urn:tdd:diff:b2", SHAPE);
        let s = summary(a, b);
        for k in ["removed", "added"] {
            assert_eq!(n(&s, &["ground", k]), 0, "{s}");
            assert_eq!(n(&s, &["components", k]), 0, "{s}");
        }
        assert!(n(&s, &["components", "common"]) >= 2);
        assert!(rows(&format!("{a}, {b}")).is_empty());
        assert_eq!(digest(a), digest(b), "empty diff ⇔ equal graph_digest");
    }

    /// One changed sh:minCount: no ground change, exactly one component
    /// removed and one added — the whole property shape, not one triple.
    #[pg_test]
    fn changed_shape_reports_the_whole_component() {
        let a = load("urn:tdd:diff:a3", SHAPE);
        let b = load(
            "urn:tdd:diff:b3",
            &SHAPE.replace("sh:minCount 1", "sh:minCount 2"),
        );
        let s = summary(a, b);
        assert_eq!(n(&s, &["ground", "removed"]), 0);
        assert_eq!(n(&s, &["ground", "added"]), 0);
        assert_eq!(n(&s, &["components", "removed"]), 1);
        assert_eq!(n(&s, &["components", "added"]), 1);
        // ex:S sh:property _:b + the shape's sh:path and sh:minCount = 3
        assert_eq!(n(&s, &["components", "triples_removed"]), 3);
        assert_eq!(n(&s, &["components", "triples_added"]), 3);
        let r = rows(&format!("{a}, {b}"));
        assert_eq!(r.len(), 6);
        assert!(
            r.iter().all(|x| x.4.is_some()),
            "component rows carry their digest"
        );
        assert!(
            r.iter()
                .any(|x| x.1.starts_with("_:") || x.3.starts_with("_:"))
        );
        assert_ne!(
            digest(a),
            digest(b),
            "non-empty diff ⇔ unequal graph_digest"
        );
    }

    /// Components are a multiset: two isomorphic copies in A against one
    /// in B leaves exactly one removed.
    #[pg_test]
    fn components_are_a_multiset() {
        let a = load(
            "urn:tdd:diff:a4",
            "<urn:m:s> <urn:m:p> [ <urn:m:q> \"v\" ] . <urn:m:s> <urn:m:p> [ <urn:m:q> \"v\" ] .",
        );
        let b = load(
            "urn:tdd:diff:b4",
            "<urn:m:s> <urn:m:p> [ <urn:m:q> \"v\" ] .",
        );
        let s = summary(a, b);
        assert_eq!(n(&s, &["components", "common"]), 1);
        assert_eq!(n(&s, &["components", "removed"]), 1);
        assert_eq!(n(&s, &["components", "added"]), 0);
    }

    /// The counting units hold arithmetically: per-predicate counts cover
    /// ground and component triples; per-type counts are rdf:type triples.
    #[pg_test]
    fn counting_invariants_hold() {
        let a = load("urn:tdd:diff:a5", SHAPE);
        let b = load(
            "urn:tdd:diff:b5",
            &format!(
                "{} <urn:ex:T> a <urn:ex:C> .",
                SHAPE.replace("sh:minCount 1", "sh:minCount 2")
            ),
        );
        let s = summary(a, b);
        for (side, comp) in [("removed", "triples_removed"), ("added", "triples_added")] {
            let by_p: i64 = s["by_predicate"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e[side].as_i64().unwrap())
                .sum();
            assert_eq!(
                by_p,
                n(&s, &["ground", side]) + n(&s, &["components", comp]),
                "{s}"
            );
            let by_t: i64 = s["by_type"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e[side].as_i64().unwrap())
                .sum();
            let type_p = s["by_predicate"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["p"] == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
                .map(|e| e[side].as_i64().unwrap())
                .unwrap_or(0);
            assert_eq!(by_t, type_p, "{s}");
        }
        assert_eq!(n(&s, &["ground", "added"]), 1, "ex:T a ex:C");
    }

    /// Narrowing: side, predicate and class filters; a component is
    /// returned whole if ANY of its triples matches.
    #[pg_test]
    fn narrowing_filters_rows() {
        let a = load("urn:tdd:diff:a6", SHAPE);
        let b = load(
            "urn:tdd:diff:b6",
            &format!(
                "{} <urn:ex:T> a <urn:ex:C> .",
                SHAPE.replace("sh:minCount 1", "sh:minCount 2")
            ),
        );
        let plus = rows(&format!("{a}, {b}, side => '+'"));
        assert!(plus.iter().all(|r| r.0 == "+"));
        assert_eq!(plus.len(), 4, "1 ground + 3 component triples");
        let min = rows(&format!(
            "{a}, {b}, predicate => 'http://www.w3.org/ns/shacl#minCount'"
        ));
        assert_eq!(min.len(), 6, "both changed components, whole: {min:?}");
        let cls = rows(&format!("{a}, {b}, class => 'urn:ex:C'"));
        assert_eq!(cls.len(), 1, "{cls:?}");
        assert_eq!(cls[0].1, "<urn:ex:T>");
    }

    /// OFFSET/LIMIT windows concatenate to the full ordered result.
    #[pg_test]
    fn paging_is_deterministic() {
        let a = load("urn:tdd:diff:a7", SHAPE);
        let b = load(
            "urn:tdd:diff:b7",
            &format!(
                "{} <urn:ex:T> a <urn:ex:C> .",
                SHAPE.replace("sh:minCount 1", "sh:minCount 2")
            ),
        );
        let full = rows(&format!("{a}, {b}"));
        let mut paged = Vec::new();
        for off in (0..full.len()).step_by(2) {
            let mut page = Vec::new();
            Spi::connect(|c| {
                for r in c
                    .select(
                        &format!(
                            "SELECT side, subject, predicate, object, component \
                             FROM pgrdf.graph_diff({a}, {b}) OFFSET {off} LIMIT 2"
                        ),
                        None,
                        &[],
                    )
                    .unwrap()
                {
                    page.push((
                        r.get::<String>(1).unwrap().unwrap(),
                        r.get::<String>(2).unwrap().unwrap(),
                        r.get::<String>(3).unwrap().unwrap(),
                        r.get::<String>(4).unwrap().unwrap(),
                        r.get::<String>(5).unwrap(),
                    ));
                }
            });
            paged.extend(page);
        }
        assert_eq!(paged, full);
    }

    /// An absent graph refuses 42704; a bad side refuses 22023.
    #[pg_test(error = "graph_diff: graph 987654 does not exist")]
    fn absent_graph_refuses() {
        let a = load("urn:tdd:diff:a8", "<urn:d:s> <urn:d:p> <urn:d:o> .");
        Spi::run(&format!("SELECT pgrdf.graph_diff_summary({a}, 987654)")).unwrap();
    }

    #[pg_test(error = "graph_diff: side must be '-', '+' or NULL")]
    fn bad_side_refuses() {
        let a = load("urn:tdd:diff:a9", "<urn:d:s> <urn:d:p> <urn:d:o> .");
        Spi::run(&format!(
            "SELECT * FROM pgrdf.graph_diff({a}, {a}, side => 'x')"
        ))
        .unwrap();
    }

    /// An adversarial blank-node component keeps RDFC's fail-closed
    /// budget: the diff refuses rather than degrading or skipping it.
    #[pg_test(
        error = "pgRDF#117: canonicalization budget exceeded (adversarial blank-node structure); refusing rather than degrading"
    )]
    fn adversarial_component_refuses() {
        let a = load(
            "urn:tdd:diff:poison",
            include_str!("../../tests/fixtures/rdfc10/test074-in.nq"),
        );
        let b = load("urn:tdd:diff:empty", "<urn:d:s> <urn:d:p> <urn:d:o> .");
        Spi::run(&format!("SELECT pgrdf.graph_diff_summary({a}, {b})")).unwrap();
    }

    /// pgrdf.diff_max_rows bounds what the diff holds in memory.
    #[pg_test(error = "graph_diff: diff rows (1001) exceeds pgrdf.diff_max_rows=1000")]
    fn diff_budget_refuses() {
        let mut ttl = String::new();
        for i in 0..1100 {
            ttl.push_str(&format!("<urn:big:s{i}> <urn:big:p> <urn:big:o> .\n"));
        }
        let a = load("urn:tdd:diff:big", &ttl);
        let b = load("urn:tdd:diff:small", "<urn:d:s> <urn:d:p> <urn:d:o> .");
        Spi::run("SET LOCAL pgrdf.diff_max_rows = 1000").unwrap();
        Spi::run(&format!("SELECT count(*) FROM pgrdf.graph_diff({a}, {b})")).unwrap();
    }
}
