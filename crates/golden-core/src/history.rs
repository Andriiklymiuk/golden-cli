//! Persisted request/response history at <workspace>/.golden/history.jsonl.
//! Newline-delimited JSON, cap 100, sensitive values masked at rest by default.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Path to the history file for a workspace.
pub fn history_path(workspace: &Path) -> PathBuf {
    workspace.join(".golden").join("history.jsonl")
}

/// Path to the toggle sentinel; if it exists, recording is disabled.
pub fn disabled_flag_path(workspace: &Path) -> PathBuf {
    workspace.join(".golden").join("history.disabled")
}

/// Mask sensitive header values in-place (reuses curl::is_sensitive_header / mask_value).
pub fn mask_entry(mut entry: HistoryEntry) -> HistoryEntry {
    for (k, v) in entry.request_headers.iter_mut() {
        if crate::curl::is_sensitive_header(k) || crate::curl::mask_value(v) == "***" {
            *v = "***".to_string();
        }
    }
    entry
}

/// Whether recording is enabled (no disabled sentinel present).
pub fn is_enabled(workspace: &Path) -> bool {
    !disabled_flag_path(workspace).exists()
}

/// Enable/disable recording via the sentinel file.
pub fn set_enabled(workspace: &Path, enabled: bool) -> std::io::Result<()> {
    let flag = disabled_flag_path(workspace);
    if enabled {
        if flag.exists() {
            fs::remove_file(&flag)?;
        }
    } else {
        if let Some(dir) = flag.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&flag, b"")?;
    }
    Ok(())
}

/// Read all entries (oldest first). Missing file => empty. Bad lines are skipped.
pub fn read_all(workspace: &Path) -> std::io::Result<Vec<HistoryEntry>> {
    let path = history_path(workspace);
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    Ok(content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<HistoryEntry>(l).ok())
        .collect())
}

/// Append one entry. No-op if disabled. Masks at rest when `mask` is true.
/// Enforces MAX_HISTORY by rewriting the file with the newest tail when exceeded.
pub fn append(workspace: &Path, entry: HistoryEntry, mask: bool) -> std::io::Result<()> {
    if !is_enabled(workspace) {
        return Ok(());
    }
    let entry = if mask { mask_entry(entry) } else { entry };
    let path = history_path(workspace);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }

    let mut all = read_all(workspace)?;
    all.push(entry);
    if all.len() > MAX_HISTORY {
        let start = all.len() - MAX_HISTORY;
        all = all.split_off(start);
        // rewrite whole file (cap exceeded)
        let mut f = fs::File::create(&path)?;
        for e in &all {
            writeln!(f, "{}", serde_json::to_string(e).unwrap())?;
        }
    } else {
        // fast path: append a single line
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(f, "{}", serde_json::to_string(all.last().unwrap()).unwrap())?;
    }
    Ok(())
}

/// Remove all history (truncate file if present).
pub fn clear(workspace: &Path) -> std::io::Result<()> {
    let path = history_path(workspace);
    if path.exists() {
        fs::write(&path, b"")?;
    }
    Ok(())
}

/// One run/send record. Mirrors the extension's HistoryEntry shape, flattened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryEntry {
    pub timestamp: String, // RFC3339
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub request_headers: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default)]
    pub time_ms: u128,
}

impl HistoryEntry {
    /// Build a history entry from a request that has just been sent and its
    /// response. URL, header values and the raw body are resolved against `vars`
    /// (reusing `crate::subst::substitute`) so the stored entry is replayable
    /// without the original variable set. Disabled headers are dropped to match
    /// what was actually sent. `status` is `None` on transport failure.
    pub fn from_sent(
        request: &crate::model::Request,
        vars: &std::collections::HashMap<String, String>,
        status: Option<u16>,
        time_ms: u128,
    ) -> Self {
        let request_headers = request
            .header
            .iter()
            .filter(|h| !h.disabled)
            .map(|h| {
                (
                    crate::subst::substitute(&h.key, vars),
                    crate::subst::substitute(&h.value, vars),
                )
            })
            .collect();
        let request_body = request.body.as_ref().and_then(|b| match &b.raw {
            Some(serde_json::Value::String(s)) => Some(crate::subst::substitute(s, vars)),
            Some(other) => Some(crate::subst::substitute(&other.to_string(), vars)),
            None => None,
        });
        HistoryEntry {
            timestamp: now_rfc3339(),
            method: request.method.clone(),
            url: crate::subst::substitute(request.url.raw(), vars),
            request_headers,
            request_body,
            status,
            time_ms,
        }
    }
}

/// Current UTC time as an RFC3339 string (seconds precision), e.g.
/// `2026-06-09T12:34:56Z`. Computed from the Unix epoch without a date
/// dependency via the civil-from-days algorithm (Howard Hinnant).
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_from_unix(secs)
}

/// Format a Unix timestamp (seconds) as an RFC3339 UTC string.
fn rfc3339_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // civil_from_days: convert days since 1970-01-01 to (year, month, day).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, m, d, hour, min, sec
    )
}

/// The cap (extension parity).
pub const MAX_HISTORY: usize = 100;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn entry(url: &str) -> HistoryEntry {
        HistoryEntry {
            timestamp: "2026-06-05T00:00:00Z".into(),
            method: "GET".into(),
            url: url.into(),
            request_headers: vec![("Accept".into(), "application/json".into())],
            request_body: None,
            status: Some(200),
            time_ms: 12,
        }
    }

    #[test]
    fn entry_round_trips_as_one_json_line() {
        let e = entry("https://api.test/x");
        let line = serde_json::to_string(&e).unwrap();
        assert!(!line.contains('\n'));
        let back: HistoryEntry = serde_json::from_str(&line).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn paths_are_under_golden_dir() {
        let ws = PathBuf::from("/tmp/ws");
        assert_eq!(
            history_path(&ws),
            PathBuf::from("/tmp/ws/.golden/history.jsonl")
        );
        assert_eq!(
            disabled_flag_path(&ws),
            PathBuf::from("/tmp/ws/.golden/history.disabled")
        );
    }

    #[test]
    fn masking_replaces_sensitive_header_values() {
        let mut e = entry("https://api.test/x");
        e.request_headers
            .push(("Authorization".into(), "Bearer abc".into()));
        let masked = mask_entry(e);
        let auth = masked
            .request_headers
            .iter()
            .find(|(k, _)| k == "Authorization")
            .unwrap();
        assert_eq!(auth.1, "***");
        let accept = masked
            .request_headers
            .iter()
            .find(|(k, _)| k == "Accept")
            .unwrap();
        assert_eq!(accept.1, "application/json");
    }

    use tempfile::tempdir;

    #[test]
    fn append_skips_when_disabled() {
        let ws = tempdir().unwrap();
        set_enabled(ws.path(), false).unwrap();
        append(ws.path(), entry("https://api.test/x"), true).unwrap();
        assert!(read_all(ws.path()).unwrap().is_empty());
    }

    #[test]
    fn append_masks_then_persists_and_reads_back() {
        let ws = tempdir().unwrap();
        let mut e = entry("https://api.test/x");
        e.request_headers
            .push(("Authorization".into(), "Bearer abc".into()));
        append(ws.path(), e, true).unwrap();
        let all = read_all(ws.path()).unwrap();
        assert_eq!(all.len(), 1);
        let auth = all[0]
            .request_headers
            .iter()
            .find(|(k, _)| k == "Authorization")
            .unwrap();
        assert_eq!(auth.1, "***"); // masked at rest
    }

    #[test]
    fn append_enforces_cap_keeping_newest() {
        let ws = tempdir().unwrap();
        for i in 0..(MAX_HISTORY + 5) {
            append(ws.path(), entry(&format!("https://api.test/{i}")), false).unwrap();
        }
        let all = read_all(ws.path()).unwrap();
        assert_eq!(all.len(), MAX_HISTORY);
        // newest kept (last appended)
        assert_eq!(
            all.last().unwrap().url,
            format!("https://api.test/{}", MAX_HISTORY + 4)
        );
        // oldest dropped
        assert!(all.iter().all(|e| e.url != "https://api.test/0"));
    }

    #[test]
    fn clear_truncates() {
        let ws = tempdir().unwrap();
        append(ws.path(), entry("https://api.test/x"), false).unwrap();
        clear(ws.path()).unwrap();
        assert!(read_all(ws.path()).unwrap().is_empty());
    }

    #[test]
    fn rfc3339_from_unix_matches_known_instants() {
        assert_eq!(rfc3339_from_unix(0), "1970-01-01T00:00:00Z");
        // 2009-02-13T23:31:30Z (a commonly cited epoch milestone).
        assert_eq!(rfc3339_from_unix(1_234_567_890), "2009-02-13T23:31:30Z");
        // A leap-year date to exercise the civil-from-days month math.
        assert_eq!(rfc3339_from_unix(1_582_934_400), "2020-02-29T00:00:00Z");
    }

    #[test]
    fn now_rfc3339_is_well_formed() {
        let s = now_rfc3339();
        assert!(s.ends_with('Z'), "{s}");
        assert_eq!(s.len(), 20, "expected YYYY-MM-DDTHH:MM:SSZ, got {s}");
    }

    #[test]
    fn from_sent_resolves_vars_and_drops_disabled_headers() {
        use crate::model::{Body, Header, Request, Url};
        let mut vars = std::collections::HashMap::new();
        vars.insert("base".to_string(), "https://api.test".to_string());
        vars.insert("tok".to_string(), "abc123".to_string());
        let req = Request {
            method: "POST".into(),
            url: Url::Raw("{{base}}/login".into()),
            header: vec![
                Header {
                    key: "Authorization".into(),
                    value: "Bearer {{tok}}".into(),
                    disabled: false,
                    extra: serde_json::Map::new(),
                },
                Header {
                    key: "X-Skip".into(),
                    value: "no".into(),
                    disabled: true,
                    extra: serde_json::Map::new(),
                },
            ],
            body: Some(Body {
                mode: "raw".into(),
                raw: Some(serde_json::Value::String("{\"t\":\"{{tok}}\"}".into())),
                graphql: None,
                formdata: vec![],
            }),
        };
        let e = HistoryEntry::from_sent(&req, &vars, Some(200), 42);
        assert_eq!(e.method, "POST");
        assert_eq!(e.url, "https://api.test/login");
        assert_eq!(e.status, Some(200));
        assert_eq!(e.time_ms, 42);
        // Disabled header is dropped; the enabled one is resolved.
        assert_eq!(e.request_headers.len(), 1);
        assert_eq!(
            e.request_headers[0],
            ("Authorization".to_string(), "Bearer abc123".to_string())
        );
        assert_eq!(e.request_body.as_deref(), Some("{\"t\":\"abc123\"}"));
    }
}
