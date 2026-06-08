use std::fs;

use assert_cmd::Command;
use httpmock::prelude::*;
use predicates::prelude::*;
use tempfile::tempdir;

fn write(dir: &std::path::Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn run_passes_with_exit_0_and_pretty_output() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.path("/ok");
        then.status(200);
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"OK"}},"item":[
            {{"name":"ping","request":{{"method":"GET","url":"{}/ok"}}}}
        ]}}"#,
        server.base_url()
    );
    write(ws.path(), "collections/ok.json", &coll);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ping"))
        .stdout(predicate::str::contains("1 requests, 0 failed"));
}

#[test]
fn run_with_failed_request_exits_2() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.path("/boom");
        then.status(500);
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"Boom"}},"item":[
            {{"name":"boom","request":{{"method":"GET","url":"{}/boom"}}}}
        ]}}"#,
        server.base_url()
    );
    write(ws.path(), "collections/boom.json", &coll);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run"])
        .assert()
        .code(2);
}

#[test]
fn run_no_collections_exits_2_with_message() {
    let ws = tempdir().unwrap();
    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no collections found"));
}

#[test]
fn run_writes_output_to_file() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.path("/ok");
        then.status(200);
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"OK"}},"item":[
            {{"name":"ping","request":{{"method":"GET","url":"{}/ok"}}}}
        ]}}"#,
        server.base_url()
    );
    write(ws.path(), "collections/ok.json", &coll);
    let out = ws.path().join("report.txt");

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run", "--output", out.to_str().unwrap()])
        .assert()
        .success();
    let content = fs::read_to_string(&out).unwrap();
    assert!(
        content.contains("ping"),
        "file report missing request:\n{content}"
    );
}

#[test]
fn run_filter_limits_requests() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.path("/a");
        then.status(200);
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"C"}},"item":[
            {{"name":"keep","request":{{"method":"GET","url":"{base}/a"}}}},
            {{"name":"drop","request":{{"method":"GET","url":"{base}/zzz"}}}}
        ]}}"#,
        base = server.base_url()
    );
    write(ws.path(), "collections/c.json", &coll);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run", "--filter", "keep"])
        .assert()
        .success()
        .stdout(predicate::str::contains("keep"))
        .stdout(predicate::str::contains("drop").not())
        .stdout(predicate::str::contains("1 requests"));
}

#[test]
fn run_with_junit_reporter_emits_xml() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.path("/ok");
        then.status(200);
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"OK"}},"item":[
            {{"name":"ping","request":{{"method":"GET","url":"{}/ok"}}}}
        ]}}"#,
        server.base_url()
    );
    write(ws.path(), "collections/ok.json", &coll);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run", "--reporter", "junit"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<testsuites"));
}

#[test]
fn run_with_json_reporter_emits_parseable_json() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.path("/ok");
        then.status(200);
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"OK"}},"item":[
            {{"name":"ping","request":{{"method":"GET","url":"{}/ok"}}}}
        ]}}"#,
        server.base_url()
    );
    write(ws.path(), "collections/ok.json", &coll);

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run", "--reporter", "json"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let v: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["totals"]["requests"], 1);
}
