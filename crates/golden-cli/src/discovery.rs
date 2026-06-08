//! Collection discovery: scan default roots, --collections overrides, and
//! GOLDEN_COLLECTIONS_PATHS for *.json files. Also expands explicit run/list args.

use std::path::{Path, PathBuf};

const DEFAULT_ROOTS: [&str; 3] = ["collections", ".retriever", ".golden"];

/// The env var holding extra collection roots (PATH-separated).
pub const ENV_PATHS: &str = "GOLDEN_COLLECTIONS_PATHS";

/// Read GOLDEN_COLLECTIONS_PATHS from the process environment, if set.
pub fn env_paths() -> Option<String> {
    std::env::var(ENV_PATHS).ok()
}

/// Discover all collection JSON files under the default roots plus any
/// `--collections` overrides and the `GOLDEN_COLLECTIONS_PATHS` env value.
/// `env_var` is passed explicitly so tests do not mutate the process env.
pub fn discover(workspace: &Path, overrides: &[String], env_var: Option<String>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = DEFAULT_ROOTS.iter().map(|r| workspace.join(r)).collect();
    for o in overrides {
        roots.push(workspace.join(o));
    }
    if let Some(env_val) = env_var {
        for part in env_val.split(path_separator()) {
            let part = part.trim();
            if !part.is_empty() {
                roots.push(workspace.join(part));
            }
        }
    }

    let mut out = Vec::new();
    for root in roots {
        if root.is_dir() {
            scan_dir(&root, &mut out);
        }
    }
    dedupe(out)
}

/// Expand explicit PATHS args: a file is returned as-is (resolved against
/// workspace if relative); a directory is scanned for top-level *.json.
pub fn expand_paths(workspace: &Path, paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let resolved = resolve_path(workspace, p);
        if resolved.is_dir() {
            scan_dir(&resolved, &mut out);
        } else if resolved.is_file() {
            out.push(resolved);
        }
    }
    dedupe(out)
}

fn resolve_path(workspace: &Path, p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_absolute() {
        pb
    } else {
        workspace.join(pb)
    }
}

/// Push every top-level `*.json` file in `dir` into `out` (non-recursive,
/// mirroring the extension's per-root scan).
fn scan_dir(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().map(|e| e == "json").unwrap_or(false) {
            out.push(path);
        }
    }
}

fn dedupe(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| {
        let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
        seen.insert(canon)
    });
    paths
}

fn path_separator() -> char {
    if cfg!(windows) {
        ';'
    } else {
        ':'
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn touch(dir: &std::path::Path, rel: &str, contents: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, contents).unwrap();
    }

    #[test]
    fn discovers_json_in_default_roots() {
        let ws = tempdir().unwrap();
        touch(ws.path(), "collections/a.json", "{}");
        touch(ws.path(), ".retriever/b.json", "{}");
        touch(ws.path(), ".golden/c.json", "{}");
        touch(ws.path(), "collections/note.txt", "ignore me");
        touch(ws.path(), "unrelated/d.json", "{}");

        let found = discover(ws.path(), &[], None);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"a.json".to_string()));
        assert!(names.contains(&"b.json".to_string()));
        assert!(names.contains(&"c.json".to_string()));
        assert!(!names.contains(&"note.txt".to_string()));
        assert!(!names.contains(&"d.json".to_string()));
    }

    #[test]
    fn collections_override_adds_extra_root() {
        let ws = tempdir().unwrap();
        touch(ws.path(), "custom/x.json", "{}");
        let found = discover(ws.path(), &["custom".to_string()], None);
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"x.json".to_string()));
    }

    #[test]
    fn env_var_paths_are_scanned() {
        let ws = tempdir().unwrap();
        touch(ws.path(), "envroot/y.json", "{}");
        let found = discover(ws.path(), &[], Some("envroot".to_string()));
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"y.json".to_string()));
    }

    #[test]
    fn explicit_file_path_is_returned_directly() {
        let ws = tempdir().unwrap();
        touch(ws.path(), "any/one.json", "{}");
        let file = ws.path().join("any/one.json");
        let found = expand_paths(ws.path(), &[file.to_string_lossy().into_owned()]);
        assert_eq!(found, vec![file]);
    }

    #[test]
    fn explicit_dir_path_is_scanned_for_json() {
        let ws = tempdir().unwrap();
        touch(ws.path(), "bundle/p.json", "{}");
        touch(ws.path(), "bundle/q.json", "{}");
        touch(ws.path(), "bundle/r.md", "skip");
        let dir = ws.path().join("bundle");
        let mut found = expand_paths(ws.path(), &[dir.to_string_lossy().into_owned()]);
        found.sort();
        let names: Vec<String> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["p.json".to_string(), "q.json".to_string()]);
    }

    #[test]
    fn results_are_deduped() {
        let ws = tempdir().unwrap();
        touch(ws.path(), "collections/dup.json", "{}");
        // override points at the same default root -> must dedupe
        let found = discover(ws.path(), &["collections".to_string()], None);
        let count = found
            .iter()
            .filter(|p| p.file_name().unwrap() == "dup.json")
            .count();
        assert_eq!(count, 1);
    }
}
