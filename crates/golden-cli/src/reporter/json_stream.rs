//! NDJSON streaming reporter — one compact JSON object per line, flushed as the
//! run progresses so machine consumers (e.g. the VS Code extension) see live
//! per-request results instead of one blob at the end.
//!
//! The stream is: a `collection` line when each collection starts, a `request`
//! line after each request completes (emitted by the run command via the core
//! per-request event callback), and a single terminal `done` line carrying the
//! full `RunResult` in the same shape the plain json reporter prints.

use std::io::{self, Write};
use std::path::Path;

use golden_core::result::{RequestResult, RunResult};
use serde_json::json;

use super::Reporter;
use crate::cli::ReporterKind;

pub struct JsonStreamReporter;

impl Reporter for JsonStreamReporter {
    fn kind(&self) -> ReporterKind {
        ReporterKind::JsonStream
    }

    /// Write the terminal `done` event. The `collection`/`request` lines are
    /// streamed live by the run command while the run is in flight.
    fn report(&self, result: &RunResult, out: &mut dyn Write, _color: bool) -> io::Result<()> {
        writeln!(out, "{}", done_line(result))
    }
}

/// `{"type":"collection",...}` — emitted when a collection file starts running.
pub fn collection_line(name: &str, file: &Path) -> String {
    json!({"type": "collection", "name": name, "file": file.display().to_string()}).to_string()
}

/// `{"type":"request",...}` — emitted after each request completes, with the
/// 1-based iteration index and the full RequestResult serialized verbatim.
pub fn request_line(collection: &str, iteration: u32, request: &RequestResult) -> String {
    json!({"type": "request", "collection": collection, "iteration": iteration, "request": request})
        .to_string()
}

/// `{"type":"done",...}` — emitted once after ALL files, with the merged
/// RunResult serialized verbatim (same shape as the plain json reporter).
pub fn done_line(result: &RunResult) -> String {
    json!({"type": "done", "result": result}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::result::{Assertion, RequestResult, RunResult, Totals};

    fn sample_request() -> RequestResult {
        RequestResult {
            name: "login".into(),
            method: "POST".into(),
            url: "https://api.test/login".into(),
            status: Some(200),
            time_ms: 12,
            assertions: vec![Assertion {
                name: "status is 200".into(),
                passed: true,
                error: None,
            }],
            error: None,
        }
    }

    #[test]
    fn collection_line_is_one_json_object() {
        let line = collection_line("Sample", Path::new("collections/sample.json"));
        assert!(!line.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "collection");
        assert_eq!(v["name"], "Sample");
        assert_eq!(v["file"], "collections/sample.json");
    }

    #[test]
    fn request_line_carries_full_result_and_iteration() {
        let line = request_line("Sample", 2, &sample_request());
        assert!(!line.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["type"], "request");
        assert_eq!(v["collection"], "Sample");
        assert_eq!(v["iteration"], 2);
        assert_eq!(v["request"]["name"], "login");
        assert_eq!(v["request"]["status"], 200);
        assert_eq!(v["request"]["assertions"][0]["passed"], true);
    }

    #[test]
    fn report_writes_done_line_with_run_result() {
        let result = RunResult {
            collections: vec![],
            totals: Totals {
                requests: 1,
                failed_requests: 0,
                assertions: 1,
                failed_assertions: 0,
                total_ms: 12,
            },
        };
        let mut buf = Vec::new();
        JsonStreamReporter.report(&result, &mut buf, false).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(text.lines().count(), 1);
        let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap();
        assert_eq!(v["type"], "done");
        assert_eq!(v["result"]["totals"]["requests"], 1);
    }
}
