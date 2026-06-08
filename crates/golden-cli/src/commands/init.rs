//! `golden init` — seed sample collection + create collections/.

use golden_core::init::{seed_sample, SeedOutcome};

const SAMPLE: &str = include_str!("../../assets/sample-collection.json");

/// Execute `golden init`. Returns a process exit code.
pub fn execute() -> i32 {
    let workspace = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("golden: cannot read current dir: {e}");
            return 2;
        }
    };
    let root = workspace.join("collections");
    match seed_sample(&root, SAMPLE) {
        Ok(SeedOutcome::Created(p)) => {
            println!("created sample collection: {}", p.display());
            0
        }
        Ok(SeedOutcome::AlreadyExists(p)) => {
            println!("sample collection already exists: {}", p.display());
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    #[test]
    fn init_creates_sample_collection_in_collections_subdir() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("collections");
        let sample = r#"{"info":{"name":"Sample Collection"},"item":[]}"#;
        let outcome = golden_core::init::seed_sample(&root, sample).unwrap();
        assert!(matches!(
            outcome,
            golden_core::init::SeedOutcome::Created(_)
        ));
        assert!(root.join("sample-collection.json").exists());
    }
}
