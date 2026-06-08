//! A flat, comparable shape extracted from either oracle (newman) or golden-core.

use serde::Serialize;

/// One request's comparable outcome.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NormRequest {
    pub name: String,
    pub status: Option<u16>,
    /// (assertion name, passed) pairs, in order.
    pub assertions: Vec<(String, bool)>,
}

/// A whole collection run, flattened (iteration 1 only — corpus uses 1 iteration).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NormRun {
    pub requests: Vec<NormRequest>,
}

/// Extract the normalized run from a golden-core RunResult (first collection, first iteration).
pub fn from_golden(result: &golden_core::result::RunResult) -> NormRun {
    let mut requests = Vec::new();
    if let Some(coll) = result.collections.first() {
        if let Some(iter) = coll.iterations.first() {
            for r in &iter.requests {
                requests.push(NormRequest {
                    name: r.name.clone(),
                    status: r.status,
                    assertions: r
                        .assertions
                        .iter()
                        .map(|a| (a.name.clone(), a.passed))
                        .collect(),
                });
            }
        }
    }
    NormRun { requests }
}

#[cfg(test)]
mod tests {
    use super::*;
    use golden_core::result::*;

    #[test]
    fn extracts_names_and_pass_flags() {
        let result = RunResult {
            collections: vec![CollectionResult {
                name: "c".into(),
                stats: vec![],
                iterations: vec![Iteration {
                    index: 1,
                    requests: vec![RequestResult {
                        name: "r".into(),
                        method: "GET".into(),
                        url: "u".into(),
                        status: Some(200),
                        time_ms: 1,
                        assertions: vec![
                            Assertion {
                                name: "ok".into(),
                                passed: true,
                                error: None,
                            },
                            Assertion {
                                name: "bad".into(),
                                passed: false,
                                error: Some("e".into()),
                            },
                        ],
                        error: None,
                    }],
                }],
            }],
            totals: Totals::default(),
        };
        let n = from_golden(&result);
        assert_eq!(n.requests.len(), 1);
        assert_eq!(n.requests[0].status, Some(200));
        assert_eq!(
            n.requests[0].assertions,
            vec![("ok".into(), true), ("bad".into(), false)]
        );
    }
}
