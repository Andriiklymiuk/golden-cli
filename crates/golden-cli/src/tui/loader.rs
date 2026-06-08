//! Discover and parse collection JSON files using the existing `discovery` and
//! `load` crate modules. This is a thin TUI adapter — the TUI and the headless
//! CLI see exactly the same files.

use std::path::{Path, PathBuf};

use golden_core::model::Collection;

/// One loaded collection plus the file it came from (for hot-reload + display).
#[derive(Debug, Clone)]
pub struct LoadedCollection {
    pub path: PathBuf,
    pub collection: Collection,
}

/// Find selectable env profiles: the workspace `.env` plus any `.env.<name>`
/// files in `workspace`. Returns display names ("default" for plain `.env`).
pub fn discover_env_profiles(workspace: &Path) -> Vec<(String, PathBuf)> {
    let mut profiles = Vec::new();
    let default = workspace.join(".env");
    if default.exists() {
        profiles.push(("default".to_string(), default));
    }
    if let Ok(entries) = std::fs::read_dir(workspace) {
        let mut named: Vec<(String, PathBuf)> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter_map(|p| {
                let name = p.file_name()?.to_str()?.to_string();
                name.strip_prefix(".env.")
                    .map(|n| (n.to_string(), p.clone()))
            })
            .collect();
        named.sort();
        profiles.extend(named);
    }
    profiles
}

/// Discover every `*.json` file reachable from `collections_overrides` (or the
/// default roots — `collections/`, `.retriever/`, `.golden/`) under `workspace`,
/// then parse each as a Postman v2.1 Collection.
///
/// Files that fail to parse are skipped; a human-readable error is pushed into
/// the returned `errors` vec so the TUI status bar can surface them.
/// Results are sorted by file path for stable ordering.
pub fn load_collections(
    workspace: &Path,
    collections_overrides: &[String],
) -> (Vec<LoadedCollection>, Vec<String>) {
    // Reuse the existing discovery logic — same roots the headless CLI uses.
    let env_var = crate::discovery::env_paths();
    let mut paths = crate::discovery::discover(workspace, collections_overrides, env_var);
    paths.sort();

    let mut loaded = Vec::new();
    let mut errors = Vec::new();

    for path in paths {
        match crate::load::load(&path) {
            Ok(l) => loaded.push(LoadedCollection {
                path: l.path,
                collection: l.collection,
            }),
            Err(e) => errors.push(e),
        }
    }

    (loaded, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn discovers_default_and_named_env_profiles() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".env"), "A=1").unwrap();
        fs::write(dir.path().join(".env.staging"), "A=2").unwrap();
        fs::write(dir.path().join(".env.prod"), "A=3").unwrap();
        let profiles = discover_env_profiles(dir.path());
        let names: Vec<&str> = profiles.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["default", "prod", "staging"]);
    }

    #[test]
    fn loads_valid_skips_invalid_and_non_json() {
        let dir = tempdir().unwrap();
        // Put a valid collection in a `collections/` sub-dir so the default root scan finds it.
        fs::create_dir_all(dir.path().join("collections")).unwrap();
        fs::write(
            dir.path().join("collections/a.json"),
            r#"{"info":{"name":"A"},"item":[{"name":"r","request":{"method":"GET","url":"https://x/y"}}]}"#,
        )
        .unwrap();
        fs::write(dir.path().join("collections/bad.json"), "{not json").unwrap();
        fs::write(dir.path().join("collections/notes.txt"), "ignore me").unwrap();

        let (loaded, errors) = load_collections(dir.path(), &[]);
        assert_eq!(loaded.len(), 1, "one valid collection");
        assert_eq!(loaded[0].collection.info.name, "A");
        assert_eq!(errors.len(), 1, "one parse error");
        assert!(
            errors[0].contains("bad.json"),
            "error names the file: {}",
            errors[0]
        );
    }

    #[test]
    fn collections_override_is_respected() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("custom")).unwrap();
        fs::write(
            dir.path().join("custom/b.json"),
            r#"{"info":{"name":"B"},"item":[]}"#,
        )
        .unwrap();

        let (loaded, errors) = load_collections(dir.path(), &["custom".to_string()]);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].collection.info.name, "B");
        assert!(errors.is_empty());
    }
}
