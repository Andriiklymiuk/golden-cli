//! newman-vs-golden conformance diff harness.
//!
//! Usage: cargo run -p xtask-conformance
//! Globs fixtures/*.json, runs each through newman (oracle) and golden-core,
//! diffs status + assertion names/pass-fail, prints divergences, exits non-zero
//! if any fixture diverges.

mod diff;
mod normalize;
mod oracle;

use std::path::{Path, PathBuf};

use golden_core::env::VarScopes;
use golden_core::http::HttpConfig;
use golden_core::model::Collection;
use golden_core::runner;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures")
}

fn list_fixtures() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(fixtures_dir()) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("json") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn run_golden(path: &Path) -> Result<normalize::NormRun, String> {
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let coll: Collection = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
    // Collection-run precedence: seed live scope from collection.variable only
    // (the corpus targets public endpoints; no .env needed).
    let mut scopes = VarScopes::default();
    for v in &coll.variable {
        if !v.key.is_empty() {
            scopes.set(v.key.clone(), v.value.clone());
        }
    }
    let result = runner::run(&coll, &scopes, 1, &HttpConfig::default());
    Ok(normalize::from_golden(&result))
}

fn main() {
    let fixtures = list_fixtures();
    if fixtures.is_empty() {
        eprintln!("no fixtures found in {}", fixtures_dir().display());
        std::process::exit(2);
    }

    let mut all_divergences = Vec::new();
    for path in &fixtures {
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let golden = match run_golden(path) {
            Ok(g) => g,
            Err(e) => {
                all_divergences.push(diff::Divergence {
                    fixture: name.clone(),
                    detail: format!("golden error: {e}"),
                });
                continue;
            }
        };
        let oracle = match oracle::run_newman(path) {
            Ok(o) => o,
            Err(e) => {
                all_divergences.push(diff::Divergence {
                    fixture: name.clone(),
                    detail: format!("oracle error: {e}"),
                });
                continue;
            }
        };
        let mut d = diff::diff(&name, &oracle, &golden);
        if d.is_empty() {
            println!("PASS  {name}  ({} requests)", golden.requests.len());
        } else {
            println!("DIVERGE  {name}");
            all_divergences.append(&mut d);
        }
    }

    println!("\n=== conformance summary ===");
    if all_divergences.is_empty() {
        println!("zero divergences across {} fixtures", fixtures.len());
    } else {
        for div in &all_divergences {
            println!("[{}] {}", div.fixture, div.detail);
        }
        println!("{} divergence(s)", all_divergences.len());
        std::process::exit(1);
    }
}
