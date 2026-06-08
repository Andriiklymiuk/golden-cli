use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn write(dir: &std::path::Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

const SAMPLE: &str = r#"{
  "info": {"name":"Sample"},
  "item": [
    {"name":"auth","item":[
      {"name":"login","request":{"method":"POST","url":"https://x/login"}}
    ]},
    {"name":"health","request":{"method":"GET","url":"https://x/health"}}
  ]
}"#;

#[test]
fn list_prints_collection_folders_and_requests() {
    let ws = tempdir().unwrap();
    write(ws.path(), "collections/sample.json", SAMPLE);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Sample"))
        .stdout(predicate::str::contains("auth"))
        .stdout(predicate::str::contains("login"))
        .stdout(predicate::str::contains("POST"))
        .stdout(predicate::str::contains("GET"))
        .stdout(predicate::str::contains("health"));
}

#[test]
fn list_with_filter_limits_requests() {
    let ws = tempdir().unwrap();
    write(ws.path(), "collections/sample.json", SAMPLE);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["list", "--filter", "login"])
        .assert()
        .success()
        .stdout(predicate::str::contains("login"))
        .stdout(predicate::str::contains("health").not());
}

#[test]
fn list_no_collections_exits_2() {
    let ws = tempdir().unwrap();
    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["list"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no collections found"));
}
