//! Pretty (default) reporter — colored tree:
//! collection → iteration → request → assertions, with timing and a summary line.
//! Colors are applied only when `color` is true (the dispatcher passes the
//! tty/NO_COLOR-aware decision via owo-colors' if_supports_color upstream).

use std::io::{self, Write};

use owo_colors::OwoColorize;

use golden_core::result::{Assertion, RequestResult, RunResult};

use super::Reporter;
use crate::cli::ReporterKind;

pub struct PrettyReporter;

impl Reporter for PrettyReporter {
    fn kind(&self) -> ReporterKind {
        ReporterKind::Pretty
    }

    fn report(&self, result: &RunResult, out: &mut dyn Write, color: bool) -> io::Result<()> {
        for collection in &result.collections {
            writeln!(out, "{}", paint_bold(&collection.name, color))?;
            let multi = collection.iterations.len() > 1;
            for iteration in &collection.iterations {
                if multi {
                    writeln!(out, "  iteration {}", iteration.index)?;
                }
                for req in &iteration.requests {
                    write_request(out, req, color, multi)?;
                }
            }
            if !collection.stats.is_empty() {
                writeln!(out, "  stats:")?;
                for s in &collection.stats {
                    writeln!(
                        out,
                        "    {} — avg {:.2}ms min {}ms max {}ms",
                        s.name, s.avg_ms, s.min_ms, s.max_ms
                    )?;
                }
            }
            writeln!(out)?;
        }
        write_summary(out, result, color)
    }
}

fn write_request(
    out: &mut dyn Write,
    req: &RequestResult,
    color: bool,
    nested: bool,
) -> io::Result<()> {
    let indent = if nested { "    " } else { "  " };
    let req_ok = req.error.is_none()
        && req.status.map(|s| s < 400).unwrap_or(false)
        && req.assertions.iter().all(|a| a.passed);
    let marker = if req_ok {
        paint_green("\u{2713}", color)
    } else {
        paint_red("\u{2717}", color)
    };
    let status = match req.status {
        Some(s) => s.to_string(),
        None => "ERR".to_string(),
    };
    writeln!(
        out,
        "{indent}{marker} {} {} [{}] {}ms",
        req.method, req.name, status, req.time_ms
    )?;
    if let Some(err) = &req.error {
        writeln!(out, "{indent}    {}", paint_red(err, color))?;
    }
    for a in &req.assertions {
        write_assertion(out, a, color, indent)?;
    }
    Ok(())
}

fn write_assertion(
    out: &mut dyn Write,
    a: &Assertion,
    color: bool,
    indent: &str,
) -> io::Result<()> {
    if a.passed {
        writeln!(
            out,
            "{indent}    {} {}",
            paint_green("\u{2713}", color),
            a.name
        )?;
    } else {
        writeln!(
            out,
            "{indent}    {} {}",
            paint_red("\u{2717}", color),
            a.name
        )?;
        if let Some(err) = &a.error {
            writeln!(out, "{indent}      {}", paint_red(err, color))?;
        }
    }
    Ok(())
}

fn write_summary(out: &mut dyn Write, result: &RunResult, color: bool) -> io::Result<()> {
    let t = &result.totals;
    let line = format!(
        "{} requests, {} failed | {} assertions, {} failed | {}ms",
        t.requests, t.failed_requests, t.assertions, t.failed_assertions, t.total_ms
    );
    if t.failed_assertions == 0 && t.failed_requests == 0 {
        writeln!(out, "{}", paint_green(&line, color))
    } else {
        writeln!(out, "{}", paint_red(&line, color))
    }
}

fn paint_bold(s: &str, color: bool) -> String {
    if color {
        s.bold().to_string()
    } else {
        s.to_string()
    }
}

fn paint_green(s: &str, color: bool) -> String {
    if color {
        s.green().to_string()
    } else {
        s.to_string()
    }
}

fn paint_red(s: &str, color: bool) -> String {
    if color {
        s.red().to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::result::{
        Assertion, CollectionResult, Iteration, RequestResult, RequestStats, RunResult, Totals,
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
                                name: "status is 200".into(),
                                passed: false,
                                error: Some("expected 200 got 500".into()),
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

    fn render(result: &RunResult) -> String {
        let mut buf: Vec<u8> = Vec::new();
        PrettyReporter.report(result, &mut buf, false).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn renders_tree_with_collection_requests_and_assertions() {
        let out = render(&sample());
        assert!(out.contains("Sample"), "collection name missing:\n{out}");
        assert!(out.contains("POST"), "method missing:\n{out}");
        assert!(out.contains("login"), "request name missing:\n{out}");
        assert!(out.contains("200"), "status missing:\n{out}");
        assert!(
            out.contains("status is 200"),
            "assertion name missing:\n{out}"
        );
    }

    #[test]
    fn shows_pass_and_fail_markers_and_assertion_error() {
        let out = render(&sample());
        assert!(out.contains("\u{2713}"), "no check mark (pass):\n{out}"); // ✓
        assert!(out.contains("\u{2717}"), "no cross (fail):\n{out}"); // ✗
        assert!(
            out.contains("expected 200 got 500"),
            "assertion error missing:\n{out}"
        );
    }

    #[test]
    fn summary_line_reports_totals() {
        let out = render(&sample());
        assert!(out.contains("2 requests"), "request total missing:\n{out}");
        assert!(out.contains("1 failed"), "failed total missing:\n{out}");
        assert!(
            out.contains("assertions"),
            "assertion total missing:\n{out}"
        );
    }

    #[test]
    fn prints_transport_error_for_failed_request() {
        let mut r = sample();
        r.collections[0].iterations[0].requests[1].status = None;
        r.collections[0].iterations[0].requests[1].error =
            Some("Connection refused: Server not running or unreachable".into());
        let out = render(&r);
        assert!(
            out.contains("Connection refused"),
            "transport error missing:\n{out}"
        );
    }

    #[test]
    fn prints_stats_block_when_present() {
        let mut r = sample();
        r.collections[0].stats = vec![RequestStats {
            name: "login".into(),
            avg_ms: 11.5,
            min_ms: 8,
            max_ms: 15,
        }];
        let out = render(&r);
        assert!(out.contains("avg"), "stats avg missing:\n{out}");
        assert!(
            out.contains("11.5") || out.contains("11.50"),
            "avg value missing:\n{out}"
        );
        assert!(out.contains("login"), "stats request name missing:\n{out}");
    }
}
