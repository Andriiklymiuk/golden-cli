//! UI-side aggregation of a run: turns RunResult + a known total into a progress
//! fraction and flat per-request status lines.

use golden_core::result::{RequestResult, RequestStats, RunResult};

/// Live state of a run, displayed in the run overlay.
#[derive(Debug, Default, Clone)]
pub struct RunState {
    pub running: bool,
    /// Total requests expected (sum of requests across iterations).
    pub total: usize,
    /// Requests completed so far.
    pub done: usize,
    /// Finished result (None until the run completes).
    pub result: Option<RunResult>,
}

impl RunState {
    /// Progress as a 0.0..=1.0 ratio (0 when total is 0).
    pub fn ratio(&self) -> f64 {
        if self.total == 0 {
            return 0.0;
        }
        (self.done as f64 / self.total as f64).clamp(0.0, 1.0)
    }

    /// Flattened (name, ok, status_or_error) lines across all iterations.
    pub fn request_lines(&self) -> Vec<(String, bool, String)> {
        let Some(result) = &self.result else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for coll in &result.collections {
            for iter in &coll.iterations {
                for rr in &iter.requests {
                    out.push(line_for(rr));
                }
            }
        }
        out
    }

    /// Stats rows from all collections (empty unless iterations > 1).
    pub fn stats(&self) -> Vec<RequestStats> {
        self.result
            .as_ref()
            .map(|r| r.collections.iter().flat_map(|c| c.stats.clone()).collect())
            .unwrap_or_default()
    }
}

fn line_for(rr: &RequestResult) -> (String, bool, String) {
    if let Some(err) = &rr.error {
        (rr.name.clone(), false, err.clone())
    } else {
        let ok =
            rr.status.map(|s| s < 400).unwrap_or(false) && rr.assertions.iter().all(|a| a.passed);
        let status = rr
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "\u{2014}".into());
        (rr.name.clone(), ok, status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::result::{Assertion, CollectionResult, Iteration, Totals};

    fn rr(name: &str, status: Option<u16>, pass_assert: bool, err: Option<&str>) -> RequestResult {
        RequestResult {
            name: name.into(),
            method: "GET".into(),
            url: "http://x".into(),
            status,
            time_ms: 5,
            assertions: vec![Assertion {
                name: "a".into(),
                passed: pass_assert,
                error: None,
            }],
            error: err.map(|e| e.into()),
        }
    }

    fn result_with(reqs: Vec<RequestResult>) -> RunResult {
        RunResult {
            collections: vec![CollectionResult {
                name: "C".into(),
                iterations: vec![Iteration {
                    index: 1,
                    requests: reqs,
                }],
                stats: vec![],
            }],
            totals: Totals::default(),
        }
    }

    #[test]
    fn ratio_clamps_and_handles_zero() {
        let mut s = RunState::default();
        assert_eq!(s.ratio(), 0.0);
        s.total = 4;
        s.done = 2;
        assert_eq!(s.ratio(), 0.5);
        s.done = 10;
        assert_eq!(s.ratio(), 1.0);
    }

    #[test]
    fn lines_mark_ok_vs_failed() {
        let s = RunState {
            running: false,
            total: 3,
            done: 3,
            result: Some(result_with(vec![
                rr("ok", Some(200), true, None),
                rr("bad-status", Some(500), true, None),
                rr("conn", None, true, Some("Connection refused")),
            ])),
        };
        let lines = s.request_lines();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], ("ok".into(), true, "200".into()));
        assert!(!lines[1].1); // 500 -> not ok
        assert_eq!(
            lines[2],
            ("conn".into(), false, "Connection refused".into())
        );
    }
}
