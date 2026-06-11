//! Shell completion for golden.
//!
//! Uses clap_complete's **dynamic** engine: `run()` (lib.rs) calls `CompleteEnv`, so the
//! completers below run at tab-time and suggest real collection names, request names and
//! env names discovered from the workspace — not just static flags. `golden completion
//! <shell>` prints the one line that wires this into a shell.

use std::ffi::OsStr;
use std::path::PathBuf;

use clap_complete::CompletionCandidate;
use clap_complete::Shell;
use golden_core::model::{Collection, Item};

use crate::discovery::{discover, env_paths};
use crate::load::load;

/// `golden completion <shell>` — print how to enable dynamic completion for `shell`.
pub fn execute(shell: Shell) -> i32 {
    let line = match shell {
        Shell::Bash => "source <(COMPLETE=bash golden)",
        Shell::Zsh => "source <(COMPLETE=zsh golden)",
        Shell::Fish => "COMPLETE=fish golden | source",
        Shell::PowerShell => "COMPLETE=powershell golden | Invoke-Expression",
        Shell::Elvish => "eval (COMPLETE=elvish golden | slurp)",
        _ => "source <(COMPLETE=bash golden)",
    };
    println!(
        "# golden dynamic completion for {shell} — add to your shell rc, then restart the shell:"
    );
    println!("{line}");
    0
}

/// Load every discovered collection in the cwd workspace, silently. Completion must never
/// print, panic, or be slow on error — any failure yields no candidates.
fn loaded() -> Vec<(PathBuf, Collection)> {
    let Ok(cwd) = std::env::current_dir() else {
        return Vec::new();
    };
    discover(&cwd, &[], env_paths())
        .into_iter()
        .filter_map(|path| load(&path).ok().map(|l| (path, l.collection)))
        .collect()
}

/// Case-insensitive prefix match against the word being typed (empty = match all).
fn matches(value: &str, current: &OsStr) -> bool {
    let cur = current.to_string_lossy();
    cur.is_empty() || value.to_lowercase().starts_with(&cur.to_lowercase())
}

fn collect_request_names(items: &[Item], out: &mut Vec<String>) {
    for item in items {
        match &item.item {
            Some(children) => collect_request_names(children, out),
            None if item.request.is_some() => out.push(item.name.clone()),
            None => {}
        }
    }
}

fn collect_folder_names(items: &[Item], out: &mut Vec<String>) {
    for item in items {
        if let Some(children) = &item.item {
            out.push(item.name.clone());
            collect_folder_names(children, out);
        }
    }
}

fn candidates(mut names: Vec<String>, current: &OsStr) -> Vec<CompletionCandidate> {
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|name| matches(name, current))
        .map(CompletionCandidate::new)
        .collect()
}

/// Complete the COLLECTION positional (send / curl): discovered `info.name`s.
pub fn complete_collections(current: &OsStr) -> Vec<CompletionCandidate> {
    candidates(
        loaded().into_iter().map(|(_, c)| c.info.name).collect(),
        current,
    )
}

/// Complete the REQUEST positional: every request name across discovered collections.
pub fn complete_requests(current: &OsStr) -> Vec<CompletionCandidate> {
    let mut names = Vec::new();
    for (_, collection) in loaded() {
        collect_request_names(&collection.item, &mut names);
    }
    candidates(names, current)
}

/// Complete `--filter GLOB` (run / list): request and folder names — the things the
/// glob matches against. Picking one runs exactly that request / that folder's requests.
pub fn complete_filter(current: &OsStr) -> Vec<CompletionCandidate> {
    let mut names = Vec::new();
    for (_, collection) in loaded() {
        collect_request_names(&collection.item, &mut names);
        collect_folder_names(&collection.item, &mut names);
    }
    candidates(names, current)
}

/// Complete `--env NAME`: env names from `.env.<name>` files beside the collections.
pub fn complete_envs(current: &OsStr) -> Vec<CompletionCandidate> {
    let mut dirs: Vec<PathBuf> = loaded()
        .into_iter()
        .filter_map(|(path, _)| path.parent().map(|dir| dir.to_path_buf()))
        .collect();
    dirs.sort();
    dirs.dedup();

    let mut names = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let file = entry.file_name();
            let file = file.to_string_lossy();
            if let Some(rest) = file.strip_prefix(".env.") {
                let name = rest.strip_suffix(".example").unwrap_or(rest);
                if !name.is_empty() {
                    names.push(name.to_string());
                }
            }
        }
    }
    candidates(names, current)
}

/// Complete collection PATHS (run / list positional): discovered file paths.
pub fn complete_paths(current: &OsStr) -> Vec<CompletionCandidate> {
    candidates(
        loaded()
            .into_iter()
            .map(|(path, _)| path.display().to_string())
            .collect(),
        current,
    )
}
