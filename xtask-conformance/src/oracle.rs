//! Run a collection through newman (Node oracle) and extract the normalized run.
//! Requires `newman` available on PATH or via `npx newman` (the extension depends
//! on newman ^6.2.1).

use std::path::Path;
use std::process::Command;

use crate::normalize::{NormRequest, NormRun};

/// Invoke newman with the JSON reporter, parse its report, return the normalized run.
pub fn run_newman(collection_path: &Path) -> Result<NormRun, String> {
    let out_path = std::env::temp_dir().join(format!(
        "newman-{}.json",
        collection_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("run")
    ));
    // Prefer `npx --yes newman` so CI without a global install still works.
    let status = Command::new("npx")
        .args([
            "--yes",
            "newman",
            "run",
            collection_path.to_str().ok_or("bad path")?,
            "--reporters",
            "json",
            "--reporter-json-export",
            out_path.to_str().ok_or("bad out path")?,
        ])
        .status()
        .map_err(|e| format!("failed to spawn newman: {e}"))?;
    // newman exits non-zero when assertions fail; that is expected — we read the report regardless.
    let _ = status;

    let raw = std::fs::read_to_string(&out_path)
        .map_err(|e| format!("newman report missing ({}): {e}", out_path.display()))?;
    let report: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("bad newman json: {e}"))?;
    parse_report(&report)
}

/// Parse newman's JSON report into NormRun. newman v6 shape:
/// run.executions[] = { item.name, response.code, assertions[]{ assertion, error? } }.
pub fn parse_report(report: &serde_json::Value) -> Result<NormRun, String> {
    let execs = report
        .get("run")
        .and_then(|r| r.get("executions"))
        .and_then(|e| e.as_array())
        .ok_or("no run.executions in newman report")?;
    let mut requests = Vec::new();
    for ex in execs {
        let name = ex
            .get("item")
            .and_then(|i| i.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        let status = ex
            .get("response")
            .and_then(|r| r.get("code"))
            .and_then(|c| c.as_u64())
            .map(|c| c as u16);
        let mut assertions = Vec::new();
        if let Some(arr) = ex.get("assertions").and_then(|a| a.as_array()) {
            for a in arr {
                let aname = a
                    .get("assertion")
                    .and_then(|n| n.as_str())
                    .unwrap_or("")
                    .to_string();
                // newman marks a failure by presence of an `error` object.
                let passed = a.get("error").map(|e| e.is_null()).unwrap_or(true);
                assertions.push((aname, passed));
            }
        }
        requests.push(NormRequest {
            name,
            status,
            assertions,
        });
    }
    Ok(NormRun { requests })
}
