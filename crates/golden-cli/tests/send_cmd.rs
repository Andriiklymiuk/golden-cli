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
fn send_reporter_json_prints_single_object_with_tests() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/health");
        then.status(200)
            .header("content-type", "application/json")
            .header("set-cookie", "session=abc; Path=/; HttpOnly")
            .body("{\"ok\":true}");
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"Sample"}},
            "variable":[{{"key":"base","value":"{}"}}],
            "item":[{{
              "name":"health",
              "request":{{"method":"GET","url":"{{{{base}}}}/health",
                "header":[{{"key":"Accept","value":"application/json"}}]}},
              "event":[{{"listen":"test","script":{{"exec":[
                "pm.test('status 200', function () {{ pm.response.to.have.status(200); }});",
                "pm.test('FAILS', function () {{ pm.expect(1).to.equal(2); }});"
              ]}}}}]
            }}]
        }}"#,
        server.base_url()
    );
    write(ws.path(), "collections/sample.json", &coll);

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["send", "Sample", "health", "--reporter", "json"])
        .assert()
        .code(1); // one failing assertion -> exit 1

    // stdout is EXACTLY one JSON object (plus trailing newline), nothing else.
    let out = assert.get_output().stdout.clone();
    assert_eq!(out.last(), Some(&b'\n'));
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();

    assert_eq!(v["request"]["name"], "health");
    assert_eq!(v["request"]["method"], "GET");
    assert_eq!(
        v["request"]["url"],
        format!("{}/health", server.base_url()),
        "url must be resolved after variable substitution"
    );
    assert_eq!(v["request"]["headers"][0]["key"], "Accept");

    assert_eq!(v["response"]["status"], 200);
    assert_eq!(v["response"]["status_text"], "OK");
    assert_eq!(v["response"]["body_truncated"], false);
    assert_eq!(v["response"]["size_bytes"], 11);
    let cookies = v["response"]["cookies"].as_array().unwrap();
    assert_eq!(cookies[0]["name"], "session");
    assert_eq!(cookies[0]["value"], "abc");
    assert_eq!(cookies[0]["raw"], "session=abc; Path=/; HttpOnly");
    let body = base64_decode(v["response"]["body_base64"].as_str().unwrap());
    assert_eq!(body, b"{\"ok\":true}");

    let tests = v["tests"].as_array().unwrap();
    assert_eq!(tests.len(), 2);
    assert_eq!(tests[0]["name"], "status 200");
    assert_eq!(tests[0]["passed"], true);
    assert_eq!(tests[1]["passed"], false);
    assert!(tests[1]["error"].is_string());

    assert_eq!(v["error"], serde_json::Value::Null);
}

#[test]
fn send_reporter_json_transport_error_has_null_response() {
    let ws = tempdir().unwrap();
    let coll = r#"{"info":{"name":"Sample"},"item":[
        {"name":"down","request":{"method":"GET","url":"http://127.0.0.1:1/x"}}
    ]}"#;
    write(ws.path(), "collections/sample.json", coll);

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["send", "Sample", "down", "--reporter", "json"])
        .assert()
        .code(2); // existing transport-error exit code

    let v: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["response"], serde_json::Value::Null);
    assert!(v["error"].is_string());
    assert!(v["tests"].as_array().unwrap().is_empty());
}

#[test]
fn send_reporter_json_success_without_tests_exits_zero() {
    let server = MockServer::start();
    server.mock(|when, then| {
        when.method(GET).path("/ok");
        then.status(200).body("ok");
    });
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"Sample"}},"item":[
            {{"name":"ok","request":{{"method":"GET","url":"{}/ok"}}}}
        ]}}"#,
        server.base_url()
    );
    write(ws.path(), "collections/sample.json", &coll);

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["send", "Sample", "ok", "-r", "json"])
        .assert()
        .success();
    let v: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(v["response"]["status"], 200);
    assert!(v["tests"].as_array().unwrap().is_empty());
}

/// Minimal standard-alphabet base64 decoder for test assertions (no extra dep).
fn base64_decode(s: &str) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bits = 0u32;
    let mut n = 0u32;
    let mut out = Vec::new();
    for &c in s.as_bytes() {
        if c == b'=' {
            break;
        }
        let v = ALPHABET.iter().position(|&a| a == c).unwrap() as u32;
        bits = (bits << 6) | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
        }
    }
    out
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
