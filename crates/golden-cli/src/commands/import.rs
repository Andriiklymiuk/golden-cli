//! `golden import <source>` — normalize + merge into collections/.

use std::path::{Path, PathBuf};

use golden_core::import::{
    curl_to_collection, load_folder, merge, normalize, openapi_to_collection, MergeStrategy,
};
use golden_core::model::Collection;
use golden_core::store::collection_filename;

fn parse_strategy(s: &str) -> Result<MergeStrategy, String> {
    match s {
        "add" => Ok(MergeStrategy::Add),
        "replace" => Ok(MergeStrategy::Replace),
        "skip" => Ok(MergeStrategy::Skip),
        other => Err(format!("unknown strategy '{other}' (use add|replace|skip)")),
    }
}

/// Outcome of a single collection import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportOutcome {
    /// A collection was written to this path.
    Imported(PathBuf),
    /// The destination existed and the merge strategy was Skip — nothing written.
    Skipped(PathBuf),
}

/// Normalize `source` and merge each resolved collection into `root`, honoring the
/// merge `strategy` for existing files. This is the shared code path used by both
/// the headless `golden import` command and the TUI import prompt — neither
/// re-implements the normalize/merge/write logic.
pub fn import_into(
    root: &Path,
    source: &str,
    name: Option<&str>,
    strategy: MergeStrategy,
    from: &str,
) -> Result<Vec<ImportOutcome>, String> {
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let collections = resolve_inputs(source, name, from)?;
    let mut outcomes = Vec::new();
    for coll in collections {
        let target = root.join(collection_filename(&coll.info.name));
        let final_coll = if target.exists() {
            let existing_raw = std::fs::read_to_string(&target).map_err(|e| e.to_string())?;
            let existing: Collection = serde_json::from_str(&existing_raw)
                .map_err(|e| format!("parse {}: {e}", target.display()))?;
            match merge(existing, coll, strategy) {
                Some(c) => c,
                None => {
                    outcomes.push(ImportOutcome::Skipped(target));
                    continue;
                }
            }
        } else {
            coll
        };
        let pretty = serde_json::to_string_pretty(&final_coll).map_err(|e| e.to_string())?;
        std::fs::write(&target, pretty).map_err(|e| e.to_string())?;
        outcomes.push(ImportOutcome::Imported(target));
    }
    Ok(outcomes)
}

/// Execute `golden import`. Returns a process exit code.
pub fn execute(source: &str, name: Option<&str>, strategy: &str, from: &str) -> i32 {
    let strategy = match parse_strategy(strategy) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    let workspace = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("golden: cannot read current dir: {e}");
            return 2;
        }
    };
    let root = workspace.join("collections");

    match import_into(&root, source, name, strategy, from) {
        Ok(outcomes) => {
            for outcome in outcomes {
                match outcome {
                    ImportOutcome::Imported(p) => println!("imported: {}", p.display()),
                    ImportOutcome::Skipped(p) => {
                        println!("skipped (exists): {}", p.display())
                    }
                }
            }
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

fn resolve_inputs(source: &str, name: Option<&str>, from: &str) -> Result<Vec<Collection>, String> {
    let path = Path::new(source);
    let effective_from = if from == "auto" {
        if path.is_dir() {
            "folder"
        } else if source.trim_start().starts_with("curl ") {
            "curl"
        } else {
            "postman"
        }
    } else {
        from
    };
    match effective_from {
        "folder" => Ok(load_folder(path)?.into_iter().map(|(_, c)| c).collect()),
        "curl" => Ok(vec![curl_to_collection(
            source,
            name.unwrap_or("Imported"),
        )?]),
        "openapi" => {
            let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            Ok(vec![openapi_to_collection(&raw)?])
        }
        _ => {
            // postman / raw
            let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let derived = name.map(|s| s.to_string()).unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Imported")
                    .to_string()
            });
            Ok(vec![normalize(&raw, &derived)?])
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    #[test]
    fn import_postman_json_writes_collection_file() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("api.json");
        std::fs::write(&src, r#"{"info":{"name":"API"},"item":[]}"#).unwrap();
        let root = dir.path().join("collections");
        std::fs::create_dir_all(&root).unwrap();

        let collections = resolve_inputs(src.to_str().unwrap(), None, "auto").unwrap();
        assert_eq!(collections.len(), 1);
        assert_eq!(collections[0].info.name, "api");
    }

    #[test]
    fn import_folder_resolves_multiple_collections() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("one.json"),
            r#"{"info":{"name":"One"},"item":[]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("two.json"),
            r#"{"info":{"name":"Two"},"item":[]}"#,
        )
        .unwrap();

        let collections = resolve_inputs(dir.path().to_str().unwrap(), None, "folder").unwrap();
        assert_eq!(collections.len(), 2);
    }

    #[test]
    fn parse_strategy_roundtrips() {
        assert_eq!(parse_strategy("add").unwrap(), MergeStrategy::Add);
        assert_eq!(parse_strategy("replace").unwrap(), MergeStrategy::Replace);
        assert_eq!(parse_strategy("skip").unwrap(), MergeStrategy::Skip);
        assert!(parse_strategy("bad").is_err());
    }

    #[test]
    fn import_into_writes_collection_file_under_root() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("api.json");
        std::fs::write(&src, r#"{"info":{"name":"API"},"item":[]}"#).unwrap();
        let root = dir.path().join("collections");

        let outcomes = import_into(
            &root,
            src.to_str().unwrap(),
            None,
            MergeStrategy::Add,
            "auto",
        )
        .unwrap();
        assert_eq!(outcomes.len(), 1);
        let written = match &outcomes[0] {
            ImportOutcome::Imported(p) => p.clone(),
            other => panic!("expected Imported, got {other:?}"),
        };
        assert!(written.exists(), "the collection file should exist on disk");
        // The file lives under the supplied root.
        assert!(written.starts_with(&root));
    }

    #[test]
    fn import_into_skip_strategy_leaves_existing_file_untouched() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("collections");
        std::fs::create_dir_all(&root).unwrap();
        // Pre-existing collection file named to collide with the import.
        let existing = root.join(collection_filename("api"));
        std::fs::write(
            &existing,
            r#"{"info":{"name":"api"},"item":[{"name":"keep","request":{"method":"GET","url":"https://x/keep"}}]}"#,
        )
        .unwrap();

        let src = dir.path().join("api.json");
        std::fs::write(&src, r#"{"info":{"name":"API"},"item":[]}"#).unwrap();

        let outcomes = import_into(
            &root,
            src.to_str().unwrap(),
            None,
            MergeStrategy::Skip,
            "auto",
        )
        .unwrap();
        assert_eq!(outcomes, vec![ImportOutcome::Skipped(existing.clone())]);
        // The original item is preserved (skip wrote nothing).
        let raw = std::fs::read_to_string(&existing).unwrap();
        assert!(raw.contains("keep"), "skip must not overwrite the file");
    }
}
