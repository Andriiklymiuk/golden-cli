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
fn send_fires_named_request_and_prints_response() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/health");
        then.status(200)
            .header("content-type", "application/json")
            .body("{\"ok\":true}");
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
        .success()
        .stdout(predicate::str::contains("200"))
        .stdout(predicate::str::contains("{\"ok\":true}"));
}

#[test]
fn send_unknown_request_exits_2() {
    let ws = tempdir().unwrap();
    let coll = r#"{"info":{"name":"Sample"},"item":[]}"#;
    write(ws.path(), "collections/sample.json", coll);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["send", "Sample", "missing"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("request 'missing' not found"));
}

#[test]
fn send_unknown_collection_exits_2() {
    let ws = tempdir().unwrap();
    let coll = r#"{"info":{"name":"Sample"},"item":[]}"#;
    write(ws.path(), "collections/sample.json", coll);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["send", "Nope", "x"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("collection 'Nope' not found"));
}

#[test]
fn send_substitutes_variables_from_collection() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/u/42");
        then.status(200).body("ok");
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"Vars"}},
            "variable":[{{"key":"base","value":"{}"}},{{"key":"id","value":"42"}}],
            "item":[{{"name":"user","request":{{"method":"GET","url":"{{{{base}}}}/u/{{{{id}}}}"}}}}]
        }}"#,
        server.base_url()
    );
    write(ws.path(), "collections/vars.json", &coll);

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["send", "Vars", "user"])
        .assert()
        .success()
        .stdout(predicate::str::contains("200"));
}
