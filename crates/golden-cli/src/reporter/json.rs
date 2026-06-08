//! JSON reporter — the full RunResult serialized pretty for machine consumers.

use std::io::{self, Write};

use golden_core::result::RunResult;

use super::Reporter;
use crate::cli::ReporterKind;

pub struct JsonReporter;

impl Reporter for JsonReporter {
    fn kind(&self) -> ReporterKind {
        ReporterKind::Json
    }

    fn report(&self, result: &RunResult, out: &mut dyn Write, _color: bool) -> io::Result<()> {
        serde_json::to_writer_pretty(&mut *out, result).map_err(io::Error::other)?;
        writeln!(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::result::{
        Assertion, CollectionResult, Iteration, RequestResult, RunResult, Totals,
    };

    fn sample() -> RunResult {
        RunResult {
            collections: vec![CollectionResult {
                name: "Sample".into(),
                iterations: vec![Iteration {
                    index: 1,
                    requests: vec![RequestResult {
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
                    }],
                }],
                stats: vec![],
            }],
            totals: Totals {
                requests: 1,
                failed_requests: 0,
                assertions: 1,
                failed_assertions: 0,
                total_ms: 12,
            },
        }
    }

    #[test]
    fn emits_valid_json_with_totals_and_collections() {
        let mut buf = Vec::new();
        JsonReporter.report(&sample(), &mut buf, false).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["totals"]["requests"], 1);
        assert_eq!(v["totals"]["assertions"], 1);
        assert_eq!(v["collections"][0]["name"], "Sample");
        assert_eq!(
            v["collections"][0]["iterations"][0]["requests"][0]["name"],
            "login"
        );
        assert_eq!(
            v["collections"][0]["iterations"][0]["requests"][0]["status"],
            200
        );
        assert_eq!(
            v["collections"][0]["iterations"][0]["requests"][0]["assertions"][0]["passed"],
            true
        );
    }

    #[test]
    fn output_ends_with_newline() {
        let mut buf = Vec::new();
        JsonReporter.report(&sample(), &mut buf, false).unwrap();
        assert_eq!(buf.last(), Some(&b'\n'));
    }
}
