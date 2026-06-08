//! TAP version 13 reporter. One test point per assertion; a request with no
//! assertions is a single point keyed on "METHOD name". Failures carry a YAML
//! diagnostic block (message + status).

use std::io::{self, Write};

use golden_core::result::RunResult;

use super::Reporter;
use crate::cli::ReporterKind;

pub struct TapReporter;

impl Reporter for TapReporter {
    fn kind(&self) -> ReporterKind {
        ReporterKind::Tap
    }

    fn report(&self, result: &RunResult, out: &mut dyn Write, _color: bool) -> io::Result<()> {
        writeln!(out, "TAP version 13")?;
        let total = count_points(result);
        writeln!(out, "1..{total}")?;

        let mut n = 0;
        for collection in &result.collections {
            for iteration in &collection.iterations {
                for req in &iteration.requests {
                    if req.assertions.is_empty() {
                        n += 1;
                        let label = format!("{} {}", req.method, req.name);
                        let passed =
                            req.error.is_none() && req.status.map(|s| s < 400).unwrap_or(false);
                        write_point(out, passed, n, &label, req.error.as_deref(), req.status)?;
                    } else {
                        for a in &req.assertions {
                            n += 1;
                            write_point(out, a.passed, n, &a.name, a.error.as_deref(), req.status)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

fn count_points(result: &RunResult) -> usize {
    let mut total = 0;
    for c in &result.collections {
        for it in &c.iterations {
            for r in &it.requests {
                if r.assertions.is_empty() {
                    total += 1;
                } else {
                    total += r.assertions.len();
                }
            }
        }
    }
    total
}

fn write_point(
    out: &mut dyn Write,
    passed: bool,
    n: usize,
    label: &str,
    error: Option<&str>,
    status: Option<u16>,
) -> io::Result<()> {
    if passed {
        writeln!(out, "ok {n} - {label}")
    } else {
        writeln!(out, "not ok {n} - {label}")?;
        writeln!(out, "  ---")?;
        let message = error.unwrap_or("failed");
        writeln!(out, "  message: {}", yaml_scalar(message))?;
        if let Some(s) = status {
            writeln!(out, "  status: {s}")?;
        }
        writeln!(out, "  ...")
    }
}

/// Quote a YAML scalar if it contains characters that would break a bare scalar.
fn yaml_scalar(s: &str) -> String {
    if s.contains([':', '#', '\n', '"']) {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
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
                    requests: vec![
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
                        },
                        RequestResult {
                            name: "me".into(),
                            method: "GET".into(),
                            url: "https://api.test/me".into(),
                            status: Some(500),
                            time_ms: 8,
                            assertions: vec![Assertion {
                                name: "status < 400".into(),
                                passed: false,
                                error: Some("expected < 400 got 500".into()),
                            }],
                            error: None,
                        },
                    ],
                }],
                stats: vec![],
            }],
            totals: Totals {
                requests: 2,
                failed_requests: 1,
                assertions: 2,
                failed_assertions: 1,
                total_ms: 20,
            },
        }
    }

    fn render(r: &RunResult) -> String {
        let mut buf = Vec::new();
        TapReporter.report(r, &mut buf, false).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn emits_version_and_plan() {
        let out = render(&sample());
        assert!(out.starts_with("TAP version 13\n"), "{out}");
        assert!(out.contains("1..2"), "{out}");
    }

    #[test]
    fn numbers_ok_and_not_ok_lines() {
        let out = render(&sample());
        assert!(out.contains("ok 1 - status is 200"), "{out}");
        assert!(out.contains("not ok 2 - status < 400"), "{out}");
    }

    #[test]
    fn failure_detail_is_a_yaml_block() {
        let out = render(&sample());
        assert!(out.contains("  ---"), "{out}");
        assert!(out.contains("message:"), "{out}");
        assert!(out.contains("expected < 400 got 500"), "{out}");
        assert!(out.contains("  ..."), "{out}");
    }

    #[test]
    fn request_without_assertions_is_a_single_point() {
        let mut r = sample();
        r.collections[0].iterations[0].requests[0].assertions = vec![];
        let out = render(&r);
        // login has no assertions -> one ok point named after the request
        assert!(out.contains("ok 1 - POST login"), "{out}");
    }
}
