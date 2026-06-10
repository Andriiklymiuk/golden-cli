//! `golden upgrade` (alias `update`): self-upgrade via the original install
//! method — Homebrew, the cargo-dist shell installer, or (Windows) print the
//! PowerShell one-liner since a running .exe can't overwrite itself.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::exit::FATAL;

const REPO: &str = "Andriiklymiuk/golden-cli";
const TAP_FORMULA: &str = "andriiklymiuk/homebrew-tools/golden";
const INSTALLER_SH: &str =
    "https://github.com/Andriiklymiuk/golden-cli/releases/latest/download/golden-cli-installer.sh";
const INSTALLER_PS1: &str =
    "https://github.com/Andriiklymiuk/golden-cli/releases/latest/download/golden-cli-installer.ps1";

#[derive(Debug, PartialEq, Eq)]
pub enum InstallMethod {
    Homebrew,
    Script,
    Windows,
    Unknown,
}

pub fn execute() -> i32 {
    let current = env!("CARGO_PKG_VERSION");
    let latest = match latest_tag() {
        Ok(t) => t.trim_start_matches('v').to_string(),
        Err(e) => {
            eprintln!("golden: could not fetch the latest version: {e}");
            return FATAL;
        }
    };
    if !is_outdated(current, &latest) {
        println!("golden is already the latest version ({current}).");
        return 0;
    }
    println!("current: {current}");
    println!("latest:  {latest}");

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("golden: cannot locate own executable: {e}");
            return FATAL;
        }
    };
    let exe_dir = exe.parent().unwrap_or(&exe);

    match detect_method(
        exe_dir,
        brew_prefix().as_deref(),
        &script_dirs(),
        cfg!(windows),
    ) {
        InstallMethod::Homebrew => {
            println!("Homebrew install detected — upgrading via brew…");
            let _ = Command::new("brew").arg("update").status();
            // Homebrew 5 "ask mode" prompts before upgrading; the user already
            // confirmed by running `golden upgrade`, so skip the second prompt.
            match Command::new("brew")
                .env("HOMEBREW_NO_ASK", "1")
                .args(["upgrade", TAP_FORMULA])
                .status()
            {
                Ok(s) if s.success() => {
                    println!("Upgraded.");
                    0
                }
                _ => {
                    eprintln!("golden: brew upgrade failed");
                    FATAL
                }
            }
        }
        InstallMethod::Script => {
            println!("Re-running the install script for the latest release…");
            match run_installer() {
                Ok(()) => {
                    println!("Upgraded.");
                    0
                }
                Err(e) => {
                    eprintln!("golden: {e}");
                    FATAL
                }
            }
        }
        InstallMethod::Windows => {
            println!("Windows install — run this in a new PowerShell window to upgrade:");
            println!("  irm {INSTALLER_PS1} | iex");
            0
        }
        InstallMethod::Unknown => {
            println!(
                "Could not detect how golden was installed ({}).",
                exe.display()
            );
            println!("Upgrade with whichever you used:");
            println!("  brew upgrade {TAP_FORMULA}");
            println!("  curl --proto '=https' --tlsv1.2 -LsSf {INSTALLER_SH} | sh");
            if cfg!(windows) {
                println!("  irm {INSTALLER_PS1} | iex");
            }
            0
        }
    }
}

/// Decide the install method from the executable's dir, the Homebrew prefix (if
/// `brew` is on PATH), the known shell-installer dirs, and the OS.
pub fn detect_method(
    exe_dir: &Path,
    brew_prefix: Option<&Path>,
    script_dirs: &[PathBuf],
    is_windows: bool,
) -> InstallMethod {
    if is_windows {
        return InstallMethod::Windows;
    }
    if let Some(prefix) = brew_prefix {
        if exe_dir.starts_with(prefix) {
            return InstallMethod::Homebrew;
        }
    }
    if script_dirs.iter().any(|d| paths_equal(exe_dir, d)) {
        return InstallMethod::Script;
    }
    InstallMethod::Unknown
}

/// True when `current` differs from `latest` (both compared without a leading `v`).
pub fn is_outdated(current: &str, latest: &str) -> bool {
    current.trim_start_matches('v') != latest.trim_start_matches('v')
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let ca = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let cb = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

fn brew_prefix() -> Option<PathBuf> {
    let out = Command::new("brew").arg("--prefix").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?;
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| PathBuf::from(trimmed))
}

fn script_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/usr/local/bin")];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        dirs.push(home.join(".local/bin"));
        dirs.push(home.join(".cargo/bin"));
        dirs.push(home.join(".golden/bin"));
    }
    dirs
}

fn run_installer() -> Result<(), String> {
    let fetch = if which("curl") {
        format!("curl --proto '=https' --tlsv1.2 -LsSf {INSTALLER_SH}")
    } else if which("wget") {
        format!("wget -qO- {INSTALLER_SH}")
    } else {
        return Err("need curl or wget on PATH to fetch the installer".to_string());
    };
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{fetch} | sh"))
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("install script failed".to_string())
    }
}

fn which(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn latest_tag() -> Result<String, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| e.to_string())?;
    let body = client
        .get(&url)
        .header("User-Agent", "golden-cli")
        .send()
        .map_err(|e| e.to_string())?
        .text()
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    v.get("tag_name")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "no tag_name in the GitHub response".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_is_detected_first() {
        assert_eq!(
            detect_method(Path::new("/anything"), None, &[], true),
            InstallMethod::Windows
        );
    }

    #[test]
    fn homebrew_when_exe_under_brew_prefix() {
        assert_eq!(
            detect_method(
                Path::new("/opt/homebrew/bin"),
                Some(Path::new("/opt/homebrew")),
                &[],
                false
            ),
            InstallMethod::Homebrew
        );
    }

    #[test]
    fn script_when_exe_in_known_dir() {
        let dir = std::env::temp_dir();
        assert_eq!(
            detect_method(&dir, None, std::slice::from_ref(&dir), false),
            InstallMethod::Script
        );
    }

    #[test]
    fn unknown_when_nothing_matches() {
        assert_eq!(
            detect_method(
                Path::new("/some/random/place"),
                Some(Path::new("/opt/homebrew")),
                &[],
                false
            ),
            InstallMethod::Unknown
        );
    }

    #[test]
    fn outdated_ignores_v_prefix() {
        assert!(!is_outdated("2.0.8", "v2.0.8"));
        assert!(!is_outdated("v2.0.8", "2.0.8"));
        assert!(is_outdated("2.0.8", "v2.1.0"));
    }
}
