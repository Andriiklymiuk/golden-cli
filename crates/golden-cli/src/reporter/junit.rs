//! JUnit XML reporter. One <testsuite> per collection iteration; one <testcase>
//! per assertion. A request with no assertions but a transport error becomes a
//! single errored <testcase>. XML is hand-built with explicit escaping so the
//! output is deterministic and validates in GitHub Actions / GitLab / Jenkins.

use std::io::{self, Write};

use golden_core::result::{RequestResult, RunResult};

use super::Reporter;
use crate::cli::ReporterKind;

pub struct JunitReporter;

impl Reporter for JunitReporter {
    fn kind(&self) -> ReporterKind {
        ReporterKind::Junit
    }

    fn report(&self, result: &RunResult, out: &mut dyn Write, _color: bool) -> io::Result<()> {
        let t = &result.totals;
        let total_secs = t.total_ms as f64 / 1000.0;
        writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>")?;
        writeln!(
            out,
            "<testsuites tests=\"{}\" failures=\"{}\" time=\"{:.3}\">",
            t.assertions, t.failed_assertions, total_secs
        )?;

        for collection in &result.collections {
            for iteration in &collection.iterations {
                let suite_name = if collection.iterations.len() > 1 {
                    format!("{} (iteration {})", collection.name, iteration.index)
                } else {
                    collection.name.clone()
                };
                let (tests, failures, errors) = suite_counts(&iteration.requests);
                writeln!(
                    out,
                    "  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"{}\">",
                    escape_attr(&suite_name),
                    tests,
                    failures,
                    errors
                )?;
                for req in &iteration.requests {
                    write_request(out, req)?;
                }
                writeln!(out, "  </testsuite>")?;
            }
        }

        writeln!(out, "</testsuites>")
    }
}

fn suite_counts(requests: &[RequestResult]) -> (usize, usize, usize) {
    let mut tests = 0;
    let mut failures = 0;
    let mut errors = 0;
    for req in requests {
        if req.assertions.is_empty() {
            // a request with no assertions still appears as one testcase
            tests += 1;
            if req.error.is_some() {
                errors += 1;
            }
        } else {
            for a in &req.assertions {
                tests += 1;
                if !a.passed {
                    failures += 1;
                }
            }
        }
    }
    (tests, failures, errors)
}

fn write_request(out: &mut dyn Write, req: &RequestResult) -> io::Result<()> {
    let secs = req.time_ms as f64 / 1000.0;
    if req.assertions.is_empty() {
        let case_name = format!("{} {}", req.method, req.name);
        writeln!(
            out,
            "    <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">",
            escape_attr(&case_name),
            escape_attr(&req.name),
            secs
        )?;
        if let Some(err) = &req.error {
            writeln!(
                out,
                "      <error message=\"{}\">{}</error>",
                escape_attr(err),
                escape_text(err)
            )?;
        }
        writeln!(out, "    </testcase>")?;
    } else {
        for a in &req.assertions {
            writeln!(
                out,
                "    <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">",
                escape_attr(&a.name),
                escape_attr(&req.name),
                secs
            )?;
            if !a.passed {
                let msg = a.error.clone().unwrap_or_else(|| "assertion failed".into());
                writeln!(
                    out,
                    "      <failure message=\"{}\">{}</failure>",
                    escape_attr(&msg),
                    escape_text(&msg)
                )?;
            }
            writeln!(out, "    </testcase>")?;
        }
    }
    Ok(())
}

/// Escape for an XML attribute value (quotes included).
fn escape_attr(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Escape for XML element text (no quote handling needed).
fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
                name: "Sample & Co".into(),
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
        JunitReporter.report(r, &mut buf, false).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn emits_xml_prolog_and_testsuites_with_counts() {
        let out = render(&sample());
        assert!(
            out.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"),
            "{out}"
        );
        assert!(out.contains("<testsuites"), "{out}");
        assert!(out.contains("tests=\"2\""), "{out}");
        assert!(out.contains("failures=\"1\""), "{out}");
    }

    #[test]
    fn each_assertion_is_a_testcase() {
        let out = render(&sample());
        assert!(out.contains("name=\"status is 200\""), "{out}");
        assert!(out.contains("name=\"status &lt; 400\""), "{out}");
    }

    #[test]
    fn failure_has_failure_element_with_message() {
        let out = render(&sample());
        assert!(out.contains("<failure"), "{out}");
        assert!(out.contains("expected &lt; 400 got 500"), "{out}");
    }

    #[test]
    fn escapes_xml_special_chars_in_suite_name() {
        let out = render(&sample());
        assert!(out.contains("Sample &amp; Co"), "{out}");
    }

    #[test]
    fn transport_error_request_becomes_errored_testcase() {
        let mut r = sample();
        r.collections[0].iterations[0].requests[1].status = None;
        r.collections[0].iterations[0].requests[1].assertions = vec![];
        r.collections[0].iterations[0].requests[1].error = Some("Connection refused".into());
        let out = render(&r);
        assert!(out.contains("<error"), "{out}");
        assert!(out.contains("Connection refused"), "{out}");
    }
}
