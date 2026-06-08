//! Diff a golden NormRun against the newman oracle NormRun. Produces human-readable
//! divergence lines; empty == conformant.

use crate::normalize::NormRun;

#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    pub fixture: String,
    pub detail: String,
}

/// Compare oracle vs golden for one fixture. Returns one Divergence per mismatch.
pub fn diff(fixture: &str, oracle: &NormRun, golden: &NormRun) -> Vec<Divergence> {
    let mut out = Vec::new();
    if oracle.requests.len() != golden.requests.len() {
        out.push(Divergence {
            fixture: fixture.to_string(),
            detail: format!(
                "request count: oracle={} golden={}",
                oracle.requests.len(),
                golden.requests.len()
            ),
        });
    }
    for (i, o) in oracle.requests.iter().enumerate() {
        let Some(g) = golden.requests.get(i) else {
            continue;
        };
        if o.name != g.name {
            out.push(Divergence {
                fixture: fixture.into(),
                detail: format!("req#{i} name: oracle={:?} golden={:?}", o.name, g.name),
            });
        }
        if o.status != g.status {
            out.push(Divergence {
                fixture: fixture.into(),
                detail: format!(
                    "req '{}' status: oracle={:?} golden={:?}",
                    o.name, o.status, g.status
                ),
            });
        }
        if o.assertions != g.assertions {
            out.push(Divergence {
                fixture: fixture.into(),
                detail: format!(
                    "req '{}' assertions: oracle={:?} golden={:?}",
                    o.name, o.assertions, g.assertions
                ),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::normalize::{NormRequest, NormRun};

    fn run(name: &str, status: u16, asserts: Vec<(&str, bool)>) -> NormRun {
        NormRun {
            requests: vec![NormRequest {
                name: name.into(),
                status: Some(status),
                assertions: asserts
                    .into_iter()
                    .map(|(n, p)| (n.to_string(), p))
                    .collect(),
            }],
        }
    }

    #[test]
    fn identical_runs_have_no_divergence() {
        let a = run("r", 200, vec![("ok", true)]);
        let b = run("r", 200, vec![("ok", true)]);
        assert!(diff("f", &a, &b).is_empty());
    }

    #[test]
    fn status_and_assertion_mismatch_reported() {
        let oracle = run("r", 200, vec![("ok", true)]);
        let golden = run("r", 500, vec![("ok", false)]);
        let d = diff("f", &oracle, &golden);
        assert_eq!(d.len(), 2);
        assert!(d.iter().any(|x| x.detail.contains("status")));
        assert!(d.iter().any(|x| x.detail.contains("assertions")));
    }
}
