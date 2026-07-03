//! `pgrdf-oracle run` — the w3c-sparql harness orchestrator (issue #17,
//! course-correction 2: orchestration moves out of bash into this
//! binary; `tests/w3c-sparql/run.sh` shrinks to one invocation).
//!
//! One `run` walks every fixture directory, executes the fixture's
//! query against the ENGINE (an external command — psql inside the
//! compose container — received via `--engine-cmd`, so this crate
//! stays buildable without any pgrx/Postgres linkage), compares the
//! normalized rows against the fixture's `expected.jsonl` golden, and
//! — for oracle-eligible fixtures — differentially against spareval
//! (in-process `eval::eval` + `compare::compare`; no subprocess
//! round-trip, which the bash spike needed).
//!
//! ## Per-fixture `oracle` marker (one line)
//!
//! * `eligible` — differential runs; a divergence FAILS the run (an
//!   executor divergence is the signal this mode exists to catch).
//! * `ineligible: <reason>` — golden-only (UPDATE forms, reasoning
//!   profiles, setup.sql multi-graph datasets, DESCRIBE, …).
//! * `known-divergence: <issue-ref> — <reason>` — differential runs;
//!   a divergence is EXPECTED and non-fatal (tracked by the issue).
//!   If such a fixture suddenly MATCHES the run says so — the marker
//!   is stale and should flip back to `eligible`.
//! * absent — counts as ineligible (golden-only).
//!
//! The golden gate is blocking either way; the oracle adds a second,
//! independent judge on top — including in ACCEPT mode, which
//! second-opinions a golden at the exact moment it is (re)generated,
//! the window where an engine bug could otherwise be codified as a
//! passing test.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;
use sha1::{Digest, Sha1};

use crate::{compare, eval};

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

pub struct Config {
    pub fixtures_dir: PathBuf,
    /// Shell command that receives the assembled SQL stream on stdin
    /// and emits psql `-X -A -t -q` rows on stdout (typically
    /// `docker exec -i <container> psql …`).
    pub engine_cmd: String,
    /// Exact fixture name to run (positional filter), or all.
    pub filter: Option<String>,
    /// Regenerate `expected.jsonl` instead of diffing against it.
    pub accept: bool,
}

#[derive(Default)]
pub struct Totals {
    pub pass: u32,
    pub fail: u32,
    pub baselined: u32,
    pub oracle_match: u32,
    pub oracle_known: u32,
    pub oracle_diverge: u32,
    pub oracle_skip: u32,
}

impl Totals {
    pub fn ok(&self) -> bool {
        self.fail == 0 && self.oracle_diverge == 0
    }
}

/// Parsed `oracle` marker.
enum Marker {
    Eligible,
    Ineligible,
    Known(String),
}

pub fn run(cfg: &Config) -> Result<Totals, String> {
    let mut names = discover(&cfg.fixtures_dir)?;
    if let Some(f) = &cfg.filter {
        names.retain(|n| n == f);
    }
    if names.is_empty() {
        return Err("no w3c-sparql tests matched".into());
    }

    let mut t = Totals::default();
    for name in &names {
        let test_dir = cfg.fixtures_dir.join(name);
        let gid = graph_id_for(name);
        let sql = assemble_sql(&test_dir, &gid).map_err(|e| format!("{name}: {e}"))?;
        let raw = engine(&cfg.engine_cmd, &sql)?;
        let actual = normalize(&raw);

        // Differential oracle pass. BEFORE the baseline branch on
        // purpose (see module docs — ACCEPT-window second opinion).
        oracle_pass(&test_dir, name, &actual, &mut t);

        // Golden gate.
        let expected_path = test_dir.join("expected.jsonl");
        if cfg.accept || !expected_path.exists() {
            let mut body = actual.join("\n");
            body.push('\n');
            std::fs::write(&expected_path, body)
                .map_err(|e| format!("{name}: write golden: {e}"))?;
            println!("  {YELLOW}BASELINE{RESET} {name}");
            t.baselined += 1;
        } else {
            // Bag-equivalent golden gate: BOTH sides sort before the
            // compare (bash-harness parity — goldens on disk are not
            // guaranteed byte-sorted; fixture 35's predates the
            // C-locale ACCEPT convention).
            let mut expected = read_lines(&expected_path)?;
            expected.sort();
            if expected == actual {
                println!("  {GREEN}PASS{RESET}     {name}");
                t.pass += 1;
            } else {
                println!("  {RED}FAIL{RESET}     {name}");
                print_diff(&expected, &actual);
                t.fail += 1;
            }
        }
    }

    println!(
        "\nw3c-sparql summary: {} pass, {} fail, {} new baselines",
        t.pass, t.fail, t.baselined
    );
    println!(
        "differential(oracle): {} match, {} known-divergence, {} diverge, {} skipped (ineligible)",
        t.oracle_match, t.oracle_known, t.oracle_diverge, t.oracle_skip
    );
    Ok(t)
}

/// Fixture discovery — a directory is a test if it provides EITHER a
/// `data.ttl` (the single-graph default) OR a `setup.sql` (the
/// slice-111 multi-graph extension). `fixtures/` is reserved for the
/// official W3C rdf-tests submodule (course-correction 1) and skipped.
fn discover(dir: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "fixtures" {
            continue;
        }
        let p = entry.path();
        if p.join("data.ttl").is_file() || p.join("setup.sql").is_file() {
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

/// Deterministic graph id from the test name — byte-for-byte the rule
/// the bash harness used (sha1 hex → ASCII digits → first 4 →
/// `"10" + suffix`), so goldens that happen to surface a graph id
/// stay valid. Guards the all-leading-zero hash (graph 0 clashes
/// with the default partition).
fn graph_id_for(name: &str) -> String {
    let hex: String = Sha1::digest(name.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let digits: String = hex.chars().filter(|c| c.is_ascii_digit()).take(4).collect();
    if digits.is_empty() || digits == "0000" {
        "101234".into()
    } else {
        format!("10{digits}")
    }
}

/// Assemble the per-fixture SQL stream — identical to the bash
/// harness: DROP/CREATE/shmem_reset/plan_cache_clear scaffolding,
/// optional `setup.sql`, optional `add_graph` + `parse_turtle` (only
/// when `data.ttl` exists AND is non-empty), then the query routed by
/// the optional `kind` file (`sparql` default | `construct` |
/// `describe`).
fn assemble_sql(test_dir: &Path, gid: &str) -> Result<String, String> {
    let query =
        std::fs::read_to_string(test_dir.join("query.rq")).map_err(|e| format!("query.rq: {e}"))?;
    let q_esc = query.replace('\'', "''");

    let mut sql = String::from(
        "DROP EXTENSION IF EXISTS pgrdf CASCADE;\n\
         CREATE EXTENSION pgrdf;\n\
         SELECT pgrdf.shmem_reset();\n\
         SELECT pgrdf.plan_cache_clear();\n",
    );
    let setup = test_dir.join("setup.sql");
    if setup.is_file() {
        let content = std::fs::read_to_string(&setup).map_err(|e| format!("setup.sql: {e}"))?;
        sql.push_str(&content);
        sql.push('\n');
    }
    let data = test_dir.join("data.ttl");
    if data.is_file() {
        let content = std::fs::read_to_string(&data).map_err(|e| format!("data.ttl: {e}"))?;
        if !content.is_empty() {
            let content_esc = content.replace('\'', "''");
            sql.push_str(&format!("SELECT pgrdf.add_graph({gid});\n"));
            sql.push_str(&format!(
                "SELECT pgrdf.parse_turtle('{content_esc}', {gid});\n"
            ));
        }
    }
    let kind = match std::fs::read_to_string(test_dir.join("kind")) {
        Ok(k) => k.split_whitespace().collect::<String>(),
        Err(_) => "sparql".into(),
    };
    match kind.as_str() {
        "sparql" | "" => sql.push_str(&format!(
            "SELECT sparql::text FROM pgrdf.sparql('{q_esc}');\n"
        )),
        "construct" => sql.push_str(&format!(
            "SELECT j::text FROM pgrdf.construct('{q_esc}') AS t(j);\n"
        )),
        "describe" => sql.push_str(&format!(
            "SELECT j::text FROM pgrdf.describe('{q_esc}') AS t(j);\n"
        )),
        other => {
            return Err(format!(
                "unknown kind '{other}' in {}/kind",
                test_dir.display()
            ))
        }
    }
    Ok(sql)
}

/// Pipe the SQL stream to the engine command's stdin, return stdout.
/// The engine's exit status is deliberately ignored (matching the
/// bash harness): a psql SQL error yields partial output that the
/// golden diff then reports — the failure surfaces where the
/// evidence is.
fn engine(engine_cmd: &str, sql: &str) -> Result<String, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(engine_cmd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn engine: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("engine stdin unavailable")?
        .write_all(sql.as_bytes())
        .map_err(|e| format!("write engine stdin: {e}"))?;
    let out = child
        .wait_with_output()
        .map_err(|e| format!("engine wait: {e}"))?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Normalize raw engine output into canonical golden lines: keep only
/// JSON-looking rows (scaffolding return values dropped), zero the
/// non-deterministic `elapsed_ms` measurement in UPDATE summary rows,
/// byte-sort (bag-equivalent comparison; `LC_ALL=C sort` parity).
fn normalize(raw: &str) -> Vec<String> {
    let mut lines: Vec<String> = raw
        .lines()
        .filter(|l| l.starts_with('{') || l.starts_with('['))
        .map(zero_elapsed_ms)
        .collect();
    lines.sort();
    lines
}

/// Textual `"elapsed_ms": <number>` → `"elapsed_ms": 0`. Textual on
/// purpose: goldens are byte-compared against psql's jsonb rendering,
/// so a parse → re-serialize round-trip would perturb formatting.
fn zero_elapsed_ms(line: &str) -> String {
    const KEY: &str = "\"elapsed_ms\": ";
    let Some(start) = line.find(KEY) else {
        return line.to_string();
    };
    let num_start = start + KEY.len();
    let rest = &line[num_start..];
    let num_len = rest
        .find(|c: char| !(c.is_ascii_digit() || "eE.+-".contains(c)))
        .unwrap_or(rest.len());
    if num_len == 0 {
        return line.to_string();
    }
    format!("{}0{}", &line[..num_start], &rest[num_len..])
}

fn read_lines(path: &Path) -> Result<Vec<String>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(text.lines().map(str::to_string).collect())
}

/// Minimal ordered diff for FAIL reporting (`<` expected-only, `>`
/// actual-only) — both sides are sorted, so a single merge walk finds
/// every difference.
fn print_diff(expected: &[String], actual: &[String]) {
    let (mut i, mut j) = (0, 0);
    while i < expected.len() || j < actual.len() {
        match (expected.get(i), actual.get(j)) {
            (Some(e), Some(a)) if e == a => {
                i += 1;
                j += 1;
            }
            (Some(e), Some(a)) if e < a => {
                println!("    < {e}");
                i += 1;
            }
            (Some(_), Some(a)) => {
                println!("    > {a}");
                j += 1;
            }
            (Some(e), None) => {
                println!("    < {e}");
                i += 1;
            }
            (None, Some(a)) => {
                println!("    > {a}");
                j += 1;
            }
            (None, None) => unreachable!(),
        }
    }
}

fn parse_marker(test_dir: &Path) -> Marker {
    let Ok(text) = std::fs::read_to_string(test_dir.join("oracle")) else {
        return Marker::Ineligible; // absent = golden-only
    };
    let first = text.lines().next().unwrap_or("").trim().to_string();
    if first == "eligible" {
        Marker::Eligible
    } else if let Some(rest) = first.strip_prefix("known-divergence:") {
        Marker::Known(rest.trim().to_string())
    } else {
        Marker::Ineligible
    }
}

/// The differential pass for one fixture. Engine rows are the already
/// normalized golden lines re-parsed as JSON; the oracle side is an
/// in-process spareval evaluation over `data.ttl` (default graph —
/// setup.sql multi-graph datasets are marker-ineligible).
fn oracle_pass(test_dir: &Path, name: &str, actual: &[String], t: &mut Totals) {
    let marker = parse_marker(test_dir);
    let known = match &marker {
        Marker::Ineligible => {
            t.oracle_skip += 1;
            return;
        }
        Marker::Eligible => None,
        Marker::Known(r) => Some(r.clone()),
    };
    let data_path = test_dir.join("data.ttl");
    let (Ok(data), Ok(query)) = (
        std::fs::read_to_string(&data_path),
        std::fs::read_to_string(test_dir.join("query.rq")),
    ) else {
        t.oracle_skip += 1;
        return;
    };

    let oracle_rows = match eval::eval(&data, &query) {
        Ok(rows) => rows,
        Err(e) => {
            println!("  {RED}ORACLE-ERROR{RESET} {name} (eval failed: {e})");
            t.oracle_diverge += 1;
            return;
        }
    };
    let engine_rows: Result<Vec<Value>, _> =
        actual.iter().map(|l| serde_json::from_str(l)).collect();
    let engine_rows = match engine_rows {
        Ok(rows) => rows,
        Err(e) => {
            println!("  {RED}ORACLE-ERROR{RESET} {name} (engine row not JSON: {e})");
            t.oracle_diverge += 1;
            return;
        }
    };

    match (compare::compare(&engine_rows, &oracle_rows), known) {
        (compare::Verdict::Match, None) => {
            println!("  {GREEN}ORACLE{RESET}   {name}");
            t.oracle_match += 1;
        }
        (compare::Verdict::Match, Some(r)) => {
            println!(
                "  {YELLOW}ORACLE-RESOLVED?{RESET} {name} — matches now; flip marker back to eligible ({r})"
            );
            t.oracle_match += 1;
        }
        (compare::Verdict::Diverge { .. }, Some(r)) => {
            println!("  {YELLOW}ORACLE-KNOWN{RESET} {name} ({r})");
            t.oracle_known += 1;
        }
        (compare::Verdict::Diverge { detail }, None) => {
            println!("  {RED}ORACLE-DIVERGE{RESET} {name}\n    {detail}");
            t.oracle_diverge += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_id_shape_and_determinism() {
        let a = graph_id_for("01-basic-bgp");
        let b = graph_id_for("01-basic-bgp");
        assert_eq!(a, b, "stable across calls");
        assert!(a.starts_with("10"));
        assert!(a.len() <= 6 && a.len() >= 3);
        assert!(a.chars().all(|c| c.is_ascii_digit()));
        assert_ne!(graph_id_for("02-distinct"), a, "names hash apart");
    }

    #[test]
    fn zero_elapsed_ms_narrow() {
        assert_eq!(
            zero_elapsed_ms(r#"{"_update": "ok", "elapsed_ms": 12.5e+1}"#),
            r#"{"_update": "ok", "elapsed_ms": 0}"#
        );
        // SELECT rows untouched.
        let row = r#"{"s": "x", "n": "elapsed_ms is data here"}"#;
        assert_eq!(zero_elapsed_ms(row), row);
    }

    #[test]
    fn normalize_filters_and_sorts() {
        let raw = "CREATE EXTENSION\n1\n{\"b\": \"2\"}\n{\"a\": \"1\"}\nt\n";
        assert_eq!(normalize(raw), vec![r#"{"a": "1"}"#, r#"{"b": "2"}"#]);
    }

    #[test]
    fn sql_assembly_escapes_and_routes() {
        let dir = std::env::temp_dir().join(format!("pgrdf-oracle-run-t-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("query.rq"), "SELECT ?s WHERE { ?s ?p 'it''s' }").unwrap();
        std::fs::write(dir.join("data.ttl"), "<a:s> <a:p> 'x' .").unwrap();
        let sql = assemble_sql(&dir, "101111").unwrap();
        assert!(sql.starts_with("DROP EXTENSION IF EXISTS pgrdf CASCADE;"));
        assert!(sql.contains("SELECT pgrdf.add_graph(101111);"));
        assert!(sql.contains("pgrdf.parse_turtle("));
        assert!(sql.contains("'it''''s'"), "query quotes doubled");
        assert!(sql
            .trim_end()
            .ends_with("FROM pgrdf.sparql('SELECT ?s WHERE { ?s ?p ''it''''s'' }');"));
        std::fs::write(dir.join("kind"), "construct\n").unwrap();
        let sql = assemble_sql(&dir, "101111").unwrap();
        assert!(sql.contains("FROM pgrdf.construct("));
        std::fs::write(dir.join("kind"), "bogus").unwrap();
        assert!(assemble_sql(&dir, "101111").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn marker_parse_variants() {
        let dir = std::env::temp_dir().join(format!("pgrdf-oracle-run-m-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(parse_marker(&dir), Marker::Ineligible), "absent");
        std::fs::write(dir.join("oracle"), "eligible\n").unwrap();
        assert!(matches!(parse_marker(&dir), Marker::Eligible));
        std::fs::write(dir.join("oracle"), "ineligible: UPDATE form\n").unwrap();
        assert!(matches!(parse_marker(&dir), Marker::Ineligible));
        std::fs::write(dir.join("oracle"), "known-divergence: #55 — HAVING alias\n").unwrap();
        match parse_marker(&dir) {
            Marker::Known(r) => assert!(r.starts_with("#55")),
            _ => panic!("expected Known"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod parity_dump {
    /// cargo test parity_dump -- --nocapture  (dev aid; asserts nothing)
    #[test]
    fn dump_gids() {
        if std::env::var("GID_DUMP").is_err() {
            return;
        }
        let dir = std::path::Path::new("../w3c-sparql");
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n != "fixtures")
            .collect();
        names.sort();
        for n in names {
            println!("{n} {}", super::graph_id_for(&n));
        }
    }
}
