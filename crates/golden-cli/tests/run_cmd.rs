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

#[test]
fn run_json_stream_emits_ordered_ndjson_across_iterations() {
    let server = MockServer::start();
    for p in ["/a", "/b"] {
        server.mock(|when, then| {
            when.path(p);
            then.status(200);
        });
    }
    let ws = tempdir().unwrap();
    let coll = format!(
        r#"{{"info":{{"name":"Stream"}},"item":[
            {{"name":"first","request":{{"method":"GET","url":"{base}/a"}}}},
            {{"name":"second","request":{{"method":"GET","url":"{base}/b"}}}}
        ]}}"#,
        base = server.base_url()
    );
    write(ws.path(), "collections/stream.json", &coll);

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run", "-n", "2", "--reporter", "json-stream"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();

    // (a) every line is a standalone JSON object
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).expect("each line must parse as JSON standalone"))
        .collect();

    // (b) order is collection -> N request lines -> done
    assert_eq!(lines.len(), 6, "collection + 4 requests + done:\n{stdout}");
    assert_eq!(lines[0]["type"], "collection");
    assert_eq!(lines[0]["name"], "Stream");
    assert!(
        lines[0]["file"].as_str().unwrap().ends_with("stream.json"),
        "collection line carries the file path: {}",
        lines[0]
    );
    for line in &lines[1..5] {
        assert_eq!(line["type"], "request");
        assert_eq!(line["collection"], "Stream");
    }
    assert_eq!(lines[5]["type"], "done");

    // (c) request lines cover both iterations with correct 1-based numbers
    let seen: Vec<(i64, &str)> = lines[1..5]
        .iter()
        .map(|l| {
            (
                l["iteration"].as_i64().unwrap(),
                l["request"]["name"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        vec![(1, "first"), (1, "second"), (2, "first"), (2, "second")]
    );
    for line in &lines[1..5] {
        assert_eq!(line["request"]["status"], 200);
        assert_eq!(line["request"]["method"], "GET");
    }

    // (d) done.result.totals matches the request lines seen
    assert_eq!(lines[5]["result"]["totals"]["requests"], 4);
    assert_eq!(lines[5]["result"]["totals"]["failed_requests"], 0);
    assert_eq!(
        lines[5]["result"]["collections"][0]["name"], "Stream",
        "done carries the full RunResult"
    );
}

#[test]
fn run_json_stream_rejects_other_reporters() {
    let ws = tempdir().unwrap();
    write(
        ws.path(),
        "collections/c.json",
        r#"{"info":{"name":"C"},"item":[]}"#,
    );
    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run", "-r", "json-stream", "-r", "json"])
        .assert()
        .code(2)
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "json-stream cannot be combined with other reporters",
        ));
}

#[test]
fn run_json_stream_failed_request_exits_2_and_streams() {
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

    let assert = Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run", "-r", "json-stream"])
        .assert()
        .code(2);
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[1]["request"]["status"], 500);
    assert_eq!(lines[2]["result"]["totals"]["failed_requests"], 1);
}

#[test]
fn run_json_stream_writes_stream_to_output_file() {
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
    let out = ws.path().join("stream.ndjson");

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(ws.path())
        .args(["run", "-r", "json-stream", "-o", out.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
    let content = fs::read_to_string(&out).unwrap();
    let lines: Vec<serde_json::Value> = content
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["type"], "collection");
    assert_eq!(lines[1]["type"], "request");
    assert_eq!(lines[2]["type"], "done");
}
