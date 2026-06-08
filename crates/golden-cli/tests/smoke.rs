use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

/// Path to the repo's real sample collection (committed at <repo>/collections).
fn repo_root() -> PathBuf {
    // crate is at crates/golden-cli -> repo root is two levels up.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // golden-cli -> crates
    p.pop(); // crates -> repo root
    p
}

#[test]
fn list_runs_against_real_sample_collection() {
    let root = repo_root();
    // Sanity: the sample exists.
    assert!(
        root.join("collections/sample-collection.json").is_file(),
        "sample collection missing at repo root"
    );

    Command::cargo_bin("golden")
        .unwrap()
        .current_dir(&root)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fake APIs Collection"));
}

/// The `gr` alias binary must behave identically to `golden`.
#[test]
fn gr_alias_lists_same_as_golden() {
    let root = repo_root();
    Command::cargo_bin("gr")
        .unwrap()
        .current_dir(&root)
        .args(["list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fake APIs Collection"));
}
