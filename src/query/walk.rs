//! Breadth-first property-path walk (SPEC 0.6.37 §3.2, issue #138).
//!
//! Before 0.6.37 a `+` / `*` path compiled to a recursive CTE with
//! `UNION ALL` + `CYCLE … USING path`, seeded with every edge of the
//! predicate. That enumerates every simple path up to
//! `pgrdf.path_max_depth` — exponential on any graph with fan-out and
//! recursion (a 168k-edge call graph passed 1 GB of temp at depth 4).
//! SPARQL path semantics are reachability, so this walk keeps one
//! visited set per start node instead:
//!
//! * **termination** comes from the visited set, so cycles cost one
//!   lap and never reach the depth cap;
//! * **depth is shortest distance**, and **truncation is exact**: it is
//!   recorded iff some node is reachable from a start only beyond
//!   `path_max_depth` (the old probe could over-count);
//! * every start advances together — one SPI query per level for all of
//!   them, so an unbound walk costs `levels` queries, not `starts ×
//!   levels`;
//! * `check_for_interrupts!()` runs every level, so `statement_timeout`
//!   and cancel always bite;
//! * the pairs held in backend memory are capped by
//!   `pgrdf.path_max_pairs` (54000 past it) — `temp_file_limit` does not
//!   cover backend memory.
//!
//! The walk returns `(start, reached, gid)` pairs for distance ≥ 1 (the
//! `+` relation); `*` adds the zero-length set in SQL around it. The
//! translator seeds it from whichever endpoint is already bound — a
//! constant, or a variable an earlier pattern bound (LATERAL) — so a
//! selective query walks from its few bound nodes, not the whole graph.

use pgrx::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;

thread_local! {
    /// Path ids whose walk was cut by `path_max_depth` during the current
    /// statement. Backend-local: the executor resets it before running a
    /// query and reads it after, then applies `pgrdf.on_path_truncation`
    /// once per truncated path pattern — the contract the old post-query
    /// probe implemented.
    static TRUNCATED: RefCell<HashSet<i32>> = RefCell::new(HashSet::new());
}

/// Forget every recorded truncation (start of a statement).
pub(crate) fn reset_truncations() {
    TRUNCATED.with(|t| t.borrow_mut().clear());
}

/// Was path pattern `path_id` truncated since the last reset?
pub(crate) fn was_truncated(path_id: i32) -> bool {
    TRUNCATED.with(|t| t.borrow().contains(&path_id))
}

fn note_truncated(path_id: i32) {
    TRUNCATED.with(|t| {
        t.borrow_mut().insert(path_id);
    });
}

/// Internal: the breadth-first walk behind SPARQL `+` / `*`.
///
/// * `preds` — the predicate dictionary ids (one for `p+`, several for
///   `(a|b)+`).
/// * `graph_id` — restrict every hop to this graph; NULL = all graphs.
/// * `seed` — walk from this node only; NULL = from every node that has
///   an outgoing edge in `preds` (the unbound case).
/// * `follow_inverse` — traverse edges object → subject.
/// * `max_depth` — `pgrdf.path_max_depth`, baked in at translate time.
/// * `path_id` — which path pattern of the statement this is, for
///   truncation accounting.
/// * `per_graph` — `GRAPH ?g` with no seed: walk each named graph
///   separately (named graphs only, W3C §13.3); `gid` is surfaced.
#[pg_extern(name = "_path_walk")]
fn path_walk(
    preds: Vec<i64>,
    graph_id: Option<i64>,
    seed: Option<i64>,
    follow_inverse: bool,
    max_depth: i32,
    path_id: i32,
    per_graph: bool,
) -> TableIterator<
    'static,
    (
        name!(start, i64),
        name!(reached, i64),
        name!(gid, Option<i64>),
    ),
> {
    let (from_col, to_col) = if follow_inverse {
        ("object_id", "subject_id")
    } else {
        ("subject_id", "object_id")
    };
    let budget = crate::query::guc::path_max_pairs().max(1) as usize;
    let mut out: Vec<(i64, i64, Option<i64>)> = Vec::new();

    Spi::connect(|client| {
        // ── starts: (node, graph) per walk ────────────────────────────
        let starts: Vec<(i64, Option<i64>)> = match seed {
            Some(s) => vec![(s, graph_id)],
            None if per_graph => client
                .select(
                    &format!(
                        "SELECT DISTINCT {from_col}, graph_id FROM pgrdf._pgrdf_quads \
                         WHERE predicate_id = ANY($1) AND graph_id <> 0"
                    ),
                    None,
                    &[preds.clone().into()],
                )
                .expect("path walk: start enumeration failed")
                .map(|r| (r.get::<i64>(1).unwrap().unwrap(), r.get::<i64>(2).unwrap()))
                .collect(),
            None => {
                let sql = match graph_id {
                    Some(_) => format!(
                        "SELECT DISTINCT {from_col} FROM pgrdf._pgrdf_quads \
                         WHERE predicate_id = ANY($1) AND graph_id = $2"
                    ),
                    None => format!(
                        "SELECT DISTINCT {from_col} FROM pgrdf._pgrdf_quads \
                         WHERE predicate_id = ANY($1)"
                    ),
                };
                let rows = match graph_id {
                    Some(g) => client.select(&sql, None, &[preds.clone().into(), g.into()]),
                    None => client.select(&sql, None, &[preds.clone().into()]),
                }
                .expect("path walk: start enumeration failed");
                rows.map(|r| (r.get::<i64>(1).unwrap().unwrap(), graph_id))
                    .collect()
            }
        };

        // A walk's graph is fixed for its whole life, so one expansion
        // query serves all starts: per-walk graphs travel in the unnest
        // when they differ (`GRAPH ?g`), a single literal graph is a
        // plain parameter (plan-time partition pruning).
        let per_walk_graph = per_graph;
        let expand_sql = if per_walk_graph {
            format!(
                "SELECT f.w, q.{to_col} FROM unnest($1::int4[], $2::int8[], $3::int8[]) \
                   AS f(w, node, g) \
                 JOIN pgrdf._pgrdf_quads q ON q.{from_col} = f.node AND q.graph_id = f.g \
                 WHERE q.predicate_id = ANY($4)"
            )
        } else if graph_id.is_some() {
            format!(
                "SELECT f.w, q.{to_col} FROM unnest($1::int4[], $2::int8[]) AS f(w, node) \
                 JOIN pgrdf._pgrdf_quads q ON q.{from_col} = f.node \
                 WHERE q.predicate_id = ANY($3) AND q.graph_id = $4"
            )
        } else {
            format!(
                "SELECT f.w, q.{to_col} FROM unnest($1::int4[], $2::int8[]) AS f(w, node) \
                 JOIN pgrdf._pgrdf_quads q ON q.{from_col} = f.node \
                 WHERE q.predicate_id = ANY($3)"
            )
        };
        let expand = |frontier: &[(i32, i64)]| -> Vec<(i32, i64)> {
            let ws: Vec<i32> = frontier.iter().map(|f| f.0).collect();
            let nodes: Vec<i64> = frontier.iter().map(|f| f.1).collect();
            let rows = if per_walk_graph {
                let gs: Vec<i64> = frontier
                    .iter()
                    .map(|f| starts[f.0 as usize].1.unwrap_or(0))
                    .collect();
                client.select(
                    &expand_sql,
                    None,
                    &[ws.into(), nodes.into(), gs.into(), preds.clone().into()],
                )
            } else if let Some(g) = graph_id {
                client.select(
                    &expand_sql,
                    None,
                    &[ws.into(), nodes.into(), preds.clone().into(), g.into()],
                )
            } else {
                client.select(
                    &expand_sql,
                    None,
                    &[ws.into(), nodes.into(), preds.clone().into()],
                )
            }
            .expect("path walk: expansion failed");
            rows.map(|r| {
                (
                    r.get::<i32>(1).unwrap().unwrap(),
                    r.get::<i64>(2).unwrap().unwrap(),
                )
            })
            .collect()
        };

        // ── level-synchronous BFS over every walk at once ─────────────
        // The start node is NOT pre-visited: `+` is distance ≥ 1, so a
        // cycle back to the start is a genuine solution (x p+ x).
        let mut visited: Vec<HashSet<i64>> = vec![HashSet::new(); starts.len()];
        let mut frontier: Vec<(i32, i64)> = starts
            .iter()
            .enumerate()
            .map(|(w, (node, _))| (w as i32, *node))
            .collect();
        let mut depth = 0;
        while !frontier.is_empty() && depth < max_depth {
            pgrx::check_for_interrupts!();
            let mut next: Vec<(i32, i64)> = Vec::new();
            for (w, n) in expand(&frontier) {
                if visited[w as usize].insert(n) {
                    out.push((starts[w as usize].0, n, starts[w as usize].1));
                    next.push((w, n));
                }
            }
            if out.len() > budget {
                crate::refuse_with_hint(
                    pgrx::pg_sys::errcodes::PgSqlErrorCode::ERRCODE_PROGRAM_LIMIT_EXCEEDED,
                    format!(
                        "sparql: property path walk exceeded pgrdf.path_max_pairs={budget} \
                         reachable pairs held in memory"
                    ),
                    "bind an endpoint of the path (the walk then starts there), or raise \
                     pgrdf.path_max_pairs deliberately"
                        .to_string(),
                );
            }
            frontier = next;
            depth += 1;
        }
        // Exact truncation: the cap stopped the walk with nodes whose
        // continuation reaches something not yet reached.
        if !frontier.is_empty() && depth >= max_depth {
            pgrx::check_for_interrupts!();
            if expand(&frontier)
                .into_iter()
                .any(|(w, n)| !visited[w as usize].contains(&n))
            {
                note_truncated(path_id);
            }
        }
    });

    TableIterator::new(out)
}
