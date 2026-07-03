//! End-to-end CLI contract: the exact interface tests/w3c-sparql/run.sh
//! drives in differential mode. Exit codes: 0 = match, 1 = divergence,
//! 2 = usage/eval error.

use std::fs;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_pgrdf-oracle"))
}

fn tmpdir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pgrdf-oracle-cli-{name}-{}", std::process::id()));
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn eval_emits_canonical_jsonl() {
    let d = tmpdir("eval");
    let data = d.join("data.ttl");
    let query = d.join("query.rq");
    fs::write(
        &data,
        "@prefix foaf: <http://xmlns.com/foaf/0.1/> .\n\
         <http://ex.com/a> foaf:name \"Alice\" .\n\
         <http://ex.com/b> foaf:name \"Bob\" .\n",
    )
    .unwrap();
    fs::write(
        &query,
        "PREFIX foaf: <http://xmlns.com/foaf/0.1/>\nSELECT ?s ?n WHERE { ?s foaf:name ?n }",
    )
    .unwrap();
    let out = bin().arg("eval").arg(&data).arg(&query).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: Vec<serde_json::Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .all(|r| r.get("s").is_some() && r.get("n").is_some()));
}

#[test]
fn compare_match_exits_zero() {
    let d = tmpdir("cmp0");
    let engine = d.join("engine.jsonl");
    let oracle = d.join("oracle.jsonl");
    fs::write(&engine, "{\"n\": \"Alice\"}\n{\"n\": \"Bob\"}\n").unwrap();
    fs::write(&oracle, "{\"n\": \"Bob\"}\n{\"n\": \"Alice\"}\n").unwrap();
    let out = bin()
        .arg("compare")
        .arg(&engine)
        .arg(&oracle)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn compare_divergence_exits_one_with_detail() {
    let d = tmpdir("cmp1");
    let engine = d.join("engine.jsonl");
    let oracle = d.join("oracle.jsonl");
    fs::write(&engine, "{\"n\": \"Alice\"}\n").unwrap();
    fs::write(&oracle, "{\"n\": \"Bob\"}\n").unwrap();
    let out = bin()
        .arg("compare")
        .arg(&engine)
        .arg(&oracle)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("diverge"), "stdout: {stdout}");
}

#[test]
fn bad_usage_exits_two() {
    let out = bin().arg("frobnicate").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = bin().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn eval_error_exits_two() {
    let d = tmpdir("evalerr");
    let data = d.join("data.ttl");
    let query = d.join("query.rq");
    fs::write(&data, "@@ not turtle").unwrap();
    fs::write(&query, "ASK { ?s ?p ?o }").unwrap();
    let out = bin().arg("eval").arg(&data).arg(&query).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(!String::from_utf8_lossy(&out.stderr).is_empty());
}
