//! Load a discovered collection file into a golden_core Collection and derive
//! the workspace + collections-root paths needed for env resolution.

use std::path::{Path, PathBuf};

use golden_core::model::Collection;

/// A parsed collection plus the path context env resolution needs.
#[derive(Debug)]
pub struct Loaded {
    pub collection: Collection,
    pub path: PathBuf,
    /// The directory the file lives in (the "collections root").
    pub collections_root: PathBuf,
    /// The parent of the collections root (the "workspace"); see discrepancy #2.
    pub workspace: PathBuf,
}

/// Read + parse a collection file. Errors are stringified with the file path so
/// the CLI can surface a useful message and exit with code 2.
pub fn load(path: &Path) -> Result<Loaded, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    let collection: Collection =
        serde_json::from_str(&raw).map_err(|e| format!("{}: {}", path.display(), e))?;
    let collections_root = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let workspace = collections_root
        .parent()
        .unwrap_or(&collections_root)
        .to_path_buf();
    Ok(Loaded {
        collection,
        path: path.to_path_buf(),
        collections_root,
        workspace,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn loads_collection_and_reports_root_and_workspace() {
        let ws = tempdir().unwrap();
        let dir = ws.path().join("collections");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("c.json");
        fs::write(&file, r#"{"info":{"name":"Sample"},"item":[]}"#).unwrap();

        let loaded = load(&file).unwrap();
        assert_eq!(loaded.collection.info.name, "Sample");
        assert_eq!(loaded.collections_root, dir);
        assert_eq!(loaded.workspace, ws.path());
    }

    #[test]
    fn load_error_includes_path_on_bad_json() {
        let ws = tempdir().unwrap();
        let file = ws.path().join("bad.json");
        fs::write(&file, "{ not json").unwrap();
        let err = load(&file).unwrap_err();
        assert!(err.contains("bad.json"), "got: {err}");
    }
}
