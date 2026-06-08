//! `golden import <source>` — normalize + merge into collections/.

use std::path::Path;

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
    if let Err(e) = std::fs::create_dir_all(&root) {
        eprintln!("error: {e}");
        return 2;
    }

    let collections: Vec<Collection> = match resolve_inputs(source, name, from) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };

    for coll in collections {
        let target = root.join(collection_filename(&coll.info.name));
        let final_coll = if target.exists() {
            let existing_raw = match std::fs::read_to_string(&target) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    return 2;
                }
            };
            let existing: Collection = match serde_json::from_str(&existing_raw) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("error: parse {}: {e}", target.display());
                    return 2;
                }
            };
            match merge(existing, coll, strategy) {
                Some(c) => c,
                None => {
                    println!("skipped (exists): {}", target.display());
                    continue;
                }
            }
        } else {
            coll
        };
        let pretty = match serde_json::to_string_pretty(&final_coll) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("error: {e}");
                return 2;
            }
        };
        if let Err(e) = std::fs::write(&target, pretty) {
            eprintln!("error: {e}");
            return 2;
        }
        println!("imported: {}", target.display());
    }
    0
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
}
