//! Response-to-file download helpers: Content-Disposition parsing, size cap,
//! and overwrite/unique-name resolution. Mirrors the extension's stream-download.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

fn disposition_quoted_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"filename[^;=\n]*=["']([^"'\n]+)["']"#).unwrap())
}

fn disposition_unquoted_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"filename[^;=\n]*=([^"';\n][^;\n]*)"#).unwrap())
}

/// Extract a filename from a Content-Disposition header value. Strips quotes.
pub fn filename_from_disposition(header: &str) -> Option<String> {
    // Try quoted form first (double or single quotes)
    if let Some(caps) = disposition_quoted_re().captures(header) {
        let raw = caps.get(1)?.as_str().trim();
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
    }
    // Fall back to unquoted form
    if let Some(caps) = disposition_unquoted_re().captures(header) {
        let raw = caps.get(1)?.as_str().trim();
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
    }
    None
}

/// What to do when the target file already exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverwritePolicy {
    Overwrite,
    Rename,
    Fail,
}

/// Generate a non-colliding path by appending " (n)" before the extension.
pub fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let ext = path.extension().and_then(|s| s.to_str());
    for n in 1.. {
        let name = match ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// Resolve the actual write target given an overwrite policy.
pub fn resolve_target(path: &Path, policy: OverwritePolicy) -> Result<PathBuf, String> {
    if !path.exists() {
        return Ok(path.to_path_buf());
    }
    match policy {
        OverwritePolicy::Overwrite => Ok(path.to_path_buf()),
        OverwritePolicy::Rename => Ok(unique_path(path)),
        OverwritePolicy::Fail => Err(format!("file already exists: {}", path.display())),
    }
}

/// True if `len` bytes are within an optional cap.
pub fn within_cap(len: u64, cap: Option<u64>) -> bool {
    match cap {
        Some(max) => len <= max,
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_filename_from_content_disposition() {
        assert_eq!(
            filename_from_disposition("attachment; filename=\"report.pdf\""),
            Some("report.pdf".to_string())
        );
        assert_eq!(
            filename_from_disposition("attachment; filename=data.csv"),
            Some("data.csv".to_string())
        );
        assert_eq!(
            filename_from_disposition("inline; filename='a b.txt'"),
            Some("a b.txt".to_string())
        );
        assert_eq!(filename_from_disposition("attachment"), None);
        assert_eq!(filename_from_disposition(""), None);
    }

    use tempfile::tempdir;

    #[test]
    fn unique_name_appends_counter_when_exists() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a.txt");
        std::fs::write(&p, b"x").unwrap();
        let u = unique_path(&p);
        assert_eq!(u, dir.path().join("a (1).txt"));
        std::fs::write(&u, b"x").unwrap();
        assert_eq!(unique_path(&p), dir.path().join("a (2).txt"));
    }

    #[test]
    fn resolve_overwrite_decides_target() {
        let dir = tempdir().unwrap();
        let p = dir.path().join("a.txt");
        // missing file: target is itself regardless of policy
        assert_eq!(resolve_target(&p, OverwritePolicy::Fail).unwrap(), p);
        std::fs::write(&p, b"x").unwrap();
        // exists + Overwrite: same path
        assert_eq!(resolve_target(&p, OverwritePolicy::Overwrite).unwrap(), p);
        // exists + Rename: unique
        assert_eq!(
            resolve_target(&p, OverwritePolicy::Rename).unwrap(),
            dir.path().join("a (1).txt")
        );
        // exists + Fail: Err
        assert!(resolve_target(&p, OverwritePolicy::Fail).is_err());
    }

    #[test]
    fn size_cap_check() {
        assert!(within_cap(100, Some(200)));
        assert!(within_cap(100, None));
        assert!(!within_cap(300, Some(200)));
    }
}
