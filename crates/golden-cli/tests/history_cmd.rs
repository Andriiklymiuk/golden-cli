use std::fs;

use assert_cmd::Command;
use httpmock::prelude::*;
use tempfile::tempdir;

fn write(dir: &std::path::Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn history_list_json_prints_entries_as_array_newest_last() {
    let ws = tempdir().unwrap();
    write(
        ws.path(),
        ".golden/history.jsonl",
        concat!(
            r#"{"timestamp":"2026-06-01T00:00:00Z","method":"GET","url":"https://api.test/old","request_headers":[["Accept","application/json"]],"status":200,"time_ms":10}"#,
            "\n",
            r#"{"timestamp":"2026-06-02T00:00:00Z","method":"POST","url":"https://api.test/new","request_headers":[],"request_body":"{}","status":201,"time_ms":20}"#,
            "\n",
        ),
    );

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["history", "list", "--json"])
        .assert()
        .success();

    let out = assert.get_output().stdout.clone();
    assert_eq!(out.last(), Some(&b'\n'));
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 2);
    // Same ordering as the table view: newest last.
    assert_eq!(arr[0]["url"], "https://api.test/old");
    assert_eq!(arr[1]["url"], "https://api.test/new");
    // Entry struct serialized verbatim.
    assert_eq!(arr[0]["request_headers"][0][0], "Accept");
    assert_eq!(arr[1]["method"], "POST");
    assert_eq!(arr[1]["request_body"], "{}");
    assert_eq!(arr[1]["status"], 201);
    assert_eq!(arr[1]["time_ms"], 20);
}

#[test]
fn history_list_json_empty_history_prints_empty_array() {
    let ws = tempdir().unwrap();

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["history", "list", "--json"])
        .assert()
        .success();

    let v: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v, serde_json::json!([]));
}

#[test]
fn history_list_json_includes_entry_recorded_by_send() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/health");
        then.status(200).body("ok");
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"Sample"}},"item":[
            {{"name":"health","request":{{"method":"GET","url":"{}/health"}}}}
        ]}}"#,
        server.base_url()
    );
    write(ws.path(), "collections/sample.json", &coll);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["send", "Sample", "health"])
        .assert()
        .success();

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["history", "list", "--json"])
        .assert()
        .success();

    let v: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["method"], "GET");
    assert_eq!(arr[0]["status"], 200);
    assert_eq!(
        arr[0]["url"],
        format!("{}/health", server.base_url()),
        "stored url is the resolved one"
    );
}
