//! `golden init`: create collections/ and seed a sample collection.
//! Mirrors welcomeViewProvider.handleSampleImport: derives the filename from
//! info.name (lowercased, spaces->'-'), skips if it already exists.

use std::path::{Path, PathBuf};

use crate::model::Collection;
use crate::store::collection_filename;

/// Outcome of seeding.
#[derive(Debug, Clone, PartialEq)]
pub enum SeedOutcome {
    Created(PathBuf),
    AlreadyExists(PathBuf),
}

/// Create `collections_root` if missing and write `sample_json` pretty-printed
/// under the name derived from its info.name. Skips if the target already exists.
pub fn seed_sample(collections_root: &Path, sample_json: &str) -> Result<SeedOutcome, String> {
    let coll: Collection =
        serde_json::from_str(sample_json).map_err(|e| format!("invalid sample: {e}"))?;
    std::fs::create_dir_all(collections_root).map_err(|e| e.to_string())?;
    let filename = collection_filename(&coll.info.name);
    let target = collections_root.join(filename);
    if target.exists() {
        return Ok(SeedOutcome::AlreadyExists(target));
    }
    let pretty = serde_json::to_string_pretty(&coll).map_err(|e| e.to_string())?;
    std::fs::write(&target, pretty).map_err(|e| e.to_string())?;
    Ok(SeedOutcome::Created(target))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    const SAMPLE: &str = r#"{"info":{"name":"Sample Collection"},"item":[]}"#;

    #[test]
    fn creates_collections_dir_and_writes_sample() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("collections");
        let outcome = seed_sample(&root, SAMPLE).unwrap();
        let expected = root.join("sample-collection.json");
        assert_eq!(outcome, SeedOutcome::Created(expected.clone()));
        assert!(expected.exists());
        // pretty-printed (2-space indent)
        let written = std::fs::read_to_string(&expected).unwrap();
        assert!(
            written.contains("\n  \"info\""),
            "should be pretty: {written}"
        );
    }

    #[test]
    fn skips_when_already_present() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("collections");
        seed_sample(&root, SAMPLE).unwrap();
        let outcome = seed_sample(&root, SAMPLE).unwrap();
        assert!(matches!(outcome, SeedOutcome::AlreadyExists(_)));
    }
}
