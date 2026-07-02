//! pgrdf-oracle — W3C SPARQL differential oracle (pgRDF issue #17).
//!
//! Three subcommands:
//!   run --fixtures <dir> --engine-cmd <cmd> [--accept] [--filter <name>]
//!                                      the whole w3c-sparql harness:
//!                                      fixture walk, engine execution
//!                                      (via <cmd>: SQL on stdin → psql
//!                                      rows on stdout), golden gate,
//!                                      marker-driven differential
//!                                      oracle, verdict reporting.
//!                                      Exit 0 = all green, 1 = golden
//!                                      fail or eligible divergence,
//!                                      2 = harness error.
//!   eval <data-file> <query-file>      evaluate with spareval over an
//!                                      in-memory oxrdf dataset, emit
//!                                      canonical JSONL on stdout
//!   compare <engine.jsonl> <oracle.jsonl>
//!                                      verdict: exit 0 = match,
//!                                      1 = divergence, 2 = error
//!
//! The canonical row shapes mirror what the `run` harness collects
//! from the engine: SELECT/ASK rows are flat `{var: "lexical"}`
//! objects (ASK is `{"_ask": "true"|"false"}`); CONSTRUCT/DESCRIBE rows
//! are structured-term triples `{"subject": {...}, "predicate": {...},
//! "object": {...}}`. Comparison is bag-equivalent with blank-node
//! matching up to isomorphism — the two things per-row text diffing
//! cannot express (see the design on issue #17).

mod compare;
mod eval;
mod run;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["run", rest @ ..] => run_run(rest),
        ["eval", data_path, query_path] => run_eval(data_path, query_path),
        ["compare", engine_path, oracle_path] => run_compare(engine_path, oracle_path),
        _ => {
            eprintln!(
                "usage: pgrdf-oracle run --fixtures <dir> --engine-cmd <cmd> [--accept] [--filter <name>]\n\
                 \x20      pgrdf-oracle eval <data.ttl> <query.rq>\n\
                 \x20      pgrdf-oracle compare <engine.jsonl> <oracle.jsonl>"
            );
            ExitCode::from(2)
        }
    }
}

fn run_run(rest: &[&str]) -> ExitCode {
    let mut fixtures: Option<String> = None;
    let mut engine_cmd: Option<String> = None;
    let mut filter: Option<String> = None;
    let mut accept = false;
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "--fixtures" => fixtures = it.next().map(|s| s.to_string()),
            "--engine-cmd" => engine_cmd = it.next().map(|s| s.to_string()),
            "--filter" => filter = it.next().map(|s| s.to_string()),
            "--accept" => accept = true,
            other => return fail(&format!("run: unknown argument '{other}'")),
        }
    }
    let (Some(fixtures), Some(engine_cmd)) = (fixtures, engine_cmd) else {
        return fail("run: --fixtures and --engine-cmd are required");
    };
    let cfg = run::Config {
        fixtures_dir: fixtures.into(),
        engine_cmd,
        filter,
        accept,
    };
    match run::run(&cfg) {
        Ok(totals) if totals.ok() => ExitCode::SUCCESS,
        Ok(_) => ExitCode::from(1),
        Err(e) => fail(&e),
    }
}

fn run_eval(data_path: &str, query_path: &str) -> ExitCode {
    let (data, query) = match (
        std::fs::read_to_string(data_path),
        std::fs::read_to_string(query_path),
    ) {
        (Ok(d), Ok(q)) => (d, q),
        (Err(e), _) => return fail(&format!("read {data_path}: {e}")),
        (_, Err(e)) => return fail(&format!("read {query_path}: {e}")),
    };
    match eval::eval(&data, &query) {
        Ok(rows) => {
            for row in rows {
                println!("{row}");
            }
            ExitCode::SUCCESS
        }
        Err(e) => fail(&e),
    }
}

fn run_compare(engine_path: &str, oracle_path: &str) -> ExitCode {
    let (engine, oracle) = match (read_jsonl(engine_path), read_jsonl(oracle_path)) {
        (Ok(e), Ok(o)) => (e, o),
        (Err(e), _) | (_, Err(e)) => return fail(&e),
    };
    match compare::compare(&engine, &oracle) {
        compare::Verdict::Match => {
            println!("{}", serde_json::json!({"verdict": "match"}));
            ExitCode::SUCCESS
        }
        compare::Verdict::Diverge { detail } => {
            println!(
                "{}",
                serde_json::json!({"verdict": "diverge", "detail": detail})
            );
            ExitCode::from(1)
        }
    }
}

fn read_jsonl(path: &str) -> Result<Vec<serde_json::Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).map_err(|e| format!("{path}: bad JSON line: {e}")))
        .collect()
}

fn fail(message: &str) -> ExitCode {
    eprintln!("pgrdf-oracle: {message}");
    ExitCode::from(2)
}
